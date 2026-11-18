//! Установленный контент и индекс `content.json`. Ключ — путь от папки игры без `.disabled`.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::{ProjectType, SourceKind};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;

pub const INDEX_FILE: &str = "content.json";
pub const DISABLED_SUFFIX: &str = ".disabled";
const INDEX_VERSION: u32 = 1;
/// Не определённые каталогом файлы проверяются снова не раньше чем через сутки.
pub const RECHECK_SECS: i64 = 24 * 60 * 60;

/// Откуда файл.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
	pub kind: SourceKind,
	pub project_id: String,
	pub version_id: String,
	#[serde(default)]
	pub title: Option<String>,
	#[serde(default)]
	pub icon_url: Option<String>,
	#[serde(default)]
	pub version_number: Option<String>,
	#[serde(default)]
	pub author: Option<String>,
	/// Slug проекта в его каталоге: по нему узнаётся тот же проект в другом каталоге.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub slug: Option<String>,
}

/// Найденное обновление (по последней проверке).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
	pub version_id: String,
	pub version_number: String,
	/// Новую версию можно скачать только на сайте каталога.
	#[serde(default)]
	pub site_only: bool,
	/// Найдено в другом каталоге (свой не ответил): ставится оттуда.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub via: Option<SourceKind>,
	/// Проект в каталоге `via`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub via_project_id: Option<String>,
}

impl UpdateInfo {
	/// Каталог, откуда ставится обновление.
	pub fn catalog(&self, source: &SourceRef) -> SourceKind {
		self.via.unwrap_or(source.kind)
	}
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
	pub sha1: String,
	pub size: u64,
	/// Время изменения файла, мс; вместе с размером решает, считать ли sha1 заново.
	pub modified: u64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub source: Option<SourceRef>,
	/// Когда файл последний раз пытались определить через каталог (Unix, с).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub checked_at: Option<i64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub update: Option<UpdateInfo>,
	/// Поставлен как зависимость другого проекта.
	#[serde(default)]
	pub dependency: bool,
	/// Отпечаток CurseForge (считается при опознании, когда Modrinth файл не узнал).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fingerprint: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
	#[serde(default)]
	pub version: u32,
	#[serde(default)]
	pub files: BTreeMap<String, Entry>,
}

/// Откуда файл — для интерфейса.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemProject {
	pub source: SourceKind,
	pub id: String,
	pub version_id: String,
	pub title: String,
	pub author: String,
	pub icon_url: Option<String>,
	pub version_number: String,
	/// Страница проекта на сайте каталога (для «Открыть на сайте», когда каталог выключен).
	pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemUpdate {
	pub version_id: String,
	pub number: String,
	pub site_only: bool,
	/// Обновление из другого каталога (`project.source` не ответил); `null` — из своего.
	pub via: Option<SourceKind>,
}

/// Файл контента для интерфейса. Файл в командах задаётся тройкой `{ file, type, world? }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentItem {
	/// Ключ индекса (путь от папки игры без `.disabled`).
	#[serde(skip)]
	pub key: String,
	/// Имя файла без `.disabled`.
	pub file: String,
	#[serde(rename = "type")]
	pub project_type: ProjectType,
	pub enabled: bool,
	pub size: u64,
	/// Датапак внутри мира (`saves/<мир>/datapacks`); нет — общая папка.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub world: Option<String>,
	/// `null` — файл добавлен вручную или каталог его не знает.
	pub project: Option<ItemProject>,
	pub update: Option<ItemUpdate>,
	/// Файл положил модпак сборки (`false` — добавил пользователь; у сборок без модпака всегда `false`).
	pub from_modpack: bool,
	#[serde(skip)]
	pub sha1: String,
}

impl ContentItem {
	fn new(f: &Found, entry: &Entry) -> Self {
		Self {
			key: f.key.clone(),
			file: f.key.rsplit('/').next().unwrap_or_default().to_owned(),
			project_type: f.project_type,
			enabled: f.enabled,
			size: f.size,
			world: f.world.clone(),
			project: entry.source.as_ref().map(|s| ItemProject {
				source: s.kind,
				id: s.project_id.clone(),
				version_id: s.version_id.clone(),
				title: s
					.title
					.clone()
					.unwrap_or_else(|| f.key.rsplit('/').next().unwrap_or_default().to_owned()),
				author: s.author.clone().unwrap_or_default(),
				icon_url: s.icon_url.clone(),
				version_number: s.version_number.clone().unwrap_or_default(),
				url: Some(super::project_url(s.kind, &s.project_id)),
			}),
			update: entry.update.as_ref().map(|u| ItemUpdate {
				version_id: u.version_id.clone(),
				number: u.version_number.clone(),
				site_only: u.site_only,
				via: u.via,
			}),
			from_modpack: false,
			sha1: entry.sha1.clone(),
		}
	}
}

