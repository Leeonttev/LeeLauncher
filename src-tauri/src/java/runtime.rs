//! Скачивание Java: официальные сборки Mojang (те же, что ставит лаунчер Minecraft).
//! Файлы по одному с проверкой sha1, с piston-data.mojang.com (есть зеркало BMCLAPI).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::{JavaInstall, parse_major};
use crate::download::{Downloader, Job, Progress, Verify};
use crate::error::{AppError, ErrorCode};
use crate::fs_util::{self, safe_join};
use crate::meta::Meta;
use crate::meta::model::Download;

pub const RUNTIME_INDEX_URL: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
const INDEX_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Файл в папке Java: установка закончена (без него папка считается недокачанной). Имя не менять:
/// с другим уже скачанная Java считалась бы недокачанной.
const MARKER: &str = ".leelauncher-runtime.json";
const INDEX_CACHE: &str = "java-runtime.json";
/// Новая версия Java качается сюда, потом подменяет прежнюю.
const STAGING_PREFIX: &str = ".update-";
/// Прежняя версия после подмены, удаляется в фоне.
const OLD_PREFIX: &str = ".old-";
/// Недокачанное обновление моложе этого при запуске не трогаем.
const STAGING_AGE: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Deserialize)]
struct RuntimeVersion {
	name: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeEntry {
	manifest: Download,
	version: RuntimeVersion,
}

/// Платформа → компонент (`java-runtime-gamma`, `jre-legacy`…) → сборки.
type RuntimeIndex = HashMap<String, HashMap<String, Vec<RuntimeEntry>>>;

#[derive(Debug, Deserialize)]
struct RawDownloads {
	raw: Download,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RuntimeFile {
	File {
		downloads: RawDownloads,
		#[serde(default)]
		executable: bool,
	},
	Directory,
	Link {
		target: String,
	},
}

#[derive(Debug, Deserialize)]
struct RuntimeManifest {
	files: HashMap<String, RuntimeFile>,
}

/// Что лежит в папке скачанной Java.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
	pub component: String,
	pub version: String,
	/// Разрядность сборки; нет поля (скачано до появления) — родная для ОС.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub arch: Option<String>,
}

impl Marker {
	pub fn arch(&self) -> &str {
		match self.arch.as_deref() {
			Some(arch) => arch,
			None => host_arch(),
		}
	}
}

fn host_arch() -> &'static str {
	crate::meta::rules::Platform::current().arch
}

/// Папка компонента: родная разрядность — `<компонент>`, другая (x64 на arm64) — `<компонент>-<arch>`.
fn folder_name(component: &str, arch: &str) -> String {
	if arch == host_arch() {
		component.to_owned()
	} else {
		format!("{component}-{arch}")
	}
}

/// Отметка папки Java; нет — папка не наша или недокачана.
pub fn marker(dir: &Path) -> Option<Marker> {
	let bytes = std::fs::read(dir.join(MARKER)).ok()?;
	serde_json::from_slice(&bytes).ok()
}

/// Папка нашей Java по пути к `java`: `<java>/<компонент>/bin/javaw.exe`, на macOS
/// `<java>/<компонент>/jre.bundle/Contents/Home/bin/java`.
pub fn home_of(java_path: &str, java_dir: &Path) -> Option<PathBuf> {
	let root = std::fs::canonicalize(java_dir).ok().map(super::simplify)?;
	Path::new(java_path)
		.ancestors()
		.skip(2)
		.take(4)
		.find(|home| {
			std::fs::canonicalize(home)
				.ok()
				.map(super::simplify)
				.is_some_and(|real| real.parent() == Some(root.as_path()))
		})
		.map(Path::to_path_buf)
}

