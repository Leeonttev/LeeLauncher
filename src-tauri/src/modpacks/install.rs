//! Установка и обновление модпаков, перенос из других лаунчеров. Одна задача на всё:
//! игра (до 60 %), файлы модпака (до 95 %), `overrides` или копирование папок.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::launchers::{Candidate, Category};
use super::mrpack::{self, Index, OverridePolicy};
use super::{ModpackLink, PackOrigin, PackRequest, PackState, PackTask, Requirements, invalid};
use crate::content::manual::ManualFile;
use crate::content::{self, ProjectType, Side, SourceKind, VersionFilter};
use crate::download::{Job, Progress, Verify};
use crate::downloads::{FailedFile, Target, Warnings};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::instances::install::{
	InstallEvents, InstallFinished, InstallMode, InstallResult, InstallStage, Reporter, Tracked,
	install_files,
};
use crate::instances::{InstallState, Installed, Instance, Loader};
use crate::launcher::Launcher;
use crate::meta::LoaderKind;

/// Опознание в каталоге — не дольше этого (без сети не ждём).
const IDENTIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
/// Папки контента: файлы модпака в них при обновлении удаляются, даже если пользователь их менял.
const CONTENT_FOLDERS: [&str; 4] = ["mods/", "resourcepacks/", "shaderpacks/", "datapacks/"];
/// Файлы модпака качаются одновременно не больше этого.
const PACK_CONCURRENCY: usize = 8;
/// Доли общего процента: игра и загрузчик, моды модпака (дальше `overrides`).
const GAME_END: f32 = 60.0;
const FILES_END: f32 = 95.0;

/// Формат архива модпака.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackFormat {
	/// `.mrpack`: `overrides/`, поверх — `client-overrides/`.
	Mrpack,
	/// CurseForge `.zip`: папка из поля `overrides` манифеста.
	Curseforge { overrides: String },
}

/// Разобранный модпак; архив лежит в `meta/cache/modpacks/<sha1>.mrpack`.
#[derive(Debug, Clone)]
pub struct Prepared {
	/// sha1 архива: `token` в командах.
	pub token: String,
	pub archive: PathBuf,
	pub index: Index,
	pub link: ModpackLink,
	pub requirements: Requirements,
	pub format: PackFormat,
	/// CurseForge: файлы «только с сайта», не найденные на Modrinth.
	pub manual: Vec<crate::content::manual::ManualFile>,
	/// Пути необязательных среди `manual`.
	pub manual_optional: Vec<String>,
	/// CurseForge: файлы, которые не удалось даже узнать (пойдут в `warnings.failed`).
	pub unresolved: Vec<FailedFile>,
	/// Путь → (название, иконка) из каталога (CurseForge).
	pub titles: HashMap<String, (String, Option<String>)>,
}

impl Prepared {
	/// Папки архива, которые распаковываются в папку игры (по порядку, следующая поверх).
	pub fn override_prefixes(&self) -> Vec<String> {
		match &self.format {
			PackFormat::Mrpack => vec!["overrides/".into(), "client-overrides/".into()],
			PackFormat::Curseforge { overrides } => vec![format!("{overrides}/")],
		}
	}

	/// Откуда можно качать файлы модпака.
	pub fn allowed_hosts(&self) -> &'static [&'static str] {
		match self.format {
			PackFormat::Mrpack => &mrpack::ALLOWED_HOSTS,
			PackFormat::Curseforge { .. } => &super::curseforge::ALLOWED_HOSTS,
		}
	}

	/// Необязательные по умолчанию: все, кроме невыбранных в прошлый раз (и ручные необязательные).
	pub fn default_optional(&self, old: &PackState) -> Vec<String> {
		let mut list = default_optional(&self.index, old);
		list.extend(
			self.manual_optional
				.iter()
				.filter(|p| !old.excluded.contains(p))
				.cloned(),
		);
		list
	}

	/// Ручные файлы, которые ставятся при таком выборе необязательных.
	pub fn manual_for(&self, optional: &[String]) -> Vec<crate::content::manual::ManualFile> {
		self.manual
			.iter()
			.filter(|f| {
				let path = f.dest_key().unwrap_or_default();
				!self.manual_optional.contains(&path) || optional.contains(&path)
			})
			.cloned()
			.collect()
	}
}

/// Архив прочитан: индекс `.mrpack` или манифест CurseForge (его ещё надо разложить через API).
enum ReadPack {
	Mrpack(Index),
	Curseforge(super::curseforge::Manifest),
}

fn read_pack_sync(archive: &Path) -> Result<ReadPack, AppError> {
	match mrpack::read_index(archive) {
		Ok(index) => Ok(ReadPack::Mrpack(index)),
		Err(err) if err.code == ErrorCode::CurseforgeUnsupported => Ok(ReadPack::Curseforge(
			super::curseforge::read_manifest(archive)?,
		)),
		Err(err) => Err(err),
	}
}

/// Разобранное содержимое архива.
struct PackContent {
	index: Index,
	requirements: Requirements,
	format: PackFormat,
	converted: super::curseforge::Converted,
}

/// Индекс архива; модпак CurseForge раскладывается через API.
async fn pack_content(
	l: &Launcher,
	read: ReadPack,
	cancel: &CancellationToken,
) -> Result<PackContent, AppError> {
	match read {
		ReadPack::Mrpack(index) => Ok(PackContent {
			requirements: index.requirements()?,
			index,
			format: PackFormat::Mrpack,
			converted: Default::default(),
		}),
		ReadPack::Curseforge(manifest) => {
			if !l.sources.enabled(SourceKind::Curseforge) {
				return Err(super::curseforge::disabled(manifest.display_name()));
			}
			let cf = l.sources.get(SourceKind::Curseforge);
			let modrinth = l.sources.get(SourceKind::Modrinth);
			let mut converted = cancellable(
				cancel,
				super::curseforge::convert(cf.as_ref(), Some(modrinth.as_ref()), &manifest),
			)
			.await?;
			let mut index = converted
				.index
				.take()
				.ok_or_else(|| invalid("пустой манифест"))?;
			index.validate()?;
			Ok(PackContent {
				requirements: manifest.requirements()?,
				index,
				format: PackFormat::Curseforge {
					overrides: manifest.overrides_prefix().trim_end_matches('/').to_owned(),
				},
				converted,
			})
		}
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
	pub source: SourceKind,
	pub project_id: String,
	pub version_id: String,
	pub title: String,
	pub icon_url: Option<String>,
}

/// Необязательный файл модпака (выбор до установки).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionalFile {
	pub path: String,
	pub size: u64,
	pub title: Option<String>,
	pub icon_url: Option<String>,
	/// `null` — не контент (`config/…` и т. п.).
	#[serde(rename = "type")]
	pub project_type: Option<ProjectType>,
}

/// Модпак до установки (`modpack_inspect`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
	pub token: String,
	pub name: String,
	pub version_number: String,
	pub summary: Option<String>,
	pub game_version: String,
	pub loader: Loader,
	/// Опознан в каталоге (по версии или по sha1 файла) — сборка будет получать обновления.
	pub project: Option<ProjectInfo>,
	/// Модов (`mods/*.jar`) по умолчанию.
	pub mods: usize,
	/// Байт скачать по умолчанию.
	pub download_bytes: u64,
	/// Java для этой версии игры; `null` — не узнать без сети.
	pub java_major: Option<u32>,
	pub optional: Vec<OptionalFile>,
	/// Файлов «только с сайта» при выборе по умолчанию (после установки — ждущие сборки).
	pub manual_count: usize,
}

fn join_error(err: tokio::task::JoinError) -> AppError {
	AppError::unknown(err.to_string())
}

/// Запрос, который можно бросить при отмене (запросы каталога: файлов они не пишут).
async fn cancellable<T>(
	cancel: &CancellationToken,
	fut: impl std::future::Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
	tokio::select! {
		_ = cancel.cancelled() => Err(AppError::new(ErrorCode::Cancelled)),
		r = fut => r,
	}
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), AppError> {
	if cancel.is_cancelled() {
		Err(AppError::new(ErrorCode::Cancelled))
	} else {
		Ok(())
	}
}

/// Тип файла модпака по папке; `None` — не контент.
fn type_of(path: &str) -> Option<ProjectType> {
	let (dir, name) = path.split_once('/')?;
	match dir {
		"mods" if !name.contains('/') && name.to_ascii_lowercase().ends_with(".jar") => {
			Some(ProjectType::Mod)
		}
		"resourcepacks" => Some(ProjectType::Resourcepack),
		"shaderpacks" => Some(ProjectType::Shader),
		"datapacks" => Some(ProjectType::Datapack),
		_ => None,
	}
}