/// Файл контента в запросе фронта.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
	/// Имя файла без `.disabled`.
	pub file: String,
	#[serde(rename = "type")]
	pub project_type: ProjectType,
	#[serde(default)]
	pub world: Option<String>,
}

impl FileRef {
	/// Ключ индекса с проверкой имени и мира.
	pub fn key(&self) -> Result<String, AppError> {
		let world = self
			.world
			.as_deref()
			.filter(|_| self.project_type == ProjectType::Datapack);
		let key = format!("{}/{}", folder_key(self.project_type, world)?, self.file);
		parse_key(&key)?;
		Ok(key)
	}
}

/// Файл, найденный в папках игры.
#[derive(Debug, Clone)]
pub struct Found {
	pub key: String,
	pub disk: PathBuf,
	pub project_type: ProjectType,
	pub world: Option<String>,
	pub enabled: bool,
	pub size: u64,
	pub modified: u64,
}

fn storage(path: &Path, err: impl std::fmt::Display) -> AppError {
	AppError::storage(format!("{}: {err}", path.display()))
}

pub fn now_secs() -> i64 {
	chrono::Utc::now().timestamp()
}

/// Ключ без `.disabled` → (ключ, включён ли).
pub fn split_disabled(name: &str) -> (&str, bool) {
	match name.strip_suffix(DISABLED_SUFFIX) {
		Some(base) => (base, false),
		None => (name, true),
	}
}

fn has_extension(name: &str, ext: &str) -> bool {
	name.rsplit_once('.')
		.is_some_and(|(base, e)| !base.is_empty() && e.eq_ignore_ascii_case(ext))
}

/// Папка игры, куда класть контент этого типа (для датапака в мир — `saves/<мир>/datapacks`).
pub fn folder_key(kind: ProjectType, world: Option<&str>) -> Result<String, AppError> {
	let folder = kind
		.folder()
		.ok_or_else(|| AppError::coded(ErrorCode::ContentIncompatible, "modpack"))?;
	match world {
		None => Ok(folder.to_owned()),
		Some(world) if kind == ProjectType::Datapack && fs_util::is_safe_file_name(world) => {
			Ok(format!("saves/{world}/datapacks"))
		}
		Some(world) => Err(AppError::coded(
			ErrorCode::InvalidValue,
			format!("world: {world}"),
		)),
	}
}

/// Путь на диске по ключу (с проверкой, что он внутри папки игры).
pub fn disk_path(game_dir: &Path, key: &str, enabled: bool) -> Result<PathBuf, AppError> {
	let rel = if enabled {
		key.to_owned()
	} else {
		format!("{key}{DISABLED_SUFFIX}")
	};
	fs_util::safe_join(game_dir, &rel)
		.ok_or_else(|| AppError::coded(ErrorCode::InvalidValue, format!("path: {key}")))
}

/// Проверяет ключ от фронта: тип по папке, безопасное имя.
pub fn parse_key(key: &str) -> Result<(ProjectType, Option<String>), AppError> {
	let bad = || AppError::coded(ErrorCode::InvalidValue, format!("path: {key}"));
	let parts: Vec<&str> = key.split('/').collect();
	let (kind, world, name) = match parts.as_slice() {
		[folder, name] => {
			let kind = ProjectType::INSTALLABLE
				.into_iter()
				.find(|k| k.folder() == Some(*folder))
				.ok_or_else(bad)?;
			(kind, None, *name)
		}
		["saves", world, "datapacks", name] if fs_util::is_safe_file_name(world) => {
			(ProjectType::Datapack, Some((*world).to_owned()), *name)
		}
		_ => return Err(bad()),
	};
	if !fs_util::is_safe_file_name(name) || name.ends_with(DISABLED_SUFFIX) {
		return Err(bad());
	}
	Ok((kind, world))
}