/// Наша Java этой мажорной версии (и разрядности, если указана): папка и отметка.
/// Родная разрядность — первой.
pub fn installed(java_dir: &Path, major: u32, arch: Option<&str>) -> Option<(PathBuf, Marker)> {
	let mut found: Vec<(PathBuf, Marker)> = std::fs::read_dir(java_dir)
		.ok()?
		.flatten()
		.map(|e| e.path())
		.filter(|p| p.is_dir() && !is_hidden(p))
		.filter_map(|p| marker(&p).map(|m| (p, m)))
		.filter(|(_, m)| parse_major(&m.version) == Some(major))
		.filter(|(_, m)| arch.is_none_or(|a| m.arch() == a))
		.collect();
	found.sort_by(|a, b| {
		(b.1.arch() == host_arch())
			.cmp(&(a.1.arch() == host_arch()))
			.then_with(|| a.1.component.cmp(&b.1.component))
	});
	found.into_iter().next()
}

/// Служебные папки (недокачанное обновление, прежняя версия) — не Java.
pub fn is_hidden(dir: &Path) -> bool {
	dir.file_name()
		.is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

/// Версия компонента в индексе для этой ОС и разрядности.
fn latest_in(index: &RuntimeIndex, component: &str, arch: &str) -> Option<String> {
	index
		.get(platform_key(arch)?)?
		.get(component)?
		.first()
		.map(|e| e.version.name.clone())
}

fn newer(marker: &Marker, latest: Option<String>) -> Option<String> {
	latest.filter(|v| *v != marker.version)
}

/// Есть ли новая версия нашей Java — только по индексу, уже лежащему на диске (без сети).
pub fn update_cached(meta: &Meta, home: &Path) -> Option<String> {
	let marker = marker(home)?;
	let bytes = std::fs::read(meta.cache_path(INDEX_CACHE)).ok()?;
	let index: RuntimeIndex = serde_json::from_slice(&bytes).ok()?;
	newer(&marker, latest_in(&index, &marker.component, marker.arch()))
}

/// Есть ли новая версия нашей Java; индекс берётся из сети раз в сутки. Ошибки — «нет».
pub async fn update_available(
	meta: &Meta,
	home: &Path,
	cancel: &CancellationToken,
) -> Option<String> {
	let marker = marker(home)?;
	let index = load_index(meta, cancel).await.ok()?;
	newer(&marker, latest_in(&index, &marker.component, marker.arch()))
}

async fn load_index(meta: &Meta, cancel: &CancellationToken) -> Result<RuntimeIndex, AppError> {
	let path = meta.cache_path(INDEX_CACHE);
	Ok(meta
		.cached_json(RUNTIME_INDEX_URL, &path, Some(INDEX_TTL), false, cancel)
		.await?
		.value)
}

/// Индекс устарел — обновить его в фоне, чтобы список Java узнал о новых версиях.
pub fn refresh_index_soon(l: std::sync::Arc<crate::launcher::Launcher>) {
	if crate::meta::is_fresh(&l.meta.cache_path(INDEX_CACHE), Some(INDEX_TTL)) {
		return;
	}
	tauri::async_runtime::spawn(async move {
		if let Err(err) = load_index(&l.meta, &CancellationToken::new()).await {
			log::debug!("индекс Java не обновлён: {:?}", err.code);
		}
	});
}

/// Остатки обновлений Java прошлых запусков. Прежняя папка (`.old-<компонент>-<время>`)
/// удаляется, только если компонент на месте; иначе подмена оборвалась посередине —
/// целая прежняя папка возвращается на место, недоудалённая остаётся как есть.
pub fn cleanup(java_dir: &Path) {
	let Ok(entries) = std::fs::read_dir(java_dir) else {
		return;
	};
	let mut olds: Vec<(String, u128, PathBuf)> = Vec::new();
	for entry in entries.flatten() {
		let name = entry.file_name().to_string_lossy().into_owned();
		let path = entry.path();
		if !path.is_dir() {
			continue;
		}
		if let Some((component, stamp)) = old_parts(&name) {
			olds.push((component.to_owned(), stamp, path));
			continue;
		}
		let stale = name.starts_with(STAGING_PREFIX)
			&& entry
				.metadata()
				.and_then(|m| m.modified())
				.ok()
				.and_then(|t| t.elapsed().ok())
				.is_some_and(|age| age > STAGING_AGE);
		if stale {
			remove_leftover(&path);
		}
	}
	// Самая новая прежняя папка компонента — первой: её и возвращаем.
	olds.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
	for (component, _, path) in olds {
		let target = java_dir.join(&component);
		if target.exists() {
			remove_leftover(&path);
		} else if is_complete(&path) {
			match std::fs::rename(&path, &target) {
				Ok(()) => log::warn!("Java {component} возвращена из {}", path.display()),
				Err(err) => log::warn!("не удалось вернуть {}: {err}", path.display()),
			}
		} else {
			log::warn!(
				"оставлено как есть: {} (компонента {component} нет)",
				path.display()
			);
		}
	}
}

/// `.old-<компонент>-<время>` → компонент и время.
fn old_parts(name: &str) -> Option<(&str, u128)> {
	let rest = name.strip_prefix(OLD_PREFIX)?;
	let (component, stamp) = rest.rsplit_once('-')?;
	if component.is_empty() || !fs_util::is_safe_id(component) {
		return None;
	}
	Some((component, stamp.parse().ok()?))
}

fn remove_leftover(path: &Path) {
	match std::fs::remove_dir_all(path) {
		Ok(()) => log::info!("дочищено: {}", path.display()),
		Err(err) => log::warn!("не удалось дочистить {}: {err}", path.display()),
	}
}

/// Java занята запущенной игрой (`details: "game"`).
pub(crate) fn in_use_error(dir: &Path) -> AppError {
	log::info!("Java занята игрой: {}", dir.display());
	AppError::coded(ErrorCode::FileInUse, "game")
}

/// Папку Java держит не игра, а другая программа (процессоры Forge, антивирус…): `details: "locked"`.
fn locked_error(dir: &Path, err: &std::io::Error) -> AppError {
	log::warn!("папка Java занята: {}: {err}", dir.display());
	AppError::coded(ErrorCode::FileInUse, "locked")
}

/// Новая версия встаёт на место прежней переименованием: путь к Java не меняется.
fn swap(target: &Path, staging: &Path, java_dir: &Path, name: &str) -> Result<(), AppError> {
	let stamp = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_millis())
		.unwrap_or(0);
	let old = java_dir.join(format!("{OLD_PREFIX}{name}-{stamp}"));
	std::fs::rename(target, &old).map_err(|err| match err.raw_os_error() {
		// Отказ в доступе, файл занят: из этой Java что-то запущено, но не игра — её проверили раньше.
		Some(5 | 32 | 33) => locked_error(target, &err),
		_ => AppError::storage(format!("{}: {err}", target.display())),
	})?;
	if let Err(err) = std::fs::rename(staging, target) {
		// Не вернулась и прежняя — она остаётся в `.old-*`, `cleanup` вернёт её при запуске.
		if let Err(back) = std::fs::rename(&old, target) {
			log::error!(
				"Java {name}: новая не встала ({err}), прежняя не вернулась ({back}), лежит в {}",
				old.display()
			);
		}
		return Err(AppError::storage(format!("{}: {err}", target.display())));
	}
	std::thread::spawn(move || {
		if let Err(err) = std::fs::remove_dir_all(&old) {
			log::warn!("не удалось удалить прежнюю Java {}: {err}", old.display());
		}
	});
	Ok(())
}

