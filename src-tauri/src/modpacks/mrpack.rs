//! Формат модпака Modrinth (`.mrpack`): `modrinth.index.json` + `overrides/` + `client-overrides/`.
//! Описание формата — <https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack>.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::{Requirements, check_rel_path, invalid};
use crate::content::Side;
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::instances::Loader;
use crate::meta::LoaderKind;

pub const INDEX_NAME: &str = "modrinth.index.json";
/// Индекс больше этого не читаем (защита от мусора).
const INDEX_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Файлов в индексе не больше этого.
const INDEX_MAX_FILES: usize = 20_000;

/// Лимиты против zip-бомб; байты считаем по факту, заголовок архива может врать.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
	pub files: usize,
	pub bytes: u64,
}

impl Limits {
	pub const DEFAULT: Self = Self {
		files: 100_000,
		bytes: 8 * 1024 * 1024 * 1024,
	};
}

/// Откуда разрешено качать файлы модпака (как требует Modrinth для своих модпаков).
pub const ALLOWED_HOSTS: [&str; 4] = [
	"cdn.modrinth.com",
	"github.com",
	"raw.githubusercontent.com",
	"gitlab.com",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Env {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub server: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexFile {
	pub path: String,
	pub hashes: BTreeMap<String, String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub env: Option<Env>,
	pub downloads: Vec<String>,
	#[serde(default)]
	pub file_size: u64,
}

impl IndexFile {
	/// Нужен ли файл клиенту: `required` (и без `env`), `optional` или не нужен (`unsupported`).
	pub fn client_side(&self) -> Side {
		match self.env.as_ref().and_then(|e| e.client.as_deref()) {
			None => Side::Required,
			Some(text) => match Side::parse(text) {
				Side::Unknown => Side::Required,
				side => side,
			},
		}
	}

	pub fn sha1(&self) -> Option<&str> {
		self.hashes.get("sha1").map(String::as_str)
	}

	pub fn sha512(&self) -> Option<&str> {
		self.hashes
			.get("sha512")
			.map(String::as_str)
			.filter(|h| h.len() == 128 && h.bytes().all(|b| b.is_ascii_hexdigit()))
	}

	/// Первый адрес с разрешённого хоста.
	#[cfg(test)]
	pub fn allowed_url(&self) -> Option<&str> {
		self.allowed_url_in(&ALLOWED_HOSTS)
	}

	/// Первый адрес `https://` с хоста из `hosts`.
	pub fn allowed_url_in(&self, hosts: &[&str]) -> Option<&str> {
		self.downloads
			.iter()
			.map(String::as_str)
			.find(|u| is_url_in(u, hosts))
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
	pub format_version: u32,
	pub game: String,
	pub version_id: String,
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub summary: Option<String>,
	#[serde(default)]
	pub files: Vec<IndexFile>,
	#[serde(default)]
	pub dependencies: BTreeMap<String, String>,
}

/// Адрес `https://` с хостом из [`ALLOWED_HOSTS`].
pub fn is_allowed_url(url: &str) -> bool {
	is_url_in(url, &ALLOWED_HOSTS)
}

/// Адрес `https://` с хостом из `hosts`.
pub fn is_url_in(url: &str, hosts: &[&str]) -> bool {
	url::Url::parse(url).is_ok_and(|u| {
		u.scheme() == "https"
			&& u.host_str()
				.is_some_and(|h| hosts.iter().any(|a| h.eq_ignore_ascii_case(a)))
	})
}

impl Index {
	/// Проверка после разбора: формат, игра, пути и хеши файлов.
	pub fn validate(&mut self) -> Result<(), AppError> {
		if self.format_version != 1 {
			return Err(invalid(format!("formatVersion {}", self.format_version)));
		}
		if self.game != "minecraft" {
			return Err(invalid(format!("game {}", self.game)));
		}
		if self.files.len() > INDEX_MAX_FILES {
			return Err(invalid(format!(
				"слишком много файлов: {}",
				self.files.len()
			)));
		}
		// Windows не различает регистр: два одинаковых пути писали бы в один файл одновременно.
		let mut seen = std::collections::HashSet::new();
		for file in &mut self.files {
			file.path = check_rel_path(&file.path)
				.ok_or_else(|| invalid(format!("небезопасный путь: {}", file.path)))?;
			if !seen.insert(file.path.to_lowercase()) {
				return Err(invalid(format!("путь повторяется: {}", file.path)));
			}
			let sha1 = file
				.sha1()
				.filter(|h| fs_util::is_sha1_hex(h))
				.ok_or_else(|| invalid(format!("нет sha1: {}", file.path)))?
				.to_ascii_lowercase();
			file.hashes.insert("sha1".into(), sha1);
		}
		Ok(())
	}

	/// Версия игры и загрузчик из `dependencies`. Незнакомые ключи пропускаются.
	pub fn requirements(&self) -> Result<Requirements, AppError> {
		let game_version = self
			.dependencies
			.get("minecraft")
			.filter(|v| fs_util::is_safe_id(v))
			.ok_or_else(|| invalid("нет версии minecraft"))?
			.clone();
		let mut loader = Loader {
			kind: LoaderKind::Vanilla,
			version: None,
		};
		for (key, kind) in [
			("fabric-loader", LoaderKind::Fabric),
			("quilt-loader", LoaderKind::Quilt),
			("forge", LoaderKind::Forge),
			("neoforge", LoaderKind::Neoforge),
			("neo-forge", LoaderKind::Neoforge),
		] {
			if let Some(version) = self.dependencies.get(key) {
				loader = Loader {
					kind,
					version: Some(super::loader_version(&game_version, version)?),
				};
				break;
			}
		}
		for key in self.dependencies.keys() {
			if !matches!(
				key.as_str(),
				"minecraft" | "fabric-loader" | "quilt-loader" | "forge" | "neoforge" | "neo-forge"
			) {
				log::warn!("модпак {}: незнакомая зависимость {key}", self.name);
			}
		}
		Ok(Requirements {
			game_version,
			loader,
		})
	}

	/// Файлы для клиента: нужные всегда и выбранные необязательные (`optional = None` — все).
	/// Возвращает их и пути невыбранных необязательных.
	pub fn select(&self, optional: Option<&[String]>) -> (Vec<&IndexFile>, Vec<String>) {
		let mut chosen = Vec::new();
		let mut excluded = Vec::new();
		for file in &self.files {
			match file.client_side() {
				Side::Unsupported => {}
				Side::Optional if optional.is_some_and(|o| !o.contains(&file.path)) => {
					excluded.push(file.path.clone());
				}
				_ => chosen.push(file),
			}
		}
		(chosen, excluded)
	}
}

/// Читает `modrinth.index.json` из архива. Не zip или нет индекса — `modpack_invalid`,
/// модпак CurseForge — `curseforge_unsupported` (имя модпака в `details`).
pub fn read_index(archive: &Path) -> Result<Index, AppError> {
	let file = std::fs::File::open(archive).map_err(|e| {
		AppError::coded(
			ErrorCode::FileNotFound,
			format!("{}: {e}", archive.display()),
		)
	})?;
	let mut zip = zip::ZipArchive::new(file).map_err(|e| invalid(format!("не zip: {e}")))?;
	if zip.index_for_name(INDEX_NAME).is_none() {
		return Err(match super::curseforge::detect(&mut zip) {
			Some(name) => super::curseforge::unsupported(name),
			None => invalid(format!("нет {INDEX_NAME}")),
		});
	}
	let entry = zip
		.by_name(INDEX_NAME)
		.map_err(|_| invalid(format!("нет {INDEX_NAME}")))?;
	let mut text = Vec::new();
	entry
		.take(INDEX_MAX_BYTES)
		.read_to_end(&mut text)
		.map_err(|e| invalid(e.to_string()))?;
	let mut index: Index =
		serde_json::from_slice(&text).map_err(|e| invalid(format!("{INDEX_NAME}: {e}")))?;
	index.validate()?;
	Ok(index)
}

/// Что делать с файлами `overrides`, которые уже есть в папке игры.
#[derive(Debug, Clone, Default)]
pub struct OverridePolicy {
	/// Обновление: что модпак записал в прошлый раз (путь → sha1). `None` — новая установка,
	/// всё пишется поверх.
	pub previous: Option<BTreeMap<String, String>>,
	/// Обновление: изменённые пользователем файлы копируются сюда перед перезаписью.
	pub backup_dir: Option<PathBuf>,
}

/// Недописанный файл `overrides` рядом с целевым (старые дочищает [`super::cleanup`]).
pub const PACK_TMP_SUFFIX: &str = ".pack-tmp";

/// Файлы игрока, которые обновление модпака не перезаписывает, если они уже есть.
/// Без учёта регистра: на Windows `Options.txt` и `Saves/…` — те же файлы.
pub fn is_user_data(rel: &str) -> bool {
	let rel = rel.to_ascii_lowercase();
	matches!(
		rel.as_str(),
		"options.txt" | "optionsof.txt" | "optionsshaders.txt" | "servers.dat"
	) || rel.starts_with("saves/")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverridesResult {
	/// Что записано или уже совпадало: путь → sha1.
	pub written: BTreeMap<String, String>,
	/// Скопированы в `backup_dir` перед перезаписью.
	pub backed_up: Vec<String>,
	/// Не тронуты: файлы игрока (см. [`is_user_data`]).
	pub kept: Vec<String>,
}

/// Распаковывает папки `prefixes` по порядку (следующая поверх). Синхронно.
pub fn extract_overrides(
	archive: &Path,
	prefixes: &[&str],
	game_dir: &Path,
	policy: &OverridePolicy,
	limits: Limits,
	cancel: &CancellationToken,
) -> Result<OverridesResult, AppError> {
	let file = std::fs::File::open(archive)
		.map_err(|e| AppError::storage(format!("{}: {e}", archive.display())))?;
	let mut zip = zip::ZipArchive::new(file).map_err(|e| invalid(format!("не zip: {e}")))?;
	// Путь → (слой, номер записи): более поздний слой побеждает.
	let mut chosen: BTreeMap<String, (usize, usize)> = BTreeMap::new();
	for i in 0..zip.len() {
		let entry = zip.by_index_raw(i).map_err(|e| invalid(e.to_string()))?;
		if entry.is_dir() {
			continue;
		}
		let name = entry
			.name()
			.map_err(|e| invalid(e.to_string()))?
			.replace('\\', "/");
		for (layer, prefix) in prefixes.iter().enumerate() {
			let Some(rel) = name.strip_prefix(prefix) else {
				continue;
			};
			let rel = check_rel_path(rel)
				.ok_or_else(|| invalid(format!("небезопасный путь в архиве: {name}")))?;
			if chosen.get(&rel).is_none_or(|(l, _)| *l <= layer) {
				chosen.insert(rel, (layer, i));
			}
		}
	}

	if chosen.len() > limits.files {
		return Err(invalid(format!("слишком много файлов: {}", chosen.len())));
	}
	let mut total = 0u64;
	for (_, i) in chosen.values() {
		let entry = zip.by_index_raw(*i).map_err(|e| invalid(e.to_string()))?;
		total = total.saturating_add(entry.size());
	}
	if total > limits.bytes {
		return Err(invalid(format!(
			"распакованный размер {total} байт больше допустимого"
		)));
	}
	// Остаток на всё, что ещё распакуется (по настоящим байтам).
	let mut budget = limits.bytes;

	let mut result = OverridesResult::default();
	for (rel, (_, i)) in chosen {
		if cancel.is_cancelled() {
			return Err(AppError::new(ErrorCode::Cancelled));
		}
		let target = fs_util::safe_join(game_dir, &rel)
			.ok_or_else(|| invalid(format!("небезопасный путь: {rel}")))?;
		let existing = if target.is_file() {
			Some(
				crate::content::installed::sha1_file(&target)
					.map_err(|e| AppError::storage(format!("{}: {e}", target.display())))?,
			)
		} else {
			None
		};
		if let (Some(previous), Some(existing)) = (&policy.previous, &existing) {
			if is_user_data(&rel) {
				result.kept.push(rel);
				continue;
			}
			// Изменён пользователем (или создан им) — копия перед перезаписью.
			if previous.get(&rel) != Some(existing)
				&& let Some(backup_dir) = &policy.backup_dir
			{
				let entry_sha1 = sha1_of_entry(&mut zip, i, &mut budget)?;
				if &entry_sha1 == existing {
					result.written.insert(rel, entry_sha1);
					continue;
				}
				let copy = fs_util::safe_join(backup_dir, &rel)
					.ok_or_else(|| invalid(format!("небезопасный путь: {rel}")))?;
				if let Some(dir) = copy.parent() {
					std::fs::create_dir_all(dir)
						.map_err(|e| AppError::storage(format!("{}: {e}", dir.display())))?;
				}
				std::fs::copy(&target, &copy)
					.map_err(|e| AppError::storage(format!("{}: {e}", copy.display())))?;
				result.backed_up.push(rel.clone());
			}
		}
		let mut entry = zip.by_index(i).map_err(|e| invalid(e.to_string()))?;
		let declared = entry.size();
		let sha1 = write_entry(&mut entry, declared, &mut budget, &target)?;
		result.written.insert(rel, sha1);
	}
	Ok(result)
}

/// Читает запись кусками, не больше заявленного размера и остатка `budget`.
fn read_limited(
	entry: &mut impl Read,
	declared: u64,
	budget: &mut u64,
	name: &str,
	mut sink: impl FnMut(&[u8]) -> Result<(), AppError>,
) -> Result<(), AppError> {
	let mut buf = vec![0u8; 256 * 1024];
	let mut read = 0u64;
	loop {
		let n = entry
			.read(&mut buf)
			.map_err(|e| invalid(format!("{name}: {e}")))?;
		if n == 0 {
			return Ok(());
		}
		read += n as u64;
		if read > declared || n as u64 > *budget {
			return Err(invalid(format!("{name}: распаковано больше, чем заявлено")));
		}
		*budget -= n as u64;
		sink(&buf[..n])?;
	}
}

fn sha1_of_entry<R: std::io::Read + std::io::Seek>(
	zip: &mut zip::ZipArchive<R>,
	i: usize,
	budget: &mut u64,
) -> Result<String, AppError> {
	let mut entry = zip.by_index(i).map_err(|e| invalid(e.to_string()))?;
	let declared = entry.size();
	let name: String = entry.name().map(|n| n.to_string()).unwrap_or_default();
	let mut hasher = <sha1::Sha1 as sha1::Digest>::new();
	read_limited(&mut entry, declared, budget, &name, |chunk| {
		sha1::Digest::update(&mut hasher, chunk);
		Ok(())
	})?;
	Ok(crate::download::to_hex(&sha1::Digest::finalize(hasher)))
}

/// Пишет запись архива во временный файл рядом и переименовывает поверх; возвращает sha1.
fn write_entry(
	entry: &mut impl Read,
	declared: u64,
	budget: &mut u64,
	target: &Path,
) -> Result<String, AppError> {
	let storage = |p: &Path, e: std::io::Error| crate::content::installed::io_error(p, &e);
	if let Some(dir) = target.parent() {
		std::fs::create_dir_all(dir).map_err(|e| storage(dir, e))?;
	}
	let tmp = PathBuf::from(format!("{}{PACK_TMP_SUFFIX}", target.display()));
	let written = (|| -> Result<String, AppError> {
		let mut out = std::fs::File::create(&tmp).map_err(|e| storage(&tmp, e))?;
		let mut hasher = <sha1::Sha1 as sha1::Digest>::new();
		let name = target.display().to_string();
		read_limited(entry, declared, budget, &name, |chunk| {
			sha1::Digest::update(&mut hasher, chunk);
			std::io::Write::write_all(&mut out, chunk).map_err(|e| storage(&tmp, e))
		})?;
		out.sync_all().map_err(|e| storage(&tmp, e))?;
		Ok(crate::download::to_hex(&sha1::Digest::finalize(hasher)))
	})();
	let sha1 = match written {
		Ok(sha1) => sha1,
		Err(err) => {
			let _ = std::fs::remove_file(&tmp);
			return Err(err);
		}
	};
	std::fs::rename(&tmp, target).map_err(|e| {
		let _ = std::fs::remove_file(&tmp);
		storage(target, e)
	})?;
	Ok(sha1)
}

/// Пути файлов `overrides`/`client-overrides` архива (безопасные, без повторов).
pub fn override_paths(archive: &Path, prefixes: &[&str]) -> Vec<String> {
	let Ok(file) = std::fs::File::open(archive) else {
		return Vec::new();
	};
	let Ok(mut zip) = zip::ZipArchive::new(file) else {
		return Vec::new();
	};
	let mut seen = std::collections::BTreeSet::new();
	for i in 0..zip.len() {
		if let Ok(entry) = zip.by_index_raw(i)
			&& !entry.is_dir()
			&& let Ok(name) = entry.name()
		{
			for prefix in prefixes {
				if let Some(rel) = name.strip_prefix(prefix).and_then(check_rel_path) {
					seen.insert(rel);
				}
			}
		}
	}
	seen.into_iter().collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;

	pub fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
		let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
		for (name, data) in entries {
			zip.start_file(*name, zip::write::SimpleFileOptions::default())
				.unwrap();
			zip.write_all(data).unwrap();
		}
		zip.finish().unwrap();
	}

	const SHA1_A: &str = "a9993e364706816aba3e25717850c26c9cd0d89d";

	fn index_json(files: serde_json::Value, deps: serde_json::Value) -> String {
		serde_json::json!({
			"formatVersion": 1, "game": "minecraft", "versionId": "1.2.0", "name": "Pack",
			"files": files, "dependencies": deps
		})
		.to_string()
	}

	#[test]
	fn reads_and_validates_index() {
		let dir = tempfile::tempdir().unwrap();
		let pack = dir.path().join("p.mrpack");
		let json = index_json(
			serde_json::json!([
				{ "path": "mods/a.jar", "hashes": { "sha1": SHA1_A.to_uppercase(), "sha512": "x" },
				  "downloads": ["https://cdn.modrinth.com/data/a/versions/b/a.jar"], "fileSize": 3 },
				{ "path": "mods/opt.jar", "hashes": { "sha1": SHA1_A }, "env": { "client": "optional", "server": "required" },
				  "downloads": ["https://evil.example.com/opt.jar"], "fileSize": 5 },
				{ "path": "mods/server.jar", "hashes": { "sha1": SHA1_A }, "env": { "client": "unsupported", "server": "required" },
				  "downloads": ["https://cdn.modrinth.com/x"], "fileSize": 1 }
			]),
			serde_json::json!({ "minecraft": "1.20.1", "fabric-loader": "0.15.11", "something-new": "1" }),
		);
		write_zip(&pack, &[(INDEX_NAME, json.as_bytes())]);
		let index = read_index(&pack).unwrap();
		assert_eq!(index.files[0].sha1(), Some(SHA1_A));
		let req = index.requirements().unwrap();
		assert_eq!(req.game_version, "1.20.1");
		assert_eq!(req.loader.kind, LoaderKind::Fabric);
		assert_eq!(req.loader.version.as_deref(), Some("0.15.11"));

		let (all, excluded) = index.select(None);
		assert_eq!(all.len(), 2);
		assert!(excluded.is_empty());
		let (chosen, excluded) = index.select(Some(&[]));
		assert_eq!(chosen.len(), 1);
		assert_eq!(excluded, vec!["mods/opt.jar"]);
		assert!(index.files[0].allowed_url().is_some());
		assert!(index.files[1].allowed_url().is_none());

		for bad in [
			r#"{"path":"../evil.jar"}"#,
			r#"{"path":"mods/../../evil.jar"}"#,
			r#"{"path":"C:/Windows/evil.jar"}"#,
			r#"{"path":"mods\\..\\evil.jar"}"#,
			r#"{"path":"mods/CON.jar"}"#,
		] {
			let mut file: serde_json::Value = serde_json::from_str(bad).unwrap();
			file["hashes"] = serde_json::json!({ "sha1": SHA1_A });
			file["downloads"] = serde_json::json!([]);
			let json = index_json(
				serde_json::json!([file]),
				serde_json::json!({ "minecraft": "1.20.1" }),
			);
			write_zip(&pack, &[(INDEX_NAME, json.as_bytes())]);
			assert_eq!(
				read_index(&pack).unwrap_err().code,
				ErrorCode::ModpackInvalid,
				"{bad}"
			);
		}
		write_zip(&pack, &[("other.json", b"{}")]);
		assert_eq!(
			read_index(&pack).unwrap_err().code,
			ErrorCode::ModpackInvalid
		);
		std::fs::write(&pack, b"not a zip").unwrap();
		assert_eq!(
			read_index(&pack).unwrap_err().code,
			ErrorCode::ModpackInvalid
		);
	}

	#[test]
	fn allowed_hosts() {
		for good in [
			"https://cdn.modrinth.com/data/x/y.jar",
			"https://github.com/a/b/releases/download/1/x.jar",
			"https://raw.githubusercontent.com/a/b/main/x.jar",
			"https://gitlab.com/a/b/-/x.jar",
		] {
			assert!(is_allowed_url(good), "{good}");
		}
		for bad in [
			"http://cdn.modrinth.com/x.jar",
			"https://cdn.modrinth.com.evil.com/x.jar",
			"https://evil.com/cdn.modrinth.com/x.jar",
			"https://user@evil.com/x",
			"file:///C:/x.jar",
			"not a url",
		] {
			assert!(!is_allowed_url(bad), "{bad}");
		}
	}

	#[test]
	fn requirements_for_loaders() {
		let req = |deps: serde_json::Value| -> Result<Requirements, AppError> {
			let index: Index =
				serde_json::from_str(&index_json(serde_json::json!([]), deps)).unwrap();
			index.requirements()
		};
		let forge =
			req(serde_json::json!({ "minecraft": "1.20.1", "forge": "1.20.1-47.2.0" })).unwrap();
		assert_eq!(forge.loader.kind, LoaderKind::Forge);
		assert_eq!(forge.loader.version.as_deref(), Some("47.2.0"));
		let neo = req(serde_json::json!({ "minecraft": "1.21.1", "neoforge": "21.1.77" })).unwrap();
		assert_eq!(neo.loader.kind, LoaderKind::Neoforge);
		let vanilla = req(serde_json::json!({ "minecraft": "1.21.1" })).unwrap();
		assert_eq!(vanilla.loader.kind, LoaderKind::Vanilla);
		assert!(req(serde_json::json!({ "forge": "47.2.0" })).is_err());
		assert!(req(serde_json::json!({ "minecraft": "../1" })).is_err());
		assert!(req(serde_json::json!({ "minecraft": "1.20.1", "forge": "a/../b" })).is_err());
	}

	#[test]
	fn extracts_overrides_safely() {
		let dir = tempfile::tempdir().unwrap();
		let pack = dir.path().join("p.mrpack");
		let game = dir.path().join("game");
		write_zip(
			&pack,
			&[
				(INDEX_NAME, b"{}"),
				("overrides/config/a.toml", b"base"),
				("overrides/config/b.toml", b"b"),
				("client-overrides/config/a.toml", b"client"),
				("server-overrides/config/s.toml", b"server"),
				("overrides/options.txt", b"pack options"),
			],
		);
		let cancel = CancellationToken::new();
		let prefixes = ["overrides/", "client-overrides/"];
		let result = extract_overrides(
			&pack,
			&prefixes,
			&game,
			&OverridePolicy::default(),
			Limits::DEFAULT,
			&cancel,
		)
		.unwrap();
		assert_eq!(
			std::fs::read(game.join("config/a.toml")).unwrap(),
			b"client"
		);
		assert_eq!(std::fs::read(game.join("config/b.toml")).unwrap(), b"b");
		assert!(!game.join("config/s.toml").exists());
		assert_eq!(result.written.len(), 3);
		assert_eq!(override_paths(&pack, &prefixes).len(), 3);
		assert!(!game.join("config/a.toml.pack-tmp").exists());

		// Обновление: файл игрока не трогаем, изменённый пользователем — копия и перезапись.
		std::fs::write(game.join("options.txt"), b"my options").unwrap();
		std::fs::write(game.join("config/b.toml"), b"edited").unwrap();
		let backup = dir.path().join("backup");
		let policy = OverridePolicy {
			previous: Some(result.written.clone()),
			backup_dir: Some(backup.clone()),
		};
		let again =
			extract_overrides(&pack, &prefixes, &game, &policy, Limits::DEFAULT, &cancel).unwrap();
		assert_eq!(
			std::fs::read(game.join("options.txt")).unwrap(),
			b"my options"
		);
		assert_eq!(again.kept, vec!["options.txt"]);
		assert_eq!(std::fs::read(game.join("config/b.toml")).unwrap(), b"b");
		assert_eq!(
			std::fs::read(backup.join("config/b.toml")).unwrap(),
			b"edited"
		);
		assert_eq!(again.backed_up, vec!["config/b.toml"]);

		// zip-slip.
		for evil in [
			"overrides/../evil.txt",
			"overrides/../../evil.txt",
			"overrides/C:/evil.txt",
			"overrides/a/../../evil.txt",
			"overrides\\..\\evil.txt",
		] {
			write_zip(&pack, &[(evil, b"x")]);
			let err = extract_overrides(
				&pack,
				&prefixes,
				&game,
				&OverridePolicy::default(),
				Limits::DEFAULT,
				&cancel,
			)
			.unwrap_err();
			assert_eq!(err.code, ErrorCode::ModpackInvalid, "{evil}");
		}
		assert!(!dir.path().join("evil.txt").exists());
	}

	#[test]
	fn extraction_limits() {
		let dir = tempfile::tempdir().unwrap();
		let pack = dir.path().join("p.mrpack");
		let game = dir.path().join("game");
		let big = vec![0u8; 64 * 1024];
		write_zip(
			&pack,
			&[
				("overrides/a.bin", &big),
				("overrides/b.bin", &big),
				("overrides/c.txt", b"c"),
			],
		);
		let cancel = CancellationToken::new();
		let policy = OverridePolicy::default();
		let run = |limits: Limits| {
			extract_overrides(&pack, &["overrides/"], &game, &policy, limits, &cancel)
		};
		let files = run(Limits {
			files: 2,
			bytes: u64::MAX,
		})
		.unwrap_err();
		assert_eq!(files.code, ErrorCode::ModpackInvalid);
		let bytes = run(Limits {
			files: 10,
			bytes: 100 * 1024,
		})
		.unwrap_err();
		assert_eq!(bytes.code, ErrorCode::ModpackInvalid);
		assert!(!game.exists(), "до проверки лимитов ничего не пишется");
		assert_eq!(run(Limits::DEFAULT).unwrap().written.len(), 3);

		// Распаковано больше заявленного (заголовок врёт) — отказ.
		let mut budget = u64::MAX;
		let mut reader: &[u8] = &big;
		let err = read_limited(&mut reader, 10, &mut budget, "x", |_| Ok(())).unwrap_err();
		assert_eq!(err.code, ErrorCode::ModpackInvalid);
		let mut budget = 10;
		let mut reader: &[u8] = &big;
		assert!(read_limited(&mut reader, u64::MAX, &mut budget, "x", |_| Ok(())).is_err());
	}

	#[test]
	fn rejects_duplicate_paths_and_detects_curseforge() {
		let dir = tempfile::tempdir().unwrap();
		let pack = dir.path().join("p.mrpack");
		let file = |path: &str| {
			serde_json::json!({ "path": path, "hashes": { "sha1": SHA1_A },
				"downloads": ["https://cdn.modrinth.com/x"], "fileSize": 1 })
		};
		let json = index_json(
			serde_json::json!([file("mods/a.jar"), file("mods/A.jar")]),
			serde_json::json!({ "minecraft": "1.20.1" }),
		);
		write_zip(&pack, &[(INDEX_NAME, json.as_bytes())]);
		assert_eq!(
			read_index(&pack).unwrap_err().code,
			ErrorCode::ModpackInvalid
		);

		let bad_version = serde_json::json!({
			"formatVersion": 2, "game": "minecraft", "versionId": "1", "name": "P",
			"files": [], "dependencies": { "minecraft": "1.20.1" }
		});
		write_zip(&pack, &[(INDEX_NAME, bad_version.to_string().as_bytes())]);
		assert_eq!(
			read_index(&pack).unwrap_err().code,
			ErrorCode::ModpackInvalid
		);

		let manifest = serde_json::json!({
			"manifestType": "minecraftModpack", "manifestVersion": 1, "name": "All the Mods 9",
			"files": [], "overrides": "overrides"
		});
		write_zip(&pack, &[("manifest.json", manifest.to_string().as_bytes())]);
		let err = read_index(&pack).unwrap_err();
		assert_eq!(err.code, ErrorCode::CurseforgeUnsupported);
		assert_eq!(err.details.as_deref(), Some("All the Mods 9"));
	}

	#[test]
	fn user_data_ignores_case() {
		for rel in [
			"options.txt",
			"Options.txt",
			"SERVERS.DAT",
			"Saves/w/level.dat",
			"saves/x",
		] {
			assert!(is_user_data(rel), "{rel}");
		}
		for rel in ["config/options.txt", "mods/saves.jar", "savesx/a"] {
			assert!(!is_user_data(rel), "{rel}");
		}
	}
}