fn scan_dir(
	game_dir: &Path,
	folder: &str,
	kind: ProjectType,
	world: Option<&str>,
	out: &mut Vec<Found>,
) {
	let dir = game_dir.join(folder);
	let Ok(entries) = std::fs::read_dir(&dir) else {
		return;
	};
	for entry in entries.flatten() {
		let Ok(meta) = entry.metadata() else { continue };
		if !meta.is_file() {
			continue;
		}
		let file_name = entry.file_name().to_string_lossy().into_owned();
		let (base, enabled) = split_disabled(&file_name);
		if !has_extension(base, kind.extension()) || !fs_util::is_safe_file_name(base) {
			continue;
		}
		let modified = meta
			.modified()
			.ok()
			.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
			.map_or(0, |d| d.as_millis() as u64);
		out.push(Found {
			key: format!("{folder}/{base}"),
			disk: entry.path(),
			project_type: kind,
			world: world.map(str::to_owned),
			enabled,
			size: meta.len(),
			modified,
		});
	}
}

/// Все файлы контента в папке игры: `mods/*.jar`, `resourcepacks|shaderpacks|datapacks/*.zip` и
/// `saves/*/datapacks/*.zip` (с `.disabled` и без). Папки (распакованные паки) не считаются.
pub fn scan(game_dir: &Path) -> Vec<Found> {
	let mut out = Vec::new();
	for kind in ProjectType::INSTALLABLE {
		if let Some(folder) = kind.folder() {
			scan_dir(game_dir, folder, kind, None, &mut out);
		}
	}
	if let Ok(worlds) = std::fs::read_dir(game_dir.join("saves")) {
		for world in worlds.flatten() {
			let name = world.file_name().to_string_lossy().into_owned();
			if world.path().is_dir() && fs_util::is_safe_file_name(&name) {
				let folder = format!("saves/{name}/datapacks");
				scan_dir(
					game_dir,
					&folder,
					ProjectType::Datapack,
					Some(&name),
					&mut out,
				);
			}
		}
	}
	// Один и тот же файл включённым и выключенным одновременно — берём включённый.
	out.sort_by(|a, b| a.key.cmp(&b.key).then(b.enabled.cmp(&a.enabled)));
	out.dedup_by(|b, a| a.key == b.key);
	out
}

/// Описания мода в `.jar` больше этого не читаем.
const MOD_META_MAX: u64 = 1024 * 1024;

/// id модов внутри `.jar` (Fabric, Quilt, Forge, NeoForge) — чтобы не поставить мод дважды
/// из разных каталогов.
pub fn mod_ids(path: &Path) -> Vec<String> {
	let Ok(file) = std::fs::File::open(path) else {
		return Vec::new();
	};
	let Ok(mut zip) = zip::ZipArchive::new(file) else {
		return Vec::new();
	};
	let mut read = |name: &str| -> Option<String> {
		let entry = zip.by_name(name).ok()?;
		let mut text = String::new();
		entry.take(MOD_META_MAX).read_to_string(&mut text).ok()?;
		Some(text)
	};
	let mut ids = Vec::new();
	if let Some(text) = read("fabric.mod.json")
		&& let Ok(json) =
			serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
		&& let Some(id) = json["id"].as_str()
	{
		ids.push(id.to_owned());
	}
	if let Some(text) = read("quilt.mod.json")
		&& let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
		&& let Some(id) = json["quilt_loader"]["id"].as_str()
	{
		ids.push(id.to_owned());
	}
	for name in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
		if let Some(text) = read(name) {
			ids.extend(toml_mod_ids(&text));
		}
	}
	ids.sort();
	ids.dedup();
	ids
}

/// `modId = "x"` в разделах `[[mods]]` (в `[[dependencies.*]]` тоже есть `modId` — их пропускаем).
fn toml_mod_ids(text: &str) -> Vec<String> {
	let mut in_mods = false;
	let mut ids = Vec::new();
	for line in text.lines() {
		let line = line.trim();
		if line.starts_with('[') {
			// `[[mods]] #mandatory` — комментарий после заголовка.
			let header = line.split('#').next().unwrap_or_default();
			in_mods = header.split_whitespace().collect::<String>() == "[[mods]]";
			continue;
		}
		if !in_mods {
			continue;
		}
		let Some(rest) = line.strip_prefix("modId") else {
			continue;
		};
		let Some(value) = rest.trim_start().strip_prefix('=') else {
			continue;
		};
		let value = value.trim();
		let quote = value.chars().next();
		if let Some(q @ ('"' | '\'')) = quote
			&& let Some(end) = value[1..].find(q)
		{
			let id = &value[1..=end];
			if !id.is_empty() {
				ids.push(id.to_owned());
			}
		}
	}
	ids
}