/// Откуда модпак для `modpack_inspect`.
#[derive(Debug, Clone)]
pub enum InspectSource {
	Catalog {
		source: SourceKind,
		project_id: String,
		version_id: Option<String>,
	},
	File(PathBuf),
}

pub fn origin_of(source: SourceKind) -> PackOrigin {
	match source {
		SourceKind::Modrinth => PackOrigin::Modrinth,
		SourceKind::Curseforge => PackOrigin::Curseforge,
	}
}

pub fn source_of(origin: PackOrigin) -> Option<SourceKind> {
	match origin {
		PackOrigin::Modrinth => Some(SourceKind::Modrinth),
		PackOrigin::Curseforge => Some(SourceKind::Curseforge),
		PackOrigin::File => None,
	}
}

/// Кладёт архив в кэш и разбирает; файл с диска ещё и пробует опознать в каталоге.
pub async fn prepare(
	l: &Launcher,
	source: &InspectSource,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<Prepared, AppError> {
	let prepared = match source {
		InspectSource::Catalog {
			source,
			project_id,
			version_id,
		} => {
			prepare_catalog(
				l,
				*source,
				project_id,
				version_id.as_deref(),
				progress,
				cancel,
			)
			.await?
		}
		InspectSource::File(path) => prepare_file(l, path, cancel).await?,
	};
	check_cancel(cancel)?;
	l.packs.put(prepared.clone());
	Ok(prepared)
}

/// Недокопированный в кэш архив: `<sha1>.mrpack.tmp` (старые дочищает [`super::cleanup`]).
pub const TMP_SUFFIX: &str = ".mrpack.tmp";

/// Копия файла кусками с проверкой отмены (отмена — `Interrupted`).
fn copy_cancellable(src: &Path, dst: &Path, cancel: &CancellationToken) -> std::io::Result<()> {
	use std::io::{Read, Write};
	let mut from = std::fs::File::open(src)?;
	let mut to = std::fs::File::create(dst)?;
	let mut buf = vec![0u8; 1024 * 1024];
	loop {
		if cancel.is_cancelled() {
			return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
		}
		let n = from.read(&mut buf)?;
		if n == 0 {
			break;
		}
		to.write_all(&buf[..n])?;
	}
	to.sync_all()
}

fn cache_path(l: &Launcher, sha1: &str) -> PathBuf {
	l.layout
		.cache_dir()
		.join("modpacks")
		.join(format!("{sha1}.mrpack"))
}

async fn read_pack(archive: &Path) -> Result<ReadPack, AppError> {
	let path = archive.to_path_buf();
	tokio::task::spawn_blocking(move || read_pack_sync(&path))
		.await
		.map_err(join_error)?
}

async fn prepare_catalog(
	l: &Launcher,
	source: SourceKind,
	project_id: &str,
	version_id: Option<&str>,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<Prepared, AppError> {
	content::check_ref(project_id)?;
	let catalog = l.sources.get(source);
	let version = match version_id {
		Some(id) => {
			content::check_ref(id)?;
			cancellable(cancel, catalog.version(id)).await?
		}
		None => {
			let filter = VersionFilter::default();
			let versions =
				cancellable(cancel, catalog.project_versions(project_id, &filter)).await?;
			content::pick_best(&versions)
				.cloned()
				.ok_or_else(|| AppError::coded(ErrorCode::ContentNotFound, project_id.to_owned()))?
		}
	};
	content::check_ref(&version.id)?;
	if version.site_only {
		// Сам модпак скачать можно только на сайте: поставить его так нельзя.
		return Err(AppError::coded(
			ErrorCode::DistributionBlocked,
			version.page_url.clone().unwrap_or_default(),
		));
	}
	let file = version
		.primary_file()
		.ok_or_else(|| invalid(format!("у версии {} нет файлов", version.id)))?;
	if !fs_util::is_sha1_hex(&file.sha1) {
		return Err(invalid(format!("у файла {} нет sha1", file.file_name)));
	}
	let token = file.sha1.to_ascii_lowercase();
	let archive = cache_path(l, &token);
	let job = Job {
		url: file.url.clone(),
		path: archive.clone(),
		sha1: Some(token.clone()),
		size: (file.size > 0).then_some(file.size),
	};
	l.dl.files(vec![job], Verify::Hash, progress.clone(), cancel)
		.await?;
	progress.flush();
	check_cancel(cancel)?;
	let read = read_pack(&archive).await?;
	let content = pack_content(l, read, cancel).await?;
	let projects = catalog.projects(std::slice::from_ref(&version.project_id));
	let hit = match cancellable(cancel, projects).await {
		Err(err) if err.code == ErrorCode::Cancelled => return Err(err),
		Ok(hits) => hits.into_iter().next(),
		Err(err) => {
			log::warn!(
				"модпак {}: проект не получен: {:?}",
				version.project_id,
				err.code
			);
			None
		}
	};
	let index = content.index;
	Ok(Prepared {
		token,
		archive,
		requirements: content.requirements,
		link: ModpackLink {
			source: origin_of(source),
			project_id: Some(version.project_id.clone()),
			version_id: Some(version.id.clone()),
			title: hit
				.as_ref()
				.map(|h| h.title.clone())
				.filter(|t| !t.is_empty())
				.unwrap_or_else(|| index.name.clone()),
			version_number: version.version_number.clone(),
			icon_url: hit.and_then(|h| h.icon_url),
			installed_at: 0,
		},
		index,
		format: content.format,
		manual: content.converted.manual,
		manual_optional: content.converted.manual_optional,
		unresolved: content.converted.unresolved,
		titles: content.converted.titles,
	})
}

async fn prepare_file(
	l: &Launcher,
	path: &Path,
	cancel: &CancellationToken,
) -> Result<Prepared, AppError> {
	if !path.is_file() {
		return Err(AppError::coded(
			ErrorCode::FileNotFound,
			path.display().to_string(),
		));
	}
	let src = path.to_path_buf();
	let cache_dir = l.layout.cache_dir().join("modpacks");
	let token = cancel.clone();
	let (read, sha1, archive) = tokio::task::spawn_blocking(move || {
		let index = read_pack_sync(&src)?;
		check_cancel(&token)?;
		let sha1 = content::installed::sha1_file(&src)
			.map_err(|e| AppError::storage(format!("{}: {e}", src.display())))?;
		let archive = cache_dir.join(format!("{sha1}.mrpack"));
		if !archive.is_file() {
			let tmp = cache_dir.join(format!("{sha1}{TMP_SUFFIX}"));
			let copied = std::fs::create_dir_all(&cache_dir)
				.and_then(|()| copy_cancellable(&src, &tmp, &token))
				.and_then(|()| std::fs::rename(&tmp, &archive));
			if let Err(e) = copied {
				let _ = std::fs::remove_file(&tmp);
				check_cancel(&token)?;
				return Err(AppError::storage(format!("{}: {e}", archive.display())));
			}
		}
		Ok::<_, AppError>((index, sha1, archive))
	})
	.await
	.map_err(join_error)??;
	let token = sha1;
	let is_cf = matches!(read, ReadPack::Curseforge(_));
	let content = pack_content(l, read, cancel).await?;
	let index = content.index;
	let requirements = content.requirements;
	let mut link = ModpackLink {
		source: PackOrigin::File,
		project_id: None,
		version_id: None,
		title: Some(index.name.clone())
			.filter(|n| !n.trim().is_empty())
			.or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
			.unwrap_or_default(),
		version_number: index.version_id.clone(),
		icon_url: None,
		installed_at: 0,
	};
	// Архив есть в каталоге — сборку можно будет обновлять.
	let source = if is_cf {
		SourceKind::Curseforge
	} else {
		SourceKind::Modrinth
	};
	let catalog = l.sources.get(source);
	let fingerprint = if is_cf {
		let a = archive.clone();
		tokio::task::spawn_blocking(move || content::fingerprint::of_file(&a).ok())
			.await
			.map_err(join_error)?
	} else {
		None
	};
	let keys = [content::FileKey {
		sha1: token.clone(),
		fingerprint,
		project_id: None,
	}];
	let identify = tokio::time::timeout(IDENTIFY_TIMEOUT, catalog.identify(&keys));
	match cancellable(cancel, async { Ok(identify.await) }).await? {
		Ok(Ok(found)) => {
			if let Some(version) = found.into_values().next() {
				link.source = origin_of(source);
				link.project_id = Some(version.project_id.clone());
				link.version_id = Some(version.id.clone());
				link.version_number = version.version_number.clone();
				let ids = [version.project_id];
				let projects = tokio::time::timeout(IDENTIFY_TIMEOUT, catalog.projects(&ids));
				if let Ok(Ok(hits)) = cancellable(cancel, async { Ok(projects.await) }).await?
					&& let Some(hit) = hits.into_iter().next()
				{
					link.title = hit.title;
					link.icon_url = hit.icon_url;
				}
			}
		}
		Ok(Err(err)) => log::info!("модпак с диска не опознан: {:?}", err.code),
		Err(_) => log::info!("модпак с диска не опознан: нет ответа"),
	}
	Ok(Prepared {
		token,
		archive,
		index,
		link,
		requirements,
		format: content.format,
		manual: content.converted.manual,
		manual_optional: content.converted.manual_optional,
		unresolved: content.converted.unresolved,
		titles: content.converted.titles,
	})
}

/// Модпак по токену из `modpack_inspect` (после перезапуска — из кэша на диске, как файл).
pub async fn by_token(l: &Launcher, token: &str) -> Result<Prepared, AppError> {
	if let Some(p) = l.packs.get(token) {
		return Ok(p);
	}
	if !fs_util::is_sha1_hex(token) {
		return Err(AppError::coded(
			ErrorCode::InvalidValue,
			format!("token: {token}"),
		));
	}
	let archive = cache_path(l, &token.to_ascii_lowercase());
	if !archive.is_file() {
		return Err(AppError::coded(
			ErrorCode::FileNotFound,
			format!("token: {token}"),
		));
	}
	prepare(
		l,
		&InspectSource::File(archive),
		Progress::silent(),
		&CancellationToken::new(),
	)
	.await
}

/// Описание для диалога установки.
pub async fn preview(
	l: &Launcher,
	p: &Prepared,
	cancel: &CancellationToken,
) -> Result<Preview, AppError> {
	let (chosen, _) = p.index.select(None);
	let optional_files: Vec<&mrpack::IndexFile> = p
		.index
		.files
		.iter()
		.filter(|f| f.client_side() == Side::Optional)
		.collect();
	// Названия необязательных файлов по sha1; без сети их не будет.
	let mut titles: HashMap<String, (String, Option<String>)> = p
		.index
		.files
		.iter()
		.filter_map(|f| Some((f.sha1()?.to_owned(), p.titles.get(&f.path)?.clone())))
		.collect();
	if !optional_files.is_empty() && p.format == PackFormat::Mrpack {
		let catalog = l.sources.get(SourceKind::Modrinth);
		let sha1s: Vec<String> = optional_files
			.iter()
			.filter_map(|f| f.sha1().map(str::to_owned))
			.collect();
		let keys = content::FileKey::of_sha1s(&sha1s);
		let identify = tokio::time::timeout(IDENTIFY_TIMEOUT, catalog.identify(&keys));
		if let Ok(Ok(found)) = cancellable(cancel, async { Ok(identify.await) }).await? {
			let mut ids: Vec<String> = found.values().map(|v| v.project_id.clone()).collect();
			ids.sort();
			ids.dedup();
			let projects = tokio::time::timeout(IDENTIFY_TIMEOUT, catalog.projects(&ids));
			if let Ok(Ok(hits)) = cancellable(cancel, async { Ok(projects.await) }).await? {
				let hits: HashMap<_, _> = hits.into_iter().map(|h| (h.id.clone(), h)).collect();
				for (sha1, version) in found {
					if let Some(hit) = hits.get(&version.project_id) {
						titles.insert(sha1, (hit.title.clone(), hit.icon_url.clone()));
					}
				}
			}
		}
	}
	let java_major = match l
		.meta
		.find_version(&p.requirements.game_version, cancel)
		.await
	{
		Ok(entry) => l
			.meta
			.version_info(&entry, cancel)
			.await
			.ok()
			.map(|i| i.java_major()),
		Err(_) => None,
	};
	check_cancel(cancel)?;
	Ok(Preview {
		token: p.token.clone(),
		name: p.index.name.clone(),
		version_number: p.link.version_number.clone(),
		summary: p.index.summary.clone(),
		game_version: p.requirements.game_version.clone(),
		loader: p.requirements.loader.clone(),
		project: match (
			source_of(p.link.source),
			&p.link.project_id,
			&p.link.version_id,
		) {
			(Some(source), Some(project), Some(version)) => Some(ProjectInfo {
				source,
				project_id: project.clone(),
				version_id: version.clone(),
				title: p.link.title.clone(),
				icon_url: p.link.icon_url.clone(),
			}),
			_ => None,
		},
		manual_count: p.manual.len(),
		mods: chosen
			.iter()
			.filter(|f| type_of(&f.path) == Some(ProjectType::Mod))
			.count(),
		download_bytes: chosen.iter().map(|f| f.file_size).sum(),
		java_major,
		optional: optional_files
			.into_iter()
			.map(|f| {
				let found = f.sha1().and_then(|s| titles.get(s));
				OptionalFile {
					path: f.path.clone(),
					size: f.file_size,
					title: found.map(|t| t.0.clone()),
					icon_url: found.and_then(|t| t.1.clone()),
					project_type: type_of(&f.path),
				}
			})
			.chain(p.manual.iter().filter_map(|f| {
				let path = f.dest_key().ok()?;
				p.manual_optional.contains(&path).then(|| OptionalFile {
					project_type: type_of(&path),
					path,
					size: f.size,
					title: Some(f.title.clone()),
					icon_url: f.icon_url.clone(),
				})
			}))
			.collect(),
	})
}

/// Проверяет версию игры и загрузчика. Версия загрузчика ищется как есть и как
/// `<версия игры>-<версия>`; не нашлась: `strict` — `loader_not_found`, иначе рекомендуемая.
pub async fn check_requirements(
	l: &Launcher,
	req: &mut Requirements,
	strict: bool,
) -> Result<(), AppError> {
	let cancel = CancellationToken::new();
	l.meta.find_version(&req.game_version, &cancel).await?;
	let kind = req.loader.kind;
	if kind == LoaderKind::Vanilla {
		req.loader.version = None;
		return Ok(());
	}
	let list = l
		.meta
		.loader_versions(kind, &req.game_version, &cancel)
		.await?
		.items;
	let not_found = |wanted: &str| {
		AppError::coded(
			ErrorCode::LoaderNotFound,
			format!(
				"{} {wanted} для Minecraft {}",
				kind.as_str(),
				req.game_version
			),
		)
	};
	if list.is_empty() {
		return Err(not_found("(любая)"));
	}
	if let Some(wanted) = req.loader.version.clone() {
		if list.iter().any(|v| v.id == wanted) {
			return Ok(());
		}
		let long = format!("{}-{wanted}", req.game_version);
		if list.iter().any(|v| v.id == long) {
			req.loader.version = Some(long);
		} else if strict {
			return Err(not_found(&wanted));
		} else {
			log::warn!(
				"{} {wanted} для {} нет, будет рекомендуемая",
				kind.as_str(),
				req.game_version
			);
			req.loader.version = None;
		}
	}
	Ok(())
}

/// Что изменит обновление (`modpack_update_preview`).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePreview {
	pub added: Vec<String>,
	pub updated: Vec<String>,
	pub removed: Vec<String>,
	/// Файлы контента не из модпака (останутся).
	pub user_files: usize,
	/// Файлы `overrides`, изменённые пользователем: заменятся (копия — в `backups/`).
	pub modified_overrides: Vec<String>,
	/// Размер копии сборки (если выбрать «Сначала сделать копию»).
	pub backup_bytes: u64,
}

/// Имя файла без версии: `sodium-fabric-0.5.8+mc1.20.1.jar` → `mods/sodium-fabric` (для пары
/// «удалён — добавлен» = «обновлён»).
fn stem(path: &str) -> String {
	let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
	let base: Vec<&str> = name
		.split(['-', '_', '+', ' '])
		.take_while(|p| {
			!p.trim_start_matches(['v', 'V'])
				.starts_with(|c: char| c.is_ascii_digit())
		})
		.collect();
	format!("{dir}/{}", base.join("-").to_ascii_lowercase())
}

/// Файлы индекса: что добавится, обновится и уберётся при переходе со старой версии на новую.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilePlan {
	pub added: Vec<String>,
	pub updated: Vec<String>,
	pub removed: Vec<String>,
	/// Пары «старый путь → новый путь» среди `updated` (новая версия с другим именем файла).
	pub renamed: Vec<(String, String)>,
}

