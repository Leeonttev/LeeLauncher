//! `update-state.json`: версия прошлого запуска и заметки к ставящейся версии — для тоста
//! «Обновлено до …» на первом запуске новой версии.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs_util;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateFile {
	#[serde(default)]
	pub last_version: Option<String>,
	/// Записывается перед запуском установщика.
	#[serde(default)]
	pub pending: Option<Pending>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
	pub version: String,
	#[serde(default)]
	pub notes: Option<String>,
}

/// Ответ `app_last_update`: этот запуск — первый после обновления.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastUpdate {
	pub updated_from: String,
	pub version: String,
	pub notes: Option<String>,
}

pub fn path(root: &Path) -> PathBuf {
	root.join("update-state.json")
}

pub fn read(root: &Path) -> StateFile {
	match std::fs::read(path(root)) {
		Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
			log::warn!("update-state.json не разобрался, начинаем заново: {err}");
			StateFile::default()
		}),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => StateFile::default(),
		Err(err) => {
			log::warn!("update-state.json не прочитался: {err}");
			StateFile::default()
		}
	}
}

pub fn write(root: &Path, state: &StateFile) {
	let result = serde_json::to_vec_pretty(state)
		.map_err(std::io::Error::other)
		.and_then(|bytes| fs_util::write_atomic(&path(root), &bytes));
	if let Err(err) = result {
		log::warn!("не удалось записать update-state.json: {err}");
	}
}

/// Сравнивает прошлую версию с текущей и запоминает текущую. Первая установка (записи нет) —
/// `None`: тост не нужен.
pub fn on_start(root: &Path, current: &str) -> Option<LastUpdate> {
	let state = read(root);
	let result = state
		.last_version
		.as_deref()
		.filter(|last| *last != current)
		.map(|last| LastUpdate {
			updated_from: last.to_owned(),
			version: current.to_owned(),
			notes: state
				.pending
				.as_ref()
				.filter(|p| p.version == current)
				.and_then(|p| p.notes.clone()),
		});
	let next = StateFile {
		last_version: Some(current.to_owned()),
		pending: state.pending.clone().filter(|p| p.version != current),
	};
	if next != state {
		write(root, &next);
	}
	result
}

pub fn set_pending(root: &Path, pending: Pending) {
	let mut state = read(root);
	state.pending = Some(pending);
	write(root, &state);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reports_update_once() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		// Первая установка — без тоста, версия запоминается.
		assert_eq!(on_start(root, "0.1.0"), None);
		assert_eq!(on_start(root, "0.1.0"), None);
		set_pending(
			root,
			Pending {
				version: "0.2.0".into(),
				notes: Some("Новое".into()),
			},
		);
		let last = on_start(root, "0.2.0").unwrap();
		assert_eq!(last.updated_from, "0.1.0");
		assert_eq!(last.notes.as_deref(), Some("Новое"));
		assert_eq!(read(root).pending, None);
		assert_eq!(on_start(root, "0.2.0"), None);
	}

	#[test]
	fn broken_file_is_reset() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(path(dir.path()), b"{oops").unwrap();
		assert_eq!(on_start(dir.path(), "0.1.0"), None);
		assert_eq!(read(dir.path()).last_version.as_deref(), Some("0.1.0"));
	}

	#[test]
	fn pending_of_other_version_is_kept_out() {
		let dir = tempfile::tempdir().unwrap();
		on_start(dir.path(), "0.1.0");
		set_pending(
			dir.path(),
			Pending {
				version: "0.3.0".into(),
				notes: Some("x".into()),
			},
		);
		// Установка не прошла, запустилась та же версия: тоста нет, заметки ждут своей версии.
		assert_eq!(on_start(dir.path(), "0.1.0"), None);
		assert!(read(dir.path()).pending.is_some());
	}
}