pub fn sha1_file(path: &Path) -> std::io::Result<String> {
	let mut file = std::fs::File::open(path)?;
	let mut hasher = <sha1::Sha1 as sha1::Digest>::new();
	std::io::copy(&mut file, &mut hasher)?;
	Ok(crate::download::to_hex(&sha1::Digest::finalize(hasher)))
}

/// Файл индекса сборки.
pub fn index_path(instance_dir: &Path) -> PathBuf {
	instance_dir.join(INDEX_FILE)
}

/// Читает индекс. Нет файла или он повреждён (копия рядом) — пустой; не прочитался — `storage`.
pub fn load_index(path: &Path) -> Result<Index, AppError> {
	fs_util::read_json_or_backup::<Index>(path)
		.map(Option::unwrap_or_default)
		.map_err(|e| storage(path, e))
}

pub fn save_index(path: &Path, index: &Index) -> Result<(), AppError> {
	let mut index = index.clone();
	index.version = INDEX_VERSION;
	let json = serde_json::to_vec_pretty(&index).map_err(|e| AppError::storage(e.to_string()))?;
	fs_util::write_atomic(path, &json).map_err(|e| storage(path, e))
}

/// Сверяет индекс с папками. Долго (хеши) — вызывать в `spawn_blocking`.
pub fn sync(game_dir: &Path, index: &mut Index) -> (Vec<ContentItem>, bool) {
	let found = scan(game_dir);
	let mut changed = false;
	let mut seen = HashMap::with_capacity(found.len());
	let mut list = Vec::with_capacity(found.len());
	for f in found {
		let entry = match index.files.get(&f.key) {
			Some(e) if e.size == f.size && e.modified == f.modified && !e.sha1.is_empty() => {
				e.clone()
			}
			old => {
				let sha1 = match sha1_file(&f.disk) {
					Ok(sha1) => sha1,
					Err(err) => {
						log::warn!("не удалось прочитать {}: {err}", f.disk.display());
						continue;
					}
				};
				changed = true;
				match old {
					// Тот же файл (например, вернули дату изменения) — сведения сохраняем.
					Some(e) if e.sha1 == sha1 => Entry {
						size: f.size,
						modified: f.modified,
						..e.clone()
					},
					_ => Entry {
						sha1,
						size: f.size,
						modified: f.modified,
						..Entry::default()
					},
				}
			}
		};
		list.push(ContentItem::new(&f, &entry));
		seen.insert(f.key, entry);
	}
	if seen.len() != index.files.len() || seen.iter().any(|(k, e)| index.files.get(k) != Some(e)) {
		changed = true;
	}
	index.files = seen.into_iter().collect();
	(list, changed)
}

/// Файлы, которые стоит определить через каталог: без источника и давно не проверялись.
pub fn unidentified(index: &Index, now: i64) -> Vec<String> {
	let mut sha1s: Vec<String> = index
		.files
		.values()
		.filter(|e| e.source.is_none())
		.filter(|e| e.checked_at.is_none_or(|t| now - t >= RECHECK_SECS))
		.map(|e| e.sha1.clone())
		.collect();
	sha1s.sort();
	sha1s.dedup();
	sha1s
}

/// Что за файл при добавлении с диска.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detected {
	Kind(ProjectType),
	/// `.zip`, но по содержимому не понять: ресурспак, шейдер или датапак.
	NeedsType,
	/// Модпак Modrinth (`.mrpack` или `.zip` с `modrinth.index.json`): ставится отдельно новой сборкой.
	Modpack,
	/// Модпак CurseForge (`.zip` с `manifest.json`): пока не поддерживается.
	CurseforgeModpack,
	/// Не `.jar`/`.zip`.
	Unsupported,
}

