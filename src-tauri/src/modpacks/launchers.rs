//! Поиск сборок других лаунчеров для переноса. Чужие папки только читаются: ничего не создаётся
//! и не меняется (база Modrinth App читается из копии во временной папке).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};

use super::{ModpackLink, PackOrigin};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::instances::Loader;
use crate::meta::LoaderKind;
use crate::settings::Memory;

/// Картинка сборки отдаётся, только если она не больше этого.
const ICON_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LauncherKind {
	Modrinth,
	Prism,
	Polymc,
	Multimc,
	Curseforge,
	Atlauncher,
	Gdlauncher,
	/// Официальный лаунчер (`.minecraft`, профили `launcher_profiles.json`).
	Official,
	/// Папка игры без сведений о версии.
	Folder,
}

/// Что переносить (верхний уровень папки игры по категориям).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
	Mods,
	/// `config/`, `defaultconfigs/`, `kubejs/`… — всё, что не в других категориях и не служебное.
	Config,
	Saves,
	Resourcepacks,
	Shaderpacks,
	Screenshots,
	/// `options.txt`, `optionsof.txt`, `optionsshaders.txt`.
	Options,
	/// `servers.dat`.
	Servers,
}

impl Category {
	pub const ALL: [Self; 8] = [
		Self::Mods,
		Self::Config,
		Self::Saves,
		Self::Resourcepacks,
		Self::Shaderpacks,
		Self::Screenshots,
		Self::Options,
		Self::Servers,
	];
}

/// Служебное лаунчеров и кэши: не переносятся и не экспортируются никогда.
fn is_junk(name: &str, dir: bool) -> bool {
	let lower = name.to_ascii_lowercase();
	if lower.starts_with(crate::instances::CONTENT_TMP_PREFIX) {
		return true;
	}
	if dir {
		return matches!(
			lower.as_str(),
			"versions"
				| "libraries"
				| "assets" | "runtime"
				| "natives" | "logs"
				| "crash-reports"
				| "debug" | ".fabric"
				| ".mixin.out"
				| ".cache" | "cache"
				| "webcache" | "webcache2"
				| "bin" | "downloads"
				| ".curseclient"
				| "profilekeys"
				| ".quilt" | "jarmods"
				| "backups"
		);
	}
	lower.starts_with("launcher_")
		|| lower.ends_with(".log")
		|| matches!(
			lower.as_str(),
			"instance.json"
				| "instance.cfg"
				| "mmc-pack.json"
				| "minecraftinstance.json"
				| "instance.png"
				| "profile.json"
				| "clientid.txt"
				| "treatment_tags.json"
				| "modpack.json"
				| "content.json"
				| "launcher-output.log"
		)
}

/// Категория элемента верхнего уровня папки игры; служебное — `None`.
pub fn category_of(name: &str, dir: bool) -> Option<Category> {
	if is_junk(name, dir) || !fs_util::is_safe_file_name(name) {
		return None;
	}
	let lower = name.to_ascii_lowercase();
	Some(match (lower.as_str(), dir) {
		("mods", true) => Category::Mods,
		("saves", true) => Category::Saves,
		("resourcepacks" | "texturepacks", true) => Category::Resourcepacks,
		("shaderpacks", true) => Category::Shaderpacks,
		("screenshots", true) => Category::Screenshots,
		("options.txt" | "optionsof.txt" | "optionsshaders.txt", false) => Category::Options,
		("servers.dat" | "servers.dat_old", false) => Category::Servers,
		_ => Category::Config,
	})
}

/// Верхние элементы папки игры по категориям (без служебного и симлинков).
pub fn top_entries(game_dir: &Path) -> Vec<(String, bool, Category)> {
	let Ok(list) = std::fs::read_dir(game_dir) else {
		return Vec::new();
	};
	let mut out: Vec<_> = list
		.flatten()
		.filter_map(|e| {
			let kind = e.file_type().ok()?;
			if kind.is_symlink() {
				return None;
			}
			let name = e.file_name().to_string_lossy().into_owned();
			let dir = kind.is_dir();
			Some((name.clone(), dir, category_of(&name, dir)?))
		})
		.collect();
	out.sort();
	out
}

/// Что за папка (для импорта): папка сборки известного лаунчера.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
	pub path: String,
	pub launcher: LauncherKind,
	/// Официальный лаунчер: ключ профиля в `launcher_profiles.json` (у профилей общая папка).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub profile: Option<String>,
	/// Заменяет найденную версию игры (обязательна, если её не удалось определить).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub game_version: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub loader: Option<Loader>,
}

/// Откуда перенесена сборка (`Instance.importedFrom`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFrom {
	pub launcher: LauncherKind,
	pub path: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub profile: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedReason {
	/// Загрузчик, которого у нас нет (LiteLoader, Rift…).
	Loader,
	/// Загрузчик есть, но такой старой версии для этой версии игры нет.
	OldLoader,
	/// Сведения сборки не читаются.
	Damaged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsupported {
	pub reason: UnsupportedReason,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
	pub path: String,
	pub launcher: LauncherKind,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub profile: Option<String>,
	pub name: String,
	/// Папка игры, которая копируется.
	#[serde(skip)]
	pub game_dir: PathBuf,
	/// `null` — не удалось определить (интерфейс просит указать).
	pub game_version: Option<String>,
	pub loader: Option<Loader>,
	/// `.jar` в `mods/` (с выключенными).
	pub mods: usize,
	/// Байты по категориям (при `quick` — нули).
	pub sizes: BTreeMap<Category, u64>,
	/// `data:` URL картинки (до 64 КБ) или `https://` адрес.
	pub icon: Option<String>,
	/// Общая папка `.minecraft` официального лаунчера.
	pub shared: bool,
	/// Уже переносили (есть сборка с таким `importedFrom`).
	pub imported: bool,
	pub unsupported: Option<Unsupported>,
	pub modpack: Option<ModpackLink>,
	pub memory: Option<Memory>,
	pub jvm_args: Option<String>,
}

impl Candidate {
	fn new(launcher: LauncherKind, path: &Path, name: String, game_dir: PathBuf) -> Self {
		Self {
			path: path.to_string_lossy().into_owned(),
			launcher,
			profile: None,
			name,
			game_dir,
			game_version: None,
			loader: None,
			mods: 0,
			sizes: Category::ALL.iter().map(|c| (*c, 0)).collect(),
			icon: None,
			shared: false,
			imported: false,
			unsupported: None,
			modpack: None,
			memory: None,
			jvm_args: None,
		}
	}

	fn damaged(launcher: LauncherKind, path: &Path, detail: impl Into<String>) -> Self {
		let mut c = Self::new(launcher, path, file_name(path), path.to_path_buf());
		c.unsupported = Some(Unsupported {
			reason: UnsupportedReason::Damaged,
			detail: Some(detail.into()),
		});
		c
	}

	pub fn imported_from(&self) -> ImportedFrom {
		ImportedFrom {
			launcher: self.launcher,
			path: self.path.clone(),
			profile: self.profile.clone(),
		}
	}

	/// Версия игры и загрузчик, найденные или из [`ImportItem`].
	fn set_version(&mut self, game: Option<String>, loader: Result<Option<Loader>, String>) {
		self.game_version = game.filter(|g| fs_util::is_safe_id(g));
		match loader {
			Ok(loader) if self.game_version.is_some() => self.loader = loader,
			Ok(_) => {}
			Err(name) => {
				self.unsupported = Some(Unsupported {
					reason: UnsupportedReason::Loader,
					detail: Some(name),
				});
			}
		}
	}

	/// Считает моды и размеры по категориям (долго на больших сборках).
	pub fn measure(&mut self) {
		self.mods = std::fs::read_dir(self.game_dir.join("mods"))
			.map(|d| {
				d.flatten()
					.filter(|e| {
						let n = e.file_name().to_string_lossy().to_ascii_lowercase();
						n.ends_with(".jar") || n.ends_with(".jar.disabled")
					})
					.count()
			})
			.unwrap_or(0);
		for (name, dir, category) in top_entries(&self.game_dir) {
			let path = self.game_dir.join(&name);
			let size = if dir {
				dir_size(&path)
			} else {
				std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
			};
			*self.sizes.entry(category).or_default() += size;
		}
	}
}

/// Найденные сборки одного лаунчера (или указанной папки).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
	pub id: String,
	pub launcher: LauncherKind,
	pub root: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<AppError>,
	pub items: Vec<Candidate>,
}