/// Ключ платформы в индексе Mojang для этой ОС и разрядности Java (`x64`, `x86`, `arm64`).
fn platform_key(arch: &str) -> Option<&'static str> {
	match (std::env::consts::OS, arch) {
		("windows", "x64") => Some("windows-x64"),
		("windows", "x86") => Some("windows-x86"),
		("windows", "arm64") => Some("windows-arm64"),
		("linux", "x64") => Some("linux"),
		("linux", "x86") => Some("linux-i386"),
		("macos", "x64") => Some("mac-os"),
		("macos", "arm64") => Some("mac-os-arm64"),
		_ => None,
	}
}

/// Ссылка из манифеста рантайма (`file` → `target`, путь от папки ссылки) не выводит из папки Java.
fn link_inside(file: &str, target: &str) -> bool {
	use std::path::Component;
	if target.is_empty() || Path::new(target).has_root() {
		return false;
	}
	let mut depth: usize = 0;
	let parent = Path::new(file).parent().unwrap_or(Path::new(""));
	for part in parent.components().chain(Path::new(target).components()) {
		match part {
			Component::Normal(_) => depth += 1,
			Component::CurDir => {}
			Component::ParentDir => {
				let Some(up) = depth.checked_sub(1) else {
					return false;
				};
				depth = up;
			}
			Component::RootDir | Component::Prefix(_) => return false,
		}
	}
	true
}