/// Тип файла для импорта: `.jar` — мод; `.zip` — по содержимому (`shaders/` — шейдер,
/// `pack.mcmeta` + `data/` без `assets/` — датапак, `pack.mcmeta` — ресурспак).
pub fn detect_type(path: &Path) -> Detected {
	let name = path
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_default();
	if has_extension(&name, "jar") {
		return Detected::Kind(ProjectType::Mod);
	}
	if has_extension(&name, "mrpack") {
		return Detected::Modpack;
	}
	if !has_extension(&name, "zip") {
		return Detected::Unsupported;
	}
	let Ok(file) = std::fs::File::open(path) else {
		return Detected::Unsupported;
	};
	let Ok(mut zip) = zip::ZipArchive::new(file) else {
		return Detected::Unsupported;
	};
	let (mut mcmeta, mut data, mut assets, mut shaders, mut modpack, mut curseforge) =
		(false, false, false, false, false, false);
	for i in 0..zip.len() {
		let Ok(entry) = zip.by_index_raw(i) else {
			continue;
		};
		let entry_name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
		let top = entry_name.split('/').next().unwrap_or_default();
		match top {
			"pack.mcmeta" => mcmeta = true,
			"data" => data = true,
			"assets" => assets = true,
			"shaders" => shaders = true,
			"modrinth.index.json" => modpack = true,
			"manifest.json" => curseforge = true,
			_ => {}
		}
	}
	if shaders {
		Detected::Kind(ProjectType::Shader)
	} else if mcmeta && data && !assets {
		Detected::Kind(ProjectType::Datapack)
	} else if mcmeta && assets && !data {
		Detected::Kind(ProjectType::Resourcepack)
	} else if modpack && !mcmeta {
		Detected::Modpack
	} else if curseforge && !mcmeta {
		Detected::CurseforgeModpack
	} else {
		Detected::NeedsType
	}
}

/// Копирует файл в папку игры; одноимённый старый — в корзину. Возвращает, был ли он выключен.
pub fn import_file(src: &Path, dest: &Path) -> Result<bool, AppError> {
	import_file_checked(src, dest, None)
}

/// [`import_file`] со сверкой sha1 самой копии: файл могли подменить после проверки.
pub fn import_file_checked(src: &Path, dest: &Path, sha1: Option<&str>) -> Result<bool, AppError> {
	let disabled = PathBuf::from(format!("{}{DISABLED_SUFFIX}", dest.display()));
	let dir = dest.parent().ok_or_else(|| storage(dest, "нет папки"))?;
	std::fs::create_dir_all(dir).map_err(|e| storage(dir, e))?;
	let tmp = PathBuf::from(format!("{}.import-tmp", dest.display()));
	copy_checked(src, &tmp, sha1)?;
	let was_disabled = disabled.exists() && !dest.exists();
	for old in [dest, disabled.as_path()] {
		if old.exists()
			&& let Err(err) = crate::trash::delete(old)
		{
			let _ = std::fs::remove_file(&tmp);
			return Err(io_error(old, &err));
		}
	}
	let target = if was_disabled { &disabled } else { dest };
	std::fs::rename(&tmp, target).map_err(|e| {
		let _ = std::fs::remove_file(&tmp);
		io_error(target, &e)
	})?;
	Ok(was_disabled)
}

/// Копирует `src` в `tmp`, считая sha1 по ходу. При ошибке `tmp` удаляется.
pub fn copy_checked(src: &Path, tmp: &Path, sha1: Option<&str>) -> Result<(), AppError> {
	let copied = (|| {
		let mut from = std::fs::File::open(src).map_err(|e| storage(src, e))?;
		let mut to = std::fs::File::create(tmp).map_err(|e| storage(tmp, e))?;
		let mut hasher = <sha1::Sha1 as sha1::Digest>::new();
		let mut buf = vec![0u8; 256 * 1024];
		loop {
			let n = from.read(&mut buf).map_err(|e| storage(src, e))?;
			if n == 0 {
				break;
			}
			sha1::Digest::update(&mut hasher, &buf[..n]);
			std::io::Write::write_all(&mut to, &buf[..n]).map_err(|e| storage(tmp, e))?;
		}
		to.sync_all().map_err(|e| storage(tmp, e))?;
		let got = crate::download::to_hex(&sha1::Digest::finalize(hasher));
		match sha1 {
			Some(want) if !want.eq_ignore_ascii_case(&got) => Err(AppError::coded(
				ErrorCode::FingerprintMismatch,
				src.file_name()
					.map(|n| n.to_string_lossy().into_owned())
					.unwrap_or_default(),
			)),
			_ => Ok(()),
		}
	})();
	if copied.is_err() {
		let _ = std::fs::remove_file(tmp);
	}
	copied
}

/// Ошибка файла: занят другим процессом (Windows, ERROR_SHARING_VIOLATION / LOCK_VIOLATION) —
/// `file_in_use`, иначе `storage`.
pub fn io_error(path: &Path, err: &std::io::Error) -> AppError {
	if matches!(err.raw_os_error(), Some(32 | 33)) {
		AppError::coded(ErrorCode::FileInUse, path.display().to_string())
	} else {
		storage(path, err)
	}
}

