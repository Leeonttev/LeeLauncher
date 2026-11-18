use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::content::installed::ContentLocks;
use crate::content::{Sources, curseforge, modrinth};
use crate::download::{self, Downloader};
use crate::downloads::Downloads;
use crate::error::AppError;
use crate::game::Games;
use crate::gamedata::watch::Watch;
use crate::instances::Instances;
use crate::java::JavaCache;
use crate::meta::Meta;
use crate::paths::Layout;
use crate::settings::SettingsStore;

/// Общее состояние для сборок, Java и загрузок.
pub struct Launcher {
	pub layout: Layout,
	pub dl: Downloader,
	pub meta: Meta,
	pub settings: SettingsStore,
	pub instances: Instances,
	/// Установки идут по одной: у сборок общие файлы и процессоры Forge.
	pub install_lock: Mutex<()>,
	/// Одна загрузка Java за раз.
	pub java_lock: Mutex<()>,
	/// Отмена идущих `java_install`: мажорная версия → (номер попытки, токен).
	pub java_installs: std::sync::Mutex<HashMap<u32, (u64, CancellationToken)>>,
	/// Запущенные игры и их логи.
	pub games: Games,
	/// Найденные на компьютере Java (кэш поиска).
	pub java_found: JavaCache,
	/// Общий список загрузок (панель «Загрузки»).
	pub downloads: Arc<Downloads>,
	/// Каталоги контента (Modrinth, CurseForge — если в сборке есть ключ API).
	pub sources: Sources,
	/// Сеансы диалога «Скачайте вручную» (слежение за «Загрузками»).
	pub manual: crate::content::manual::Sessions,
	/// Замки и отметки контента сборок.
	pub content_locks: ContentLocks,
	/// Наблюдение за папками открытой сборки.
	pub watch: Watch,
	/// Разобранные модпаки (токены `modpack_inspect`).
	pub packs: crate::modpacks::PackCache,
	/// Последняя проверка обновления модпака сборки: id → (когда, результат).
	pub pack_updates: std::sync::Mutex<
		HashMap<String, (std::time::Instant, crate::modpacks::commands::ModpackUpdate)>,
	>,
	/// Отмена идущих `modpack_inspect`: requestId → (номер попытки, токен).
	pub pack_inspects: std::sync::Mutex<HashMap<String, (u64, CancellationToken)>>,
	/// Пути из `instance_export_pick_path`: путь → можно ли заменить существующий файл.
	pub export_paths: std::sync::Mutex<HashMap<std::path::PathBuf, bool>>,
}

impl Launcher {
	/// Игра запускается или идёт.
	pub fn game_running(&self) -> bool {
		self.instances.any_running() || !self.games.running().is_empty()
	}

	pub fn new(layout: Layout) -> Result<Arc<Self>, AppError> {
		let client =
			|r: reqwest::Result<reqwest::Client>| r.map_err(|e| AppError::unknown(e.to_string()));
		let key = curseforge::api_key();
		let mut dl = Downloader::new(
			client(download::build_client_with(&curseforge::CDN_HOSTS))?,
			download::CONCURRENCY,
		);
		let modrinth = modrinth::Modrinth::new(
			client(modrinth::build_client())?,
			modrinth::API_BASE,
			layout.cache_dir().join("modrinth-categories.json"),
		)?;
		let curseforge = match &key {
			None => {
				log::info!("CurseForge выключен: в сборке нет ключа API");
				None
			}
			Some(key) => {
				let made = curseforge::build_client(key).and_then(|c| {
					curseforge::CurseForge::new(
						c,
						curseforge::API_BASE,
						layout.cache_dir().join("curseforge-categories.json"),
					)
				});
				match (made, curseforge::key_header(key)) {
					(Ok(cf), Ok(value)) => {
						// CDN CurseForge требует ключ и для файлов.
						dl = dl.with_host_header(download::HostHeader {
							hosts: &curseforge::CDN_HOSTS,
							name: reqwest::header::HeaderName::from_static("x-api-key"),
							value,
						});
						Some(cf)
					}
					(Err(err), _) | (_, Err(err)) => {
						log::warn!("CurseForge выключен: {:?}", err.details);
						None
					}
				}
			}
		};
		let settings = SettingsStore::load(layout.settings_file());
		dl.set_source(settings.get().download_source);
		let instances = Instances::load(layout.clone());
		spawn_cleanup(&layout, instances.take_leftovers());
		let running_file = layout.running_file();
		Ok(Arc::new(Self {
			meta: Meta::new(layout.clone(), dl.clone()),
			instances,
			settings,
			dl,
			layout,
			install_lock: Mutex::new(()),
			java_lock: Mutex::new(()),
			java_installs: std::sync::Mutex::new(HashMap::new()),
			games: Games::new(Some(running_file)),
			java_found: JavaCache::default(),
			downloads: Arc::default(),
			sources: Sources::new(modrinth, curseforge),
			manual: Default::default(),
			content_locks: ContentLocks::default(),
			watch: Watch::default(),
			packs: Default::default(),
			pack_updates: Default::default(),
			pack_inspects: Default::default(),
			export_paths: Default::default(),
		}))
	}
}