#[cfg(unix)]
fn make_links(links: &[(PathBuf, String)]) -> Result<(), AppError> {
	for (path, target) in links {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)
				.map_err(|e| AppError::storage(format!("{}: {e}", parent.display())))?;
		}
		if std::fs::symlink_metadata(path).is_ok() {
			let _ = std::fs::remove_file(path);
		}
		std::os::unix::fs::symlink(target, path)
			.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?;
	}
	Ok(())
}

#[cfg(not(unix))]
fn make_links(_links: &[(PathBuf, String)]) -> Result<(), AppError> {
	Ok(())
}

/// Компонент для мажорной версии: указанный в описании версии игры, иначе первый с той же версией.
fn choose<'a>(
	components: &'a HashMap<String, Vec<RuntimeEntry>>,
	wanted: Option<&str>,
	major: u32,
) -> Option<(&'a str, &'a RuntimeEntry)> {
	if let Some(name) = wanted
		&& let Some((key, list)) = components.get_key_value(name)
		&& let Some(entry) = list.first()
		&& parse_major(&entry.version.name) == Some(major)
	{
		return Some((key.as_str(), entry));
	}
	let mut matches: Vec<(&str, &RuntimeEntry)> = components
		.iter()
		.filter(|(name, _)| name.starts_with("java-runtime") || name.starts_with("jre-"))
		.filter(|(name, _)| !name.ends_with("-snapshot"))
		.filter_map(|(name, list)| list.first().map(|e| (name.as_str(), e)))
		.filter(|(_, e)| parse_major(&e.version.name) == Some(major))
		.collect();
	matches.sort_by(|a, b| a.0.cmp(b.0));
	matches.into_iter().next()
}

/// Сборка Java в индексе: для этой разрядности, а если её нет на arm64 (Java 8 и 16 у Mojang
/// только x64) — x64, она работает через эмуляцию.
fn choose_build<'a>(
	index: &'a RuntimeIndex,
	component: Option<&str>,
	major: u32,
	arch: &str,
) -> Result<(&'static str, &'a str, &'a RuntimeEntry), AppError> {
	let not_found = |why: String| AppError::coded(ErrorCode::JavaNotFound, why);
	let mut archs = vec![crate::meta::rules::normalize_arch(arch)];
	if archs[0] == "arm64" {
		archs.push("x64");
	}
	let mut last = None;
	for arch in archs {
		let Some(platform) = platform_key(arch) else {
			continue;
		};
		last = Some(platform);
		if let Some((name, entry)) = index
			.get(platform)
			.and_then(|components| choose(components, component, major))
		{
			return Ok((arch, name, entry));
		}
	}
	Err(not_found(match last {
		Some(platform) => format!("нет Java {major} для {platform}"),
		None => format!("нет сборок Java для {} {arch}", std::env::consts::OS),
	}))
}