/// Замки индекса по сборкам и отметки «идёт опознание».
#[derive(Default)]
pub struct ContentLocks {
	locks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
	identifying: std::sync::Mutex<std::collections::HashSet<String>>,
	/// Когда последний раз проверяли обновления сборки.
	updates_checked: std::sync::Mutex<HashMap<String, std::time::Instant>>,
	/// Последнее определение файлов или проверка обновлений не дошли до каталога (нет сети).
	offline: std::sync::Mutex<HashMap<String, bool>>,
	/// Каталоги, которые не ответили при последней проверке (остальные ответили).
	failed: std::sync::Mutex<HashMap<String, Vec<super::SourceFailure>>>,
}

impl ContentLocks {
	pub async fn lock(&self, instance_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
		let lock = self
			.locks
			.lock()
			.map(|mut m| m.entry(instance_id.to_owned()).or_default().clone())
			.unwrap_or_default();
		lock.lock_owned().await
	}

	/// `true` — определение для сборки начато этим вызовом; потом обязательно [`Self::end_identify`].
	pub fn begin_identify(&self, instance_id: &str) -> bool {
		self.identifying
			.lock()
			.is_ok_and(|mut s| s.insert(instance_id.to_owned()))
	}

	pub fn end_identify(&self, instance_id: &str) {
		if let Ok(mut s) = self.identifying.lock() {
			s.remove(instance_id);
		}
	}

	/// Обновления проверялись не раньше `within` назад.
	pub fn updates_fresh(&self, instance_id: &str, within: std::time::Duration) -> bool {
		self.updates_checked
			.lock()
			.ok()
			.and_then(|m| m.get(instance_id).copied())
			.is_some_and(|t| t.elapsed() < within)
	}

	pub fn mark_updates_checked(&self, instance_id: &str) {
		if let Ok(mut m) = self.updates_checked.lock() {
			m.insert(instance_id.to_owned(), std::time::Instant::now());
		}
	}

	pub fn set_offline(&self, instance_id: &str, offline: bool) {
		if let Ok(mut m) = self.offline.lock() {
			m.insert(instance_id.to_owned(), offline);
		}
	}

	/// `true` — список изменился.
	pub fn set_failed(&self, instance_id: &str, failed: Vec<super::SourceFailure>) -> bool {
		self.failed.lock().is_ok_and(|mut m| {
			let old = m.insert(instance_id.to_owned(), failed.clone());
			old.unwrap_or_default() != failed
		})
	}

	pub fn failed(&self, instance_id: &str) -> Vec<super::SourceFailure> {
		self.failed
			.lock()
			.ok()
			.and_then(|m| m.get(instance_id).cloned())
			.unwrap_or_default()
	}