/// Сравнивает индексы (путь → sha1). Новая версия мода обычно с другим именем файла: пара
/// «убран — добавлен» с тем же началом имени ([`stem`]) считается обновлением (путь новой версии).
pub fn plan_files(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> FilePlan {
	let mut added: Vec<String> = new
		.keys()
		.filter(|p| !old.contains_key(*p))
		.cloned()
		.collect();
	let mut removed: Vec<String> = old
		.keys()
		.filter(|p| !new.contains_key(*p))
		.cloned()
		.collect();
	let mut updated: Vec<String> = new
		.iter()
		.filter(|(p, h)| old.get(*p).is_some_and(|o| o != *h))
		.map(|(p, _)| p.clone())
		.collect();
	let mut stems: HashMap<String, usize> = HashMap::new();
	for (i, p) in removed.iter().enumerate() {
		stems.entry(stem(p)).or_insert(i);
	}
	let mut paired = BTreeSet::new();
	let mut renamed = Vec::new();
	added.retain(|p| match stems.get(&stem(p)) {
		Some(i) if paired.insert(*i) => {
			updated.push(p.clone());
			renamed.push((removed[*i].clone(), p.clone()));
			false
		}
		_ => true,
	});
	let mut i = 0;
	removed.retain(|_| {
		let keep = !paired.contains(&i);
		i += 1;
		keep
	});
	updated.sort();
	renamed.sort();
	FilePlan {
		added,
		updated,
		removed,
		renamed,
	}
}

/// [`plan_files`] для показа, с теми же поблажками файлам игрока, что в [`plan_downloads`].
fn preview_plan(
	old: &BTreeMap<String, String>,
	new: &BTreeMap<String, String>,
	game_dir: &Path,
) -> FilePlan {
	let mut plan = plan_files(old, new);
	let kept = |p: &String| {
		mrpack::is_user_data(p) && fs_util::safe_join(game_dir, p).is_some_and(|f| f.exists())
	};
	plan.added.retain(|p| !kept(p));
	plan.updated.retain(|p| !kept(p));
	plan.removed.retain(|p| !mrpack::is_user_data(p));
	plan
}

/// Разница между установленным модпаком и новой версией.
pub async fn update_preview(
	l: &Launcher,
	instance: &Instance,
	prepared: &Prepared,
	optional: Option<Vec<String>>,
) -> Result<UpdatePreview, AppError> {
	let instance_dir = l.layout.instance_dir(&instance.id);
	let old = previous_state(l, instance, &instance_dir).await?;
	let optional = optional.unwrap_or_else(|| prepared.default_optional(&old));
	let (files, _) = prepared.index.select(Some(&optional));
	let new: BTreeMap<String, String> = files
		.iter()
		.filter_map(|f| Some((f.path.clone(), f.sha1()?.to_owned())))
		.collect();
	let game_dir = l.layout.game_dir(&instance.id);
	let FilePlan {
		added,
		updated,
		removed,
		..
	} = preview_plan(&old.files, &new, &game_dir);

	let archive = prepared.archive.clone();
	let prefixes = prepared.override_prefixes();
	let old_files = old.files.clone();
	let old_overrides = old.overrides.clone();
	let dir = instance_dir.clone();
	let (user_files, modified_overrides, backup_bytes) = tokio::task::spawn_blocking(move || {
		let prefix_refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
		let user = content::installed::scan(&game_dir)
			.iter()
			.filter(|f| !old_files.contains_key(&f.key) && !old_overrides.contains_key(&f.key))
			.count();
		let modified: Vec<String> = mrpack::override_paths(&archive, &prefix_refs)
			.into_iter()
			.filter(|rel| !mrpack::is_user_data(rel))
			.filter(|rel| {
				let Some(path) = fs_util::safe_join(&game_dir, rel) else {
					return false;
				};
				path.is_file()
					&& content::installed::sha1_file(&path)
						.is_ok_and(|h| old_overrides.get(rel) != Some(&h))
			})
			.collect();
		(user, modified, super::launchers::dir_size(&dir))
	})
	.await
	.map_err(join_error)?;
	Ok(UpdatePreview {
		added,
		updated,
		removed,
		user_files,
		modified_overrides,
		backup_bytes,
	})
}

/// Необязательные файлы для обновления по умолчанию: все, кроме невыбранных в прошлый раз.
pub fn default_optional(index: &Index, old: &PackState) -> Vec<String> {
	index
		.files
		.iter()
		.filter(|f| f.client_side() == Side::Optional)
		.map(|f| f.path.clone())
		.filter(|p| !old.excluded.contains(p))
		.collect()
}

/// Что делает задача.
pub enum Work {
	Install {
		prepared: Prepared,
		optional: Vec<String>,
		/// Повтор после предупреждений: докачать только недостающее, `overrides` не трогать.
		repair: bool,
	},
	Update {
		prepared: Prepared,
		optional: Vec<String>,
		/// Сначала копия сборки с этим названием.
		backup: Option<String>,
		/// Повтор после предупреждений: докачать только недостающее, `overrides` не трогать.
		repair: bool,
	},
	Import {
		candidate: Candidate,
		include: Vec<Category>,
	},
}

/// Начатая задача: сборка занята, строка в списке загрузок есть.
pub struct Started {
	pub instance_id: String,
	pub task_id: String,
	pub cancel: CancellationToken,
	/// Сборка новая: при отмене она удаляется (недоустановленная сборка не нужна).
	pub fresh: bool,
}

/// Занимает сборку и ставит задачу. Под замком контента: задачи контента не пересекутся с ней.
pub async fn begin(
	l: &Launcher,
	instance_id: &str,
	request: PackRequest,
	info: PackTask,
	fresh: bool,
) -> Result<Started, AppError> {
	let _content = l.content_locks.lock(instance_id).await;
	if l.downloads.has_active_content(instance_id) {
		return Err(AppError::coded(
			ErrorCode::InstanceBusy,
			instance_id.to_owned(),
		));
	}
	let cancel = l.instances.begin_task(instance_id)?;
	let task_id = l.downloads.begin(
		Target::Pack {
			instance_id: instance_id.to_owned(),
			request,
			info,
		},
		cancel.clone(),
	);
	Ok(Started {
		instance_id: instance_id.to_owned(),
		task_id,
		cancel,
		fresh,
	})
}

/// Задача целиком. `true` — готово.
pub async fn run(
	l: Arc<Launcher>,
	started: Started,
	work: Work,
	events: Arc<dyn InstallEvents>,
) -> bool {
	let Started {
		instance_id: id,
		task_id,
		cancel,
		fresh,
	} = started;
	events.changed(l.instances.list().await);
	let result = work_run(&l, &id, &task_id, work, &events, &cancel).await;

	let finished = match &result {
		Ok(_) => InstallResult::Installed,
		Err(err) if err.code == ErrorCode::Cancelled => InstallResult::Cancelled,
		Err(_) => InstallResult::Failed,
	};
	match &result {
		Ok((failed, manual)) => log::info!(
			"модпак {id}: готово, не получилось файлов: {}, вручную: {}",
			failed.len(),
			manual.len()
		),
		Err(err) => log::warn!(
			"модпак {id}: {:?} {}",
			err.code,
			err.details.as_deref().unwrap_or("")
		),
	}
	if finished != InstallResult::Installed {
		let _ = l
			.instances
			.modify(&id, |inst| {
				// Сборка после неудачного обновления рабочая: игра установлена целиком.
				inst.install_state = match (finished, inst.installed.is_some()) {
					(InstallResult::Failed, true) if !fresh => InstallState::Installed,
					(InstallResult::Failed, _) => InstallState::Failed,
					(_, true) => InstallState::Installed,
					(_, false) => InstallState::NotInstalled,
				};
				Ok(())
			})
			.await;
	}
	l.instances.end_task(&id);
	let mut removed = false;
	if finished == InstallResult::Cancelled && fresh {
		// Как в `instance_remove`.
		let _content = l.content_locks.lock(&id).await;
		if l.downloads.has_active_content(&id) {
			log::warn!("модпак {id}: недоустановленная сборка не удалена: идёт установка контента");
		} else {
			l.watch.stop_if(&id);
			match l.instances.remove(&id).await {
				Ok(()) => {
					removed = true;
					l.games.forget(&id);
				}
				Err(err) => log::warn!("модпак {id}: недоустановленная сборка не удалена: {err}"),
			}
		}
	}
	let download_result = match result {
		Ok((failed, manual)) => {
			if !failed.is_empty() || !manual.is_empty() {
				l.downloads.annotate(&task_id, |t| {
					t.warnings = Some(Warnings { failed, manual });
				});
			}
			Ok(())
		}
		Err(err) => Err(err),
	};
	l.downloads.finish(&task_id, download_result.clone());
	if removed {
		l.downloads.forget_instance(&id);
	}
	events.changed(l.instances.list().await);
	if !removed {
		events.finished(InstallFinished {
			instance_id: id,
			result: finished,
			error: download_result
				.err()
				.filter(|e| e.code != ErrorCode::Cancelled),
		});
	}
	finished == InstallResult::Installed
}

async fn work_run(
	l: &Arc<Launcher>,
	id: &str,
	task_id: &str,
	work: Work,
	events: &Arc<dyn InstallEvents>,
	cancel: &CancellationToken,
) -> Result<(Vec<FailedFile>, Vec<ManualFile>), AppError> {
	let reporter = Reporter::new(id.to_owned(), events.clone()).tracked(Tracked {
		downloads: l.downloads.clone(),
		task_id: task_id.to_owned(),
	});
	let instance_dir = l.layout.instance_dir(id);
	let game_dir = l.layout.game_dir(id);
	let mut manual: Vec<ManualFile> = Vec::new();
	let ((installed, loader_version), failed) = match work {
		Work::Install {
			prepared,
			optional,
			repair,
		} => {
			let game = install_game(l, id, events, task_id, cancel, 0.0, None).await?;
			// Повтор: прошлое состояние нужно, чтобы не перезаписать данные игрока.
			let old = if repair {
				load_state_async(&instance_dir).await?
			} else {
				None
			};
			let (state, failed) = apply_pack(
				l,
				&prepared,
				&optional,
				old.as_ref(),
				repair,
				&instance_dir,
				&game_dir,
				&reporter,
				cancel,
			)
			.await?;
			let state = merge_repaired(state, old);
			if set_link(l, id, &prepared.link).await? {
				super::save_state(&instance_dir, &state)?;
			}
			manual = save_manual(l, id, &prepared, &optional).await;
			(game, failed)
		}
		Work::Update {
			prepared,
			optional,
			backup,
			repair,
		} => {
			let current = l.instances.get(id).await?;
			// Отвязка могла проскочить между проверкой в `modpack_update` и началом задачи.
			if current.modpack.is_none() {
				return Err(AppError::coded(ErrorCode::ModpackNotLinked, id.to_owned()));
			}
			let mut start = 0.0;
			if let Some(name) = backup.filter(|_| !repair) {
				reporter.stage(InstallStage::Copying, 0.0, 10.0);
				let copy = l.instances.duplicate_while_busy(id, &name).await?;
				log::info!("модпак {id}: копия перед обновлением — {}", copy.id);
				events.changed(l.instances.list().await);
				start = 10.0;
			}
			let old = previous_state(l, &current, &instance_dir).await?;
			let req = prepared.requirements.clone();
			let switch = current.game_version != req.game_version || current.loader != req.loader;
			// Новая версия пишется в сборку только перед заменой файлов: до этого отмена ничего не портит.
			let target = switch.then_some(&req);
			let game = install_game(l, id, events, task_id, cancel, start, target).await?;
			check_cancel(cancel)?;
			if switch {
				let (installed, loader_version) = game.clone();
				l.instances
					.modify(id, |inst| {
						inst.game_version = req.game_version.clone();
						inst.loader = req.loader.clone();
						if inst.loader.version.is_none() {
							inst.loader.version = loader_version;
						}
						inst.installed = Some(installed);
						inst.install_state = InstallState::Installed;
						Ok(())
					})
					.await?;
				log::info!(
					"модпак {id}: сборка переходит на {} {:?}",
					req.game_version,
					req.loader.kind
				);
			}
			let (state, failed) = apply_pack(
				l,
				&prepared,
				&optional,
				Some(&old),
				repair,
				&instance_dir,
				&game_dir,
				&reporter,
				cancel,
			)
			.await?;
			let state = if repair {
				merge_repaired(state, Some(old))
			} else {
				state
			};
			if set_link(l, id, &prepared.link).await? {
				super::save_state(&instance_dir, &state)?;
			}
			manual = save_manual(l, id, &prepared, &optional).await;
			(game, failed)
		}
		Work::Import { candidate, include } => {
			let game = install_game(l, id, events, task_id, cancel, 0.0, None).await?;
			let progress = reporter.stage(InstallStage::Copying, GAME_END, 99.0);
			let from = candidate.game_dir.clone();
			let to = game_dir.clone();
			let token = cancel.clone();
			let failed = tokio::task::spawn_blocking(move || {
				copy_selected(&from, &to, &include, &progress, &token)
			})
			.await
			.map_err(join_error)??;
			(game, failed)
		}
	};
	reporter.stage(InstallStage::Finishing, 99.0, 100.0);
	l.instances
		.modify(id, |inst| {
			inst.install_state = InstallState::Installed;
			inst.installed = Some(installed);
			if inst.loader.version.is_none() {
				inst.loader.version = loader_version;
			}
			Ok(())
		})
		.await?;
	Ok((failed, manual))
}

/// Ручные файлы модпака (при этом выборе необязательных) — в ждущие сборки.
async fn save_manual(
	l: &Launcher,
	id: &str,
	prepared: &Prepared,
	optional: &[String],
) -> Vec<ManualFile> {
	let manual = prepared.manual_for(optional);
	if manual.is_empty() {
		return manual;
	}
	let _guard = l.content_locks.lock(id).await;
	let dir = l.layout.instance_dir(id);
	let files = manual.clone();
	match tokio::task::spawn_blocking(move || crate::content::manual::add_waiting(&dir, &files))
		.await
	{
		Ok(Ok(_)) => {}
		Ok(Err(err)) => log::warn!("модпак {id}: ждущие файлы не записаны: {:?}", err.code),
		Err(err) => log::warn!("модпак {id}: ждущие файлы не записаны: {err}"),
	}
	manual
}

async fn load_state_async(instance_dir: &Path) -> Result<Option<PackState>, AppError> {
	let dir = instance_dir.to_path_buf();
	tokio::task::spawn_blocking(move || super::load_state(&dir))
		.await
		.map_err(join_error)?
}

/// Повтор докачивает только файлы индекса: `overrides` и невыбранные — как в прошлый раз.
fn merge_repaired(state: PackState, old: Option<PackState>) -> PackState {
	match old {
		Some(old) => PackState {
			files: state.files,
			..old
		},
		None => state,
	}
}

/// Записывает версию модпака. Сборку успели отвязать — `false`, связь не возвращаем.
async fn set_link(l: &Launcher, id: &str, link: &ModpackLink) -> Result<bool, AppError> {
	let mut link = link.clone();
	link.installed_at = super::now_ms();
	let mut linked = false;
	l.instances
		.modify(id, |inst| {
			if inst.modpack.is_some() {
				inst.modpack = Some(link);
				linked = true;
			}
			Ok(())
		})
		.await?;
	if !linked {
		log::info!("модпак {id}: сборку отвязали во время задачи, связь не восстанавливается");
	}
	Ok(linked)
}

/// Игра и загрузчик (`start`–60 %), в общей очереди установок. В сборку не записывается.
/// `target` — поставить другие версию игры и загрузчик (обновление модпака на них).
async fn install_game(
	l: &Arc<Launcher>,
	id: &str,
	events: &Arc<dyn InstallEvents>,
	task_id: &str,
	cancel: &CancellationToken,
	start: f32,
	target: Option<&Requirements>,
) -> Result<(Installed, Option<String>), AppError> {
	let reporter = Reporter::new(id.to_owned(), events.clone())
		.tracked(Tracked {
			downloads: l.downloads.clone(),
			task_id: task_id.to_owned(),
		})
		.scaled(start, GAME_END);
	let _queue = match l.install_lock.try_lock() {
		Ok(guard) => guard,
		Err(_) => {
			reporter.stage(InstallStage::Queued, 0.0, 0.0);
			tokio::select! {
				_ = cancel.cancelled() => return Err(AppError::new(ErrorCode::Cancelled)),
				g = l.install_lock.lock() => g,
			}
		}
	};
	let mut instance = l.instances.get(id).await?;
	if let Some(req) = target {
		instance.game_version = req.game_version.clone();
		instance.loader = req.loader.clone();
		instance.installed = None;
	}
	install_files(l, &instance, InstallMode::Install, &reporter, cancel).await
}

/// Что модпак положил раньше. Без `modpack.json` (перенос) индекс берётся из архива версии.
async fn previous_state(
	l: &Launcher,
	instance: &Instance,
	instance_dir: &Path,
) -> Result<PackState, AppError> {
	let dir = instance_dir.to_path_buf();
	let stored = tokio::task::spawn_blocking(move || super::load_state(&dir))
		.await
		.map_err(join_error)??;
	if let Some(state) = stored {
		return Ok(state);
	}
	let link = instance.modpack.as_ref();
	let (Some(source), Some(project), Some(version)) = (
		link.and_then(|m| source_of(m.source)),
		link.and_then(|m| m.project_id.clone()),
		link.and_then(|m| m.version_id.clone()),
	) else {
		return Ok(PackState::default());
	};
	let old = prepare_catalog(
		l,
		source,
		&project,
		Some(&version),
		Progress::silent(),
		&CancellationToken::new(),
	)
	.await;
	match old {
		Ok(p) => Ok(PackState {
			files: p
				.index
				.files
				.iter()
				.filter_map(|f| Some((f.path.clone(), f.sha1()?.to_owned())))
				.collect(),
			..PackState::default()
		}),
		Err(err) => {
			log::warn!("прошлая версия модпака не получена: {:?}", err.code);
			Ok(PackState::default())
		}
	}
}

/// Что качать из индекса модпака.
#[derive(Debug, Default)]
struct PlannedDownloads {
	/// Для `modpack.json`: путь → sha1.
	files: BTreeMap<String, String>,
	/// Путь от папки игры, задание, sha512.
	jobs: Vec<(String, Job, Option<String>)>,
	/// Адрес не из разрешённых.
	failed: Vec<FailedFile>,
}

fn disabled_path(path: &Path) -> PathBuf {
	PathBuf::from(format!(
		"{}{}",
		path.display(),
		content::installed::DISABLED_SUFFIX
	))
}

/// Задания на файлы индекса. При обновлении файлы игрока не перезаписываются, а выключенный
/// мод остаётся выключенным и в новой версии.
#[cfg(test)]
fn plan_downloads(
	files: &[&mrpack::IndexFile],
	game_dir: &Path,
	old: Option<&PackState>,
) -> Result<PlannedDownloads, AppError> {
	plan_downloads_in(files, game_dir, old, &mrpack::ALLOWED_HOSTS)
}

/// [`plan_downloads`] с хостами формата модпака (`.mrpack` — свои, CurseForge — CDN CurseForge).
fn plan_downloads_in(
	files: &[&mrpack::IndexFile],
	game_dir: &Path,
	old: Option<&PackState>,
	hosts: &[&str],
) -> Result<PlannedDownloads, AppError> {
	let renamed_from: HashMap<String, String> = match old {
		Some(old) => {
			let new: BTreeMap<String, String> = files
				.iter()
				.filter_map(|f| Some((f.path.clone(), f.sha1()?.to_owned())))
				.collect();
			plan_files(&old.files, &new)
				.renamed
				.into_iter()
				.map(|(from, to)| (to, from))
				.collect()
		}
		None => HashMap::new(),
	};
	let mut planned = PlannedDownloads::default();
	for file in files {
		let sha1 = file.sha1().unwrap_or_default().to_owned();
		let path = fs_util::safe_join(game_dir, &file.path)
			.ok_or_else(|| invalid(format!("небезопасный путь: {}", file.path)))?;
		if let Some(old) = old
			&& mrpack::is_user_data(&file.path)
			&& path.exists()
		{
			if let Some(hash) = old.files.get(&file.path) {
				planned.files.insert(file.path.clone(), hash.clone());
			}
			continue;
		}
		planned.files.insert(file.path.clone(), sha1.clone());
		let Some(url) = file.allowed_url_in(hosts) else {
			planned.failed.push(FailedFile {
				path: file.path.clone(),
				error: AppError::coded(
					ErrorCode::DownloadFailed,
					format!("адрес не из разрешённых: {}", file.downloads.join(", ")),
				),
			});
			continue;
		};
		let disabled = disabled_path(&path);
		let was_disabled = if path.exists() {
			false
		} else if disabled.is_file() {
			true
		} else {
			renamed_from
				.get(&file.path)
				.and_then(|from| fs_util::safe_join(game_dir, from))
				.is_some_and(|from| !from.exists() && disabled_path(&from).is_file())
		};
		planned.jobs.push((
			file.path.clone(),
			Job {
				url: url.to_owned(),
				path: if was_disabled { disabled } else { path },
				sha1: Some(sha1),
				size: (file.file_size > 0).then_some(file.file_size),
			},
			file.sha512().map(str::to_owned),
		));
	}
	Ok(planned)
}

/// Моды модпака (60–95 %), удаление лишнего при обновлении, `overrides` (95–99 %).
/// Возвращает состояние для `modpack.json` и файлы, которые не скачались.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_pack(
	l: &Launcher,
	prepared: &Prepared,
	optional: &[String],
	old: Option<&PackState>,
	repair: bool,
	instance_dir: &Path,
	game_dir: &Path,
	reporter: &Reporter,
	cancel: &CancellationToken,
) -> Result<(PackState, Vec<FailedFile>), AppError> {
	let progress = reporter.stage(InstallStage::ModpackFiles, GAME_END, FILES_END);
	let (files, excluded) = prepared.index.select(Some(optional));
	let planned = plan_downloads_in(&files, game_dir, old, prepared.allowed_hosts())?;
	let mut state = PackState {
		files: planned.files,
		excluded,
		..PackState::default()
	};
	let mut failed = planned.failed;
	failed.extend(prepared.unresolved.iter().cloned());
	let jobs = planned.jobs;
	let all_blocked = jobs.is_empty() && !failed.is_empty();
	failed.extend(download_files(l, jobs, progress.clone(), cancel).await?);
	progress.flush();
	if all_blocked {
		return Err(failed.swap_remove(0).error);
	}
	if repair {
		return Ok((state, failed));
	}
	if let Some(old) = old {
		let stale: Vec<(String, String)> = old
			.files
			.iter()
			.filter(|(path, _)| !state.files.contains_key(*path) && !mrpack::is_user_data(path))
			.map(|(p, h)| (p.clone(), h.clone()))
			.collect();
		let dir = game_dir.to_path_buf();
		tokio::task::spawn_blocking(move || remove_stale(&dir, &stale, true, trash_file))
			.await
			.map_err(join_error)??;
	}

	reporter.stage(InstallStage::Overrides, FILES_END, 99.0);
	let policy = OverridePolicy {
		previous: old.map(|o| o.overrides.clone()),
		backup_dir: old.map(|_| {
			instance_dir.join("backups").join(format!(
				"modpack-{}",
				chrono::Utc::now().format("%Y%m%d-%H%M%S")
			))
		}),
	};
	let archive = prepared.archive.clone();
	let dir = game_dir.to_path_buf();
	let token = cancel.clone();
	let prefixes = prepared.override_prefixes();
	let written = tokio::task::spawn_blocking(move || {
		let prefixes: Vec<&str> = prefixes.iter().map(String::as_str).collect();
		mrpack::extract_overrides(
			&archive,
			&prefixes,
			&dir,
			&policy,
			mrpack::Limits::DEFAULT,
			&token,
		)
	})
	.await
	.map_err(join_error)??;
	if !written.backed_up.is_empty() {
		log::info!(
			"модпак: перед обновлением сохранены изменённые файлы: {}",
			written.backed_up.len()
		);
	}
	state.overrides = written.written;
	if let Some(old) = old {
		for path in &written.kept {
			if let Some(hash) = old.overrides.get(path) {
				state.overrides.insert(path.clone(), hash.clone());
			}
		}
		// Файлы прошлых `overrides`, которых в новой версии нет: удаляются, если не изменены.
		let stale: Vec<(String, String)> = old
			.overrides
			.iter()
			.filter(|(p, _)| {
				!state.overrides.contains_key(*p)
					&& !state.files.contains_key(*p)
					&& !mrpack::is_user_data(p)
			})
			.map(|(p, h)| (p.clone(), h.clone()))
			.collect();
		let dir = game_dir.to_path_buf();
		tokio::task::spawn_blocking(move || remove_stale(&dir, &stale, false, trash_file))
			.await
			.map_err(join_error)??;
	}
	Ok((state, failed))
}