pub fn is_complete(dir: &Path) -> bool {
	dir.join(MARKER).is_file()
}

/// Скачивает Java нужной мажорной версии в `<java_dir>/<компонент>/` и возвращает её.
/// Там уже другая версия — новая качается рядом и подменяет её целиком; если прежней
/// пользуется игра (`in_use`), — `file_in_use`.
#[allow(clippy::too_many_arguments)]
pub async fn install(
	meta: &Meta,
	dl: &Downloader,
	java_dir: &Path,
	component: Option<&str>,
	major: u32,
	arch: &str,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
	in_use: &(dyn Fn(&Path) -> bool + Sync),
) -> Result<JavaInstall, AppError> {
	let index = load_index(meta, cancel).await?;
	let (arch, name, entry) = choose_build(&index, component, major, arch)?;
	log::info!("скачиваем Java {} ({name}, {arch})", entry.version.name);
	let folder = folder_name(name, arch);

	let bytes = dl
		.bytes(&entry.manifest.url, Some(&entry.manifest.sha1), cancel)
		.await?;
	let manifest: RuntimeManifest = serde_json::from_slice(&bytes)
		.map_err(|e| AppError::coded(ErrorCode::DownloadFailed, format!("манифест Java: {e}")))?;

	let target = java_dir.join(&folder);
	let marker_path = target.join(MARKER);
	// Другая версия того же компонента: качаем рядом, прежняя работает до подмены.
	let replace = marker(&target).is_some_and(|old| old.version != entry.version.name);
	if replace && in_use(&target) {
		return Err(in_use_error(&target));
	}
	let work = if replace {
		let staging = java_dir.join(format!("{STAGING_PREFIX}{folder}"));
		if staging.exists() {
			tokio::fs::remove_dir_all(&staging)
				.await
				.map_err(|e| AppError::storage(format!("{}: {e}", staging.display())))?;
		}
		staging
	} else {
		let _ = tokio::fs::remove_file(&marker_path).await;
		target.clone()
	};

	let mut jobs = Vec::new();
	let mut executables = Vec::new();
	let mut links = Vec::new();
	for (file, info) in &manifest.files {
		let path = safe_join(&work, file).ok_or_else(|| {
			AppError::coded(ErrorCode::DownloadFailed, format!("плохой путь: {file}"))
		})?;
		match info {
			RuntimeFile::File {
				downloads,
				executable,
			} => {
				if *executable {
					executables.push(path.clone());
				}
				jobs.push(Job {
					url: downloads.raw.url.clone(),
					path,
					sha1: Some(downloads.raw.sha1.clone()),
					size: Some(downloads.raw.size),
				});
			}
			RuntimeFile::Directory => {
				tokio::fs::create_dir_all(&path)
					.await
					.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?;
			}
			// Ссылки бывают только в сборках для macOS/Linux.
			RuntimeFile::Link { target } => {
				if !link_inside(file, target) {
					return Err(AppError::coded(
						ErrorCode::DownloadFailed,
						format!("плохая ссылка: {file} → {target}"),
					));
				}
				links.push((path, target.clone()));
			}
		}
	}
	dl.files(jobs, Verify::Hash, progress, cancel).await?;
	set_executable(&executables);
	make_links(&links)?;
	if replace {
		if in_use(&target) {
			let _ = tokio::fs::remove_dir_all(&work).await;
			return Err(in_use_error(&target));
		}
		let (target, work, root, folder) = (
			target.clone(),
			work.clone(),
			java_dir.to_path_buf(),
			folder.clone(),
		);
		tokio::task::spawn_blocking(move || swap(&target, &work, &root, &folder))
			.await
			.map_err(|e| AppError::unknown(e.to_string()))??;
		log::info!("Java {name} обновлена до {}", entry.version.name);
	}

	let marker = serde_json::to_vec(&Marker {
		component: name.to_owned(),
		version: entry.version.name.clone(),
		arch: Some(arch.to_owned()),
	})
	.map_err(|e| AppError::unknown(e.to_string()))?;
	fs_util::write_atomic(&marker_path, &marker)
		.map_err(|e| AppError::storage(format!("{}: {e}", marker_path.display())))?;

	super::check(target, java_dir.to_path_buf()).await
}

