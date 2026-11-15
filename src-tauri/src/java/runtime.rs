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
		#[allow(dead_code)]
		target: String,
	},
}

#[derive(Debug, Deserialize)]
struct RuntimeManifest {
	files: HashMap<String, RuntimeFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
	component: String,
	version: String,
}

/// Ключ платформы в индексе Mojang.
fn platform_key() -> Option<&'static str> {
	match (std::env::consts::OS, std::env::consts::ARCH) {
		("windows", "x86_64") => Some("windows-x64"),
		("windows", "x86") => Some("windows-x86"),
		("windows", "aarch64") => Some("windows-arm64"),
		("linux", "x86_64") => Some("linux"),
		("linux", "x86") => Some("linux-i386"),
		("macos", "x86_64") => Some("mac-os"),
		("macos", "aarch64") => Some("mac-os-arm64"),
		_ => None,
	}
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

pub fn is_complete(dir: &Path) -> bool {
	dir.join(MARKER).is_file()
}

/// Скачивает Java нужной мажорной версии в `<java_dir>/<компонент>/` и возвращает её.
pub async fn install(
	meta: &Meta,
	dl: &Downloader,
	java_dir: &Path,
	component: Option<&str>,
	major: u32,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<JavaInstall, AppError> {
	let not_found = |why: String| AppError::coded(ErrorCode::JavaNotFound, why);
	let platform = platform_key().ok_or_else(|| {
		not_found(format!(
			"нет сборок Java для {} {}",
			std::env::consts::OS,
			std::env::consts::ARCH
		))
	})?;
	let index_path = meta.cache_path("java-runtime.json");
	let index: RuntimeIndex = meta
		.cached_json(
			RUNTIME_INDEX_URL,
			&index_path,
			Some(INDEX_TTL),
			false,
			cancel,
		)
		.await?
		.value;
	let components = index
		.get(platform)
		.ok_or_else(|| not_found(format!("нет сборок Java для {platform}")))?;
	let (name, entry) = choose(components, component, major)
		.ok_or_else(|| not_found(format!("нет Java {major} для {platform}")))?;
	log::info!("скачиваем Java {} ({name})", entry.version.name);

	let bytes = dl
		.bytes(&entry.manifest.url, Some(&entry.manifest.sha1), cancel)
		.await?;
	let manifest: RuntimeManifest = serde_json::from_slice(&bytes)
		.map_err(|e| AppError::coded(ErrorCode::DownloadFailed, format!("манифест Java: {e}")))?;

	let target = java_dir.join(name);
	let marker_path = target.join(MARKER);
	// Другая версия того же компонента — ставим начисто.
	if let Ok(text) = tokio::fs::read(&marker_path).await
		&& let Ok(old) = serde_json::from_slice::<Marker>(&text)
		&& old.version != entry.version.name
	{
		tokio::fs::remove_dir_all(&target)
			.await
			.map_err(|e| AppError::storage(format!("{}: {e}", target.display())))?;
	}
	let _ = tokio::fs::remove_file(&marker_path).await;

	let mut jobs = Vec::new();
	let mut executables = Vec::new();
	for (file, info) in &manifest.files {
		let path = safe_join(&target, file).ok_or_else(|| {
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
			// Ссылки бывают только в сборках для macOS/Linux; на Windows их нет.
			RuntimeFile::Link { .. } => {}
		}
	}
	dl.files(jobs, Verify::Hash, progress, cancel).await?;
	set_executable(&executables);

	let marker = serde_json::to_vec(&Marker {
		component: name.to_owned(),
		version: entry.version.name.clone(),
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