/// Стандартные папки лаунчеров на этом компьютере (только существующие).
pub fn default_roots() -> Vec<(LauncherKind, PathBuf)> {
	let mut roots = Vec::new();
	if let Some(data) = dirs::data_dir() {
		roots.push((LauncherKind::Modrinth, data.join("ModrinthApp")));
		roots.push((LauncherKind::Prism, data.join("PrismLauncher")));
		roots.push((LauncherKind::Polymc, data.join("PolyMC")));
		roots.push((LauncherKind::Atlauncher, data.join("ATLauncher")));
		roots.push((
			LauncherKind::Gdlauncher,
			data.join("gdlauncher_carbon").join("data"),
		));
		roots.push((LauncherKind::Gdlauncher, data.join("gdlauncher_next")));
		roots.push((LauncherKind::Official, data.join(".minecraft")));
	}
	if let Some(home) = dirs::home_dir() {
		let cf = home.join("curseforge").join("minecraft");
		if cf.is_dir() {
			roots.push((LauncherKind::Curseforge, cf));
		} else if let Some(docs) = dirs::document_dir() {
			roots.push((
				LauncherKind::Curseforge,
				docs.join("curseforge").join("minecraft"),
			));
		}
	}
	roots.retain(|(_, p)| p.is_dir());
	roots
}

/// Сборки лаунчера в его папке. `quick` — без размеров.
pub fn scan(kind: LauncherKind, root: &Path, quick: bool) -> Result<Vec<Candidate>, AppError> {
	let unreadable =
		|dir: &Path, e: std::io::Error| AppError::storage(format!("{}: {e}", dir.display()));
	let list = |dir: PathBuf| -> Result<Vec<PathBuf>, AppError> {
		match std::fs::read_dir(&dir) {
			Ok(entries) => Ok(entries
				.flatten()
				.filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
				.map(|e| e.path())
				.collect()),
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
			Err(e) => Err(unreadable(&dir, e)),
		}
	};
	let mut found: Vec<Candidate> = match kind {
		LauncherKind::Modrinth => scan_modrinth(root)?,
		LauncherKind::Prism | LauncherKind::Polymc | LauncherKind::Multimc => {
			list(mmc_instances_dir(root))?
				.into_iter()
				.filter_map(|d| read_mmc(&d, root, kind))
				.collect()
		}
		LauncherKind::Curseforge => list(root.join("Instances"))?
			.into_iter()
			.filter_map(|d| read_curseforge(&d))
			.collect(),
		LauncherKind::Atlauncher => list(root.join("instances"))?
			.into_iter()
			.filter_map(|d| read_atlauncher(&d, root))
			.collect(),
		LauncherKind::Gdlauncher => list(root.join("instances"))?
			.into_iter()
			.filter_map(|d| read_gdlauncher(&d))
			.collect(),
		LauncherKind::Official => scan_official(root),
		LauncherKind::Folder => folder(root).into_iter().collect(),
	};
	if !quick {
		for c in &mut found {
			c.measure();
		}
	}
	found.sort_by_key(|c| c.name.to_lowercase());
	Ok(found)
}

/// Что за папку указал пользователь: папка лаунчера (её сборки), папка `instances`, папка одной
/// сборки или папка игры. Не узнали — `import_unrecognized`.
pub fn scan_path(
	path: &Path,
	quick: bool,
) -> Result<(LauncherKind, PathBuf, Vec<Candidate>), AppError> {
	let unrecognized =
		|| AppError::coded(ErrorCode::ImportUnrecognized, path.display().to_string());
	if !path.is_dir() {
		return Err(unrecognized());
	}
	let root_kind = if path.join("app.db").is_file()
		|| path.join("profiles").is_dir() && path.join("caches").is_dir()
	{
		Some(LauncherKind::Modrinth)
	} else if path.join("prismlauncher.cfg").is_file() {
		Some(LauncherKind::Prism)
	} else if path.join("polymc.cfg").is_file() {
		Some(LauncherKind::Polymc)
	} else if path.join("multimc.cfg").is_file() {
		Some(LauncherKind::Multimc)
	} else if path.join("launcher_profiles.json").is_file() {
		Some(LauncherKind::Official)
	} else if path.join("Instances").is_dir() && !path.join("instances").is_dir() {
		Some(LauncherKind::Curseforge)
	} else {
		None
	};
	if let Some(kind) = root_kind {
		let found = scan(kind, path, quick)?;
		if !found.is_empty() {
			return Ok((kind, path.to_path_buf(), found));
		}
	}
	// Папка с сборками (`instances` или корень ATLauncher/GDLauncher): узнаём каждую.
	let instances = if path.join("instances").is_dir() {
		path.join("instances")
	} else {
		path.to_path_buf()
	};
	if let Ok(entries) = std::fs::read_dir(&instances) {
		let mut found: Vec<Candidate> = entries
			.flatten()
			.filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
			.filter_map(|e| detect_instance(&e.path()))
			.filter(|c| c.launcher != LauncherKind::Folder)
			.collect();
		if !found.is_empty() {
			let kind = found[0].launcher;
			if !quick {
				found.iter_mut().for_each(Candidate::measure);
			}
			found.sort_by_key(|c| c.name.to_lowercase());
			return Ok((kind, path.to_path_buf(), found));
		}
	}
	let mut one = detect_instance(path).ok_or_else(unrecognized)?;
	if !quick {
		one.measure();
	}
	Ok((one.launcher, path.to_path_buf(), vec![one]))
}

