//! Метаданные: версии Minecraft (манифест Mojang) и загрузчики (launcher-meta.modrinth.com).
//! Всё кэшируется на диске в `meta/`; без сети работаем по кэшу.

pub mod model;
pub mod rules;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use self::model::{
	GAME_VERSION_PLACEHOLDER, LatestVersions, LoaderManifest, LoaderVersion, ManifestVersion,
	PartialVersionInfo, VersionInfo, VersionManifest, VersionType,
};
use crate::download::{DownloadError, Downloader};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::paths::Layout;

pub const MOJANG_MANIFEST_URL: &str =
	"https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const MODRINTH_META_URL: &str = "https://launcher-meta.modrinth.com";
/// Сколько манифесты версий и загрузчиков считаются свежими.
const MANIFEST_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
	Vanilla,
	Fabric,
	Forge,
	Neoforge,
	Quilt,
}

impl LoaderKind {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Vanilla => "vanilla",
			Self::Fabric => "fabric",
			Self::Forge => "forge",
			Self::Neoforge => "neoforge",
			Self::Quilt => "quilt",
		}
	}

	/// Путь манифеста на launcher-meta.modrinth.com (с версией формата, как в Modrinth App).
	fn meta_path(self) -> Option<&'static str> {
		match self {
			Self::Vanilla => None,
			Self::Fabric => Some("fabric/v0"),
			Self::Forge => Some("forge/v0"),
			Self::Neoforge => Some("neo/v0"),
			Self::Quilt => Some("quilt/v1"),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameVersionEntry {
	pub id: String,
	pub kind: VersionType,
	pub release_time: String,
	/// Клиент этой версии уже скачан в общую папку `meta/versions`.
	pub downloaded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameVersions {
	pub latest: LatestVersions,
	pub versions: Vec<GameVersionEntry>,
	/// Сети нет: показан старый кэш.
	pub offline: bool,
}

/// Список из метаданных и признак «сети нет, показан старый кэш».
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed<T> {
	pub items: Vec<T>,
	pub offline: bool,
}

/// Метаданные и откуда они: `offline` — сеть недоступна, взят старый кэш.
#[derive(Debug, Clone)]
pub struct Fetched<T> {
	pub value: T,
	pub offline: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderVersionEntry {
	pub id: String,
	pub stable: bool,
	/// Версия, которую лаунчер выберет сам, если пользователь не указал свою.
	pub recommended: bool,
}

/// Готовое описание версии для установки и запуска.
#[derive(Debug, Clone)]
pub struct Resolved {
	/// Описание версии (с загрузчиком — объединённое).
	pub info: VersionInfo,
	/// Id версии в `meta/versions`: `1.20.1` или `1.20.1-fabric-0.16.14`.
	pub version_id: String,
	/// Версия Minecraft.
	pub game_version: String,
	pub loader_version: Option<String>,
}

#[derive(Clone)]
pub struct Meta {
	layout: Layout,
	dl: Downloader,
}

impl Meta {
	pub fn new(layout: Layout, dl: Downloader) -> Self {
		Self { layout, dl }
	}

	pub fn cache_path(&self, name: &str) -> PathBuf {
		self.layout.cache_dir().join(name)
	}

	/// Файл из кэша, если свежий; иначе из сети. Сеть недоступна — старый кэш (`offline`).
	async fn cached(
		&self,
		url: &str,
		path: &Path,
		ttl: Option<Duration>,
		force: bool,
		cancel: &CancellationToken,
	) -> Result<Fetched<Vec<u8>>, AppError> {
		if !force && is_fresh(path, ttl) {
			match tokio::fs::read(path).await {
				Ok(bytes) => return Ok(Fetched::online(bytes)),
				Err(err) => log::warn!("кэш {} не прочитан: {err}", path.display()),
			}
		}
		match self.dl.bytes(url, None, cancel).await {
			Ok(bytes) => {
				if let Err(err) = fs_util::write_atomic(path, &bytes) {
					log::warn!("не удалось сохранить кэш {}: {err}", path.display());
				}
				Ok(Fetched::online(bytes))
			}
			Err(DownloadError::Cancelled) => Err(AppError::new(ErrorCode::Cancelled)),
			Err(err) => match tokio::fs::read(path).await {
				Ok(bytes) => {
					log::warn!("{err}; используем кэш {}", path.display());
					Ok(Fetched {
						value: bytes,
						offline: true,
					})
				}
				Err(_) => Err(fetch_error(err)),
			},
		}
	}

	pub async fn cached_json<T: serde::de::DeserializeOwned>(
		&self,
		url: &str,
		path: &Path,
		ttl: Option<Duration>,
		force: bool,
		cancel: &CancellationToken,
	) -> Result<Fetched<T>, AppError> {
		let fetched = self.cached(url, path, ttl, force, cancel).await?;
		match serde_json::from_slice(&fetched.value) {
			Ok(value) => Ok(Fetched {
				value,
				offline: fetched.offline,
			}),
			Err(err) if !force => {
				log::warn!("{} не разобран ({err}), качаем заново", path.display());
				let fetched = self.cached(url, path, ttl, true, cancel).await?;
				let value = serde_json::from_slice(&fetched.value).map_err(|e| bad_meta(url, e))?;
				Ok(Fetched {
					value,
					offline: fetched.offline,
				})
			}
			Err(err) => Err(bad_meta(url, err)),
		}
	}

	pub async fn version_manifest(
		&self,
		force: bool,
		cancel: &CancellationToken,
	) -> Result<VersionManifest, AppError> {
		Ok(self.version_manifest_fetched(force, cancel).await?.value)
	}

	pub(crate) async fn version_manifest_fetched(
		&self,
		force: bool,
		cancel: &CancellationToken,
	) -> Result<Fetched<VersionManifest>, AppError> {
		let path = self.cache_path("version_manifest_v2.json");
		self.cached_json(
			MOJANG_MANIFEST_URL,
			&path,
			Some(MANIFEST_TTL),
			force,
			cancel,
		)
		.await
	}

	pub async fn game_versions(
		&self,
		kinds: Option<&[VersionType]>,
		force: bool,
		cancel: &CancellationToken,
	) -> Result<GameVersions, AppError> {
		let Fetched {
			value: manifest,
			offline,
		} = self.version_manifest_fetched(force, cancel).await?;
		let dir = self.layout.versions_dir();
		let downloaded = tokio::task::spawn_blocking(move || downloaded_versions(&dir))
			.await
			.unwrap_or_default();
		let versions = manifest
			.versions
			.into_iter()
			.filter(|v| kinds.is_none_or(|k| k.contains(&v.kind)))
			.map(|v| GameVersionEntry {
				downloaded: downloaded.contains(&v.id),
				id: v.id,
				kind: v.kind,
				release_time: v.release_time,
			})
			.collect();
		Ok(GameVersions {
			latest: manifest.latest,
			versions,
			offline,
		})
	}

	/// Запись версии в манифесте; если её нет в кэше, манифест обновляется (вдруг версия вышла недавно).
	pub async fn find_version(
		&self,
		game_version: &str,
		cancel: &CancellationToken,
	) -> Result<ManifestVersion, AppError> {
		let manifest = self.version_manifest(false, cancel).await?;
		let found = match manifest.versions.into_iter().find(|v| v.id == game_version) {
			Some(v) => v,
			None => self
				.version_manifest(true, cancel)
				.await?
				.versions
				.into_iter()
				.find(|v| v.id == game_version)
				.ok_or_else(|| {
					AppError::coded(ErrorCode::VersionNotFound, game_version.to_owned())
				})?,
		};
		check_id("версия игры", &found.id)?;
		Ok(found)
	}

	/// Описание версии Mojang (`versions/<id>/<id>.json`), проверяется по sha1 из манифеста.
	pub async fn version_info(
		&self,
		entry: &ManifestVersion,
		cancel: &CancellationToken,
	) -> Result<VersionInfo, AppError> {
		let path = self.layout.version_json(&entry.id);
		if let Ok(bytes) = tokio::fs::read(&path).await {
			let hash_ok = entry
				.sha1
				.as_deref()
				.is_none_or(|sha1| crate::download::hex_sha1(&bytes) == sha1);
			if hash_ok && let Ok(info) = serde_json::from_slice(&bytes) {
				return Ok(info);
			}
		}
		let bytes = match self
			.dl
			.bytes(&entry.url, entry.sha1.as_deref(), cancel)
			.await
		{
			Ok(bytes) => bytes,
			Err(err) => return Err(fetch_error(err)),
		};
		let info = serde_json::from_slice(&bytes).map_err(|e| bad_meta(&entry.url, e))?;
		fs_util::write_atomic(&path, &bytes)
			.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?;
		Ok(info)
	}

	/// Перекачивает описание версии, если на диске его нет или sha1 разошёлся с манифестом.
	/// `true` — файл заменён.
	pub async fn refresh_version_info(
		&self,
		entry: &ManifestVersion,
		cancel: &CancellationToken,
	) -> Result<bool, AppError> {
		let Some(sha1) = entry.sha1.as_deref() else {
			return Ok(false);
		};
		let path = self.layout.version_json(&entry.id);
		if let Ok(bytes) = tokio::fs::read(&path).await
			&& crate::download::hex_sha1(&bytes) == sha1
		{
			return Ok(false);
		}
		let bytes = self
			.dl
			.bytes(&entry.url, Some(sha1), cancel)
			.await
			.map_err(fetch_error)?;
		let info: VersionInfo =
			serde_json::from_slice(&bytes).map_err(|e| bad_meta(&entry.url, e))?;
		check_version_info(&info)?;
		fs_util::write_atomic(&path, &bytes)
			.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?;
		Ok(true)
	}

	pub async fn loader_manifest(
		&self,
		kind: LoaderKind,
		force: bool,
		cancel: &CancellationToken,
	) -> Result<Fetched<LoaderManifest>, AppError> {
		let Some(meta_path) = kind.meta_path() else {
			return Ok(Fetched::online(LoaderManifest {
				game_versions: Vec::new(),
				version_groups: Vec::new(),
			}));
		};
		let url = format!("{MODRINTH_META_URL}/{meta_path}/manifest.json");
		let path = self.cache_path(&format!("loaders/{}.json", kind.as_str()));
		self.cached_json(&url, &path, Some(MANIFEST_TTL), force, cancel)
			.await
	}

	/// Версии игры, которые поддерживает загрузчик (в порядке манифеста загрузчика).
	pub async fn loader_game_versions(
		&self,
		kind: LoaderKind,
		cancel: &CancellationToken,
	) -> Result<Listed<String>, AppError> {
		if kind == LoaderKind::Vanilla {
			let fetched = self.version_manifest_fetched(false, cancel).await?;
			return Ok(Listed {
				items: fetched.value.versions.into_iter().map(|v| v.id).collect(),
				offline: fetched.offline,
			});
		}
		let fetched = self.loader_manifest(kind, false, cancel).await?;
		Ok(Listed {
			items: fetched.value.game_versions(),
			offline: fetched.offline,
		})
	}

	/// Версии загрузчика для версии игры. Пустой список — версия игры не поддерживается.
	pub async fn loader_versions(
		&self,
		kind: LoaderKind,
		game_version: &str,
		cancel: &CancellationToken,
	) -> Result<Listed<LoaderVersionEntry>, AppError> {
		if kind == LoaderKind::Vanilla {
			return Ok(Listed {
				items: Vec::new(),
				offline: false,
			});
		}
		let Fetched {
			value: manifest,
			offline,
		} = self.loader_manifest(kind, false, cancel).await?;
		Ok(Listed {
			items: loader_entries(&manifest, game_version),
			offline,
		})
	}

	/// Находит версию загрузчика (указанную или рекомендуемую). Если её нет в кэше манифеста — обновляет его.
	async fn pick_loader(
		&self,
		kind: LoaderKind,
		game_version: &str,
		wanted: Option<&str>,
		cancel: &CancellationToken,
	) -> Result<LoaderVersion, AppError> {
		let mut force = false;
		loop {
			let manifest = self.loader_manifest(kind, force, cancel).await?.value;
			let found = manifest
				.loaders_for(game_version)
				.and_then(|list| match wanted {
					Some(id) => list.iter().find(|l| l.id == id),
					None => recommended(list),
				});
			if let Some(found) = found {
				check_id("версия загрузчика", &found.id)?;
				return Ok(found.clone());
			}
			if force {
				return Err(AppError::coded(
					ErrorCode::LoaderNotFound,
					format!(
						"{} {} для Minecraft {game_version}",
						kind.as_str(),
						wanted.unwrap_or("(любая)")
					),
				));
			}
			force = true;
		}
	}

	async fn loader_partial(
		&self,
		kind: LoaderKind,
		loader: &LoaderVersion,
		game_version: &str,
		refetch: bool,
		cancel: &CancellationToken,
	) -> Result<PartialVersionInfo, AppError> {
		let safe_id: String = loader
			.id
			.chars()
			.map(|c| {
				if c.is_ascii_alphanumeric() || ".-_+".contains(c) {
					c
				} else {
					'_'
				}
			})
			.collect();
		let path = self.cache_path(&format!("loaders/{}/{safe_id}.json", kind.as_str()));
		let bytes = self
			.cached(&loader.url, &path, None, refetch, cancel)
			.await?
			.value;
		let text = String::from_utf8_lossy(&bytes).replace(GAME_VERSION_PLACEHOLDER, game_version);
		serde_json::from_str(&text).map_err(|e| bad_meta(&loader.url, e))
	}

	/// Описание версии для установки: Minecraft + загрузчик. Результат пишется в `meta/versions/<id>/<id>.json`.
	pub async fn resolve(
		&self,
		game_version: &str,
		kind: LoaderKind,
		loader_version: Option<&str>,
		refetch: bool,
		cancel: &CancellationToken,
	) -> Result<Resolved, AppError> {
		let entry = self.find_version(game_version, cancel).await?;
		if refetch {
			let _ = tokio::fs::remove_file(self.layout.version_json(&entry.id)).await;
		}
		let base = self.version_info(&entry, cancel).await?;
		if kind == LoaderKind::Vanilla {
			check_id("версия", &base.id)?;
			check_version_info(&base)?;
			return Ok(Resolved {
				version_id: base.id.clone(),
				info: base,
				game_version: game_version.to_owned(),
				loader_version: None,
			});
		}
		let loader = self
			.pick_loader(kind, game_version, loader_version, cancel)
			.await?;
		let partial = self
			.loader_partial(kind, &loader, game_version, refetch, cancel)
			.await?;
		let version_id = format!("{game_version}-{}-{}", kind.as_str(), loader.id);
		check_id("версия", &version_id)?;
		let mut info = merge(partial, base);
		check_version_info(&info)?;
		info.id = version_id.clone();
		let path = self.layout.version_json(&version_id);
		let json = serde_json::to_vec_pretty(&info)
			.map_err(|e| AppError::unknown(format!("version json: {e}")))?;
		fs_util::write_atomic(&path, &json)
			.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?;
		Ok(Resolved {
			info,
			version_id,
			game_version: game_version.to_owned(),
			loader_version: Some(loader.id),
		})
	}
}

impl<T> Fetched<T> {
	fn online(value: T) -> Self {
		Self {
			value,
			offline: false,
		}
	}
}

/// Версии загрузчика для версии игры с пометкой рекомендуемой. Пусто — версия игры не поддерживается.
fn loader_entries(manifest: &LoaderManifest, game_version: &str) -> Vec<LoaderVersionEntry> {
	let Some(list) = manifest.loaders_for(game_version) else {
		return Vec::new();
	};
	let recommended = recommended(list).map(|l| l.id.clone());
	list.iter()
		.map(|l| LoaderVersionEntry {
			id: l.id.clone(),
			stable: l.stable,
			recommended: recommended.as_deref() == Some(l.id.as_str()),
		})
		.collect()
}

/// Id версий, у которых есть `versions/<id>/<id>.jar`. Папки описаний загрузчиков (без jar) не считаются.
fn downloaded_versions(versions_dir: &Path) -> HashSet<String> {
	let Ok(entries) = std::fs::read_dir(versions_dir) else {
		return HashSet::new();
	};
	entries
		.flatten()
		.filter_map(|e| e.file_name().into_string().ok())
		.filter(|id| versions_dir.join(id).join(format!("{id}.jar")).is_file())
		.collect()
}

/// Рекомендуемая версия загрузчика: первая стабильная, иначе самая новая.
fn recommended(list: &[LoaderVersion]) -> Option<&LoaderVersion> {
	list.iter().find(|l| l.stable).or_else(|| list.first())
}

pub(crate) fn is_fresh(path: &Path, ttl: Option<Duration>) -> bool {
	let Ok(meta) = std::fs::metadata(path) else {
		return false;
	};
	let Some(ttl) = ttl else {
		return true;
	};
	meta.modified()
		.ok()
		.and_then(|m| SystemTime::now().duration_since(m).ok())
		.is_some_and(|age| age < ttl)
}

fn fetch_error(err: DownloadError) -> AppError {
	match err {
		DownloadError::Cancelled => AppError::new(ErrorCode::Cancelled),
		err if err.is_network() => AppError::coded(ErrorCode::Network, err.to_string()),
		err => AppError::from(err),
	}
}

/// Метаданные с путём наружу из нашей папки или с неверным id/хешем.
pub fn unsafe_meta(what: impl std::fmt::Display) -> AppError {
	AppError::coded(
		ErrorCode::DownloadFailed,
		format!("небезопасные метаданные: {what}"),
	)
}

/// Id из метаданных, который станет именем файла или папки (см. [`fs_util::is_safe_id`]).
pub fn check_id(kind: &str, id: &str) -> Result<(), AppError> {
	if fs_util::is_safe_id(id) {
		Ok(())
	} else {
		Err(unsafe_meta(format!("{kind} {id:?}")))
	}
}

/// Проверяет id из описания версии, которые становятся путями: индекс ассетов и конфиг логов.
pub fn check_version_info(info: &VersionInfo) -> Result<(), AppError> {
	check_id("индекс ассетов", &info.asset_index.id)?;
	if let Some(log) = info.logging.as_ref().and_then(|l| l.get("client")) {
		check_id("конфиг логов", &log.file.id)?;
	}
	Ok(())
}

fn bad_meta(url: &str, err: serde_json::Error) -> AppError {
	AppError::coded(
		ErrorCode::DownloadFailed,
		format!("{url}: неверный JSON: {err}"),
	)
}

/// Профиль загрузчика поверх версии Minecraft, как `merge_partial_version` в daedalus.
/// Заменённые библиотеки качаются, но не идут в classpath.
pub fn merge(partial: PartialVersionInfo, base: VersionInfo) -> VersionInfo {
	let artifact = |name: &str| name.rsplit_once(':').map(|(a, _)| a.to_owned());
	let mut libraries = Vec::with_capacity(base.libraries.len() + partial.libraries.len());
	for mut lib in base.libraries {
		if let Some(key) = artifact(&lib.name)
			&& partial
				.libraries
				.iter()
				.any(|p| p.include_in_classpath && artifact(&p.name).as_deref() == Some(&key))
		{
			lib.include_in_classpath = false;
		}
		libraries.push(lib);
	}
	libraries.extend(partial.libraries);

	let arguments = match (base.arguments, partial.arguments) {
		(Some(mut base), Some(extra)) => {
			base.game.extend(extra.game);
			base.jvm.extend(extra.jvm);
			Some(base)
		}
		(base, extra) => extra.or(base),
	};

	VersionInfo {
		id: partial.id,
		arguments,
		minecraft_arguments: partial.minecraft_arguments.or(base.minecraft_arguments),
		asset_index: base.asset_index,
		assets: base.assets,
		downloads: base.downloads,
		java_version: base.java_version,
		libraries,
		logging: base.logging,
		main_class: partial.main_class.unwrap_or(base.main_class),
		kind: base.kind,
		release_time: base.release_time,
		data: partial.data,
		processors: partial.processors,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn base() -> VersionInfo {
		serde_json::from_value(json!({
			"id": "1.20.1",
			"arguments": { "game": ["--username", "${auth_player_name}"], "jvm": ["-cp", "${classpath}"] },
			"assetIndex": { "id": "5", "sha1": "a", "size": 1, "totalSize": 2, "url": "https://x/5.json" },
			"assets": "5",
			"downloads": { "client": { "sha1": "c", "size": 3, "url": "https://x/client.jar" } },
			"javaVersion": { "component": "java-runtime-gamma", "majorVersion": 17 },
			"libraries": [
				{ "name": "org.ow2.asm:asm:9.3", "downloads": { "artifact": { "path": "p", "sha1": "s", "size": 1, "url": "https://x/asm" } } },
				{ "name": "com.mojang:brigadier:1.1.8" }
			],
			"mainClass": "net.minecraft.client.main.Main",
			"type": "release",
			"releaseTime": "2023-06-12T13:25:51+00:00",
			"complianceLevel": 1
		}))
		.unwrap()
	}

	#[test]
	fn parses_mojang_version() {
		let info = base();
		assert_eq!(info.java_major(), 17);
		assert_eq!(info.downloads["client"].size, 3);
		assert!(info.libraries[0].include_in_classpath);
		assert_eq!(info.arguments.as_ref().unwrap().game.len(), 2);
	}

	#[test]
	fn merges_loader_profile() {
		let partial: PartialVersionInfo = serde_json::from_value(json!({
			"id": "fabric-loader-0.16.14-1.20.1",
			"inheritsFrom": "1.20.1",
			"mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
			"arguments": { "game": [], "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "] },
			"libraries": [
				{ "name": "org.ow2.asm:asm:9.8", "url": "https://launcher-meta.modrinth.com/maven/" },
				{ "name": "net.fabricmc:intermediary:1.20.1", "url": "https://launcher-meta.modrinth.com/maven/" }
			],
			"type": "release",
			"releaseTime": "x", "time": "y"
		}))
		.unwrap();
		let merged = merge(partial, base());
		assert_eq!(
			merged.main_class,
			"net.fabricmc.loader.impl.launch.knot.KnotClient"
		);
		assert_eq!(merged.libraries.len(), 4);
		// asm 9.3 из Minecraft заменён asm 9.8 загрузчика: качаем, но не в classpath.
		assert!(!merged.libraries[0].include_in_classpath);
		assert!(merged.libraries[1].include_in_classpath);
		assert_eq!(merged.java_major(), 17);
		let args = merged.arguments.unwrap();
		assert_eq!(args.game.len(), 2);
		assert_eq!(args.jvm.len(), 3);
	}

	#[test]
	fn loader_lists() {
		let manifest: LoaderManifest = serde_json::from_value(json!({
			"gameVersions": [
				{ "id": "${modrinth.gameVersion}", "stable": true, "loaders": [
					{ "id": "0.17.0", "url": "u1", "stable": false },
					{ "id": "0.16.14", "url": "u2", "stable": true }
				]},
				{ "id": "1.20.1", "stable": true, "loaders": [] },
				{ "id": "1.21", "stable": true, "versionGroup": "v1", "loaders": [] },
				{ "id": "1.19", "stable": true, "versionGroup": "missing", "loaders": [] }
			],
			"versionGroups": [ { "id": "v1", "loaders": [ { "id": "q1", "url": "u3", "stable": false } ] } ]
		}))
		.unwrap();
		assert_eq!(manifest.loaders_for("1.20.1").unwrap().len(), 2);
		assert_eq!(manifest.loaders_for("1.21").unwrap()[0].id, "q1");
		assert!(manifest.loaders_for("1.19").is_none());
		assert!(manifest.loaders_for("1.7.10").is_none());
		assert_eq!(manifest.game_versions(), vec!["1.20.1", "1.21"]);
		let list = manifest.loaders_for("1.20.1").unwrap();
		assert_eq!(recommended(list).unwrap().id, "0.16.14");
		let unstable = [LoaderVersion {
			id: "47.4.0".into(),
			url: String::new(),
			stable: false,
		}];
		assert_eq!(recommended(&unstable).unwrap().id, "47.4.0");
	}

	#[test]
	fn parses_real_manifest_shape() {
		let manifest: VersionManifest = serde_json::from_value(json!({
			"latest": { "release": "1.21.4", "snapshot": "25w02a" },
			"versions": [
				{ "id": "25w02a", "type": "snapshot", "url": "u", "time": "t", "releaseTime": "r", "sha1": "s", "complianceLevel": 1 },
				{ "id": "1.21.4", "type": "release", "url": "u", "time": "t", "releaseTime": "r", "sha1": "s", "complianceLevel": 1 },
				{ "id": "b1.7.3", "type": "old_beta", "url": "u", "time": "t", "releaseTime": "r", "sha1": "s", "complianceLevel": 0 },
				{ "id": "x", "type": "experiment", "url": "u", "time": "t", "releaseTime": "r", "sha1": "s", "complianceLevel": 0 }
			]
		}))
		.unwrap();
		assert_eq!(manifest.versions[2].kind, VersionType::OldBeta);
		assert_eq!(manifest.versions[3].kind, VersionType::Other);
	}

	/// HTTP-сервер на 127.0.0.1, который на всё отвечает 404.
	async fn not_found_server() -> String {
		use tokio::io::{AsyncReadExt, AsyncWriteExt};
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		tokio::spawn(async move {
			while let Ok((mut sock, _)) = listener.accept().await {
				tokio::spawn(async move {
					let mut buf = [0u8; 2048];
					let _ = sock.read(&mut buf).await;
					let _ = sock
						.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
						.await;
				});
			}
		});
		format!("http://{addr}")
	}

	fn test_meta(root: &Path) -> Meta {
		let dl = Downloader::new(crate::download::build_client().unwrap(), 4);
		Meta::new(Layout::new(root.to_path_buf()), dl)
	}

	#[tokio::test]
	async fn stale_cache_marks_offline() {
		let dir = tempfile::tempdir().unwrap();
		let meta = test_meta(dir.path());
		let base = not_found_server().await;
		let url = format!("{base}/manifest.json");
		let cancel = CancellationToken::new();
		let path = meta.cache_path("test.json");

		// Нет ни сети, ни кэша — ошибка.
		assert!(
			meta.cached_json::<serde_json::Value>(&url, &path, None, false, &cancel)
				.await
				.is_err()
		);

		fs_util::write_atomic(&path, b"{\"a\":1}").unwrap();
		// Свежий кэш — сеть не нужна, это не «офлайн».
		let fresh = meta
			.cached_json::<serde_json::Value>(&url, &path, None, false, &cancel)
			.await
			.unwrap();
		assert!(!fresh.offline);
		assert_eq!(fresh.value["a"], 1);
		// Кэш устарел (или обновление попросили явно), а сеть не ответила — старый кэш и `offline`.
		let stale = meta
			.cached_json::<serde_json::Value>(&url, &path, None, true, &cancel)
			.await
			.unwrap();
		assert!(stale.offline);
		assert_eq!(stale.value["a"], 1);
	}

	#[tokio::test]
	async fn game_versions_flags() {
		let dir = tempfile::tempdir().unwrap();
		let meta = test_meta(dir.path());
		let manifest = json!({
			"latest": { "release": "1.21.4", "snapshot": "1.21.4" },
			"versions": [
				{ "id": "1.21.4", "type": "release", "url": "u", "time": "t", "releaseTime": "r", "sha1": "s" },
				{ "id": "1.20.1", "type": "release", "url": "u", "time": "t", "releaseTime": "r", "sha1": "s" }
			]
		});
		let path = meta.cache_path("version_manifest_v2.json");
		fs_util::write_atomic(&path, manifest.to_string().as_bytes()).unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		fs_util::write_atomic(&layout.version_jar("1.20.1"), b"jar").unwrap();
		// Описание версии с загрузчиком без клиента — не «скачана».
		fs_util::write_atomic(&layout.version_json("1.21.4"), b"{}").unwrap();

		let versions = meta
			.game_versions(None, false, &CancellationToken::new())
			.await
			.unwrap();
		assert!(!versions.offline);
		let downloaded: Vec<_> = versions
			.versions
			.iter()
			.map(|v| (v.id.as_str(), v.downloaded))
			.collect();
		assert_eq!(downloaded, [("1.21.4", false), ("1.20.1", true)]);
		let json = serde_json::to_value(&versions).unwrap();
		assert_eq!(json["offline"], false);
		assert_eq!(json["versions"][1]["downloaded"], true);

		let vanilla = meta
			.loader_game_versions(LoaderKind::Vanilla, &CancellationToken::new())
			.await
			.unwrap();
		assert_eq!(vanilla.items, ["1.21.4", "1.20.1"]);
		let json = serde_json::to_value(&vanilla).unwrap();
		assert_eq!(json["offline"], false);
		assert!(json["items"].is_array());
	}
}