/// Качает файлы по отдельности: ошибка одного не останавливает остальные.
async fn download_files(
	l: &Launcher,
	jobs: Vec<(String, Job, Option<String>)>,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<Vec<FailedFile>, AppError> {
	let total = jobs.len();
	progress.add_total(total as u64, jobs.iter().filter_map(|j| j.1.size).sum());
	let limit = Arc::new(tokio::sync::Semaphore::new(PACK_CONCURRENCY));
	let mut set = tokio::task::JoinSet::new();
	for (rel, job, sha512) in jobs {
		let dl = l.dl.clone();
		let progress = progress.clone();
		let cancel = cancel.clone();
		let limit = limit.clone();
		set.spawn(async move {
			let _permit = tokio::select! {
				_ = cancel.cancelled() => return (rel, Err(AppError::new(ErrorCode::Cancelled))),
				p = limit.acquire_owned() => p,
			};
			let result = match dl.file(&job, Verify::Hash, &progress, &cancel).await {
				Ok(_) => match sha512 {
					Some(expected) => check_sha512(job.path.clone(), expected).await,
					None => Ok(()),
				},
				Err(err) => {
					progress.file_done();
					Err(AppError::from(err))
				}
			};
			(rel, result)
		});
	}
	let mut failed = Vec::new();
	while let Some(joined) = set.join_next().await {
		let (rel, result) = joined.unwrap_or_else(|e| (String::new(), Err(join_error(e))));
		if let Err(error) = result {
			failed.push(FailedFile { path: rel, error });
		}
	}
	if cancel.is_cancelled() {
		return Err(AppError::new(ErrorCode::Cancelled));
	}
	if total > 0 && failed.len() == total {
		return Err(failed.swap_remove(0).error);
	}
	failed.sort_by(|a, b| a.path.cmp(&b.path));
	Ok(failed)
}

async fn check_sha512(path: PathBuf, expected: String) -> Result<(), AppError> {
	tokio::task::spawn_blocking(move || {
		let actual = super::export::sha512_file(&path)
			.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?;
		if actual.eq_ignore_ascii_case(&expected) {
			Ok(())
		} else {
			let _ = std::fs::remove_file(&path);
			Err(AppError::coded(
				ErrorCode::DownloadFailed,
				format!("{}: sha512 не совпал", path.display()),
			))
		}
	})
	.await
	.map_err(join_error)?
}

fn trash_file(path: &Path) -> Result<(), AppError> {
	crate::trash::delete(path).map_err(|e| content::installed::io_error(path, &e))
}

/// Удаляет (`delete` — в корзину) файлы прошлой версии модпака и их выключенные копии. Файл,
/// изменённый пользователем (sha1 другой), остаётся — кроме папок контента, если `content_always`.
fn remove_stale(
	game_dir: &Path,
	stale: &[(String, String)],
	content_always: bool,
	delete: impl Fn(&Path) -> Result<(), AppError>,
) -> Result<(), AppError> {
	for (rel, sha1) in stale {
		let Some(path) = fs_util::safe_join(game_dir, rel) else {
			continue;
		};
		let disabled = PathBuf::from(format!(
			"{}{}",
			path.display(),
			content::installed::DISABLED_SUFFIX
		));
		let content = content_always && CONTENT_FOLDERS.iter().any(|f| rel.starts_with(f));
		for file in [path, disabled] {
			if !file.is_file() {
				continue;
			}
			let same = content::installed::sha1_file(&file).is_ok_and(|h| &h == sha1);
			if same || content {
				delete(&file)?;
			}
		}
	}
	Ok(())
}

/// Копирует выбранные категории папки игры. Возвращает то, что не скопировалось.
pub fn copy_selected(
	from: &Path,
	to: &Path,
	include: &[Category],
	progress: &Progress,
	cancel: &CancellationToken,
) -> Result<Vec<FailedFile>, AppError> {
	let mut files = Vec::new();
	for (name, dir, category) in super::launchers::top_entries(from) {
		if !include.contains(&category) {
			continue;
		}
		if dir {
			let mut stack = vec![PathBuf::from(&name)];
			while let Some(rel) = stack.pop() {
				let Ok(list) = std::fs::read_dir(from.join(&rel)) else {
					continue;
				};
				for e in list.flatten() {
					let Ok(kind) = e.file_type() else { continue };
					let rel = rel.join(e.file_name());
					if kind.is_dir() {
						stack.push(rel);
					} else if kind.is_file() {
						files.push(rel);
					}
				}
			}
		} else {
			files.push(PathBuf::from(name));
		}
	}
	let total = files.len();
	progress.add_total(total as u64, 0);
	let mut failed = Vec::new();
	for rel in files {
		if cancel.is_cancelled() {
			return Err(AppError::new(ErrorCode::Cancelled));
		}
		let target = to.join(&rel);
		let src = from.join(&rel);
		let copied = target
			.parent()
			.map_or(Ok(()), std::fs::create_dir_all)
			.and_then(|()| std::fs::copy(&src, &target));
		if let Err(e) = copied {
			failed.push(FailedFile {
				path: rel.to_string_lossy().replace('\\', "/"),
				error: AppError::storage(format!("{}: {e}", src.display())),
			});
		}
		progress.file_done();
	}
	progress.flush();
	if total > 0 && failed.len() == total {
		return Err(failed.swap_remove(0).error);
	}
	Ok(failed)
}

/// Имя сборки: до 64 символов, без управляющих.
pub fn instance_name(name: &str) -> String {
	let clean: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
	let clean = clean.trim();
	if clean.is_empty() {
		"Modpack".into()
	} else {
		clean.to_owned()
	}
}

/// Свободное название: занято — «{name} (2)», «{name} (3)»…
pub fn unique_name(name: &str, taken: &[Instance]) -> String {
	let busy = |n: &str| taken.iter().any(|i| i.name.eq_ignore_ascii_case(n));
	if !busy(name) {
		return name.to_owned();
	}
	(2..1000)
		.map(|n| {
			let suffix = format!(" ({n})");
			let base: String = name.chars().take(64 - suffix.chars().count()).collect();
			format!("{}{suffix}", base.trim_end())
		})
		.find(|n| !busy(n))
		.unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn copies_categories_without_junk() {
		let dir = tempfile::tempdir().unwrap();
		let from = dir.path().join("from");
		for (rel, data) in [
			("mods/a.jar", "a"),
			("mods/sub/b.jar", "b"),
			("config/c.toml", "c"),
			("options.txt", "o"),
			("logs/latest.log", "l"),
			("screenshots/s.png", "s"),
		] {
			let p = from.join(rel);
			std::fs::create_dir_all(p.parent().unwrap()).unwrap();
			std::fs::write(p, data).unwrap();
		}
		let to = dir.path().join("to");
		let progress = Progress::silent();
		let include = [Category::Mods, Category::Options, Category::Config];
		let failed =
			copy_selected(&from, &to, &include, &progress, &CancellationToken::new()).unwrap();
		assert!(failed.is_empty());
		assert_eq!(std::fs::read(to.join("mods/sub/b.jar")).unwrap(), b"b");
		assert!(to.join("options.txt").is_file() && to.join("config/c.toml").is_file());
		assert!(!to.join("logs").exists() && !to.join("screenshots").exists());
		assert_eq!(progress.snapshot().files_done, 4);
		let cancel = CancellationToken::new();
		cancel.cancel();
		assert_eq!(
			copy_selected(&from, &to, &include, &progress, &cancel)
				.unwrap_err()
				.code,
			ErrorCode::Cancelled
		);
		assert_eq!(
			std::fs::read_dir(&from).unwrap().count(),
			5,
			"источник не тронут"
		);
	}

	#[test]
	fn removes_only_unmodified_or_content() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path();
		let write = |rel: &str, data: &str| {
			let p = game.join(rel);
			std::fs::create_dir_all(p.parent().unwrap()).unwrap();
			std::fs::write(p, data).unwrap();
		};
		write("mods/old.jar.disabled", "user changed");
		write("config/x.toml", "abc");
		write("config/y.toml", "edited");
		let abc = "a9993e364706816aba3e25717850c26c9cd0d89d".to_owned();
		remove_stale(
			game,
			&[
				("mods/old.jar".into(), abc.clone()),
				("config/x.toml".into(), abc.clone()),
				("config/y.toml".into(), abc),
			],
			true,
			// Не настоящая корзина: тест не должен класть файлы в корзину пользователя.
			|p| std::fs::remove_file(p).map_err(|e| AppError::storage(e.to_string())),
		)
		.unwrap();
		assert!(!game.join("mods/old.jar.disabled").exists());
		assert!(!game.join("config/x.toml").exists());
		assert!(game.join("config/y.toml").exists());
	}

	#[test]
	fn update_plan() {
		let map = |items: &[(&str, &str)]| -> BTreeMap<String, String> {
			items
				.iter()
				.map(|(p, h)| ((*p).to_owned(), (*h).to_owned()))
				.collect()
		};
		let old = map(&[
			("mods/sodium-fabric-0.5.8+mc1.20.1.jar", "1"),
			("mods/lithium-0.11.jar", "2"),
			("mods/gone-1.0.jar", "3"),
			("config/same.toml", "4"),
			("mods/rehashed.jar", "5"),
		]);
		let new = map(&[
			("mods/sodium-fabric-0.6.0+mc1.21.jar", "10"),
			("mods/lithium-0.11.jar", "2"),
			("config/same.toml", "4"),
			("mods/rehashed.jar", "50"),
			("mods/fresh-2.0.jar", "11"),
		]);
		let plan = plan_files(&old, &new);
		assert_eq!(plan.added, vec!["mods/fresh-2.0.jar"]);
		assert_eq!(
			plan.updated,
			vec!["mods/rehashed.jar", "mods/sodium-fabric-0.6.0+mc1.21.jar"]
		);
		assert_eq!(plan.removed, vec!["mods/gone-1.0.jar"]);
		assert_eq!(
			plan.renamed,
			vec![(
				"mods/sodium-fabric-0.5.8+mc1.20.1.jar".to_owned(),
				"mods/sodium-fabric-0.6.0+mc1.21.jar".to_owned()
			)]
		);
		assert_eq!(plan_files(&old, &old), FilePlan::default());
		// Откат на старую версию — та же разница в обратную сторону.
		let back = plan_files(&new, &old);
		assert_eq!(back.added, vec!["mods/gone-1.0.jar"]);
		assert_eq!(back.removed, vec!["mods/fresh-2.0.jar"]);
	}

	fn index_file(path: &str, sha1: &str) -> mrpack::IndexFile {
		mrpack::IndexFile {
			path: path.into(),
			hashes: BTreeMap::from([("sha1".to_owned(), sha1.to_owned())]),
			env: None,
			downloads: vec![format!("https://cdn.modrinth.com/data/x/{path}")],
			file_size: 1,
		}
	}

	fn write(game: &Path, rel: &str) {
		let p = game.join(rel);
		std::fs::create_dir_all(p.parent().unwrap()).unwrap();
		std::fs::write(p, b"x").unwrap();
	}

	#[test]
	fn user_data_in_index_kept_on_update() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path();
		write(game, "options.txt");
		write(game, "saves/World/level.dat");
		let files = [
			index_file("options.txt", "1"),
			index_file("saves/World/level.dat", "2"),
			index_file("servers.dat", "3"),
			index_file("mods/a-1.0.jar", "4"),
		];
		let refs: Vec<&mrpack::IndexFile> = files.iter().collect();
		let jobs = |p: &PlannedDownloads| -> Vec<String> {
			p.jobs.iter().map(|(rel, _, _)| rel.clone()).collect()
		};
		// Первая установка: ставится всё.
		let first = plan_downloads(&refs, game, None).unwrap();
		assert_eq!(jobs(&first).len(), 4);
		// Обновление: уже лежащие файлы игрока не трогаются, отсутствующий `servers.dat` ставится.
		let old = PackState {
			files: BTreeMap::from([("options.txt".to_owned(), "0".to_owned())]),
			..PackState::default()
		};
		let update = plan_downloads(&refs, game, Some(&old)).unwrap();
		assert_eq!(jobs(&update), vec!["servers.dat", "mods/a-1.0.jar"]);
		assert_eq!(
			update.files.get("options.txt").map(String::as_str),
			Some("0")
		);
		assert!(!update.files.contains_key("saves/World/level.dat"));
	}

	#[test]
	fn disabled_mod_stays_disabled_after_rename() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path();
		write(game, "mods/sodium-fabric-0.5.8.jar.disabled");
		write(game, "mods/iris-1.0.jar.disabled");
		write(game, "mods/lithium-0.11.jar");
		let old = PackState {
			files: BTreeMap::from([
				("mods/sodium-fabric-0.5.8.jar".to_owned(), "1".to_owned()),
				("mods/iris-1.0.jar".to_owned(), "2".to_owned()),
				("mods/lithium-0.11.jar".to_owned(), "3".to_owned()),
			]),
			..PackState::default()
		};
		let files = [
			index_file("mods/sodium-fabric-0.6.0.jar", "10"),
			index_file("mods/iris-1.0.jar", "20"),
			index_file("mods/lithium-0.12.jar", "30"),
			index_file("mods/fresh-1.0.jar", "40"),
		];
		let refs: Vec<&mrpack::IndexFile> = files.iter().collect();
		let planned = plan_downloads(&refs, game, Some(&old)).unwrap();
		let target = |rel: &str| {
			planned
				.jobs
				.iter()
				.find(|(r, _, _)| r == rel)
				.map(|(_, job, _)| job.path.clone())
				.unwrap()
		};
		let disabled = |rel: &str| disabled_path(&game.join(rel));
		// Новая версия с другим именем — выключена, как была старая.
		assert_eq!(
			target("mods/sodium-fabric-0.6.0.jar"),
			disabled("mods/sodium-fabric-0.6.0.jar")
		);
		// Тот же путь — тоже.
		assert_eq!(target("mods/iris-1.0.jar"), disabled("mods/iris-1.0.jar"));
		// Старая была включена — новая включена; нового мода раньше не было.
		assert_eq!(
			target("mods/lithium-0.12.jar"),
			game.join("mods/lithium-0.12.jar")
		);
		assert_eq!(
			target("mods/fresh-1.0.jar"),
			game.join("mods/fresh-1.0.jar")
		);
		// Без прошлого состояния (первая установка) пары нет.
		let fresh = plan_downloads(&refs[..1], game, None).unwrap();
		assert_eq!(
			fresh.jobs[0].1.path,
			game.join("mods/sodium-fabric-0.6.0.jar")
		);
	}

	#[test]
	fn copy_stops_on_cancel() {
		let dir = tempfile::tempdir().unwrap();
		let src = dir.path().join("a.mrpack");
		std::fs::write(&src, vec![1u8; 3 * 1024 * 1024]).unwrap();
		let dst = dir.path().join("b.mrpack.tmp");
		copy_cancellable(&src, &dst, &CancellationToken::new()).unwrap();
		assert_eq!(std::fs::metadata(&dst).unwrap().len(), 3 * 1024 * 1024);
		let cancel = CancellationToken::new();
		cancel.cancel();
		let err = copy_cancellable(&src, &dst, &cancel).unwrap_err();
		assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
	}

	#[test]
	fn types_of_pack_files() {
		assert_eq!(type_of("mods/a.jar"), Some(ProjectType::Mod));
		assert_eq!(type_of("mods/A.JAR"), Some(ProjectType::Mod));
		assert_eq!(type_of("mods/sub/a.jar"), None);
		assert_eq!(type_of("mods/a.txt"), None);
		assert_eq!(type_of("config/a.toml"), None);
		assert_eq!(type_of("options.txt"), None);
		assert_eq!(
			type_of("resourcepacks/a.zip"),
			Some(ProjectType::Resourcepack)
		);
		assert_eq!(type_of("shaderpacks/a.zip"), Some(ProjectType::Shader));
		assert_eq!(type_of("datapacks/a.zip"), Some(ProjectType::Datapack));
	}

	#[test]
	fn names_and_stems() {
		assert_eq!(instance_name("  Pack\n "), "Pack");
		assert_eq!(instance_name(""), "Modpack");
		assert_eq!(instance_name(&"я".repeat(80)).chars().count(), 64);
		assert_eq!(
			stem("mods/sodium-fabric-0.5.8+mc1.20.1.jar"),
			stem("mods/sodium-fabric-0.6.0+mc1.21.jar")
		);
		assert_ne!(stem("mods/sodium-0.5.jar"), stem("mods/iris-0.5.jar"));
		assert_eq!(stem("mods/fabric-api-0.92.jar"), "mods/fabric-api");
	}

	#[test]
	fn preview_skips_existing_user_data() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path();
		std::fs::write(game.join("options.txt"), "mine").unwrap();
		let map = |items: &[(&str, &str)]| -> BTreeMap<String, String> {
			items
				.iter()
				.map(|(p, h)| ((*p).to_owned(), (*h).to_owned()))
				.collect()
		};
		let old = map(&[
			("options.txt", "1"),
			("Saves/w/level.dat", "1"),
			("mods/a.jar", "1"),
		]);
		let new = map(&[
			("options.txt", "2"),
			("servers.dat", "2"),
			("mods/a.jar", "2"),
		]);
		let plan = preview_plan(&old, &new, game);
		// `options.txt` есть — не обновится; `servers.dat` нет — добавится.
		assert_eq!(plan.updated, vec!["mods/a.jar".to_owned()]);
		assert_eq!(plan.added, vec!["servers.dat".to_owned()]);
		// Мир убран из модпака, но остаётся у игрока.
		assert!(plan.removed.is_empty());
	}
}