	pub fn is_offline(&self, instance_id: &str) -> bool {
		self.offline
			.lock()
			.ok()
			.and_then(|m| m.get(instance_id).copied())
			.unwrap_or(false)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;

	fn touch(path: &Path, data: &[u8]) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, data).unwrap();
	}

	#[test]
	fn reads_mod_ids() {
		let dir = tempfile::tempdir().unwrap();
		let jar = dir.path().join("m.jar");
		let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
		let opts = zip::write::SimpleFileOptions::default();
		zip.start_file("fabric.mod.json", opts).unwrap();
		zip.write_all(br#"{ "schemaVersion": 1, "id": "fabric-api" }"#)
			.unwrap();
		zip.start_file("META-INF/neoforge.mods.toml", opts).unwrap();
		zip.write_all(
			b"modLoader=\"javafml\"\n[[mods]]\n  modId = \"jei\"\n[[dependencies.jei]]\n  modId=\"neoforge\"\n[[ mods ]] #mandatory\nmodId='jei_extra'\n",
		)
		.unwrap();
		zip.finish().unwrap();
		assert_eq!(mod_ids(&jar), vec!["fabric-api", "jei", "jei_extra"]);
		assert!(mod_ids(&dir.path().join("none.jar")).is_empty());
	}

	#[test]
	fn keys_and_paths() {
		assert_eq!(split_disabled("a.jar.disabled"), ("a.jar", false));
		assert_eq!(split_disabled("a.jar"), ("a.jar", true));
		assert_eq!(parse_key("mods/a.jar").unwrap(), (ProjectType::Mod, None));
		assert_eq!(
			parse_key("saves/Мир/datapacks/p.zip").unwrap(),
			(ProjectType::Datapack, Some("Мир".into()))
		);
		for bad in [
			"mods/../x.jar",
			"config/a.jar",
			"mods/a/b.jar",
			"mods/a.jar.disabled",
			"saves/../datapacks/p.zip",
			"mods",
		] {
			assert!(parse_key(bad).is_err(), "{bad}");
		}
		assert_eq!(
			folder_key(ProjectType::Shader, None).unwrap(),
			"shaderpacks"
		);
		assert_eq!(
			folder_key(ProjectType::Datapack, Some("W")).unwrap(),
			"saves/W/datapacks"
		);
		assert!(folder_key(ProjectType::Mod, Some("W")).is_err());
		assert!(folder_key(ProjectType::Datapack, Some("..")).is_err());
		assert!(folder_key(ProjectType::Modpack, None).is_err());
		let game = Path::new("g");
		assert_eq!(
			disk_path(game, "mods/a.jar", false).unwrap(),
			game.join("mods/a.jar.disabled")
		);
	}

	#[test]
	fn scan_and_sync() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path();
		touch(&game.join("mods/a.jar"), b"abc");
		touch(&game.join("mods/b.jar.disabled"), b"b");
		touch(&game.join("mods/notes.txt"), b"x");
		touch(&game.join("shaderpacks/s.zip"), b"s");
		touch(&game.join("shaderpacks/s.zip.txt"), b"iris config");
		touch(&game.join("resourcepacks/r.zip"), b"r");
		touch(&game.join("saves/W/datapacks/d.zip"), b"d");
		std::fs::create_dir_all(game.join("resourcepacks/unpacked")).unwrap();

		let mut index = Index::default();
		index.files.insert(
			"mods/gone.jar".into(),
			Entry {
				sha1: "x".into(),
				..Entry::default()
			},
		);
		let (list, changed) = sync(game, &mut index);
		assert!(changed);
		let all: Vec<&str> = list.iter().map(|c| c.key.as_str()).collect();
		assert_eq!(
			all,
			vec![
				"mods/a.jar",
				"mods/b.jar",
				"resourcepacks/r.zip",
				"saves/W/datapacks/d.zip",
				"shaderpacks/s.zip"
			]
		);
		let keys: Vec<&str> = list.iter().map(|c| c.key.as_str()).collect();
		assert_eq!(keys[1], "mods/b.jar");
		let a = &list[0];
		assert_eq!(a.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
		assert!(a.enabled && !list[1].enabled);
		assert_eq!(list[1].file, "b.jar");
		assert_eq!(list[3].world.as_deref(), Some("W"));
		assert!(!index.files.contains_key("mods/gone.jar"));
		let json = serde_json::to_value(a).unwrap();
		assert_eq!(json["type"], "mod");
		assert_eq!(json["file"], "a.jar");
		assert!(json.get("world").is_none() && json.get("key").is_none());
		assert!(json["project"].is_null());

		// Повторно ничего не меняется; сведения об источнике сохраняются.
		index.files.get_mut("mods/a.jar").unwrap().source = Some(SourceRef {
			kind: SourceKind::Modrinth,
			project_id: "p".into(),
			version_id: "v".into(),
			title: None,
			icon_url: None,
			version_number: None,
			author: None,
			slug: None,
		});
		let (list, changed) = sync(game, &mut index);
		assert!(!changed);
		assert_eq!(list[0].project.as_ref().unwrap().title, "a.jar");
		assert_eq!(unidentified(&index, 0).len(), 4);
		index.files.get_mut("mods/b.jar").unwrap().checked_at = Some(100);
		assert_eq!(unidentified(&index, 100 + RECHECK_SECS - 1).len(), 3);

		// Файл заменили — сведения сбрасываются.
		std::thread::sleep(std::time::Duration::from_millis(20));
		touch(&game.join("mods/a.jar"), b"other");
		let (list, changed) = sync(game, &mut index);
		assert!(changed);
		assert!(list[0].project.is_none());

		let r = FileRef {
			file: "p.zip".into(),
			project_type: ProjectType::Datapack,
			world: Some("W".into()),
		};
		assert_eq!(r.key().unwrap(), "saves/W/datapacks/p.zip");
		let r = FileRef {
			file: "../x.jar".into(),
			project_type: ProjectType::Mod,
			world: None,
		};
		assert!(r.key().is_err());

		let path = index_path(game);
		save_index(&path, &index).unwrap();
		let loaded = load_index(&path).unwrap();
		assert_eq!(loaded.version, INDEX_VERSION);
		assert_eq!(loaded.files, index.files);
		std::fs::write(&path, b"{broken").unwrap();
		assert!(load_index(&path).unwrap().files.is_empty());
	}

	fn zip_with(path: &Path, names: &[&str]) {
		let file = std::fs::File::create(path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		for name in names {
			zip.start_file(*name, zip::write::SimpleFileOptions::default())
				.unwrap();
			zip.write_all(b"x").unwrap();
		}
		zip.finish().unwrap();
	}

	#[test]
	fn detects_and_imports() {
		let dir = tempfile::tempdir().unwrap();
		let p = |n: &str| dir.path().join(n);
		touch(&p("m.jar"), b"jar");
		assert_eq!(detect_type(&p("m.jar")), Detected::Kind(ProjectType::Mod));
		zip_with(&p("s.zip"), &["shaders/final.fsh"]);
		assert_eq!(
			detect_type(&p("s.zip")),
			Detected::Kind(ProjectType::Shader)
		);
		zip_with(
			&p("d.zip"),
			&["pack.mcmeta", "data/x/functions/a.mcfunction"],
		);
		assert_eq!(
			detect_type(&p("d.zip")),
			Detected::Kind(ProjectType::Datapack)
		);
		zip_with(&p("r.zip"), &["pack.mcmeta", "assets/minecraft/x.png"]);
		assert_eq!(
			detect_type(&p("r.zip")),
			Detected::Kind(ProjectType::Resourcepack)
		);
		zip_with(&p("both.zip"), &["pack.mcmeta", "assets/a", "data/b"]);
		assert_eq!(detect_type(&p("both.zip")), Detected::NeedsType);
		zip_with(&p("x.zip"), &["readme.txt"]);
		assert_eq!(detect_type(&p("x.zip")), Detected::NeedsType);
		zip_with(&p("cf.zip"), &["manifest.json", "overrides/a"]);
		assert_eq!(detect_type(&p("cf.zip")), Detected::CurseforgeModpack);
		zip_with(&p("mr.zip"), &["modrinth.index.json", "overrides/a"]);
		assert_eq!(detect_type(&p("mr.zip")), Detected::Modpack);
		touch(&p("pack.mrpack"), b"x");
		assert_eq!(detect_type(&p("pack.mrpack")), Detected::Modpack);
		touch(&p("broken.zip"), b"not a zip");
		assert_eq!(detect_type(&p("broken.zip")), Detected::Unsupported);
		touch(&p("a.txt"), b"x");
		assert_eq!(detect_type(&p("a.txt")), Detected::Unsupported);

		let dest = p("game/mods/m.jar");
		assert!(!import_file(&p("m.jar"), &dest).unwrap());
		assert_eq!(std::fs::read(&dest).unwrap(), b"jar");
		assert!(!p("game/mods/m.jar.import-tmp").exists());
	}

	#[test]
	fn checked_import_rejects_other_sha1() {
		let dir = tempfile::tempdir().unwrap();
		let src = dir.path().join("abc.jar");
		std::fs::write(&src, b"abc").unwrap();
		let dest = dir.path().join("game/mods/abc.jar");
		let sha1_abc = "a9993e364706816aba3e25717850c26c9cd0d89d";
		let err = import_file_checked(&src, &dest, Some(&"0".repeat(40))).unwrap_err();
		assert_eq!(err.code, ErrorCode::FingerprintMismatch);
		assert_eq!(err.details.as_deref(), Some("abc.jar"));
		assert!(!dest.exists());
		assert!(!dir.path().join("game/mods/abc.jar.import-tmp").exists());
		// Верный sha1 (регистр не важен) — файл на месте.
		assert!(!import_file_checked(&src, &dest, Some(&sha1_abc.to_uppercase())).unwrap());
		assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
		// Отдельная копия для мира: при несовпадении временный файл не остаётся.
		let tmp = dir.path().join("w.zip");
		assert!(copy_checked(&src, &tmp, Some(&"1".repeat(40))).is_err());
		assert!(!tmp.exists());
		copy_checked(&src, &tmp, Some(sha1_abc)).unwrap();
		assert!(tmp.is_file());
	}
}
