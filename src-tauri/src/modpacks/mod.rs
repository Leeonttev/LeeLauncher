//! Модпаки: установка и обновление, перенос из других лаунчеров, экспорт в `.mrpack`.

pub mod commands;
pub mod curseforge;
pub mod export;
pub mod install;
pub mod launchers;
#[cfg(test)]
mod live_tests;
pub mod mrpack;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::content::installed::ContentItem;
use crate::downloads::DownloadKind;
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::instances::Loader;

/// Сведения о модпаке в сборке: `instances/<id>/modpack.json`.
pub const STATE_FILE: &str = "modpack.json";
const STATE_VERSION: u32 = 1;

/// Откуда модпак сборки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackOrigin {
	/// Проект каталога Modrinth: можно проверять обновления.
	Modrinth,
	/// CurseForge.
	Curseforge,
	/// `.mrpack` с диска, которого нет в каталоге: обновить можно только новым файлом.
	File,
}

/// Связь сборки с модпаком (`Instance.modpack`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackLink {
	pub source: PackOrigin,
	#[serde(default)]
	pub project_id: Option<String>,
	#[serde(default)]
	pub version_id: Option<String>,
	pub title: String,
	#[serde(default)]
	pub version_number: String,
	#[serde(default)]
	pub icon_url: Option<String>,
	/// Когда поставлена или обновлена эта версия (Unix, мс).
	#[serde(default)]
	pub installed_at: u64,
}

pub fn now_ms() -> u64 {
	u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// Что делает задача модпака.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackAction {
	Install,
	Update,
	Import,
	Export,
}

/// Задача `modpack`/`import`/`export` в списке загрузок.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackTask {
	pub action: PackAction,
	pub title: String,
	pub icon_url: Option<String>,
	pub launcher: Option<launchers::LauncherKind>,
}

impl PackTask {
	pub fn kind(&self) -> DownloadKind {
		match self.action {
			PackAction::Install | PackAction::Update => DownloadKind::Modpack,
			PackAction::Import => DownloadKind::Import,
			PackAction::Export => DownloadKind::Export,
		}
	}
}

/// Что повторить (`download_retry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackRequest {
	Install {
		token: String,
		optional: Vec<String>,
	},
	Update {
		token: String,
		optional: Vec<String>,
	},
	Import {
		item: launchers::ImportItem,
		include: Vec<launchers::Category>,
	},
	Export {
		name: String,
		version: String,
		include: Vec<launchers::Category>,
		path: PathBuf,
	},
}

/// Версия игры и загрузчик модпака.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirements {
	pub game_version: String,
	pub loader: Loader,
}

/// Что модпак положил в сборку — для обновления и пометки «добавлен вами» (`modpack.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackState {
	#[serde(default)]
	pub version: u32,
	/// Файлы индекса: путь от папки игры → sha1.
	#[serde(default)]
	pub files: BTreeMap<String, String>,
	/// Файлы из `overrides`, как модпак их записал: путь → sha1.
	#[serde(default)]
	pub overrides: BTreeMap<String, String>,
	/// Необязательные файлы, которые пользователь не выбрал.
	#[serde(default)]
	pub excluded: Vec<String>,
}

pub fn state_path(instance_dir: &Path) -> PathBuf {
	instance_dir.join(STATE_FILE)
}

/// Нет файла (или он повреждён — копия рядом) — `None`.
pub fn load_state(instance_dir: &Path) -> Result<Option<PackState>, AppError> {
	let path = state_path(instance_dir);
	fs_util::read_json_or_backup(&path)
		.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))
}

pub fn save_state(instance_dir: &Path, state: &PackState) -> Result<(), AppError> {
	let mut state = state.clone();
	state.version = STATE_VERSION;
	let path = state_path(instance_dir);
	let json = serde_json::to_vec_pretty(&state).map_err(|e| AppError::storage(e.to_string()))?;
	fs_util::write_atomic(&path, &json)
		.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))
}

/// `ContentItem.fromModpack`: файл есть в индексе или `overrides` модпака сборки.
pub fn mark_from_modpack(instance_dir: &Path, items: &mut [ContentItem]) {
	let Ok(Some(state)) = load_state(instance_dir) else {
		return;
	};
	for item in items {
		item.from_modpack =
			state.files.contains_key(&item.key) || state.overrides.contains_key(&item.key);
	}
}

/// Разобранные модпаки этого запуска: токен (sha1 архива) → модпак.
#[derive(Default)]
pub struct PackCache {
	items: std::sync::Mutex<HashMap<String, install::Prepared>>,
}

impl PackCache {
	pub fn put(&self, prepared: install::Prepared) {
		if let Ok(mut items) = self.items.lock() {
			items.insert(prepared.token.clone(), prepared);
		}
	}

