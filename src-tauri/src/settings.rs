//! Настройки лаунчера: память и Java по умолчанию, источник загрузок, Discord. Файл `settings.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Deserializer, Serialize};

use crate::download::DownloadSource;
use crate::error::{AppError, ErrorCode};
use crate::fs_util;

const CURRENT_VERSION: u32 = 1;
/// Пределы памяти для игры, МБ.
pub const MEMORY_MIN_MB: u32 = 256;
pub const MEMORY_MAX_MB: u32 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
	pub min_mb: u32,
	pub max_mb: u32,
}

impl Memory {
	pub fn validate(self) -> Result<Self, AppError> {
		if self.min_mb < MEMORY_MIN_MB || self.max_mb > MEMORY_MAX_MB || self.min_mb > self.max_mb {
			return Err(AppError::coded(
				ErrorCode::InvalidValue,
				format!(
					"память: {}–{} МБ, нужно {MEMORY_MIN_MB} ≤ min ≤ max ≤ {MEMORY_MAX_MB}",
					self.min_mb, self.max_mb
				),
			));
		}
		Ok(self)
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
	#[serde(default)]
	pub version: u32,
	/// Память для сборок без своей настройки. `null` — автоматически (см. `system_memory`).
	#[serde(default)]
	pub default_memory: Option<Memory>,
	/// Выбранная Java по мажорной версии (`"17"` → путь к javaw.exe).
	#[serde(default)]
	pub java_paths: BTreeMap<String, String>,
	#[serde(default)]
	pub download_source: DownloadSource,
	/// Статус в Discord.
	#[serde(default = "yes")]
	pub discord_presence: bool,
	/// Показывать в статусе название сборки (иначе просто «Minecraft»).
	#[serde(default = "yes")]
	pub discord_show_instance: bool,
	/// Аргументы JVM против Java4Shell для Java 17+.
	#[serde(default = "yes")]
	pub java4shell_mitigation: bool,
	/// Пока в версии уязвимый FreeType: перед запуском снимать у серверов «принимать ресурспак автоматически».
	#[serde(default = "yes")]
	pub protect_server_packs: bool,
}

fn yes() -> bool {
	true
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			version: CURRENT_VERSION,
			default_memory: None,
			java_paths: BTreeMap::new(),
			download_source: DownloadSource::default(),
			discord_presence: true,
			discord_show_instance: true,
			java4shell_mitigation: true,
			protect_server_packs: true,
		}
	}
}

/// `Some(None)` — поле пришло как `null` (сбросить), `None` — поля не было (не менять).
pub fn nullable<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
	D: Deserializer<'de>,
	T: Deserialize<'de>,
{
	Option::<T>::deserialize(de).map(Some)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
	#[serde(default, deserialize_with = "nullable")]
	pub default_memory: Option<Option<Memory>>,
	/// Значение `null` убирает выбор для этой версии.
	#[serde(default)]
	pub java_paths: Option<BTreeMap<String, Option<String>>>,
	#[serde(default)]
	pub download_source: Option<DownloadSource>,
	#[serde(default)]
	pub discord_presence: Option<bool>,
	#[serde(default)]
	pub discord_show_instance: Option<bool>,
	#[serde(default)]
	pub java4shell_mitigation: Option<bool>,
	#[serde(default)]
	pub protect_server_packs: Option<bool>,
}

pub struct SettingsStore {
	path: PathBuf,
	value: Mutex<Settings>,
	writable: bool,
}

impl SettingsStore {
	pub fn load(path: PathBuf) -> Self {
		let (value, writable) = match fs_util::read_json_or_backup::<Settings>(&path) {
			Ok(value) => (value.unwrap_or_default(), true),
			Err(err) => {
				log::error!("не удалось прочитать {}: {err}", path.display());
				(Settings::default(), false)
			}
		};
		Self {
			path,
			value: Mutex::new(Settings {
				version: CURRENT_VERSION,
				..value
			}),
			writable,
		}
	}

	pub fn get(&self) -> Settings {
		self.value.lock().map(|v| v.clone()).unwrap_or_default()
	}