/// Одна папка сборки: какой лаунчер. Папка без сведений, похожая на папку игры, — `folder`.
fn detect_instance(path: &Path) -> Option<Candidate> {
	if path.join("instance.cfg").is_file() {
		let root = mmc_root(path);
		let kind = if root.join("multimc.cfg").is_file() {
			LauncherKind::Multimc
		} else if root.join("polymc.cfg").is_file() {
			LauncherKind::Polymc
		} else {
			LauncherKind::Prism
		};
		return read_mmc(path, &root, kind);
	}
	if path.join("minecraftinstance.json").is_file() {
		return read_curseforge(path);
	}
	if path.join("instance.json").is_file() {
		let root = path.parent().and_then(Path::parent).unwrap_or(path);
		if let Some(c) = read_atlauncher(path, root).filter(|c| c.unsupported.is_none()) {
			return Some(c);
		}
		if let Some(c) = read_gdlauncher(path).filter(|c| c.unsupported.is_none()) {
			return Some(c);
		}
	}
	if path.join("config.json").is_file()
		&& let Some(c) = read_gdlauncher(path).filter(|c| c.unsupported.is_none())
	{
		return Some(c);
	}
	if let Some(root) = path.parent().and_then(Path::parent)
		&& root.join("app.db").is_file()
		&& let Ok(list) = scan_modrinth(root)
		&& let Some(c) = list.into_iter().find(|c| Path::new(&c.path) == path)
	{
		return Some(c);
	}
	folder(path)
}

/// Сборка по ссылке (для переноса: сведения читаются заново).
pub fn resolve(item: &ImportItem) -> Result<Candidate, AppError> {
	let path = PathBuf::from(&item.path);
	let not_found = || AppError::coded(ErrorCode::FileNotFound, item.path.clone());
	let found = match item.launcher {
		LauncherKind::Modrinth => {
			let root = path.parent().and_then(Path::parent).ok_or_else(not_found)?;
			scan_modrinth(root)?
				.into_iter()
				.find(|c| Path::new(&c.path) == path)
		}
		LauncherKind::Prism | LauncherKind::Polymc | LauncherKind::Multimc => {
			read_mmc(&path, &mmc_root(&path), item.launcher)
		}
		LauncherKind::Curseforge => read_curseforge(&path),
		LauncherKind::Atlauncher => {
			let root = path.parent().and_then(Path::parent).unwrap_or(&path);
			read_atlauncher(&path, root)
		}
		LauncherKind::Gdlauncher => read_gdlauncher(&path),
		LauncherKind::Official => scan_official(&path)
			.into_iter()
			.find(|c| c.profile == item.profile),
		LauncherKind::Folder => folder(&path),
	};
	let mut found = found.ok_or_else(not_found)?;
	if let Some(game) = &item.game_version {
		found.game_version = Some(game.clone()).filter(|g| fs_util::is_safe_id(g));
		if item.loader.is_none() && found.loader.is_none() {
			found.loader = Some(Loader {
				kind: LoaderKind::Vanilla,
				version: None,
			});
		}
	}
	if let Some(loader) = &item.loader {
		found.loader = Some(loader.clone());
		if found
			.unsupported
			.as_ref()
			.is_some_and(|u| u.reason == UnsupportedReason::Loader)
		{
			found.unsupported = None;
		}
	}
	Ok(found)
}

/// Папка без сведений лаунчера: если похожа на папку игры (`mods`, `saves`, `config`,
/// `options.txt` или внутри `.minecraft`/`minecraft`).
fn folder(path: &Path) -> Option<Candidate> {
	let game_dir = [".minecraft", "minecraft"]
		.iter()
		.map(|n| path.join(n))
		.find(|p| p.is_dir())
		.unwrap_or_else(|| path.to_path_buf());
	let looks_like_game = ["mods", "saves", "config", "resourcepacks", "options.txt"]
		.iter()
		.any(|n| game_dir.join(n).exists());
	looks_like_game.then(|| Candidate::new(LauncherKind::Folder, path, file_name(path), game_dir))
}

fn file_name(path: &Path) -> String {
	path.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_default()
}

/// Текстовый файл в UTF-8 (с BOM или без); не UTF-8 — как latin-1.
fn read_text(path: &Path) -> Option<String> {
	let bytes = std::fs::read(path).ok()?;
	let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
	Some(match std::str::from_utf8(bytes) {
		Ok(text) => text.to_owned(),
		Err(_) => bytes.iter().map(|b| char::from(*b)).collect(),
	})
}

/// `Ok(None)` — файла нет; `Err` — есть, но не разобран.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
	let Some(text) = read_text(path) else {
		return Ok(None);
	};
	serde_json::from_str(&text)
		.map(Some)
		.map_err(|e| format!("{}: {e}", file_name(path)))
}

/// Картинка сборки как `data:` URL (до [`ICON_MAX_BYTES`]).
fn icon_data_url(path: &Path) -> Option<String> {
	let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
	let mime = match ext.as_str() {
		"png" => "image/png",
		"jpg" | "jpeg" => "image/jpeg",
		"webp" => "image/webp",
		"gif" => "image/gif",
		_ => return None,
	};
	let meta = std::fs::metadata(path).ok()?;
	if !meta.is_file() || meta.len() > ICON_MAX_BYTES {
		return None;
	}
	let bytes = std::fs::read(path).ok()?;
	Some(format!(
		"data:{mime};base64,{}",
		base64::engine::general_purpose::STANDARD.encode(bytes)
	))
}

/// Загрузчик по имени и версии; незнакомое имя — `Err(имя)`.
fn loader_of(
	name: &str,
	version: Option<&str>,
	game_version: &str,
) -> Result<Option<Loader>, String> {
	let kind = match name.to_ascii_lowercase().as_str() {
		"vanilla" | "" => LoaderKind::Vanilla,
		"fabric" => LoaderKind::Fabric,
		"quilt" => LoaderKind::Quilt,
		"forge" => LoaderKind::Forge,
		"neoforge" | "neo_forge" | "neo-forge" => LoaderKind::Neoforge,
		_ => return Err(name.to_owned()),
	};
	if kind == LoaderKind::Vanilla {
		return Ok(Some(Loader {
			kind,
			version: None,
		}));
	}
	let version = match version.filter(|v| !v.trim().is_empty()) {
		Some(v) => Some(super::loader_version(game_version, v).map_err(|_| v.to_owned())?),
		None => None,
	};
	Ok(Some(Loader { kind, version }))
}

fn modrinth_link(
	project: Option<String>,
	version: Option<String>,
	title: &str,
	number: Option<String>,
) -> Option<ModpackLink> {
	let project = project.filter(|p| crate::content::check_ref(p).is_ok())?;
	Some(ModpackLink {
		source: PackOrigin::Modrinth,
		project_id: Some(project),
		version_id: version.filter(|v| crate::content::check_ref(v).is_ok()),
		title: title.to_owned(),
		version_number: number.unwrap_or_default(),
		icon_url: None,
		installed_at: 0,
	})
}

// --- Modrinth App -----------------------------------------------------------------------------