	pub fn get(&self, token: &str) -> Option<install::Prepared> {
		self.items.lock().ok()?.get(token).cloned()
	}
}

pub fn invalid(details: impl Into<String>) -> AppError {
	AppError::coded(ErrorCode::ModpackInvalid, details)
}

/// Путь из модпака: части через `/`, каждая — [`fs_util::is_safe_file_name`].
pub fn check_rel_path(rel: &str) -> Option<String> {
	if rel.is_empty() || rel.len() > 1024 || rel.contains('\\') {
		return None;
	}
	let parts: Vec<&str> = rel.split('/').collect();
	if parts.iter().all(|p| fs_util::is_safe_file_name(p)) {
		Some(parts.join("/"))
	} else {
		None
	}
}

/// Версия загрузчика из модпака или другого лаунчера: `1.20.1-47.2.0` → `47.2.0`.
pub fn loader_version(game_version: &str, version: &str) -> Result<String, AppError> {
	let version = version.trim();
	let version = version
		.strip_prefix(game_version)
		.and_then(|v| v.strip_prefix('-'))
		.unwrap_or(version);
	if fs_util::is_safe_id(version) {
		Ok(version.to_owned())
	} else {
		Err(invalid(format!("версия загрузчика {version:?}")))
	}
}

/// Свободное место на диске с этой папкой (ближайший существующий предок). `None` — не узнать.
pub fn free_space(path: &Path) -> Option<u64> {
	let mut dir = path;
	while !dir.exists() {
		dir = dir.parent()?;
	}
	free_space_of(dir)
}

#[cfg(windows)]
fn free_space_of(dir: &Path) -> Option<u64> {
	use std::os::windows::ffi::OsStrExt;
	use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
	let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
	let mut free = 0u64;
	// SAFETY: строка с нулём в конце живёт до конца вызова, указатели на выходные значения валидны.
	let ok = unsafe {
		GetDiskFreeSpaceExW(
			wide.as_ptr(),
			&mut free,
			std::ptr::null_mut(),
			std::ptr::null_mut(),
		)
	};
	(ok != 0).then_some(free)
}

#[cfg(not(windows))]
fn free_space_of(_dir: &Path) -> Option<u64> {
	None
}

/// Не хватает места: нужно `need` байт (с запасом 5 %), свободно меньше.
pub fn ensure_space(path: &Path, need: u64) -> Result<(), AppError> {
	let need = need.saturating_add(need / 20);
	match free_space(path) {
		Some(free) if free < need => Err(AppError::coded(
			ErrorCode::NotEnoughSpace,
			format!("нужно {need} байт, свободно {free}"),
		)),
		_ => Ok(()),
	}
}

/// Временные файлы моложе этого не трогаем: это могут быть идущие задачи.
const TMP_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// Архивы модпаков в кэше старше этого удаляются (нужны только на время установки и повтора).
const ARCHIVE_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

fn older_than(path: &Path, age: std::time::Duration, now: std::time::SystemTime) -> bool {
	std::fs::metadata(path)
		.and_then(|m| m.modified())
		.ok()
		.and_then(|t| now.duration_since(t).ok())
		.is_some_and(|a| a >= age)
}

/// Дочистка кэша модпаков и недописанных `*.pack-tmp`. Возвращает число удалённых.
pub fn cleanup(packs_dir: &Path, instances_dir: &Path) -> usize {
	cleanup_at(packs_dir, instances_dir, std::time::SystemTime::now())
}

