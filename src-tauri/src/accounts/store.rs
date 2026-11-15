use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Account;
use crate::fs_util;

pub const FILE_NAME: &str = "accounts.json";
const CURRENT_VERSION: u32 = 1;

/// `accounts.json`. Секреты — строки из [`crate::secure::protect`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsFile {
	#[serde(default)]
	pub version: u32,
	#[serde(default)]
	pub active_id: Option<String>,
	#[serde(default)]
	pub accounts: Vec<StoredAccount>,
	/// Ключ устройства Xbox и его токен (общие для всех аккаунтов Microsoft).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device: Option<String>,
}

impl Default for AccountsFile {
	fn default() -> Self {
		Self {
			version: CURRENT_VERSION,
			active_id: None,
			accounts: Vec::new(),
			device: None,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAccount {
	#[serde(flatten)]
	pub account: Account,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub secret: Option<String>,
}

impl AccountsFile {
	/// Приводит файл старой версии к текущей и чинит несогласованности (активный id без аккаунта).
	fn migrate(mut self) -> Self {
		if self.version > CURRENT_VERSION {
			log::warn!(
				"accounts.json версии {} новее поддерживаемой {CURRENT_VERSION}, читаем как есть",
				self.version
			);
		}
		// Версия 0 — файл без поля version; формат совпадает с версией 1.
		if self.version < CURRENT_VERSION {
			self.version = CURRENT_VERSION;
		}
		let active_exists = self
			.active_id
			.as_ref()
			.is_some_and(|id| self.accounts.iter().any(|a| &a.account.id == id));
		if !active_exists {
			self.active_id = self.accounts.first().map(|a| a.account.id.clone());
		}
		self
	}
}

pub fn path_in(dir: &Path) -> PathBuf {
	dir.join(FILE_NAME)
}

/// Результат чтения `accounts.json`.
pub struct Loaded {
	pub file: AccountsFile,
	/// `false` — файл есть, но прочитать или сохранить его копию не удалось: писать поверх нельзя.
	pub writable: bool,
}

/// Повреждённый файл копируется в `.bak-<время>`. Не прочитался вовсе — пустой список и
/// `writable = false`, чтобы первая же запись его не затёрла.
pub fn load(path: &Path) -> Loaded {
	let empty = |writable| Loaded {
		file: AccountsFile::default(),
		writable,
	};
	let bytes = match fs::read(path) {
		Ok(bytes) => bytes,
		Err(err) if err.kind() == io::ErrorKind::NotFound => return empty(true),
		Err(err) => {
			log::error!("не удалось прочитать {}: {err}", path.display());
			return empty(false);
		}
	};
	match serde_json::from_slice::<AccountsFile>(&bytes) {
		Ok(file) => Loaded {
			file: file.migrate(),
			writable: true,
		},
		Err(err) => {
			log::error!("{} повреждён: {err}", path.display());
			match fs_util::backup_copy(path, &bytes) {
				Ok(backup) => {
					log::error!(
						"копия повреждённого файла: {}, начинаем с пустого списка",
						backup.display()
					);
					empty(true)
				}
				Err(err) => {
					log::error!("не удалось сохранить копию повреждённого файла: {err}");
					empty(false)
				}
			}
		}
	}
}

pub fn save(path: &Path, file: &AccountsFile) -> io::Result<()> {
	let json = serde_json::to_vec_pretty(file).map_err(io::Error::other)?;
	fs_util::write_atomic(path, &json)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::accounts::AccountKind;

	fn account(id: &str) -> StoredAccount {
		StoredAccount {
			account: Account {
				id: id.into(),
				kind: AccountKind::Offline,
				username: "Steve".into(),
				uuid: "8667ba71-b85a-4004-af54-457a9734eed7".into(),
				added_at: "2026-11-02T10:00:00Z".into(),
				needs_relogin: false,
			},
			secret: None,
		}
	}

	#[test]
	fn round_trip() {
		let dir = tempfile::tempdir().unwrap();
		let path = path_in(dir.path());
		let mut file = AccountsFile::default();
		file.accounts.push(account("a"));
		file.accounts.push(StoredAccount {
			secret: Some("dpapi:AAAA".into()),
			..account("b")
		});
		file.active_id = Some("b".into());
		save(&path, &file).unwrap();

		let text = fs::read_to_string(&path).unwrap();
		assert!(text.contains("\"activeId\": \"b\""));
		assert!(text.contains("\"needsRelogin\": false"));
		assert!(text.contains("\"addedAt\""));

		let loaded = load(&path).file;
		assert_eq!(loaded.accounts.len(), 2);
		assert_eq!(loaded.active_id.as_deref(), Some("b"));
		assert_eq!(loaded.accounts[1].secret.as_deref(), Some("dpapi:AAAA"));
		assert!(!dir.path().join("accounts.json.tmp").exists());
	}

	#[test]
	fn missing_file_is_empty() {
		let dir = tempfile::tempdir().unwrap();
		let loaded = load(&path_in(dir.path()));
		assert!(loaded.writable);
		let file = loaded.file;
		assert!(file.accounts.is_empty());
		assert_eq!(file.version, CURRENT_VERSION);
	}

	#[test]
	fn corrupt_file_is_backed_up() {
		let dir = tempfile::tempdir().unwrap();
		let path = path_in(dir.path());
		fs::write(&path, b"{ not json").unwrap();

		let first = load(&path);
		assert!(first.writable);
		assert!(first.file.accounts.is_empty());
		// Второй запуск с тем же файлом не затирает первую копию.
		assert!(load(&path).writable);

		let mut backups: Vec<_> = fs::read_dir(dir.path())
			.unwrap()
			.filter_map(|e| e.ok())
			.map(|e| e.file_name().to_string_lossy().into_owned())
			.filter(|name| name.starts_with("accounts.json.bak-"))
			.collect();
		backups.sort();
		assert_eq!(backups.len(), 2);
		for name in backups {
			assert_eq!(fs::read(dir.path().join(name)).unwrap(), b"{ not json");
		}
	}

	#[test]
	fn unreadable_file_is_not_writable() {
		let dir = tempfile::tempdir().unwrap();
		let path = path_in(dir.path());
		fs::create_dir(&path).unwrap();
		let loaded = load(&path);
		assert!(!loaded.writable);
		assert!(loaded.file.accounts.is_empty());
	}

	#[test]
	fn migrates_unversioned_and_fixes_active() {
		let dir = tempfile::tempdir().unwrap();
		let path = path_in(dir.path());
		fs::write(
			&path,
			r#"{
				"activeId": "missing",
				"accounts": [{
					"id": "a", "kind": "offline", "username": "Steve",
					"uuid": "8667ba71-b85a-4004-af54-457a9734eed7",
					"addedAt": "2026-11-02T10:00:00Z", "needsRelogin": false,
					"futureField": 1
				}]
			}"#,
		)
		.unwrap();

		let file = load(&path).file;
		assert_eq!(file.version, CURRENT_VERSION);
		assert_eq!(file.active_id.as_deref(), Some("a"));
	}

	#[test]
	fn atomic_write_replaces_existing() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("x.json");
		fs_util::write_atomic(&path, b"one").unwrap();
		fs_util::write_atomic(&path, b"two").unwrap();
		assert_eq!(fs::read(&path).unwrap(), b"two");
	}
}