#[cfg(unix)]
fn set_executable(paths: &[PathBuf]) {
	use std::os::unix::fs::PermissionsExt;
	for path in paths {
		let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
	}
}

#[cfg(not(unix))]
fn set_executable(_paths: &[PathBuf]) {}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn components() -> HashMap<String, Vec<RuntimeEntry>> {
		serde_json::from_value(json!({
			"java-runtime-gamma": [{ "manifest": { "sha1": "a", "size": 1, "url": "u1" }, "version": { "name": "17.0.15" } }],
			"java-runtime-gamma-snapshot": [{ "manifest": { "sha1": "a", "size": 1, "url": "u1" }, "version": { "name": "17.0.15" } }],
			"java-runtime-delta": [{ "manifest": { "sha1": "b", "size": 1, "url": "u2" }, "version": { "name": "21.0.7" } }],
			"jre-legacy": [{ "manifest": { "sha1": "c", "size": 1, "url": "u3" }, "version": { "name": "8u51-cacert462b08" } }],
			"minecraft-java-exe": [{ "manifest": { "sha1": "d", "size": 1, "url": "u4" }, "version": { "name": "14" } }],
			"java-runtime-epsilon": []
		}))
		.unwrap()
	}

	#[test]
	fn chooses_component() {
		let c = components();
		assert_eq!(
			choose(&c, Some("java-runtime-delta"), 21).unwrap().0,
			"java-runtime-delta"
		);
		assert_eq!(
			choose(&c, Some("java-runtime-gamma"), 17).unwrap().0,
			"java-runtime-gamma"
		);
		assert_eq!(choose(&c, None, 17).unwrap().0, "java-runtime-gamma");
		assert_eq!(choose(&c, None, 8).unwrap().0, "jre-legacy");
		assert_eq!(
			choose(&c, Some("java-runtime-epsilon"), 21).unwrap().0,
			"java-runtime-delta"
		);
		assert!(choose(&c, None, 14).is_none());
		assert!(choose(&c, None, 11).is_none());
	}

	#[test]
	fn finds_installed_and_updates() {
		let dir = tempfile::tempdir().unwrap();
		let write = |name: &str, version: &str| {
			let home = dir.path().join(name);
			std::fs::create_dir_all(home.join("bin")).unwrap();
			let m = Marker {
				component: name.trim_start_matches('.').to_owned(),
				version: version.to_owned(),
				arch: None,
			};
			std::fs::write(home.join(MARKER), serde_json::to_vec(&m).unwrap()).unwrap();
			home
		};
		let delta = write("java-runtime-delta", "21.0.3");
		write(".old-java-runtime-gamma-1", "17.0.8");
		std::fs::create_dir_all(dir.path().join("half")).unwrap();

		let (home, m) = installed(dir.path(), 21, None).unwrap();
		assert_eq!(home, delta);
		assert_eq!(m.component, "java-runtime-delta");
		assert_eq!(m.arch(), host_arch());
		assert!(installed(dir.path(), 21, Some(host_arch())).is_some());
		assert!(installed(dir.path(), 21, Some("x86")).is_none());
		assert!(installed(dir.path(), 17, None).is_none());
		assert!(installed(dir.path(), 8, None).is_none());

		let java = delta.join("bin").join("javaw.exe");
		std::fs::write(&java, b"").unwrap();
		assert_eq!(
			home_of(&java.to_string_lossy(), dir.path()).as_deref(),
			Some(delta.as_path())
		);
		// Раскладка macOS: jre.bundle/Contents/Home/bin/java.
		let bundle = delta.join("jre.bundle/Contents/Home/bin");
		std::fs::create_dir_all(&bundle).unwrap();
		std::fs::write(bundle.join("java"), b"").unwrap();
		assert_eq!(
			home_of(&bundle.join("java").to_string_lossy(), dir.path()).as_deref(),
			Some(delta.as_path())
		);
		let outside = tempfile::tempdir().unwrap();
		std::fs::create_dir_all(outside.path().join("jdk/bin")).unwrap();
		let foreign = outside.path().join("jdk/bin/javaw.exe");
		assert!(home_of(&foreign.to_string_lossy(), dir.path()).is_none());

		let index: RuntimeIndex = serde_json::from_value(json!({
			platform_key(host_arch()).unwrap(): {
				"java-runtime-delta": [{ "manifest": { "sha1": "b", "size": 1, "url": "u2" }, "version": { "name": "21.0.7" } }]
			}
		}))
		.unwrap();
		assert_eq!(
			newer(&m, latest_in(&index, &m.component, m.arch())).as_deref(),
			Some("21.0.7")
		);
		let current = Marker {
			version: "21.0.7".into(),
			..m.clone()
		};
		assert!(newer(&current, latest_in(&index, &m.component, m.arch())).is_none());
		assert!(latest_in(&index, "jre-legacy", host_arch()).is_none());
	}

	#[test]
	fn swaps_whole_folder() {
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("java-runtime-delta");
		let staging = dir.path().join(".update-java-runtime-delta");
		std::fs::create_dir_all(&target).unwrap();
		std::fs::write(target.join("old.txt"), b"old").unwrap();
		std::fs::create_dir_all(&staging).unwrap();
		std::fs::write(staging.join("new.txt"), b"new").unwrap();
		swap(&target, &staging, dir.path(), "java-runtime-delta").unwrap();
		assert!(target.join("new.txt").is_file());
		assert!(!target.join("old.txt").exists());
		assert!(!staging.exists());
	}

	#[cfg(windows)]
	#[test]
	fn swap_refuses_busy_folder() {
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("java-runtime-delta");
		let staging = dir.path().join(".update-java-runtime-delta");
		std::fs::create_dir_all(&target).unwrap();
		std::fs::create_dir_all(&staging).unwrap();
		// Открытый без общего удаления файл — как запущенная из папки Java.
		let _open = std::fs::File::create(target.join("javaw.exe")).unwrap();
		let err = swap(&target, &staging, dir.path(), "java-runtime-delta").unwrap_err();
		assert_eq!(err.code, ErrorCode::FileInUse);
		assert_eq!(err.details.as_deref(), Some("locked"));
		assert!(target.is_dir() && staging.is_dir());
	}

	#[test]
	fn cleans_leftovers() {
		let dir = tempfile::tempdir().unwrap();
		for name in [
			".old-java-runtime-delta-1",
			".update-x",
			"java-runtime-delta",
		] {
			std::fs::create_dir_all(dir.path().join(name)).unwrap();
		}
		cleanup(dir.path());
		assert!(!dir.path().join(".old-java-runtime-delta-1").exists());
		// Свежее недокачанное обновление может ещё идти.
		assert!(dir.path().join(".update-x").is_dir());
		assert!(dir.path().join("java-runtime-delta").is_dir());
	}

	#[test]
	fn restores_old_when_target_missing() {
		let dir = tempfile::tempdir().unwrap();
		let older = dir.path().join(".old-java-runtime-delta-100");
		let newer = dir.path().join(".old-java-runtime-delta-200");
		let partial = dir.path().join(".old-java-runtime-gamma-5");
		for p in [&older, &newer, &partial] {
			std::fs::create_dir_all(p).unwrap();
		}
		std::fs::write(older.join(MARKER), b"{}").unwrap();
		std::fs::write(newer.join(MARKER), b"{}").unwrap();
		cleanup(dir.path());
		let target = dir.path().join("java-runtime-delta");
		assert!(target.join(MARKER).is_file());
		assert!(!newer.exists());
		// Компонент на месте — более старая прежняя удаляется.
		assert!(!older.exists());
		// Без отметки не возвращаем и не удаляем.
		assert!(partial.is_dir());
		assert!(!dir.path().join("java-runtime-gamma").exists());
	}

	#[test]
	fn falls_back_to_x64_on_arm64() {
		if std::env::consts::OS != "macos" {
			return;
		}
		let index: RuntimeIndex = serde_json::from_value(json!({
			"mac-os": {
				"jre-legacy": [{ "manifest": { "sha1": "c", "size": 1, "url": "u3" }, "version": { "name": "8u74" } }],
				"java-runtime-gamma": [{ "manifest": { "sha1": "a", "size": 1, "url": "u1" }, "version": { "name": "17.0.15" } }]
			},
			"mac-os-arm64": {
				"jre-legacy": [],
				"java-runtime-gamma": [{ "manifest": { "sha1": "a", "size": 1, "url": "u2" }, "version": { "name": "17.0.15" } }]
			}
		}))
		.unwrap();
		let (arch, name, entry) = choose_build(&index, Some("jre-legacy"), 8, "arm64").unwrap();
		assert_eq!((arch, name), ("x64", "jre-legacy"));
		assert_eq!(entry.manifest.url, "u3");
		let (arch, _, entry) =
			choose_build(&index, Some("java-runtime-gamma"), 17, "arm64").unwrap();
		assert_eq!((arch, entry.manifest.url.as_str()), ("arm64", "u2"));
		let (arch, _, _) = choose_build(&index, None, 17, "x64").unwrap();
		assert_eq!(arch, "x64");
		assert!(choose_build(&index, None, 21, "x64").is_err());
	}

	#[test]
	fn folders_by_arch() {
		assert_eq!(folder_name("jre-legacy", host_arch()), "jre-legacy");
		assert_eq!(folder_name("jre-legacy", "x86"), "jre-legacy-x86");
	}

	#[test]
	fn checks_links() {
		assert!(link_inside(
			"jre.bundle/Contents/MacOS/libjli.dylib",
			"../Home/lib/libjli.dylib"
		));
		assert!(link_inside("a/b", "c"));
		assert!(!link_inside("a/b", "../../x"));
		assert!(!link_inside("a", "/etc/passwd"));
		assert!(!link_inside("a", ""));
	}

	#[cfg(unix)]
	#[test]
	fn makes_links() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir_all(dir.path().join("Home/lib")).unwrap();
		std::fs::write(dir.path().join("Home/lib/x"), b"x").unwrap();
		let link = dir.path().join("MacOS/x");
		make_links(&[(link.clone(), "../Home/lib/x".into())]).unwrap();
		make_links(&[(link.clone(), "../Home/lib/x".into())]).unwrap();
		assert_eq!(std::fs::read(&link).unwrap(), b"x");
	}

	#[test]
	fn rejects_bad_paths() {
		let root = Path::new("root");
		assert!(safe_join(root, "bin/javaw.exe").is_some());
		assert!(safe_join(root, "../evil").is_none());
		assert!(safe_join(root, "/abs").is_none());
	}

	#[test]
	fn parses_manifest() {
		let m: RuntimeManifest = serde_json::from_value(json!({ "files": {
			"bin": { "type": "directory" },
			"bin/javaw.exe": { "type": "file", "executable": true, "downloads": {
				"raw": { "sha1": "s", "size": 3, "url": "u" },
				"lzma": { "sha1": "t", "size": 2, "url": "v" }
			}},
			"legal/x": { "type": "link", "target": "../y" }
		}}))
		.unwrap();
		assert_eq!(m.files.len(), 3);
	}
}
