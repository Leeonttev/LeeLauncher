//! Свежие описания версий установленных сборок и признак уязвимого FreeType.
//! Mojang переписывает описания уже вышедших версий (так закрыли ошибку FreeType в
//! `lwjgl-freetype`), а объединённое описание сборки лежит на диске со времени установки.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::SystemTime;

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use super::{InstallState, Instance};
use crate::launcher::Launcher;
use crate::meta::LoaderKind;
use crate::paths::Layout;

/// Сборки `org.lwjgl:lwjgl-freetype` с FreeType до 2.15.0 (выполнение кода через шрифт TTF в ресурспаке).
const VULNERABLE_FREETYPE: [&str; 6] = ["3.3.2", "3.3.3", "3.3.6", "3.4.1", "3.4.2", "3.4.3"];

#[derive(Deserialize)]
struct Libraries {
	#[serde(default)]
	libraries: Vec<LibraryName>,
}

#[derive(Deserialize)]
struct LibraryName {
	name: String,
}

/// Есть ли в библиотеках уязвимый `lwjgl-freetype` (версия совпадает точно).
pub fn has_vulnerable_freetype<'a>(names: impl IntoIterator<Item = &'a str>) -> bool {
	names.into_iter().any(|name| {
		let mut parts = name.split(':');
		parts.next() == Some("org.lwjgl")
			&& parts.next() == Some("lwjgl-freetype")
			&& parts
				.next()
				.is_some_and(|v| VULNERABLE_FREETYPE.contains(&v))
	})
}

fn file_vulnerable(path: &Path) -> bool {
	std::fs::read(path)
		.ok()
		.and_then(|bytes| serde_json::from_slice::<Libraries>(&bytes).ok())
		.is_some_and(|l| has_vulnerable_freetype(l.libraries.iter().map(|l| l.name.as_str())))
}

/// Время изменения и длина файла, по которым посчитан признак.
type Stamp = (Option<SystemTime>, u64);

/// Признак по описанию версии на диске; пересчитывается, когда файл изменился.
#[derive(Default)]
pub struct FontCache {
	known: std::sync::Mutex<HashMap<String, (Stamp, bool)>>,
}

impl FontCache {
	pub fn vulnerable(&self, layout: &Layout, version_id: &str) -> bool {
		if !crate::fs_util::is_safe_id(version_id) {
			return false;
		}
		let path = layout.version_json(version_id);
		let Ok(meta) = std::fs::metadata(&path) else {
			return false;
		};
		let stamp = (meta.modified().ok(), meta.len());
		if let Ok(known) = self.known.lock()
			&& let Some(&(known_stamp, value)) = known.get(version_id)
			&& known_stamp == stamp
		{
			return value;
		}
		let value = file_vulnerable(&path);
		if let Ok(mut known) = self.known.lock() {
			known.insert(version_id.to_owned(), (stamp, value));
		}
		value
	}
}

