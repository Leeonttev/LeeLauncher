//! Общие команды приложения: пути данных, открытие папок в проводнике, язык интерфейса.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::lang::{self, Lang};
use crate::launcher::Launcher;
use crate::paths::Layout;

/// Папки, которые интерфейс может открыть в проводнике.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderKind {
	Data,
	Logs,
	Instances,
	Java,
}

impl FolderKind {
	pub fn path(self, layout: &Layout) -> PathBuf {
		match self {
			Self::Data => layout.root().to_path_buf(),
			Self::Logs => layout.logs_dir(),
			Self::Instances => layout.instances_dir(),
			Self::Java => layout.java_dir(),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
	pub data: String,
	pub logs: String,
	pub instances: String,
	pub java: String,
}

impl AppPaths {
	pub fn new(layout: &Layout) -> Self {
		let text = |kind: FolderKind| kind.path(layout).to_string_lossy().into_owned();
		Self {
			data: text(FolderKind::Data),
			logs: text(FolderKind::Logs),
			instances: text(FolderKind::Instances),
			java: text(FolderKind::Java),
		}
	}
}

/// Обе папки одного вида: ничего не тронуто, работаем со старой (в ней данные).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictFolders {
	pub old: String,
	pub new: String,
}

/// Итог переноса данных при запуске.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationStatus {
	/// Перенос не закончен: следующий запуск его продолжит.
	pub unfinished: bool,
	pub conflicts: Vec<ConflictFolders>,
	pub data_dir: String,
	pub webview_dir: String,
}

impl MigrationStatus {
	pub fn new(outcome: &crate::migrate::Outcome) -> Self {
		let text = |path: &Path| path.to_string_lossy().into_owned();
		Self {
			unfinished: outcome.unfinished(),
			conflicts: outcome
				.conflicts
				.iter()
				.map(|folder| ConflictFolders {
					old: text(&folder.old),
					new: text(&folder.new),
				})
				.collect(),
			data_dir: text(&outcome.data_dir),
			webview_dir: text(&outcome.webview_dir),
		}
	}
}

#[tauri::command]
pub fn app_migration_status(status: State<'_, MigrationStatus>) -> MigrationStatus {
	status.inner().clone()
}

/// Открывает папку в проводнике; если её ещё нет (Java не качали), создаёт.
pub fn open_dir(app: &AppHandle, dir: &Path) -> Result<(), AppError> {
	use tauri_plugin_opener::OpenerExt;
	std::fs::create_dir_all(dir)
		.map_err(|e| AppError::storage(format!("{}: {e}", dir.display())))?;
	app.opener()
		.open_path(dir.to_string_lossy(), None::<&str>)
		.map_err(|e| AppError::unknown(e.to_string()))
}

#[tauri::command]
pub async fn app_paths(l: State<'_, Arc<Launcher>>) -> Result<AppPaths, AppError> {
	Ok(AppPaths::new(&l.layout))
}

#[tauri::command]
pub async fn app_open_folder(
	app: AppHandle,
	l: State<'_, Arc<Launcher>>,
	kind: FolderKind,
) -> Result<(), AppError> {
	open_dir(&app, &kind.path(&l.layout))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn paths_and_kinds() {
		let layout = Layout::new(PathBuf::from(r"C:\Data\LeeLauncher"));
		let paths = AppPaths::new(&layout);
		assert_eq!(paths.data, r"C:\Data\LeeLauncher");
		assert_eq!(paths.logs, r"C:\Data\LeeLauncher\logs");
		assert_eq!(paths.instances, r"C:\Data\LeeLauncher\instances");
		assert_eq!(paths.java, r"C:\Data\LeeLauncher\java");
		let json = serde_json::to_value(&paths).unwrap();
		assert!(json.get("instances").is_some());

		let kind: FolderKind = serde_json::from_str("\"logs\"").unwrap();
		assert_eq!(kind, FolderKind::Logs);
		assert!(serde_json::from_str::<FolderKind>("\"meta\"").is_err());
	}
}

/// Язык интерфейса для текстов, которые показывает ядро вне окна (статус Discord, трей).
#[tauri::command]
pub fn app_set_lang(app: AppHandle, lang: Lang) {
	lang::set_ui(lang);
	crate::presence::refresh(&app);
	crate::tray::relabel(&app);
}