	pub fn update(&self, patch: SettingsPatch) -> Result<Settings, AppError> {
		if !self.writable {
			return Err(AppError::storage(format!(
				"{} только для чтения",
				self.path.display()
			)));
		}
		let mut guard = self
			.value
			.lock()
			.map_err(|_| AppError::unknown("настройки заблокированы"))?;
		let mut next = guard.clone();
		if let Some(memory) = patch.default_memory {
			next.default_memory = memory.map(Memory::validate).transpose()?;
		}
		if let Some(paths) = patch.java_paths {
			for (major, path) in paths {
				if major.parse::<u32>().is_err() {
					return Err(AppError::coded(
						ErrorCode::InvalidValue,
						format!("javaPaths: {major}"),
					));
				}
				match path {
					Some(path) => next.java_paths.insert(major, path),
					None => next.java_paths.remove(&major),
				};
			}
		}
		if let Some(source) = patch.download_source {
			next.download_source = source;
		}
		if let Some(on) = patch.discord_presence {
			next.discord_presence = on;
		}
		if let Some(on) = patch.discord_show_instance {
			next.discord_show_instance = on;
		}
		if let Some(on) = patch.java4shell_mitigation {
			next.java4shell_mitigation = on;
		}
		if let Some(on) = patch.protect_server_packs {
			next.protect_server_packs = on;
		}
		let json =
			serde_json::to_vec_pretty(&next).map_err(|e| AppError::storage(e.to_string()))?;
		fs_util::write_atomic(&self.path, &json)
			.map_err(|e| AppError::storage(format!("{}: {e}", self.path.display())))?;
		*guard = next.clone();
		Ok(next)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn patch_and_reload() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("settings.json");
		let store = SettingsStore::load(path.clone());
		assert_eq!(store.get(), Settings::default());

		let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
			"defaultMemory": { "minMb": 512, "maxMb": 4096 },
			"javaPaths": { "17": "C:\\java\\bin\\javaw.exe" },
			"downloadSource": "mirror"
		}))
		.unwrap();
		store.update(patch).unwrap();
		let reloaded = SettingsStore::load(path.clone()).get();
		assert_eq!(reloaded.default_memory.unwrap().max_mb, 4096);
		assert_eq!(reloaded.download_source, DownloadSource::Mirror);

		let clear: SettingsPatch = serde_json::from_value(serde_json::json!({
			"defaultMemory": null,
			"javaPaths": { "17": null }
		}))
		.unwrap();
		let next = store.update(clear).unwrap();
		assert!(next.default_memory.is_none());
		assert!(next.java_paths.is_empty());
		assert_eq!(next.download_source, DownloadSource::Mirror);

		let bad: SettingsPatch = serde_json::from_value(serde_json::json!({
			"defaultMemory": { "minMb": 4096, "maxMb": 1024 }
		}))
		.unwrap();
		assert_eq!(store.update(bad).unwrap_err().code, ErrorCode::InvalidValue);
	}

	#[test]
	fn discord_defaults_and_patch() {
		let old: Settings = serde_json::from_str(r#"{ "version": 1 }"#).unwrap();
		assert!(old.discord_presence && old.discord_show_instance);
		assert!(old.java4shell_mitigation);
		assert!(old.protect_server_packs);

		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("settings.json");
		let store = SettingsStore::load(path.clone());
		let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
			"discordShowInstance": false
		}))
		.unwrap();
		let next = store.update(patch).unwrap();
		assert!(next.discord_presence);
		assert!(!next.discord_show_instance);
		assert!(
			!SettingsStore::load(path.clone())
				.get()
				.discord_show_instance
		);
		let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
			"protectServerPacks": false
		}))
		.unwrap();
		assert!(!store.update(patch).unwrap().protect_server_packs);
		assert!(!SettingsStore::load(path).get().protect_server_packs);
	}

	#[test]
	fn corrupt_file_resets() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("settings.json");
		std::fs::write(&path, b"{oops").unwrap();
		let store = SettingsStore::load(path.clone());
		assert_eq!(store.get(), Settings::default());
		store.update(SettingsPatch::default()).unwrap();
		let backups = std::fs::read_dir(dir.path())
			.unwrap()
			.flatten()
			.filter(|e| {
				e.file_name()
					.to_string_lossy()
					.starts_with("settings.json.bak-")
			})
			.count();
		assert_eq!(backups, 1);
	}
}