/// Modrinth App: `app.db` (две схемы) + `profiles/`. Базу читаем из копии: приложение может быть
/// запущено. Без базы — `profile.json` старых версий.
fn scan_modrinth(root: &Path) -> Result<Vec<Candidate>, AppError> {
	let db = root.join("app.db");
	let rows = if db.is_file() {
		let tmp =
			std::env::temp_dir().join(format!("leelauncher-import-{:08x}", rand::random::<u32>()));
		let rows = (|| -> Result<Vec<ModrinthRow>, String> {
			std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
			for suffix in ["", "-wal", "-shm"] {
				let from = root.join(format!("app.db{suffix}"));
				if from.is_file() {
					std::fs::copy(&from, tmp.join(format!("app.db{suffix}")))
						.map_err(|e| format!("{}: {e}", from.display()))?;
				}
			}
			read_modrinth_db(&tmp.join("app.db")).map_err(|e| e.to_string())
		})();
		let _ = std::fs::remove_dir_all(&tmp);
		rows.map_err(|e| AppError::storage(format!("app.db: {e}")))?
	} else {
		legacy_modrinth_profiles(&root.join("profiles"))
	};
	Ok(rows
		.into_iter()
		.filter_map(|row| {
			let dir = fs_util::safe_join(&root.join("profiles"), &row.path)?;
			if !dir.is_dir() {
				return None;
			}
			let mut c = Candidate::new(LauncherKind::Modrinth, &dir, row.name.clone(), dir.clone());
			let loader = loader_of(
				&row.loader,
				row.loader_version.as_deref(),
				&row.game_version,
			);
			c.set_version(Some(row.game_version), loader);
			c.icon = row
				.icon_path
				.as_deref()
				.map(Path::new)
				.and_then(icon_data_url);
			c.modpack = modrinth_link(row.project_id, row.version_id, &row.name, None);
			Some(c)
		})
		.collect())
}

struct ModrinthRow {
	path: String,
	name: String,
	icon_path: Option<String>,
	game_version: String,
	loader: String,
	loader_version: Option<String>,
	project_id: Option<String>,
	version_id: Option<String>,
}

fn read_modrinth_db(path: &Path) -> rusqlite::Result<Vec<ModrinthRow>> {
	use rusqlite::{Connection, OpenFlags};
	let conn = Connection::open_with_flags(
		path,
		OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
	)?;
	let has_table = |name: &str| -> bool {
		conn.query_row(
			"SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
			[name],
			|_| Ok(()),
		)
		.is_ok()
	};
	let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<ModrinthRow> {
		Ok(ModrinthRow {
			path: r.get(0)?,
			name: r.get(1)?,
			icon_path: r.get(2)?,
			game_version: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
			loader: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
			loader_version: r.get(5)?,
			project_id: r.get(6)?,
			version_id: r.get(7)?,
		})
	};
	if has_table("profiles") {
		let mut stmt = conn.prepare(
			"SELECT path, name, icon_path, game_version, mod_loader, mod_loader_version,
				linked_project_id, linked_version_id FROM profiles",
		)?;
		return stmt.query_map([], map)?.collect();
	}
	let links = if has_table("instance_links") {
		"(SELECT modrinth_project_id FROM instance_links l WHERE l.instance_id = i.id
				AND l.modrinth_project_id IS NOT NULL LIMIT 1),
			(SELECT modrinth_version_id FROM instance_links l WHERE l.instance_id = i.id
				AND l.modrinth_project_id IS NOT NULL LIMIT 1)"
	} else {
		"NULL, NULL"
	};
	let mut stmt = conn.prepare(&format!(
		"SELECT i.path, i.name, i.icon_path, c.game_version, c.loader, c.loader_version, {links}
			FROM instances i LEFT JOIN instance_content_sets c ON c.id = i.applied_content_set_id"
	))?;
	stmt.query_map([], map)?.collect()
}

#[derive(Deserialize)]
struct LegacyProfile {
	metadata: LegacyMeta,
}

#[derive(Deserialize)]
struct LegacyMeta {
	name: String,
	#[serde(default)]
	icon: Option<String>,
	game_version: String,
	#[serde(default)]
	loader: Option<String>,
	#[serde(default)]
	loader_version: Option<LegacyLoaderVersion>,
	#[serde(default)]
	linked_data: Option<LegacyLinked>,
}

#[derive(Deserialize)]
struct LegacyLoaderVersion {
	id: String,
}

#[derive(Deserialize)]
struct LegacyLinked {
	#[serde(default)]
	project_id: Option<String>,
	#[serde(default)]
	version_id: Option<String>,
}

fn legacy_modrinth_profiles(profiles: &Path) -> Vec<ModrinthRow> {
	let Ok(entries) = std::fs::read_dir(profiles) else {
		return Vec::new();
	};
	entries
		.flatten()
		.filter_map(|e| {
			let p: LegacyProfile = read_json(&e.path().join("profile.json")).ok()??;
			let m = p.metadata;
			Some(ModrinthRow {
				path: e.file_name().to_string_lossy().into_owned(),
				name: m.name,
				icon_path: m.icon,
				game_version: m.game_version,
				loader: m.loader.unwrap_or_default(),
				loader_version: m.loader_version.map(|v| v.id),
				project_id: m.linked_data.as_ref().and_then(|l| l.project_id.clone()),
				version_id: m.linked_data.and_then(|l| l.version_id),
			})
		})
		.collect()
}

// --- Prism Launcher / PolyMC / MultiMC --------------------------------------------------------

/// INI в стиле Qt (`instance.cfg`, `prismlauncher.cfg`): `ключ=значение`, секции пропускаются.
fn parse_ini(text: &str) -> HashMap<String, String> {
	let mut out = HashMap::new();
	for line in text.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with(['#', ';', '[']) {
			continue;
		}
		let Some((key, value)) = line.split_once('=') else {
			continue;
		};
		let value = value.trim();
		let value = match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
			Some(inner) => inner.replace("\\\"", "\"").replace("\\\\", "\\"),
			None => value.to_owned(),
		};
		out.insert(key.trim().to_owned(), value);
	}
	out
}

fn mmc_root(instance: &Path) -> PathBuf {
	instance
		.parent()
		.and_then(Path::parent)
		.map(Path::to_path_buf)
		.unwrap_or_else(|| instance.to_path_buf())
}

fn mmc_instances_dir(root: &Path) -> PathBuf {
	let dir = ["prismlauncher.cfg", "polymc.cfg", "multimc.cfg"]
		.iter()
		.find_map(|cfg| read_text(&root.join(cfg)))
		.map(|t| parse_ini(&t))
		.and_then(|ini| ini.get("InstanceDir").cloned())
		.filter(|d| !d.is_empty())
		.unwrap_or_else(|| "instances".into());
	let dir = PathBuf::from(dir);
	if dir.is_absolute() {
		dir
	} else {
		root.join(dir)
	}
}

#[derive(Deserialize)]
struct MmcPack {
	#[serde(default)]
	components: Vec<MmcComponent>,
}

#[derive(Deserialize)]
struct MmcComponent {
	uid: String,
	#[serde(default)]
	version: Option<String>,
	#[serde(default, rename = "cachedName")]
	cached_name: Option<String>,
}

/// Компоненты Prism, которые меняют загрузку игры, но у нас их нет.
const MMC_FOREIGN_LOADERS: [&str; 4] = [
	"com.mumfrey.liteloader",
	"org.dimdev.rift",
	"net.fabricmc.fabric-loader.legacy",
	"optifine",
];

