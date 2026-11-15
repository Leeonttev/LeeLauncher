//! Модпак CurseForge (`manifest.json` + `overrides`) раскладывается через API в индекс `.mrpack`.
//! Файлы «только с сайта» сначала ищем на Modrinth по sha1, как Prism Launcher.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use serde::Deserialize;

use super::mrpack::{self, Env, IndexFile};
use super::{Requirements, check_rel_path, invalid};
use crate::content::manual::{ManualFile, ManualState};
use crate::content::{ContentSource, FileKey, ProjectType, SourceKind, curseforge};
use crate::downloads::FailedFile;
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::instances::Loader;
use crate::meta::LoaderKind;

pub const MANIFEST_NAME: &str = "manifest.json";
const MANIFEST_MAX_BYTES: u64 = 16 * 1024 * 1024;
const MANIFEST_MAX_FILES: usize = 20_000;
/// Откуда качаются файлы модпака CurseForge: CDN CurseForge и замена с Modrinth.
pub const ALLOWED_HOSTS: [&str; 3] = [
	curseforge::CDN_HOSTS[0],
	curseforge::CDN_HOSTS[1],
	"cdn.modrinth.com",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
	#[serde(default)]
	pub manifest_type: Option<String>,
	#[serde(default)]
	pub manifest_version: Option<u32>,
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub version: Option<String>,
	#[serde(default)]
	pub minecraft: Option<Minecraft>,
	#[serde(default)]
	pub files: Vec<ManifestFile>,
	#[serde(default)]
	pub overrides: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Minecraft {
	pub version: String,
	#[serde(default)]
	pub mod_loaders: Vec<ManifestLoader>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestLoader {
	pub id: String,
	#[serde(default)]
	pub primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestFile {
	#[serde(rename = "projectID")]
	pub project_id: u64,
	#[serde(rename = "fileID")]
	pub file_id: u64,
	#[serde(default = "yes")]
	pub required: bool,
}

fn yes() -> bool {
	true
}

impl Manifest {
	pub fn display_name(&self) -> String {
		self.name.clone().unwrap_or_default()
	}

	/// Папка `overrides` в архиве с `/` на конце.
	pub fn overrides_prefix(&self) -> String {
		format!("{}/", self.overrides.as_deref().unwrap_or("overrides"))
	}

	/// Проверка после разбора: формат, версия игры, папка `overrides`, число файлов.
	pub fn validate(&mut self) -> Result<(), AppError> {
		if self.manifest_type.as_deref() != Some("minecraftModpack") {
			return Err(invalid("manifestType"));
		}
		if self.manifest_version != Some(1) {
			return Err(invalid(format!(
				"manifestVersion {:?}",
				self.manifest_version
			)));
		}
		let mc = self
			.minecraft
			.as_ref()
			.ok_or_else(|| invalid("нет minecraft"))?;
		if !fs_util::is_safe_id(&mc.version) {
			return Err(invalid(format!("версия minecraft {:?}", mc.version)));
		}
		let overrides = self.overrides.clone().unwrap_or_else(|| "overrides".into());
		let overrides = check_rel_path(overrides.trim_end_matches('/'))
			.ok_or_else(|| invalid(format!("небезопасная папка overrides: {overrides}")))?;
		self.overrides = Some(overrides);
		if self.files.len() > MANIFEST_MAX_FILES {
			return Err(invalid(format!(
				"слишком много файлов: {}",
				self.files.len()
			)));
		}
		Ok(())
	}

	/// Версия игры и загрузчик (`primary`, иначе первый).
	pub fn requirements(&self) -> Result<Requirements, AppError> {
		let mc = self
			.minecraft
			.as_ref()
			.ok_or_else(|| invalid("нет minecraft"))?;
		let loader = mc
			.mod_loaders
			.iter()
			.find(|l| l.primary)
			.or_else(|| mc.mod_loaders.first());
		let loader = match loader {
			Some(l) => parse_loader(&mc.version, &l.id)
				.map_err(|name| invalid(format!("незнакомый загрузчик {name}")))?,
			None => Loader {
				kind: LoaderKind::Vanilla,
				version: None,
			},
		};
		Ok(Requirements {
			game_version: mc.version.clone(),
			loader,
		})
	}
}

/// `forge-47.2.0`, `neoforge-1.20.1-47.1.84`, `fabric-0.15.7-1.20.1`… Незнакомый — `Err` с именем.
pub fn parse_loader(game_version: &str, id: &str) -> Result<Loader, String> {
	let (name, version) = id.split_once('-').ok_or_else(|| id.to_owned())?;
	let kind = match name.to_ascii_lowercase().as_str() {
		"forge" => LoaderKind::Forge,
		"neoforge" => LoaderKind::Neoforge,
		"fabric" => LoaderKind::Fabric,
		"quilt" => LoaderKind::Quilt,
		_ => return Err(name.to_owned()),
	};
	let version = version
		.strip_suffix(game_version)
		.and_then(|v| v.strip_suffix('-'))
		.unwrap_or(version);
	let version = super::loader_version(game_version, version).map_err(|_| id.to_owned())?;
	Ok(Loader {
		kind,
		version: Some(version),
	})
}

fn read_manifest_from<R: std::io::Read + std::io::Seek>(
	zip: &mut zip::ZipArchive<R>,
) -> Option<Result<Manifest, AppError>> {
	let entry = zip.by_name(MANIFEST_NAME).ok()?;
	let mut text = Vec::new();
	if let Err(e) = entry.take(MANIFEST_MAX_BYTES).read_to_end(&mut text) {
		return Some(Err(invalid(e.to_string())));
	}
	Some(serde_json::from_slice(&text).map_err(|e| invalid(format!("{MANIFEST_NAME}: {e}"))))
}

/// Модпак CurseForge в архиве: `Some(имя)` (может быть пустым), не он — `None`.
pub fn detect<R: std::io::Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Option<String> {
	let manifest = read_manifest_from(zip)?.ok()?;
	(manifest.manifest_type.as_deref() == Some("minecraftModpack"))
		.then(|| manifest.name.unwrap_or_default())
}

/// Читает и проверяет `manifest.json` архива.
pub fn read_manifest(archive: &std::path::Path) -> Result<Manifest, AppError> {
	let file = std::fs::File::open(archive).map_err(|e| {
		AppError::coded(
			ErrorCode::FileNotFound,
			format!("{}: {e}", archive.display()),
		)
	})?;
	let mut zip = zip::ZipArchive::new(file).map_err(|e| invalid(format!("не zip: {e}")))?;
	let mut manifest =
		read_manifest_from(&mut zip).ok_or_else(|| invalid(format!("нет {MANIFEST_NAME}")))??;
	manifest.validate()?;
	Ok(manifest)
}

/// Внутренний признак «это модпак CurseForge» из [`mrpack::read_index`] (имя в `details`);
/// дальше архив читается как CurseForge ([`read_manifest`]).
pub fn unsupported(name: String) -> AppError {
	AppError::coded(ErrorCode::CurseforgeUnsupported, name)
}

/// Модпак CurseForge, а каталог выключен (нет ключа API): `source_disabled`, в `details` — имя
/// модпака из манифеста (интерфейс предложит поискать его на Modrinth).
pub fn disabled(name: String) -> AppError {
	AppError::coded(ErrorCode::SourceDisabled, name)
}

/// Манифест, разложенный в индекс установки.
#[derive(Debug, Clone, Default)]
pub struct Converted {
	pub index: Option<mrpack::Index>,
	/// Файлы «только с сайта» (и не нашлись на Modrinth).
	pub manual: Vec<ManualFile>,
	/// Пути необязательных среди `manual` (ставятся, только если отмечены).
	pub manual_optional: Vec<String>,
	/// Не получилось даже узнать файл (удалён с CurseForge, другой проект, незнакомый раздел).
	pub unresolved: Vec<FailedFile>,
	/// Путь → (название проекта, иконка): для списка необязательных.
	pub titles: HashMap<String, (String, Option<String>)>,
}

fn folder_of(kind: ProjectType, file_name: &str) -> Option<&'static str> {
	match kind {
		ProjectType::Resourcepack => Some("resourcepacks"),
		ProjectType::Shader => Some("shaderpacks"),
		ProjectType::Datapack => Some("datapacks"),
		ProjectType::Mod if file_name.to_ascii_lowercase().ends_with(".jar") => Some("mods"),
		_ => None,
	}
}

/// Раскладывает манифест: версии файлов и проекты — из CurseForge, замены для «только с сайта» —
/// с Modrinth по sha1 (`modrinth` — `None`, если искать не надо).
pub async fn convert(
	cf: &dyn ContentSource,
	modrinth: Option<&dyn ContentSource>,
	manifest: &Manifest,
) -> Result<Converted, AppError> {
	let requirements = manifest.requirements()?;
	let file_ids: Vec<String> = manifest
		.files
		.iter()
		.map(|f| f.file_id.to_string())
		.collect();
	let mut project_ids: Vec<String> = manifest
		.files
		.iter()
		.map(|f| f.project_id.to_string())
		.collect();
	project_ids.sort();
	project_ids.dedup();
	let versions: HashMap<String, crate::content::ContentVersion> = if file_ids.is_empty() {
		HashMap::new()
	} else {
		cf.versions(&file_ids)
			.await?
			.into_iter()
			.map(|v| (v.id.clone(), v))
			.collect()
	};
	let hits: HashMap<String, crate::content::ProjectHit> = if project_ids.is_empty() {
		HashMap::new()
	} else {
		cf.projects(&project_ids)
			.await?
			.into_iter()
			.map(|h| (h.id.clone(), h))
			.collect()
	};

	let mut out = Converted::default();
	let mut files: Vec<IndexFile> = Vec::new();
	let mut seen: HashSet<String> = HashSet::new();
	// Путь → (файл манифеста, версия, проект) для «только с сайта».
	let mut blocked: Vec<(String, &ManifestFile, &crate::content::ContentVersion)> = Vec::new();
	for mf in &manifest.files {
		let label = format!("curseforge:{}/{}", mf.project_id, mf.file_id);
		let fail = |out: &mut Converted, code: ErrorCode, why: String| {
			out.unresolved.push(FailedFile {
				path: label.clone(),
				error: AppError::coded(code, why),
			});
		};
		let Some(version) = versions.get(&mf.file_id.to_string()) else {
			fail(
				&mut out,
				ErrorCode::ContentNotFound,
				format!("файла {} нет", mf.file_id),
			);
			continue;
		};
		if version.project_id != mf.project_id.to_string() {
			fail(
				&mut out,
				ErrorCode::ModpackInvalid,
				format!("файл {} из другого проекта", mf.file_id),
			);
			continue;
		}
		let Some(file) = version.primary_file() else {
			fail(
				&mut out,
				ErrorCode::ContentNotFound,
				format!("у {} нет файла", mf.file_id),
			);
			continue;
		};
		let hit = hits.get(&version.project_id);
		let kind = hit.map_or(ProjectType::Mod, |h| h.project_type);
		let Some(folder) = folder_of(kind, &file.file_name) else {
			fail(
				&mut out,
				ErrorCode::ContentIncompatible,
				format!("{}: тип {}", file.file_name, kind.as_str()),
			);
			continue;
		};
		let Some(path) = check_rel_path(&format!("{folder}/{}", file.file_name)) else {
			fail(
				&mut out,
				ErrorCode::ModpackInvalid,
				format!("небезопасное имя: {}", file.file_name),
			);
			continue;
		};
		if !seen.insert(path.to_lowercase()) {
			fail(
				&mut out,
				ErrorCode::ModpackInvalid,
				format!("путь повторяется: {path}"),
			);
			continue;
		}
		if let Some(h) = hit {
			out.titles
				.insert(path.clone(), (h.title.clone(), h.icon_url.clone()));
		}
		if version.site_only {
			blocked.push((path, mf, version));
			continue;
		}
		if !mrpack::is_url_in(&file.url, &curseforge::CDN_HOSTS) {
			fail(
				&mut out,
				ErrorCode::DownloadFailed,
				format!("{path}: адрес не с CDN CurseForge"),
			);
			continue;
		}
		files.push(index_file(
			&path,
			&file.sha1,
			file.size,
			&file.url,
			mf.required,
		));
	}

	// «Только с сайта» — сначала тот же файл на Modrinth (по sha1).
	let mut replaced: HashMap<String, String> = HashMap::new();
	if let Some(modrinth) = modrinth
		&& !blocked.is_empty()
	{
		let keys: Vec<FileKey> = blocked
			.iter()
			.filter_map(|(_, _, v)| v.primary_file())
			.map(|f| FileKey::sha1(f.sha1.clone()))
			.collect();
		match modrinth.identify(&keys).await {
			Ok(found) => {
				for (sha1, v) in found {
					if let Some(f) = v.files.iter().find(|f| f.sha1.eq_ignore_ascii_case(&sha1))
						&& mrpack::is_url_in(&f.url, &["cdn.modrinth.com"])
					{
						replaced.insert(sha1, f.url.clone());
					}
				}
			}
			Err(err) => log::info!(
				"модпак CurseForge: замены на Modrinth не найдены: {:?}",
				err.code
			),
		}
	}
	for (path, mf, version) in blocked {
		let Some(file) = version.primary_file() else {
			continue;
		};
		if let Some(url) = replaced.get(&file.sha1) {
			files.push(index_file(&path, &file.sha1, file.size, url, mf.required));
			continue;
		}
		let hit = hits.get(&version.project_id);
		let (folder, name) = path.split_once('/').unwrap_or(("mods", &path));
		let kind = match folder {
			"resourcepacks" => ProjectType::Resourcepack,
			"shaderpacks" => ProjectType::Shader,
			"datapacks" => ProjectType::Datapack,
			_ => ProjectType::Mod,
		};
		if !mf.required {
			out.manual_optional.push(path.clone());
		}
		out.manual.push(ManualFile {
			source: SourceKind::Curseforge,
			project_id: version.project_id.clone(),
			file_id: version.id.clone(),
			title: hit.map_or_else(|| name.to_owned(), |h| h.title.clone()),
			icon_url: hit.and_then(|h| h.icon_url.clone()),
			file: name.to_owned(),
			size: file.size,
			project_type: kind,
			world: None,
			page_url: version.page_url.clone().unwrap_or_else(|| {
				crate::content::project_url(SourceKind::Curseforge, &version.project_id)
			}),
			replaces: None,
			sha1: file.sha1.clone(),
			version_number: version.version_number.clone(),
			author: hit.map(|h| h.author.clone()).filter(|a| !a.is_empty()),
			dependency: false,
			state: ManualState::Waiting,
			got: None,
			error: None,
		});
	}

	let mut dependencies = std::collections::BTreeMap::new();
	dependencies.insert("minecraft".to_owned(), requirements.game_version.clone());
	if let Some(version) = &requirements.loader.version {
		let key = match requirements.loader.kind {
			LoaderKind::Fabric => Some("fabric-loader"),
			LoaderKind::Quilt => Some("quilt-loader"),
			LoaderKind::Forge => Some("forge"),
			LoaderKind::Neoforge => Some("neoforge"),
			LoaderKind::Vanilla => None,
		};
		if let Some(key) = key {
			dependencies.insert(key.to_owned(), version.clone());
		}
	}
	out.index = Some(mrpack::Index {
		format_version: 1,
		game: "minecraft".into(),
		version_id: manifest.version.clone().unwrap_or_default(),
		name: manifest.display_name(),
		summary: None,
		files,
		dependencies,
	});
	Ok(out)
}

fn index_file(path: &str, sha1: &str, size: u64, url: &str, required: bool) -> IndexFile {
	IndexFile {
		path: path.to_owned(),
		hashes: [("sha1".to_owned(), sha1.to_ascii_lowercase())].into(),
		env: (!required).then(|| Env {
			client: Some("optional".into()),
			server: None,
		}),
		downloads: vec![url.to_owned()],
		file_size: size,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::content::tests::version as cv;
	use crate::content::{
		BoxFuture, CategoryTag, ContentFile, ContentVersion, Project, ProjectHit, SearchPage,
		SearchQuery, Side, VersionChannel, VersionFilter,
	};
	use std::io::Write;

	pub fn write_pack(
		path: &std::path::Path,
		manifest: &serde_json::Value,
		extra: &[(&str, &[u8])],
	) {
		let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
		zip.start_file(MANIFEST_NAME, zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.write_all(manifest.to_string().as_bytes()).unwrap();
		for (name, data) in extra {
			zip.start_file(*name, zip::write::SimpleFileOptions::default())
				.unwrap();
			zip.write_all(data).unwrap();
		}
		zip.finish().unwrap();
	}

	fn manifest(files: serde_json::Value) -> serde_json::Value {
		serde_json::json!({
			"minecraft": { "version": "1.20.1", "modLoaders": [{ "id": "forge-47.2.0", "primary": true }] },
			"manifestType": "minecraftModpack", "manifestVersion": 1, "name": "All the Mods 9",
			"version": "0.2.44", "files": files, "overrides": "overrides"
		})
	}

	#[test]
	fn detects_manifest_and_loaders() {
		let dir = tempfile::tempdir().unwrap();
		let pack = dir.path().join("cf.zip");
		write_pack(
			&pack,
			&manifest(
				serde_json::json!([ { "projectID": 238222, "fileID": 4712345, "required": true } ]),
			),
			&[],
		);
		let mut zip = zip::ZipArchive::new(std::fs::File::open(&pack).unwrap()).unwrap();
		assert_eq!(detect(&mut zip).as_deref(), Some("All the Mods 9"));
		let m = read_manifest(&pack).unwrap();
		assert_eq!(m.overrides_prefix(), "overrides/");
		let req = m.requirements().unwrap();
		assert_eq!(
			(
				req.game_version.as_str(),
				req.loader.kind,
				req.loader.version.as_deref()
			),
			("1.20.1", LoaderKind::Forge, Some("47.2.0"))
		);

		let l = parse_loader("1.20.1", "neoforge-1.20.1-47.1.84").unwrap();
		assert_eq!(
			(l.kind, l.version.as_deref()),
			(LoaderKind::Neoforge, Some("47.1.84"))
		);
		let l = parse_loader("1.20.1", "fabric-0.15.7-1.20.1").unwrap();
		assert_eq!(
			(l.kind, l.version.as_deref()),
			(LoaderKind::Fabric, Some("0.15.7"))
		);
		assert_eq!(
			parse_loader("1.12.2", "liteloader-1").unwrap_err(),
			"liteloader"
		);
		assert!(parse_loader("1.20.1", "forge").is_err());
	}

	#[test]
	fn rejects_bad_manifests() {
		let dir = tempfile::tempdir().unwrap();
		let bad = [
			serde_json::json!({ "manifestType": "x", "manifestVersion": 1, "minecraft": { "version": "1.20.1" } }),
			serde_json::json!({ "manifestType": "minecraftModpack", "manifestVersion": 2, "minecraft": { "version": "1.20.1" } }),
			serde_json::json!({ "manifestType": "minecraftModpack", "manifestVersion": 1 }),
			serde_json::json!({ "manifestType": "minecraftModpack", "manifestVersion": 1, "minecraft": { "version": "../x" } }),
			serde_json::json!({ "manifestType": "minecraftModpack", "manifestVersion": 1, "minecraft": { "version": "1.20.1" }, "overrides": "../up" }),
			serde_json::json!({ "manifestType": "minecraftModpack", "manifestVersion": 1, "minecraft": { "version": "1.20.1" }, "overrides": "C:/x" }),
		];
		for (i, m) in bad.iter().enumerate() {
			let pack = dir.path().join(format!("{i}.zip"));
			write_pack(&pack, m, &[]);
			assert_eq!(
				read_manifest(&pack).unwrap_err().code,
				ErrorCode::ModpackInvalid,
				"{i}"
			);
		}
		let m = serde_json::json!({ "manifestType": "minecraftModpack", "manifestVersion": 1,
			"minecraft": { "version": "1.20.1", "modLoaders": [{ "id": "rift-1" }] } });
		let pack = dir.path().join("rift.zip");
		write_pack(&pack, &m, &[]);
		assert!(read_manifest(&pack).unwrap().requirements().is_err());
	}

	/// Каталог в памяти: версии по id, проекты по id, опознание по sha1.
	#[derive(Default)]
	struct Fake {
		versions: Vec<ContentVersion>,
		hits: Vec<ProjectHit>,
		known: HashMap<String, ContentVersion>,
	}

	fn no<'a, T: Send + 'a>() -> BoxFuture<'a, T> {
		Box::pin(async { Err(AppError::unknown("нет")) })
	}

	impl ContentSource for Fake {
		fn search<'a>(&'a self, _: &'a SearchQuery) -> BoxFuture<'a, SearchPage> {
			no()
		}
		fn project<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Project> {
			no()
		}
		fn projects<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ProjectHit>> {
			Box::pin(async move {
				Ok(self
					.hits
					.iter()
					.filter(|h| ids.contains(&h.id))
					.cloned()
					.collect())
			})
		}
		fn project_versions<'a>(
			&'a self,
			_: &'a str,
			_: &'a VersionFilter,
		) -> BoxFuture<'a, Vec<ContentVersion>> {
			no()
		}
		fn version<'a>(&'a self, _: &'a str) -> BoxFuture<'a, ContentVersion> {
			no()
		}
		fn versions<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ContentVersion>> {
			Box::pin(async move {
				Ok(self
					.versions
					.iter()
					.filter(|v| ids.contains(&v.id))
					.cloned()
					.collect())
			})
		}
		fn categories(&self) -> BoxFuture<'_, (Vec<CategoryTag>, bool)> {
			no()
		}
		fn identify<'a>(
			&'a self,
			files: &'a [FileKey],
		) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
			Box::pin(async move {
				Ok(files
					.iter()
					.filter_map(|f| Some((f.sha1.clone(), self.known.get(&f.sha1)?.clone())))
					.collect())
			})
		}
		fn latest<'a>(
			&'a self,
			_: &'a [FileKey],
			_: &'a VersionFilter,
		) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
			no()
		}
	}

	fn sha(n: u8) -> String {
		format!("{n:02x}").repeat(20)
	}

	fn cf_version(project: u64, file: u64, name: &str, url: &str, n: u8) -> ContentVersion {
		let mut v = cv(
			&file.to_string(),
			VersionChannel::Release,
			"1.20.1",
			"forge",
		);
		v.source = SourceKind::Curseforge;
		v.project_id = project.to_string();
		v.site_only = url.is_empty();
		v.page_url = Some(format!(
			"https://www.curseforge.com/minecraft/mc-mods/p{project}/download/{file}"
		));
		v.files = vec![ContentFile {
			url: url.into(),
			file_name: name.into(),
			sha1: sha(n),
			size: 10 + u64::from(n),
			primary: true,
		}];
		v
	}

	fn hit(id: u64, kind: ProjectType) -> ProjectHit {
		ProjectHit {
			source: SourceKind::Curseforge,
			id: id.to_string(),
			slug: format!("p{id}"),
			project_type: kind,
			title: format!("Project {id}"),
			author: "me".into(),
			description: String::new(),
			icon_url: None,
			downloads: 0,
			follows: None,
			site_only: false,
			url: String::new(),
			categories: vec![],
			updated_at: 0,
			game_versions: vec![],
			loaders: vec![],
			client_side: Side::Unknown,
			server_side: Side::Unknown,
			install: None,
			also_on: Vec::new(),
			created_at: 0,
		}
	}

	#[tokio::test]
	async fn converts_manifest_to_index() {
		let cdn = "https://edge.forgecdn.net/files/1/1/x";
		let cf = Fake {
			versions: vec![
				cf_version(1, 11, "jei.jar", cdn, 1),
				cf_version(2, 21, "blocked.jar", "", 2),
				cf_version(3, 31, "onmodrinth.jar", "", 3),
				cf_version(4, 41, "pack.zip", cdn, 4),
				cf_version(5, 51, "evil.jar", "https://evil.example.com/x", 5),
				cf_version(6, 61, "opt.jar", "", 6),
				cf_version(9, 71, "wrongproject.jar", cdn, 7),
				cf_version(8, 81, "world.zip", cdn, 8),
			],
			hits: vec![
				hit(1, ProjectType::Mod),
				hit(2, ProjectType::Mod),
				hit(3, ProjectType::Mod),
				hit(4, ProjectType::Resourcepack),
				hit(5, ProjectType::Mod),
				hit(6, ProjectType::Mod),
				hit(8, ProjectType::World),
			],
			known: HashMap::new(),
		};
		let mut on_modrinth = cv("m", VersionChannel::Release, "1.20.1", "forge");
		on_modrinth.files = vec![ContentFile {
			url: "https://cdn.modrinth.com/data/x/onmodrinth.jar".into(),
			file_name: "onmodrinth.jar".into(),
			sha1: sha(3),
			size: 13,
			primary: true,
		}];
		let modrinth = Fake {
			known: HashMap::from([(sha(3), on_modrinth)]),
			..Fake::default()
		};
		let m: Manifest = serde_json::from_value(manifest(serde_json::json!([
			{ "projectID": 1, "fileID": 11 },
			{ "projectID": 2, "fileID": 21, "required": true },
			{ "projectID": 3, "fileID": 31 },
			{ "projectID": 4, "fileID": 41, "required": false },
			{ "projectID": 5, "fileID": 51 },
			{ "projectID": 6, "fileID": 61, "required": false },
			{ "projectID": 7, "fileID": 71 },
			{ "projectID": 8, "fileID": 81 },
			{ "projectID": 10, "fileID": 999 }
		])))
		.unwrap();
		let out = convert(&cf, Some(&modrinth), &m).await.unwrap();
		let index = out.index.unwrap();
		let paths: Vec<&str> = index.files.iter().map(|f| f.path.as_str()).collect();
		assert_eq!(
			paths,
			vec![
				"mods/jei.jar",
				"resourcepacks/pack.zip",
				"mods/onmodrinth.jar"
			]
		);
		assert_eq!(index.files[1].client_side(), Side::Optional);
		assert_eq!(
			index.files[2].allowed_url_in(&ALLOWED_HOSTS),
			Some("https://cdn.modrinth.com/data/x/onmodrinth.jar")
		);
		assert_eq!(index.dependencies["forge"], "47.2.0");
		assert_eq!(index.dependencies["minecraft"], "1.20.1");
		let manual: Vec<&str> = out.manual.iter().map(|f| f.file.as_str()).collect();
		assert_eq!(manual, vec!["blocked.jar", "opt.jar"]);
		assert_eq!(out.manual_optional, vec!["mods/opt.jar"]);
		assert_eq!(
			out.manual[0].page_url,
			"https://www.curseforge.com/minecraft/mc-mods/p2/download/21"
		);
		let failed: Vec<&str> = out.unresolved.iter().map(|f| f.path.as_str()).collect();
		assert_eq!(
			failed,
			vec![
				"curseforge:5/51",
				"curseforge:7/71",
				"curseforge:8/81",
				"curseforge:10/999"
			]
		);
		assert_eq!(out.titles["mods/jei.jar"].0, "Project 1");
	}
}
