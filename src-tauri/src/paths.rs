use std::path::PathBuf;
use std::sync::OnceLock;

use crate::migrate::{self, Outcome, Plan};

/// Папки этого запуска. Пока перенос не закончен, это может быть старая папка.
static ACTIVE: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();

fn plan() -> Plan {
	Plan::system().expect("не удалось определить папки данных пользователя")
}

/// Вызывается один раз, первой строкой `run()`.
pub fn init() -> Outcome {
	let outcome = migrate::run(&plan());
	let _ = ACTIVE.set((outcome.data_dir.clone(), outcome.webview_dir.clone()));
	outcome
}

/// `%APPDATA%\LeeLauncher`. `app_data_dir()` Tauri дал бы папку по identifier.
pub fn data_dir() -> PathBuf {
	match ACTIVE.get() {
		Some((data, _)) => data.clone(),
		None => plan().data.new,
	}
}

/// Папка профиля WebView2 (localStorage, кэш): `%LOCALAPPDATA%\LeeLauncher`.
/// WKWebView на macOS свою папку не выбирает, там функция не нужна.
#[cfg(not(target_os = "macos"))]
pub fn webview_data_dir() -> PathBuf {
	match ACTIVE.get() {
		Some((_, webview)) => webview.clone(),
		None => plan().webview.new,
	}
}

/// Раскладка папки данных. Корень явный — для тестов во временной папке.
///
/// ```text
/// <root>/settings.json
/// <root>/logs/                         логи лаунчера
/// <root>/instances/<id>/instance.json    сборка
/// <root>/instances/<id>/minecraft/       папка игры (mods, saves, config…)
/// <root>/meta/versions/<id>/<id>.json|jar
/// <root>/meta/libraries/                 общие библиотеки (maven-пути)
/// <root>/meta/assets/{indexes,objects,virtual,log_configs}/
/// <root>/meta/natives/<versionId>/       распакованные natives старых версий
/// <root>/meta/cache/                     манифесты версий и загрузчиков
/// <root>/java/<component>/               Java, скачанная лаунчером
/// <root>/skins/library.json, <id>.png    библиотека скинов
/// ```
#[derive(Debug, Clone)]
pub struct Layout {
	root: PathBuf,
}

impl Layout {
	pub fn new(root: PathBuf) -> Self {
		Self { root }
	}

	/// Запущенные игры (на случай перезапуска лаунчера во время игры).
	pub fn running_file(&self) -> PathBuf {
		self.root.join("running.json")
	}

	pub fn settings_file(&self) -> PathBuf {
		self.root.join("settings.json")
	}

	pub fn instances_dir(&self) -> PathBuf {
		self.root.join("instances")
	}

	pub fn instance_dir(&self, id: &str) -> PathBuf {
		self.instances_dir().join(id)
	}

	pub fn game_dir(&self, id: &str) -> PathBuf {
		self.instance_dir(id).join("minecraft")
	}

	pub fn meta_dir(&self) -> PathBuf {
		self.root.join("meta")
	}

	pub fn versions_dir(&self) -> PathBuf {
		self.meta_dir().join("versions")
	}

	pub fn version_json(&self, id: &str) -> PathBuf {
		self.versions_dir().join(id).join(format!("{id}.json"))
	}

	pub fn version_jar(&self, id: &str) -> PathBuf {
		self.versions_dir().join(id).join(format!("{id}.jar"))
	}

	pub fn libraries_dir(&self) -> PathBuf {
		self.meta_dir().join("libraries")
	}

	pub fn assets_dir(&self) -> PathBuf {
		self.meta_dir().join("assets")
	}

	pub fn asset_index(&self, id: &str) -> PathBuf {
		self.assets_dir().join("indexes").join(format!("{id}.json"))
	}

	pub fn asset_object(&self, hash: &str) -> PathBuf {
		let prefix = hash.get(..2).unwrap_or("00");
		self.assets_dir().join("objects").join(prefix).join(hash)
	}

	pub fn virtual_assets_dir(&self, index_id: &str) -> PathBuf {
		self.assets_dir().join("virtual").join(index_id)
	}

	pub fn log_config(&self, id: &str) -> PathBuf {
		self.assets_dir().join("log_configs").join(id)
	}

	pub fn natives_dir(&self, version_id: &str) -> PathBuf {
		self.meta_dir().join("natives").join(version_id)
	}

	pub fn cache_dir(&self) -> PathBuf {
		self.meta_dir().join("cache")
	}

	/// Миниатюры скриншотов.
	pub fn thumbs_dir(&self) -> PathBuf {
		self.cache_dir().join("thumbs")
	}

	pub fn java_dir(&self) -> PathBuf {
		self.root.join("java")
	}

	/// Библиотека скинов: `library.json` и `<id>.png`.
	pub fn skins_dir(&self) -> PathBuf {
		self.root.join("skins")
	}

	pub fn root(&self) -> &std::path::Path {
		&self.root
	}

	/// Логи лаунчера (`leelauncher.log`), их пишет tauri-plugin-log.
	pub fn logs_dir(&self) -> PathBuf {
		self.root.join("logs")
	}
}