fn read_mmc(dir: &Path, root: &Path, kind: LauncherKind) -> Option<Candidate> {
	let cfg = parse_ini(&read_text(&dir.join("instance.cfg"))?);
	let name = cfg
		.get("name")
		.filter(|n| !n.trim().is_empty())
		.cloned()
		.unwrap_or_else(|| file_name(dir));
	let game_dir = [".minecraft", "minecraft"]
		.iter()
		.map(|n| dir.join(n))
		.find(|p| p.is_dir())
		.unwrap_or_else(|| dir.join(".minecraft"));
	let mut c = Candidate::new(kind, dir, name.clone(), game_dir);
	let pack: MmcPack = match read_json(&dir.join("mmc-pack.json")) {
		Ok(Some(pack)) => pack,
		Ok(None) => {
			c.unsupported = Some(Unsupported {
				reason: UnsupportedReason::Damaged,
				detail: Some("mmc-pack.json".into()),
			});
			return Some(c);
		}
		Err(err) => {
			c.unsupported = Some(Unsupported {
				reason: UnsupportedReason::Damaged,
				detail: Some(err),
			});
			return Some(c);
		}
	};
	let version_of = |uid: &str| {
		pack.components
			.iter()
			.find(|c| c.uid == uid)
			.and_then(|c| c.version.clone())
	};
	let game = version_of("net.minecraft");
	let g = game.clone().unwrap_or_default();
	let foreign = pack
		.components
		.iter()
		.find(|x| MMC_FOREIGN_LOADERS.iter().any(|f| x.uid.starts_with(f)));
	let loader = if let Some(f) = foreign {
		Err(f.cached_name.clone().unwrap_or_else(|| f.uid.clone()))
	} else {
		[
			("net.fabricmc.fabric-loader", "fabric"),
			("org.quiltmc.quilt-loader", "quilt"),
			("net.neoforged", "neoforge"),
			("net.minecraftforge", "forge"),
		]
		.iter()
		.find_map(|(uid, name)| version_of(uid).map(|v| loader_of(name, Some(&v), &g)))
		.unwrap_or_else(|| loader_of("vanilla", None, &g))
	};
	c.set_version(game, loader);
	c.icon = cfg.get("iconKey").and_then(|key| {
		if !fs_util::is_safe_file_name(key) {
			return None;
		}
		["", ".png", ".jpg", ".jpeg", ".webp", ".gif"]
			.iter()
			.find_map(|ext| icon_data_url(&root.join("icons").join(format!("{key}{ext}"))))
	});
	let on = |key: &str| cfg.get(key).is_some_and(|v| v == "true");
	if on("OverrideMemory")
		&& let (Some(min), Some(max)) = (
			cfg.get("MinMemAlloc").and_then(|v| v.parse().ok()),
			cfg.get("MaxMemAlloc").and_then(|v| v.parse().ok()),
		) {
		c.memory = Memory {
			min_mb: min,
			max_mb: max,
		}
		.validate()
		.ok();
	}
	if on("OverrideJavaArgs") {
		c.jvm_args = cfg.get("JvmArgs").filter(|a| !a.trim().is_empty()).cloned();
	}
	if on("ManagedPack")
		&& cfg
			.get("ManagedPackType")
			.is_some_and(|t| t.eq_ignore_ascii_case("modrinth"))
	{
		c.modpack = modrinth_link(
			cfg.get("ManagedPackID").cloned(),
			cfg.get("ManagedPackVersionID").cloned(),
			cfg.get("ManagedPackName").unwrap_or(&name),
			cfg.get("ManagedPackVersionName").cloned(),
		);
	}
	Some(c)
}

// --- CurseForge App ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfInstance {
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	game_version: Option<String>,
	#[serde(default)]
	base_mod_loader: Option<CfBaseLoader>,
	#[serde(default)]
	profile_image_path: Option<String>,
	#[serde(default)]
	installed_modpack: Option<CfInstalledPack>,
}

