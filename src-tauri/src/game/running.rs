//! `running.json`: запущенные игры на случай, если лаунчер закроется или упадёт раньше игры.
//! При следующем запуске лаунчер находит живые процессы и снова следит за ними.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::GameAccount;
use crate::fs_util;

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
	pub instance_id: String,
	/// Процесс, который запустил лаунчер (может быть заглушкой, а игра — её потомком).
	pub pid: u32,
	/// Время создания этого процесса: PID мог достаться другому процессу.
	pub created: u64,
	/// ISO 8601: начало запуска.
	pub started_at: String,
	#[serde(default)]
	pub java: Option<String>,
	#[serde(default)]
	pub account: Option<GameAccount>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileData {
	#[serde(default)]
	version: u32,
	#[serde(default)]
	games: Vec<Record>,
}

/// Записи в памяти и на диске. Без пути (тесты без папки данных) — только в памяти.
pub struct RunningFile {
	path: Option<PathBuf>,
	records: Mutex<Vec<Record>>,
}

impl RunningFile {
	pub fn new(path: Option<PathBuf>) -> Self {
		Self {
			path,
			records: Mutex::new(Vec::new()),
		}
	}

	/// Записи прошлого запуска. Повреждённый файл — пустой список.
	pub fn load(&self) -> Vec<Record> {
		let Some(path) = &self.path else {
			return Vec::new();
		};
		let Ok(bytes) = std::fs::read(path) else {
			return Vec::new();
		};
		match serde_json::from_slice::<FileData>(&bytes) {
			Ok(data) => data
				.games
				.into_iter()
				.filter(|r| crate::fs_util::is_safe_id(&r.instance_id))
				.collect(),
			Err(err) => {
				log::warn!("{} повреждён: {err}", path.display());
				Vec::new()
			}
		}
	}

	pub fn replace(&self, records: Vec<Record>) {
		if let Ok(mut guard) = self.records.lock() {
			*guard = records;
			self.save(&guard);
		}
	}

	pub fn add(&self, record: Record) {
		if let Ok(mut guard) = self.records.lock() {
			guard.retain(|r| r.instance_id != record.instance_id);
			guard.push(record);
			self.save(&guard);
		}
	}

	pub fn remove(&self, instance_id: &str) {
		if let Ok(mut guard) = self.records.lock() {
			let before = guard.len();
			guard.retain(|r| r.instance_id != instance_id);
			if guard.len() != before {
				self.save(&guard);
			}
		}
	}

	/// Пишется сразу и под замком: файл меньше килобайта, запись — только при запуске и выходе игры.
	/// В `spawn_blocking` записи могли бы обогнать друг друга, и старый список затёр бы новый.
	fn save(&self, records: &[Record]) {
		let Some(path) = &self.path else {
			return;
		};
		let result = if records.is_empty() {
			match std::fs::remove_file(path) {
				Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
				_ => Ok(()),
			}
		} else {
			let data = FileData {
				version: VERSION,
				games: records.to_vec(),
			};
			serde_json::to_vec_pretty(&data)
				.map_err(std::io::Error::other)
				.and_then(|json| fs_util::write_atomic(path, &json))
		};
		if let Err(err) = result {
			log::warn!("не удалось записать {}: {err}", path.display());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::accounts::AccountKind;

	fn record(id: &str, pid: u32) -> Record {
		Record {
			instance_id: id.into(),
			pid,
			created: 7,
			started_at: "2026-11-18T10:00:00Z".into(),
			java: Some(r"C:\java\bin\javaw.exe".into()),
			account: Some(GameAccount {
				id: "acc".into(),
				uuid: "u".into(),
				username: "Steve".into(),
				kind: AccountKind::Offline,
			}),
		}
	}

	#[test]
	fn writes_and_removes() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("running.json");
		let file = RunningFile::new(Some(path.clone()));
		file.add(record("a", 1));
		file.add(record("b", 2));
		file.add(record("a", 3));
		let loaded = RunningFile::new(Some(path.clone())).load();
		assert_eq!(loaded.len(), 2);
		assert_eq!(loaded.iter().find(|r| r.instance_id == "a").unwrap().pid, 3);
		assert_eq!(loaded[0].account.as_ref().unwrap().username, "Steve");

		file.remove("a");
		file.remove("b");
		assert!(!path.exists());
	}

	#[test]
	fn bad_file_is_empty() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("running.json");
		std::fs::write(&path, b"{oops").unwrap();
		assert!(RunningFile::new(Some(path.clone())).load().is_empty());
		std::fs::write(
			&path,
			br#"{"version":1,"games":[{"instanceId":"../x","pid":1,"created":1,"startedAt":"t"}]}"#,
		)
		.unwrap();
		assert!(RunningFile::new(Some(path)).load().is_empty());
	}
}