/// Временные загрузки моложе этого не трогаем: это могут быть идущие загрузки.
const STALE_PARTS_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Остатки `.content-tmp-*` и `saves/.world-tmp-*`: при запуске задач контента ещё нет.
fn remove_content_tmp(instances_dir: &std::path::Path) {
	let Ok(instances) = std::fs::read_dir(instances_dir) else {
		return;
	};
	for instance in instances.flatten() {
		let dir = instance.path();
		remove_dirs_with_prefix(&dir, crate::instances::CONTENT_TMP_PREFIX);
		remove_dirs_with_prefix(
			&dir.join("minecraft").join("saves"),
			crate::content::worlds::TMP_PREFIX,
		);
	}
}

fn remove_dirs_with_prefix(dir: &std::path::Path, prefix: &str) {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return;
	};
	for entry in entries.flatten() {
		if entry.path().is_dir() && entry.file_name().to_string_lossy().starts_with(prefix) {
			match std::fs::remove_dir_all(entry.path()) {
				Ok(()) => log::info!("дочищено: {}", entry.path().display()),
				Err(err) => {
					log::warn!("не удалось дочистить {}: {err}", entry.path().display())
				}
			}
		}
	}
}

/// Миниатюры скриншотов, которые не трогали дольше этого, удаляются.
const THUMBS_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// Дочистка после прошлого запуска, в фоне.
fn spawn_cleanup(layout: &Layout, leftovers: Vec<std::path::PathBuf>) {
	let roots = [layout.meta_dir(), layout.java_dir()];
	let java_dir = layout.java_dir();
	let thumbs = layout.thumbs_dir();
	let instances_dir = layout.instances_dir();
	let packs_dir = layout.cache_dir().join("modpacks");
	let spawned = std::thread::Builder::new()
		.name("cleanup".into())
		.spawn(move || {
			crate::instances::remove_leftovers(leftovers);
			crate::java::runtime::cleanup(&java_dir);
			remove_content_tmp(&instances_dir);
			crate::modpacks::cleanup(&packs_dir, &instances_dir);
			let parts = download::remove_stale_parts(&roots, STALE_PARTS_AGE);
			if parts > 0 {
				log::info!("удалено старых временных файлов загрузок: {parts}");
			}
			let thumbs = crate::gamedata::remove_stale_thumbs(&thumbs, THUMBS_AGE);
			if thumbs > 0 {
				log::info!("удалено старых миниатюр скриншотов: {thumbs}");
			}
		});
	if let Err(err) = spawned {
		log::warn!("не удалось начать дочистку: {err}");
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn removes_content_tmp_only() {
		let dir = tempfile::tempdir().unwrap();
		let inst = dir.path().join("abc");
		let tmp = inst.join(".content-tmp-0a1b2c3d");
		std::fs::create_dir_all(&tmp).unwrap();
		std::fs::write(tmp.join("0"), b"x").unwrap();
		std::fs::create_dir_all(inst.join("minecraft/mods")).unwrap();
		std::fs::write(inst.join("instance.json"), b"{}").unwrap();
		let world_tmp = inst.join("minecraft/saves/.world-tmp-1");
		std::fs::create_dir_all(&world_tmp).unwrap();
		std::fs::create_dir_all(inst.join("minecraft/saves/World")).unwrap();
		super::remove_content_tmp(dir.path());
		assert!(!tmp.exists() && !world_tmp.exists());
		assert!(inst.join("minecraft/saves/World").is_dir());
		assert!(inst.join("minecraft/mods").is_dir() && inst.join("instance.json").is_file());
	}
}