#[derive(Deserialize)]
struct CfBaseLoader {
	#[serde(default)]
	name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CfInstalledPack {
	#[serde(default)]
	thumbnail_url: Option<String>,
}

fn read_curseforge(dir: &Path) -> Option<Candidate> {
	let cf: CfInstance = match read_json(&dir.join("minecraftinstance.json")) {
		Ok(Some(cf)) => cf,
		Ok(None) => return None,
		Err(err) => return Some(Candidate::damaged(LauncherKind::Curseforge, dir, err)),
	};
	let name = cf
		.name
		.clone()
		.filter(|n| !n.trim().is_empty())
		.unwrap_or_else(|| file_name(dir));
	let mut c = Candidate::new(LauncherKind::Curseforge, dir, name, dir.to_path_buf());
	let g = cf.game_version.clone().unwrap_or_default();
	let loader = match cf.base_mod_loader.and_then(|l| l.name) {
		Some(id) => super::curseforge::parse_loader(&g, &id).map(Some),
		None => loader_of("vanilla", None, &g),
	};
	c.set_version(cf.game_version, loader);
	c.icon = cf
		.profile_image_path
		.as_deref()
		.map(Path::new)
		.and_then(icon_data_url)
		.or_else(|| {
			cf.installed_modpack
				.and_then(|p| p.thumbnail_url)
				.filter(|u| u.starts_with("https://"))
		});
	Some(c)
}

// --- ATLauncher -------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtInstance {
	/// Версия Minecraft.
	id: String,
	launcher: AtLauncher,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtLauncher {
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	pack: Option<String>,
	#[serde(default)]
	loader_version: Option<AtLoader>,
	#[serde(default)]
	modrinth_project: Option<AtModrinthProject>,
	#[serde(default)]
	modrinth_version: Option<AtModrinthVersion>,
}

#[derive(Deserialize)]
struct AtLoader {
	#[serde(default, rename = "type")]
	kind: Option<String>,
	#[serde(default)]
	version: Option<String>,
}

#[derive(Deserialize)]
struct AtModrinthProject {
	id: String,
}

#[derive(Deserialize)]
struct AtModrinthVersion {
	id: String,
	#[serde(default)]
	version_number: Option<String>,
}

fn read_atlauncher(dir: &Path, root: &Path) -> Option<Candidate> {
	let at: AtInstance = match read_json(&dir.join("instance.json")) {
		Ok(Some(at)) => at,
		Ok(None) => return None,
		Err(err) => return Some(Candidate::damaged(LauncherKind::Atlauncher, dir, err)),
	};
	let name = at
		.launcher
		.name
		.clone()
		.filter(|n| !n.trim().is_empty())
		.unwrap_or_else(|| file_name(dir));
	let mut c = Candidate::new(
		LauncherKind::Atlauncher,
		dir,
		name.clone(),
		dir.to_path_buf(),
	);
	let loader = match &at.launcher.loader_version {
		Some(l) => loader_of(
			l.kind.as_deref().unwrap_or_default(),
			l.version.as_deref(),
			&at.id,
		),
		None => loader_of("vanilla", None, &at.id),
	};
	c.set_version(Some(at.id.clone()), loader);
	c.icon = icon_data_url(&dir.join("instance.png")).or_else(|| {
		let pack: String = at
			.launcher
			.pack
			.as_deref()?
			.chars()
			.filter(|c| c.is_alphanumeric())
			.collect::<String>()
			.to_lowercase();
		icon_data_url(
			&root
				.join("configs")
				.join("images")
				.join(format!("{pack}.png")),
		)
	});
	c.modpack = modrinth_link(
		at.launcher.modrinth_project.map(|p| p.id),
		at.launcher.modrinth_version.as_ref().map(|v| v.id.clone()),
		&name,
		at.launcher.modrinth_version.and_then(|v| v.version_number),
	);
	Some(c)
}

// --- GDLauncher (gdlauncher_next и Carbon) ----------------------------------------------------

/// Старый GDLauncher (`config.json`) и Carbon (`instance.json`, формат не проверен вживую).
fn read_gdlauncher(dir: &Path) -> Option<Candidate> {
	let (file, value) = ["config.json", "instance.json"].iter().find_map(|f| {
		let path = dir.join(f);
		path.is_file()
			.then(|| (*f, read_json::<serde_json::Value>(&path)))
	})?;
	let value = match value {
		Ok(Some(v)) => v,
		Ok(None) => return None,
		Err(err) => return Some(Candidate::damaged(LauncherKind::Gdlauncher, dir, err)),
	};
	let text = |v: &serde_json::Value| v.as_str().map(str::to_owned);
	let (name, game, loader_name, loader_version, icon) = if let Some(loader) = value.get("loader")
	{
		(
			file_name(dir),
			loader.get("mcVersion").and_then(text),
			loader.get("loaderType").and_then(text).unwrap_or_default(),
			loader.get("loaderVersion").and_then(text),
			value.get("background").and_then(text),
		)
	} else if let Some(standard) = value.pointer("/game_configuration/version/Standard") {
		let first = standard.pointer("/modloaders/0");
		(
			value
				.get("name")
				.and_then(text)
				.unwrap_or_else(|| file_name(dir)),
			standard.get("release").and_then(text),
			first
				.and_then(|m| m.get("type_"))
				.and_then(text)
				.unwrap_or_default(),
			first.and_then(|m| m.get("version")).and_then(text),
			value.get("icon").and_then(text),
		)
	} else {
		return Some(Candidate::damaged(LauncherKind::Gdlauncher, dir, file));
	};
	let mut c = Candidate::new(LauncherKind::Gdlauncher, dir, name, dir.to_path_buf());
	let g = game.clone().unwrap_or_default();
	c.set_version(game, loader_of(&loader_name, loader_version.as_deref(), &g));
	c.icon = icon
		.filter(|b| fs_util::is_safe_file_name(b))
		.and_then(|b| icon_data_url(&dir.join(b)));
	Some(c)
}

// --- Официальный лаунчер ----------------------------------------------------------------------

#[derive(Deserialize)]
struct OfficialProfiles {
	#[serde(default)]
	profiles: HashMap<String, OfficialProfile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OfficialProfile {
	#[serde(default)]
	name: Option<String>,
	#[serde(default, rename = "type")]
	kind: Option<String>,
	#[serde(default)]
	last_version_id: Option<String>,
	#[serde(default)]
	game_dir: Option<String>,
	#[serde(default)]
	icon: Option<String>,
	#[serde(default)]
	java_args: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionJson {
	#[serde(default)]
	inherits_from: Option<String>,
}

/// Игра и загрузчик по id версии официального лаунчера (`fabric-loader-0.15.11-1.20.1`,
/// `1.20.1-forge-47.2.0`, `neoforge-21.1.77` + `inheritsFrom`…).
pub fn parse_official_version(id: &str, inherits: Option<&str>) -> Option<(String, Loader)> {
	let lower = id.to_ascii_lowercase();
	let loader = |name: &str, version: Option<&str>, game: &str| {
		loader_of(name, version, game).ok().flatten()
	};
	for (prefix, name) in [("fabric-loader-", "fabric"), ("quilt-loader-", "quilt")] {
		if lower.starts_with(prefix) {
			let rest = &id[prefix.len()..];
			let game = inherits
				.map(str::to_owned)
				.or_else(|| rest.rsplit_once('-').map(|(_, g)| g.to_owned()))?;
			let version = rest.strip_suffix(&format!("-{game}")).unwrap_or(rest);
			return Some((game.clone(), loader(name, Some(version), &game)?));
		}
	}
	if lower.starts_with("neoforge-") {
		let game = inherits?.to_owned();
		let version = &id["neoforge-".len()..];
		return Some((game.clone(), loader("neoforge", Some(version), &game)?));
	}
	if let Some(pos) = lower.find("forge") {
		let game = inherits
			.map(str::to_owned)
			.unwrap_or_else(|| id[..pos].trim_end_matches('-').to_owned());
		let rest = id[pos + "forge".len()..].trim_start_matches('-');
		let rest = rest
			.strip_prefix(&game)
			.map(|r| r.trim_start_matches('-'))
			.unwrap_or(rest);
		return Some((game.clone(), loader("forge", Some(rest), &game)?));
	}
	let game = inherits.unwrap_or(id).to_owned();
	Some((game.clone(), loader("vanilla", None, &game)?))
}

fn scan_official(root: &Path) -> Vec<Candidate> {
	let file = match read_json::<OfficialProfiles>(&root.join("launcher_profiles.json")) {
		Ok(Some(file)) => file,
		Ok(None) => return Vec::new(),
		Err(err) => return vec![Candidate::damaged(LauncherKind::Official, root, err)],
	};
	file.profiles
		.into_iter()
		.filter_map(|(key, p)| {
			let kind = p.kind.as_deref().unwrap_or("custom");
			let version_id = p.last_version_id.clone().unwrap_or_default();
			let latest = matches!(kind, "latest-release" | "latest-snapshot")
				|| version_id.starts_with("latest-");
			let game_dir = p
				.game_dir
				.as_deref()
				.filter(|d| !d.trim().is_empty())
				.map(PathBuf::from)
				.unwrap_or_else(|| root.to_path_buf());
			if !game_dir.is_dir() {
				return None;
			}
			let name = p
				.name
				.filter(|n| !n.trim().is_empty())
				.unwrap_or_else(|| match kind {
					"latest-release" => "Latest release".into(),
					"latest-snapshot" => "Latest snapshot".into(),
					_ => version_id.clone(),
				});
			let shared = game_dir == root;
			let mut c = Candidate::new(LauncherKind::Official, root, name, game_dir);
			c.profile = Some(key);
			c.shared = shared;
			if !latest && !version_id.is_empty() {
				let inherits = fs_util::is_safe_id(&version_id)
					.then(|| {
						read_json::<VersionJson>(
							&root
								.join("versions")
								.join(&version_id)
								.join(format!("{version_id}.json")),
						)
						.ok()
						.flatten()
					})
					.flatten()
					.and_then(|v| v.inherits_from);
				match parse_official_version(&version_id, inherits.as_deref()) {
					Some((g, l)) => c.set_version(Some(g), Ok(Some(l))),
					None => {
						c.unsupported = Some(Unsupported {
							reason: UnsupportedReason::Loader,
							detail: Some(version_id.clone()),
						});
					}
				}
			}
			c.icon = p.icon.filter(|i| {
				i.starts_with("data:image/") && i.len() as u64 <= ICON_MAX_BYTES * 4 / 3 + 64
			});
			c.jvm_args = p.java_args.filter(|a| !a.trim().is_empty());
			Some(c)
		})
		.collect()
}

pub fn dir_size(dir: &Path) -> u64 {
	let mut total = 0;
	let mut stack = vec![dir.to_path_buf()];
	while let Some(dir) = stack.pop() {
		let Ok(list) = std::fs::read_dir(&dir) else {
			continue;
		};
		for e in list.flatten() {
			let Ok(kind) = e.file_type() else { continue };
			if kind.is_dir() {
				stack.push(e.path());
			} else if kind.is_file() {
				total += e.metadata().map(|m| m.len()).unwrap_or(0);
			}
		}
	}
	total
}

#[cfg(test)]
mod tests {
	use super::*;

	fn write(path: &Path, text: &str) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, text).unwrap();
	}