fn cleanup_at(packs_dir: &Path, instances_dir: &Path, now: std::time::SystemTime) -> usize {
	let mut removed = 0;
	let mut remove = |path: &Path| match std::fs::remove_file(path) {
		Ok(()) => removed += 1,
		Err(err) => log::warn!("не удалось дочистить {}: {err}", path.display()),
	};
	if let Ok(entries) = std::fs::read_dir(packs_dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			if !entry.file_type().is_ok_and(|t| t.is_file()) {
				continue;
			}
			let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
			let stale = (name.ends_with(install::TMP_SUFFIX) && older_than(&path, TMP_AGE, now))
				|| (name.ends_with(".mrpack") && older_than(&path, ARCHIVE_AGE, now));
			if stale {
				remove(&path);
			}
		}
	}
	// В регионы миров не заходим: файлов там тысячи.
	let mut stack = vec![(instances_dir.to_path_buf(), 0usize, false)];
	while let Some((dir, depth, in_saves)) = stack.pop() {
		let Ok(entries) = std::fs::read_dir(&dir) else {
			continue;
		};
		for entry in entries.flatten() {
			let Ok(kind) = entry.file_type() else {
				continue;
			};
			let path = entry.path();
			if kind.is_dir() {
				let child = depth + 1;
				let saves =
					in_saves || (child == 3 && entry.file_name().eq_ignore_ascii_case("saves"));
				if !(saves && child > 4) {
					stack.push((path, child, saves));
				}
			} else if kind.is_file()
				&& entry
					.file_name()
					.to_string_lossy()
					.ends_with(mrpack::PACK_TMP_SUFFIX)
				&& older_than(&path, TMP_AGE, now)
			{
				remove(&path);
			}
		}
	}
	if removed > 0 {
		log::info!("дочищено временных файлов и старых архивов модпаков: {removed}");
	}
	removed
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cleanup_removes_only_old() {
		let dir = tempfile::tempdir().unwrap();
		let packs = dir.path().join("packs");
		let instances = dir.path().join("instances");
		let write = |p: std::path::PathBuf| {
			std::fs::create_dir_all(p.parent().unwrap()).unwrap();
			std::fs::write(&p, b"x").unwrap();
			p
		};
		let tmp = write(packs.join("a.mrpack.tmp"));
		let archive = write(packs.join("b.mrpack"));
		let other = write(packs.join("c.json"));
		let part = write(instances.join("abc/minecraft/config/x.toml.pack-tmp"));
		let config = write(instances.join("abc/minecraft/config/x.toml"));
		let now = std::time::SystemTime::now();
		// Всё свежее — ничего не удаляется.
		assert_eq!(cleanup_at(&packs, &instances, now), 0);
		// Через 2 дня: временные удалены, архив (младше 7 дней) — нет.
		let later = now + std::time::Duration::from_secs(2 * 24 * 60 * 60);
		assert_eq!(cleanup_at(&packs, &instances, later), 2);
		assert!(!tmp.exists() && !part.exists());
		assert!(archive.exists() && other.exists() && config.exists());
		let much_later = now + std::time::Duration::from_secs(8 * 24 * 60 * 60);
		assert_eq!(cleanup_at(&packs, &instances, much_later), 1);
		assert!(!archive.exists() && other.exists() && config.exists());
	}

	#[test]
	fn cleanup_skips_world_internals() {
		let dir = tempfile::tempdir().unwrap();
		let packs = dir.path().join("packs");
		let instances = dir.path().join("instances");
		let write = |rel: &str| {
			let p = instances.join(rel);
			std::fs::create_dir_all(p.parent().unwrap()).unwrap();
			std::fs::write(&p, b"x").unwrap();
			p
		};
		let world = write("abc/minecraft/saves/w/level.dat.pack-tmp");
		let region = write("abc/minecraft/saves/w/region/r.0.0.mca.pack-tmp");
		let deep = write("abc/minecraft/config/a/b/c.toml.pack-tmp");
		let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2 * 24 * 60 * 60);
		assert_eq!(cleanup_at(&packs, &instances, later), 2);
		assert!(!world.exists() && !deep.exists() && region.exists());
	}

	#[test]
	fn rel_paths_versions_state() {
		assert_eq!(check_rel_path("mods/a.jar").as_deref(), Some("mods/a.jar"));
		assert_eq!(
			check_rel_path("config/Мой мод/a b.toml").as_deref(),
			Some("config/Мой мод/a b.toml")
		);
		for bad in [
			"",
			"/abs",
			"a//b",
			"../a",
			"a/../b",
			"a/./b",
			r"a\b",
			"C:/x",
			"C:x",
			"mods/nul.jar",
			"a/b:c",
		] {
			assert!(check_rel_path(bad).is_none(), "{bad}");
		}
		assert_eq!(loader_version("1.20.1", "1.20.1-47.2.0").unwrap(), "47.2.0");
		assert_eq!(loader_version("1.20.1", "47.2.0").unwrap(), "47.2.0");
		assert!(loader_version("1.20.1", "../x").is_err());

		let state = PackState {
			files: BTreeMap::from([("mods/a.jar".into(), "x".into())]),
			overrides: BTreeMap::from([("config/c.toml".into(), "y".into())]),
			..PackState::default()
		};
		let dir = tempfile::tempdir().unwrap();
		assert!(load_state(dir.path()).unwrap().is_none());
		save_state(dir.path(), &state).unwrap();
		let loaded = load_state(dir.path()).unwrap().unwrap();
		assert_eq!(loaded.version, STATE_VERSION);
		assert_eq!(loaded.files, state.files);

		assert!(free_space(&dir.path().join("a/b/c")).is_some_and(|f| f > 0) || cfg!(not(windows)));
		assert_eq!(
			ensure_space(dir.path(), u64::MAX / 2).unwrap_err().code,
			ErrorCode::NotEnoughSpace
		);
	}
}