fn modified(path: &Path) -> Option<SystemTime> {
	std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Сверяет описания версий установленных сборок с манифестом Mojang и обновляет изменившиеся
/// (с загрузчиком — пересобирает объединённое). Библиотеки докачает запуск. Без сети ничего не делает.
/// Возвращает, изменилось ли хоть одно описание.
pub async fn refresh_installed(l: &Launcher) -> bool {
	let cancel = CancellationToken::new();
	let installed: Vec<Instance> = l
		.instances
		.list()
		.await
		.into_iter()
		.filter(|i| i.install_state == InstallState::Installed && i.installed.is_some())
		.collect();
	if installed.is_empty() {
		return false;
	}
	let manifest = match l.meta.version_manifest_fetched(true, &cancel).await {
		Ok(fetched) if !fetched.offline => fetched.value,
		Ok(_) => {
			log::info!("описания версий не проверены: нет сети");
			return false;
		}
		Err(err) => {
			log::info!(
				"описания версий не проверены: {:?} {}",
				err.code,
				err.details.as_deref().unwrap_or("")
			);
			return false;
		}
	};
	// Установка тоже пишет описания версий: не пересекаемся с ней.
	let _install = l.install_lock.lock().await;

	let mut changed = false;
	let mut base_changed: HashMap<String, bool> = HashMap::new();
	let mut done: HashSet<String> = HashSet::new();
	for instance in &installed {
		let Some(installed) = &instance.installed else {
			continue;
		};
		let game = &instance.game_version;
		let Some(entry) = manifest.versions.iter().find(|v| &v.id == game) else {
			continue;
		};
		if crate::meta::check_id("версия игры", &entry.id).is_err() {
			continue;
		}
		let fresh = match base_changed.get(game) {
			Some(&fresh) => fresh,
			None => {
				let fresh = match l.meta.refresh_version_info(entry, &cancel).await {
					Ok(fresh) => fresh,
					Err(err) => {
						log::warn!(
							"описание {game} не обновлено: {:?} {}",
							err.code,
							err.details.as_deref().unwrap_or("")
						);
						false
					}
				};
				if fresh {
					log::info!("описание версии {game} обновлено Mojang, скачано заново");
					changed = true;
				}
				base_changed.insert(game.clone(), fresh);
				fresh
			}
		};
		if instance.loader.kind == LoaderKind::Vanilla
			|| !done.insert(installed.version_id.clone())
			|| crate::meta::check_id("версия", &installed.version_id).is_err()
		{
			continue;
		}
		let Some(loader_version) = instance.loader.version.as_deref() else {
			continue;
		};
		let merged = l.layout.version_json(&installed.version_id);
		let stale = fresh
			|| match (modified(&merged), modified(&l.layout.version_json(game))) {
				(Some(merged), Some(base)) => merged < base,
				_ => false,
			};
		if !stale {
			continue;
		}
		match l
			.meta
			.resolve(
				game,
				instance.loader.kind,
				Some(loader_version),
				false,
				&cancel,
			)
			.await
		{
			Ok(resolved) if resolved.version_id == installed.version_id => {
				log::info!("описание {} пересобрано", installed.version_id);
				changed = true;
			}
			Ok(resolved) => log::warn!(
				"описание {} пересобрано как {}",
				installed.version_id,
				resolved.version_id
			),
			Err(err) => log::warn!(
				"описание {} не пересобрано: {:?} {}",
				installed.version_id,
				err.code,
				err.details.as_deref().unwrap_or("")
			),
		}
	}
	changed
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn freetype_versions() {
		assert!(has_vulnerable_freetype(["org.lwjgl:lwjgl-freetype:3.3.3"]));
		assert!(has_vulnerable_freetype([
			"org.lwjgl:lwjgl:3.4.3",
			"org.lwjgl:lwjgl-freetype:3.4.3:natives-macos-arm64"
		]));
		assert!(!has_vulnerable_freetype([
			"org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0"
		]));
		assert!(!has_vulnerable_freetype(["org.lwjgl:lwjgl-freetype:3.3.1"]));
		assert!(!has_vulnerable_freetype(["org.lwjgl:lwjgl:3.3.3"]));
		assert!(!has_vulnerable_freetype([
			"com.example:lwjgl-freetype:3.3.3"
		]));
	}

	#[test]
	fn cache_follows_file() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let cache = FontCache::default();
		assert!(!cache.vulnerable(&layout, "26.3"));
		let path = layout.version_json("26.3");
		let write = |lib: &str| {
			let json = serde_json::json!({ "id": "26.3", "libraries": [ { "name": lib } ] });
			crate::fs_util::write_atomic(&path, json.to_string().as_bytes()).unwrap();
		};
		write("org.lwjgl:lwjgl-freetype:3.4.3");
		assert!(cache.vulnerable(&layout, "26.3"));
		assert!(cache.vulnerable(&layout, "26.3"));
		// Длина другая — пересчёт и без смены времени файла.
		write("org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0");
		assert!(!cache.vulnerable(&layout, "26.3"));
		assert!(!cache.vulnerable(&layout, "../x"));
	}
}