	fn item(c: &Candidate) -> ImportItem {
		ImportItem {
			path: c.path.clone(),
			launcher: c.launcher,
			profile: c.profile.clone(),
			game_version: None,
			loader: None,
		}
	}

	#[test]
	fn prism_instances() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("PrismLauncher");
		write(
			&root.join("prismlauncher.cfg"),
			"[General]\nInstanceDir=inst\n",
		);
		let inst = root.join("inst").join("Pack 1");
		write(
			&inst.join("instance.cfg"),
			"[General]\nname=\"My \\\"Pack\\\"\"\niconKey=fancy\nOverrideMemory=true\nMinMemAlloc=1024\nMaxMemAlloc=4096\nOverrideJavaArgs=true\nJvmArgs=-XX:+UseG1GC\nManagedPack=true\nManagedPackType=modrinth\nManagedPackID=1KVo5zza\nManagedPackVersionID=abcDEF12\nManagedPackName=Fabulously Optimized\nManagedPackVersionName=5.0.0\n",
		);
		write(
			&inst.join("mmc-pack.json"),
			r#"{"formatVersion":1,"components":[
				{"uid":"org.lwjgl3","version":"3.3.1"},
				{"uid":"net.minecraft","version":"1.20.1"},
				{"uid":"net.fabricmc.intermediary","version":"1.20.1"},
				{"uid":"net.fabricmc.fabric-loader","version":"0.15.11"}]}"#,
		);
		write(&inst.join(".minecraft/mods/a.jar"), "abc");
		write(&inst.join(".minecraft/mods/b.jar.disabled"), "b");
		write(&inst.join(".minecraft/config/c.toml"), "cc");
		write(&inst.join(".minecraft/options.txt"), "o");
		write(&inst.join(".minecraft/logs/latest.log"), "x");
		std::fs::create_dir_all(root.join("icons")).unwrap();
		std::fs::write(root.join("icons/fancy.png"), [137u8, 80, 78, 71]).unwrap();
		let old = root.join("inst").join("Old");
		write(&old.join("instance.cfg"), "name=Old\n");
		write(
			&old.join("mmc-pack.json"),
			r#"{"components":[{"uid":"net.minecraft","version":"1.12.2"},{"uid":"com.mumfrey.liteloader","version":"1.12.2","cachedName":"LiteLoader"}]}"#,
		);
		let broken = root.join("inst").join("Broken");
		write(&broken.join("instance.cfg"), "name=Broken\n");
		write(&broken.join("mmc-pack.json"), "{oops");

		let found = scan(LauncherKind::Prism, &root, false).unwrap();
		assert_eq!(found.len(), 3);
		let c = found.iter().find(|c| c.name == "My \"Pack\"").unwrap();
		assert_eq!(c.game_version.as_deref(), Some("1.20.1"));
		let loader = c.loader.as_ref().unwrap();
		assert_eq!(
			(loader.kind, loader.version.as_deref()),
			(LoaderKind::Fabric, Some("0.15.11"))
		);
		assert!(
			c.icon
				.as_deref()
				.unwrap()
				.starts_with("data:image/png;base64,")
		);
		assert_eq!(c.mods, 2);
		assert_eq!(c.sizes[&Category::Mods], 4);
		assert_eq!(c.sizes[&Category::Config], 2);
		assert_eq!(c.sizes[&Category::Options], 1);
		assert_eq!(c.memory.unwrap().max_mb, 4096);
		assert_eq!(c.jvm_args.as_deref(), Some("-XX:+UseG1GC"));
		assert_eq!(c.modpack.as_ref().unwrap().version_number, "5.0.0");
		assert!(c.unsupported.is_none());
		let lite = found.iter().find(|c| c.name == "Old").unwrap();
		assert_eq!(
			lite.unsupported.as_ref().unwrap().reason,
			UnsupportedReason::Loader
		);
		assert_eq!(
			lite.unsupported.as_ref().unwrap().detail.as_deref(),
			Some("LiteLoader")
		);
		let broken = found.iter().find(|c| c.name == "Broken").unwrap();
		assert_eq!(
			broken.unsupported.as_ref().unwrap().reason,
			UnsupportedReason::Damaged
		);

		let mut again = resolve(&item(c)).unwrap();
		again.measure();
		assert_eq!(again, *c);
		// Указанная папка: и корень, и `instances`, и одна сборка.
		assert_eq!(scan_path(&root, true).unwrap().2.len(), 3);
		assert_eq!(scan_path(&root.join("inst"), true).unwrap().2.len(), 3);
		let (kind, _, one) = scan_path(&inst, true).unwrap();
		assert_eq!((kind, one.len()), (LauncherKind::Prism, 1));
		assert_eq!(one[0].sizes[&Category::Mods], 0, "quick — без размеров");
		// Только чтение: ничего не появилось.
		assert!(!inst.join("modpack.json").exists());
	}

	#[test]
	fn curseforge_atlauncher_gdlauncher() {
		let dir = tempfile::tempdir().unwrap();
		let cf = dir.path().join("cf/Instances/All the Mods");
		write(
			&cf.join("minecraftinstance.json"),
			r#"{"name":"ATM9","gameVersion":"1.20.1","baseModLoader":{"name":"forge-47.2.0","minecraftVersion":"1.20.1"},
				"installedModpack":{"thumbnailUrl":"https://media.forgecdn.net/x.png"}}"#,
		);
		write(&cf.join("mods/a.jar"), "a");
		let found = scan(LauncherKind::Curseforge, &dir.path().join("cf"), false).unwrap();
		assert_eq!(found.len(), 1);
		assert_eq!(found[0].name, "ATM9");
		assert_eq!(found[0].loader.as_ref().unwrap().kind, LoaderKind::Forge);
		assert_eq!(
			found[0].icon.as_deref(),
			Some("https://media.forgecdn.net/x.png")
		);
		assert_eq!(found[0].mods, 1);

		let at = dir.path().join("ATLauncher/instances/VanillaMinecraft");
		write(
			&at.join("instance.json"),
			r#"{"id":"1.20.4","launcher":{"name":"Vanilla 1.20.4","pack":"Vanilla Minecraft","version":"1.20.4",
				"loaderVersion":{"type":"NeoForge","version":"20.4.200"}}}"#,
		);
		let found = scan(
			LauncherKind::Atlauncher,
			&dir.path().join("ATLauncher"),
			true,
		)
		.unwrap();
		assert_eq!(found[0].name, "Vanilla 1.20.4");
		assert_eq!(
			found[0].loader.as_ref().unwrap().version.as_deref(),
			Some("20.4.200")
		);
		assert_eq!(
			detect_instance(&at).unwrap().launcher,
			LauncherKind::Atlauncher
		);

		let gd = dir.path().join("gdl/instances/Better MC");
		write(
			&gd.join("config.json"),
			r#"{"loader":{"loaderType":"forge","loaderVersion":"1.20.1-47.2.0","mcVersion":"1.20.1"}}"#,
		);
		let carbon = dir.path().join("gdl/instances/Carbon");
		write(
			&carbon.join("instance.json"),
			r#"{"name":"Carbon pack","game_configuration":{"version":{"Standard":{"release":"1.21.1",
				"modloaders":[{"type_":"Neoforge","version":"21.1.77"}]}}}}"#,
		);
		let found = scan(LauncherKind::Gdlauncher, &dir.path().join("gdl"), true).unwrap();
		assert_eq!(found.len(), 2);
		assert_eq!(found[0].name, "Better MC");
		assert_eq!(
			found[0].loader.as_ref().unwrap().version.as_deref(),
			Some("47.2.0")
		);
		assert_eq!(found[1].name, "Carbon pack");
		assert_eq!(found[1].loader.as_ref().unwrap().kind, LoaderKind::Neoforge);
	}

	#[test]
	fn official_profiles() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join(".minecraft");
		write(
			&root.join("launcher_profiles.json"),
			r#"{"profiles":{
				"a":{"name":"","type":"latest-release","lastVersionId":"latest-release","icon":"Grass"},
				"b":{"name":"Fabric","type":"custom","lastVersionId":"fabric-loader-0.15.11-1.20.1"},
				"c":{"name":"Neo","type":"custom","lastVersionId":"neoforge-21.1.77","gameDir":"MISSING"}
			}}"#,
		);
		write(&root.join("saves/W/level.dat"), "x");
		write(&root.join("libraries/big.jar"), "0123456789");
		let found = scan(LauncherKind::Official, &root, false).unwrap();
		assert_eq!(found.len(), 2, "профиль с несуществующей папкой пропущен");
		let latest = found
			.iter()
			.find(|c| c.profile.as_deref() == Some("a"))
			.unwrap();
		assert_eq!(latest.name, "Latest release");
		assert!(latest.game_version.is_none() && latest.icon.is_none() && latest.shared);
		assert_eq!(latest.sizes[&Category::Saves], 1);
		assert_eq!(
			latest.sizes.values().sum::<u64>(),
			1,
			"библиотеки не переносятся"
		);
		let fabric = found.iter().find(|c| c.name == "Fabric").unwrap();
		assert_eq!(fabric.game_version.as_deref(), Some("1.20.1"));
		assert_eq!(resolve(&item(fabric)).unwrap().name, "Fabric");

		let p = |id: &str, inherits: Option<&str>| {
			let (g, l) = parse_official_version(id, inherits).unwrap();
			(g, l.kind, l.version)
		};
		assert_eq!(
			p("1.20.1-forge-47.2.0", None),
			("1.20.1".into(), LoaderKind::Forge, Some("47.2.0".into()))
		);
		assert_eq!(
			p("1.12.2-forge1.12.2-14.23.5.2847", None),
			(
				"1.12.2".into(),
				LoaderKind::Forge,
				Some("14.23.5.2847".into())
			)
		);
		assert_eq!(
			p("neoforge-21.1.77", Some("1.21.1")),
			(
				"1.21.1".into(),
				LoaderKind::Neoforge,
				Some("21.1.77".into())
			)
		);
		assert_eq!(
			p("quilt-loader-0.26.0-1.20.1", None),
			("1.20.1".into(), LoaderKind::Quilt, Some("0.26.0".into()))
		);
		assert_eq!(
			p("1.21.4", None),
			("1.21.4".into(), LoaderKind::Vanilla, None)
		);
		assert!(parse_official_version("neoforge-21.1.77", None).is_none());
	}

	#[test]
	fn modrinth_app_schemas() {
		let dir = tempfile::tempdir().unwrap();
		for (n, sql) in [
			(
				"old",
				"CREATE TABLE profiles (path TEXT, name TEXT, icon_path TEXT, game_version TEXT, mod_loader TEXT,
					mod_loader_version TEXT, linked_project_id TEXT, linked_version_id TEXT);
				INSERT INTO profiles VALUES ('Fab', 'Fabulous', NULL, '1.21.1', 'fabric', '0.16.5', 'abcd1234', 'v1');
				INSERT INTO profiles VALUES ('../evil', 'Evil', NULL, '1.21.1', 'vanilla', NULL, NULL, NULL);",
			),
			(
				"new",
				"CREATE TABLE instances (id TEXT, path TEXT, applied_content_set_id TEXT, name TEXT, icon_path TEXT);
				CREATE TABLE instance_content_sets (id TEXT, game_version TEXT, loader TEXT, loader_version TEXT);
				CREATE TABLE instance_links (instance_id TEXT, modrinth_project_id TEXT, modrinth_version_id TEXT);
				INSERT INTO instances VALUES ('i1', 'Fab', 'c1', 'Fabulous', NULL);
				INSERT INTO instance_content_sets VALUES ('c1', '1.21.1', 'neoforge', '21.1.77');",
			),
		] {
			let root = dir.path().join(n);
			std::fs::create_dir_all(root.join("profiles/Fab/mods")).unwrap();
			std::fs::write(root.join("profiles/Fab/mods/a.jar"), "ab").unwrap();
			let conn = rusqlite::Connection::open(root.join("app.db")).unwrap();
			conn.execute_batch(sql).unwrap();
			drop(conn);
			let before = std::fs::read(root.join("app.db")).unwrap();
			let found = scan(LauncherKind::Modrinth, &root, false).unwrap();
			assert_eq!(found.len(), 1, "{n}");
			assert_eq!(found[0].name, "Fabulous");
			assert_eq!(found[0].game_version.as_deref(), Some("1.21.1"));
			assert_eq!(found[0].sizes[&Category::Mods], 2);
			if n == "old" {
				assert_eq!(found[0].loader.as_ref().unwrap().kind, LoaderKind::Fabric);
				assert_eq!(
					found[0].modpack.as_ref().unwrap().version_id.as_deref(),
					Some("v1")
				);
			} else {
				assert_eq!(found[0].loader.as_ref().unwrap().kind, LoaderKind::Neoforge);
				assert!(found[0].modpack.is_none());
			}
			assert_eq!(std::fs::read(root.join("app.db")).unwrap(), before);
			assert_eq!(
				detect_instance(&root.join("profiles/Fab")).unwrap().launcher,
				LauncherKind::Modrinth
			);
		}
		let legacy = dir.path().join("legacy");
		write(
			&legacy.join("profiles/P/profile.json"),
			r#"{"metadata":{"name":"Legacy","game_version":"1.19.2","loader":"forge","loader_version":{"id":"43.2.0"}}}"#,
		);
		let found = scan(LauncherKind::Modrinth, &legacy, true).unwrap();
		assert_eq!(
			found[0].loader.as_ref().unwrap().version.as_deref(),
			Some("43.2.0")
		);
	}

	#[test]
	fn plain_folder_and_categories() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("Old game");
		write(&path.join(".minecraft/mods/a.jar"), "a");
		let (kind, _, found) = scan_path(&path, true).unwrap();
		assert_eq!(kind, LauncherKind::Folder);
		assert_eq!(found[0].name, "Old game");
		assert!(found[0].game_version.is_none());
		let empty = dir.path().join("empty");
		std::fs::create_dir_all(&empty).unwrap();
		assert_eq!(
			scan_path(&empty, true).unwrap_err().code,
			ErrorCode::ImportUnrecognized
		);
		assert_eq!(category_of("kubejs", true), Some(Category::Config));
		assert_eq!(category_of("optionsof.txt", false), Some(Category::Options));
		assert_eq!(category_of("versions", true), None);
		assert_eq!(category_of("launcher_profiles.json", false), None);
		assert_eq!(category_of("usercache.json", false), Some(Category::Config));
	}
}
