//! Профиль аккаунта: разбор ответа Mojang и сохранённая копия `profiles/<accountId>.json` с кэшем
//! текстур `textures/<ключ>.png`. Токенов и адресов в файлах нет.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::SkinModel;
use super::textures::{Download, TextureKind, key_from_url, safe_key};
use crate::error::AppError;
use crate::fs_util;

/// Ответ Mojang, в котором уже проверены ключи и адреса.
#[derive(Debug, Clone)]
pub struct ParsedProfile {
	skin: Option<StoredSkin>,
	capes: Vec<StoredCape>,
	active_cape_id: Option<String>,
	downloads: Vec<Download>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSkin {
	pub key: String,
	pub model: SkinModel,
	pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCape {
	pub id: String,
	pub name: String,
	pub key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredProfile {
	pub skin: Option<StoredSkin>,
	pub capes: Vec<StoredCape>,
	pub active_cape_id: Option<String>,
	pub fetched_at: String,
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
	value.get(key).and_then(Value::as_str)
}

fn is_active(value: &Value) -> bool {
	text(value, "state").is_some_and(|state| state.eq_ignore_ascii_case("active"))
}

/// Ключ — `textureKey`, иначе последний сегмент адреса. Качать можно, только если адрес проверен
/// и указывает на этот же ключ.
fn texture_of(value: &Value) -> (Option<String>, bool) {
	let url = text(value, "url").unwrap_or_default();
	let from_url = key_from_url(url);
	let key = text(value, "textureKey")
		.filter(|key| safe_key(key))
		.map(str::to_owned)
		.or_else(|| {
			from_url.clone().or_else(|| {
				url.rsplit('/')
					.next()
					.filter(|key| safe_key(key))
					.map(str::to_owned)
			})
		});
	let can_download = from_url.is_some() && from_url == key;
	if !can_download {
		log::warn!("адрес текстуры в профиле не прошёл проверку");
	}
	(key, can_download)
}

fn parse_model(value: &Value) -> SkinModel {
	match text(value, "variant") {
		Some(variant) if variant.eq_ignore_ascii_case("slim") => SkinModel::Slim,
		Some(variant) if variant.eq_ignore_ascii_case("classic") => SkinModel::Classic,
		_ => {
			log::warn!("неизвестные руки в профиле, считаем широкими");
			SkinModel::Classic
		}
	}
}

/// `None` — это не профиль. Лишние поля игнорируются, плохая запись плаща отбрасывается,
/// активного скина нет — `skin: None`.
pub fn parse_profile(bytes: &[u8]) -> Option<ParsedProfile> {
	let root: Value = serde_json::from_slice(bytes).ok()?;
	let skins = root.get("skins")?.as_array()?;
	let capes = root.get("capes")?.as_array()?;
	let mut downloads = Vec::new();

	let mut skin = None;
	if let Some(active) = skins.iter().find(|item| is_active(item)) {
		let (key, can_download) = texture_of(active);
		if let Some(key) = key {
			if can_download {
				downloads.push(Download {
					key: key.clone(),
					kind: TextureKind::Skin,
				});
			}
			skin = Some(StoredSkin {
				key,
				model: parse_model(active),
				default: active.get("alias").is_some_and(|alias| !alias.is_null()),
			});
		}
	}

	let mut stored_capes = Vec::new();
	let mut active_cape_id = None;
	let mut seen = HashSet::new();
	for item in capes {
		let id = text(item, "id").and_then(|id| Uuid::parse_str(id).ok());
		let (Some(id), Some(name)) = (id, text(item, "alias")) else {
			log::warn!("плохая запись плаща в профиле отброшена");
			continue;
		};
		let id = id.hyphenated().to_string();
		if !seen.insert(id.clone()) {
			continue;
		}
		let (key, can_download) = texture_of(item);
		if let (Some(key), true) = (&key, can_download) {
			downloads.push(Download {
				key: key.clone(),
				kind: TextureKind::Cape,
			});
		}
		if is_active(item) && active_cape_id.is_none() {
			active_cape_id = Some(id.clone());
		}
		stored_capes.push(StoredCape {
			id,
			name: name.to_owned(),
			key,
		});
	}
	Some(ParsedProfile {
		skin,
		capes: stored_capes,
		active_cape_id,
		downloads,
	})
}

impl ParsedProfile {
	pub fn into_stored(self, fetched_at: String) -> (StoredProfile, Vec<Download>) {
		(
			StoredProfile {
				skin: self.skin,
				capes: self.capes,
				active_cape_id: self.active_cape_id,
				fetched_at,
			},
			self.downloads,
		)
	}
}

pub struct ProfileStore {
	profiles: PathBuf,
	textures: PathBuf,
	/// Под одним замком: иначе уборка кэша удалит свежую текстуру, на которую ещё нет ссылки.
	pub lock: tokio::sync::Mutex<()>,
}

impl ProfileStore {
	pub fn new(dir: &std::path::Path) -> Self {
		Self {
			profiles: dir.join("profiles"),
			textures: dir.join("textures"),
			lock: tokio::sync::Mutex::new(()),
		}
	}

	fn profile_path(&self, account_id: &str) -> Option<PathBuf> {
		if !fs_util::is_safe_id(account_id) {
			return None;
		}
		fs_util::safe_join(&self.profiles, &format!("{account_id}.json"))
	}

	fn texture_path(&self, key: &str) -> Option<PathBuf> {
		if !safe_key(key) {
			return None;
		}
		fs_util::safe_join(&self.textures, &format!("{key}.png"))
	}

	/// Повреждённый профиль — как будто его нет. Копия остаётся рядом, сам файл убирается,
	/// чтобы не плодить копии при каждом чтении.
	pub fn read(&self, account_id: &str) -> Option<StoredProfile> {
		let path = self.profile_path(account_id)?;
		match fs_util::read_json_or_backup::<StoredProfile>(&path) {
			Ok(Some(profile)) => {
				let keys_ok = profile
					.skin
					.iter()
					.map(|skin| &skin.key)
					.chain(profile.capes.iter().filter_map(|cape| cape.key.as_ref()))
					.all(|key| safe_key(key));
				if keys_ok {
					Some(profile)
				} else {
					log::warn!(
						"{}: чужой ключ текстуры, файл не используется",
						path.display()
					);
					None
				}
			}
			Ok(None) => {
				if path.exists() {
					let _ = std::fs::remove_file(&path);
				}
				None
			}
			Err(err) => {
				log::warn!("не удалось прочитать {}: {err}", path.display());
				None
			}
		}
	}

	pub fn write(&self, account_id: &str, profile: &StoredProfile) -> Result<(), AppError> {
		let path = self
			.profile_path(account_id)
			.ok_or_else(|| AppError::storage("небезопасный id аккаунта"))?;
		let bytes =
			serde_json::to_vec_pretty(profile).map_err(|err| AppError::storage(err.to_string()))?;
		fs_util::write_atomic(&path, &bytes)
			.map_err(|err| AppError::storage(format!("{}: {err}", path.display())))
	}

	pub fn has_texture(&self, key: &str) -> bool {
		self.texture_path(key).is_some_and(|path| path.is_file())
	}

	pub fn texture(&self, key: &str) -> Option<Vec<u8>> {
		std::fs::read(self.texture_path(key)?).ok()
	}

	pub fn write_texture(&self, key: &str, png: &[u8]) -> Result<(), AppError> {
		let path = self
			.texture_path(key)
			.ok_or_else(|| AppError::storage("небезопасный ключ текстуры"))?;
		fs_util::write_atomic(&path, png)
			.map_err(|err| AppError::storage(format!("{}: {err}", path.display())))
	}

	pub fn accounts(&self) -> Vec<String> {
		let Ok(entries) = std::fs::read_dir(&self.profiles) else {
			return Vec::new();
		};
		entries
			.flatten()
			.filter_map(|entry| {
				let name = entry.file_name();
				let id = name.to_str()?.strip_suffix(".json")?.to_owned();
				fs_util::is_safe_id(&id).then_some(id)
			})
			.collect()
	}

	/// Удаляет текстуры, на которые не ссылается ни один профиль. В корзину не кладёт: это кэш.
	pub fn collect_garbage(&self) {
		let mut used = HashSet::new();
		for account in self.accounts() {
			let Some(path) = self.profile_path(&account) else {
				continue;
			};
			// Файл есть, но не читается: неизвестно, на что он ссылается, поэтому не убираем ничего.
			let Ok(read) = fs_util::read_json_or_backup::<StoredProfile>(&path) else {
				return;
			};
			let Some(profile) = read else { continue };
			used.extend(profile.skin.map(|skin| skin.key));
			used.extend(profile.capes.into_iter().filter_map(|cape| cape.key));
		}
		let Ok(entries) = std::fs::read_dir(&self.textures) else {
			return;
		};
		for entry in entries.flatten() {
			let name = entry.file_name();
			let Some(key) = name.to_str().and_then(|name| name.strip_suffix(".png")) else {
				continue;
			};
			if !used.contains(key)
				&& let Err(err) = std::fs::remove_file(entry.path())
			{
				log::warn!("не удалось убрать текстуру из кэша: {err}");
			}
		}
	}

	pub fn forget(&self, account_id: &str) {
		let Some(path) = self.profile_path(account_id) else {
			return;
		};
		match std::fs::remove_file(&path) {
			Ok(()) => self.collect_garbage(),
			Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
			Err(err) => log::warn!("не удалось удалить {}: {err}", path.display()),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const CAPE_ID: &str = "11111111-2222-3333-4444-555555555555";

	fn stored(bytes: &str) -> (StoredProfile, Vec<Download>) {
		parse_profile(bytes.as_bytes())
			.unwrap()
			.into_stored("t".into())
	}

	/// Образец — Minecraft Wiki и `MinecraftProfile` в `D:\www\Chunk`.
	#[test]
	fn parses_wiki_sample() {
		let body = format!(
			r#"{{
				"id": "069a79f444e94726a5befca90e38aaf5", "name": "Notch", "extra": 1,
				"skins": [{{
					"id": "6a6e65e5-76dd-4c3c-a625-162924514568", "state": "ACTIVE",
					"url": "http://textures.minecraft.net/texture/1a4af718455d",
					"textureKey": "1a4af718455d", "variant": "CLASSIC"
				}}],
				"capes": [{{
					"id": "{CAPE_ID}", "state": "ACTIVE",
					"url": "https://textures.minecraft.net/texture/2340c0e0",
					"alias": "Migrator"
				}}]
			}}"#
		);
		let (profile, downloads) = stored(&body);
		let skin = profile.skin.unwrap();
		assert_eq!(skin.key, "1a4af718455d");
		assert_eq!(skin.model, SkinModel::Classic);
		assert!(!skin.default);
		assert_eq!(profile.active_cape_id.as_deref(), Some(CAPE_ID));
		assert_eq!(profile.capes[0].name, "Migrator");
		assert_eq!(profile.capes[0].key.as_deref(), Some("2340c0e0"));
		assert_eq!(
			downloads,
			[
				Download {
					key: "1a4af718455d".into(),
					kind: TextureKind::Skin
				},
				Download {
					key: "2340c0e0".into(),
					kind: TextureKind::Cape
				},
			]
		);
	}

	#[test]
	fn tolerant_details() {
		// Нет `textureKey`, регистр `state`, неизвестные руки, `alias` у обычного скина.
		let (profile, _) = stored(
			r#"{"skins":[
				{"state":"inactive","url":"https://textures.minecraft.net/texture/old","variant":"SLIM"},
				{"state":"active","url":"https://textures.minecraft.net/texture/new","variant":"wide","alias":"STEVE"}
			],"capes":[]}"#,
		);
		let skin = profile.skin.unwrap();
		assert_eq!(skin.key, "new");
		assert_eq!(skin.model, SkinModel::Classic);
		assert!(skin.default);

		let (profile, _) = stored(
			r#"{"skins":[{"state":"ACTIVE","url":"https://textures.minecraft.net/texture/x","variant":"slim"}],"capes":[]}"#,
		);
		assert_eq!(profile.skin.unwrap().model, SkinModel::Slim);
		// Активного нет.
		let (profile, downloads) = stored(r#"{"skins":[],"capes":[]}"#);
		assert_eq!(profile.skin, None);
		assert!(downloads.is_empty());
	}

	#[test]
	fn bad_capes_are_dropped() {
		let body = format!(
			r#"{{"skins":[],"capes":[
				{{"id":"не uuid","state":"ACTIVE","url":"https://textures.minecraft.net/texture/a","alias":"A"}},
				{{"id":"{CAPE_ID}","state":"INACTIVE","url":"https://textures.minecraft.net/texture/b"}},
				5,
				{{"id":"{CAPE_ID}","state":"INACTIVE","url":"https://evil.com/texture/c","alias":"C"}}
			]}}"#
		);
		let (profile, downloads) = stored(&body);
		assert_eq!(profile.capes.len(), 1);
		assert_eq!(profile.capes[0].name, "C");
		// Адрес чужой: плащ виден, но не качается.
		assert!(downloads.is_empty());
		assert_eq!(profile.active_cape_id, None);
	}

	#[test]
	fn foreign_texture_is_not_downloaded() {
		for url in [
			"https://textures.minecraft.net.evil.com/texture/abc",
			"https://user@textures.minecraft.net/texture/abc",
			"https://textures.minecraft.net:8443/texture/abc",
		] {
			let body = format!(
				r#"{{"skins":[{{"state":"ACTIVE","url":"{url}","variant":"CLASSIC"}}],"capes":[]}}"#
			);
			let (profile, downloads) = stored(&body);
			assert!(downloads.is_empty(), "{url}");
			assert_eq!(profile.skin.unwrap().key, "abc");
		}
		// Ключ из `textureKey` не совпал с адресом.
		let (_, downloads) = stored(
			r#"{"skins":[{"state":"ACTIVE","url":"https://textures.minecraft.net/texture/abc","textureKey":"other","variant":"CLASSIC"}],"capes":[]}"#,
		);
		assert!(downloads.is_empty());
		// Небезопасный ключ: скина нет.
		let (profile, _) = stored(
			r#"{"skins":[{"state":"ACTIVE","url":"https://evil.com/..","textureKey":"../x","variant":"CLASSIC"}],"capes":[]}"#,
		);
		assert_eq!(profile.skin, None);
	}

	#[test]
	fn not_a_profile() {
		for body in [
			"",
			"{}",
			"[]",
			"текст",
			r#"{"skins":[]}"#,
			r#"{"skins":5,"capes":[]}"#,
		] {
			assert!(parse_profile(body.as_bytes()).is_none(), "{body}");
		}
	}

	fn profile_with(key: &str) -> StoredProfile {
		StoredProfile {
			skin: Some(StoredSkin {
				key: key.into(),
				model: SkinModel::Slim,
				default: false,
			}),
			capes: Vec::new(),
			active_cape_id: None,
			fetched_at: "t".into(),
		}
	}

	#[test]
	fn store_read_write_and_garbage() {
		let dir = tempfile::tempdir().unwrap();
		let store = ProfileStore::new(dir.path());
		assert_eq!(store.read("acc"), None);
		assert!(store.accounts().is_empty());

		store.write_texture("one", b"1").unwrap();
		store.write_texture("two", b"2").unwrap();
		store.write_texture("stale", b"3").unwrap();
		store.write("acc", &profile_with("one")).unwrap();
		store.write("other", &profile_with("two")).unwrap();
		assert_eq!(store.read("acc"), Some(profile_with("one")));

		store.collect_garbage();
		assert!(store.has_texture("one") && store.has_texture("two"));
		assert!(!store.has_texture("stale"));

		store.forget("other");
		assert_eq!(store.read("other"), None);
		assert!(store.has_texture("one"));
		assert!(!store.has_texture("two"));

		// В файле нет адресов и токенов.
		let text = std::fs::read_to_string(dir.path().join("profiles/acc.json")).unwrap();
		assert!(!text.contains("http"));
		assert!(text.contains("fetchedAt") && text.contains("activeCapeId"));
	}

	#[test]
	fn broken_profile_is_as_if_absent() {
		let dir = tempfile::tempdir().unwrap();
		let store = ProfileStore::new(dir.path());
		std::fs::create_dir_all(dir.path().join("profiles")).unwrap();
		let path = dir.path().join("profiles/acc.json");
		std::fs::write(&path, b"{ broken").unwrap();
		assert_eq!(store.read("acc"), None);
		// Следующее чтение новой копии не делает.
		assert!(!path.exists());
		assert_eq!(store.read("acc"), None);

		// Чужой ключ в файле не становится путём.
		std::fs::write(
			&path,
			serde_json::to_vec(&profile_with("../../evil")).unwrap(),
		)
		.unwrap();
		assert_eq!(store.read("acc"), None);
		assert_eq!(store.texture("../../evil"), None);
		assert_eq!(store.read("../evil"), None);
		assert!(store.write("../evil", &profile_with("a")).is_err());
	}
}
