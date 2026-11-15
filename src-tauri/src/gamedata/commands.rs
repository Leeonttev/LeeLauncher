//! Команды Tauri страницы сборки: миры, скриншоты, файлы логов, наблюдение за папками.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use super::{LogContent, LogFile, ScreenshotPage, World};
use crate::error::{AppError, ErrorCode};
use crate::instances::Instance;
use crate::launcher::Launcher;

type L<'a> = State<'a, Arc<Launcher>>;

fn join_error(err: tokio::task::JoinError) -> AppError {
	AppError::unknown(err.to_string())
}

async fn instance(l: &Launcher, id: &str) -> Result<(Instance, std::path::PathBuf), AppError> {
	let instance = l.instances.get(id).await?;
	Ok((instance, l.layout.game_dir(id)))
}

/// Умеет ли установленная версия сборки заходить сразу в мир.
pub fn quick_play(l: &Launcher, instance: &Instance) -> bool {
	instance
		.installed
		.as_ref()
		.is_some_and(|i| super::supports_quick_play(&l.layout.version_json(&i.version_id)))
}

/// Миры сборки, последние сыгранные первыми.
#[tauri::command]
pub async fn instance_worlds(l: L<'_>, id: String) -> Result<Vec<World>, AppError> {
	let (instance, dir) = instance(&l, &id).await?;
	let quick = quick_play(&l, &instance);
	let instance_dir = l.layout.instance_dir(&id);
	tokio::task::spawn_blocking(move || {
		let sources = crate::content::worlds::load(&instance_dir);
		let mut list = super::worlds(&dir, quick);
		for w in &mut list {
			w.source = sources.worlds.get(&w.folder).map(|s| super::WorldSource {
				source: s.source,
				project_id: s.project_id.clone(),
			});
		}
		list
	})
	.await
	.map_err(join_error)
}

/// Удаляет мир в корзину. Пока игра сборки запущена — `instance_busy`.
#[tauri::command]
pub async fn instance_world_remove(
	app: AppHandle,
	l: L<'_>,
	id: String,
	folder: String,
) -> Result<(), AppError> {
	let (_, dir) = instance(&l, &id).await?;
	if l.instances.is_running(&id) {
		return Err(AppError::coded(ErrorCode::InstanceBusy, id));
	}
	let world = super::world_dir(&dir, &folder)?;
	tokio::task::spawn_blocking(move || {
		crate::trash::delete(&world).map_err(|e| crate::content::installed::io_error(&world, &e))
	})
	.await
	.map_err(join_error)??;
	log::info!("сборка {id}: мир удалён в корзину");
	super::watch::emit(&app, super::watch::EVENT_WORLDS, &id);
	// Датапаки мира были в списке контента.
	super::watch::emit(&app, crate::content::EVENT_CHANGED, &id);
	Ok(())
}

/// Скриншоты порцией, новые первыми. Открывает их папки для asset protocol.
#[tauri::command]
pub async fn instance_screenshots(
	app: AppHandle,
	l: L<'_>,
	id: String,
	offset: Option<usize>,
	limit: Option<usize>,
) -> Result<ScreenshotPage, AppError> {
	let (_, dir) = instance(&l, &id).await?;
	let thumbs = l.layout.thumbs_dir();
	let scope = app.asset_protocol_scope();
	for allowed in [super::screenshots_dir(&dir), thumbs.clone()] {
		if let Err(err) = scope.allow_directory(&allowed, false) {
			log::warn!("asset protocol: {}: {err}", allowed.display());
		}
	}
	let offset = offset.unwrap_or(0);
	let limit = limit.unwrap_or(super::SCREENSHOTS_LIMIT_DEFAULT);
	tokio::task::spawn_blocking(move || super::screenshots(&dir, &thumbs, offset, limit))
		.await
		.map_err(join_error)
}

/// Копирует скриншот в буфер обмена картинкой.
#[tauri::command]
pub async fn instance_screenshot_copy(l: L<'_>, id: String, file: String) -> Result<(), AppError> {
	let (_, dir) = instance(&l, &id).await?;
	let path = super::screenshot_path(&dir, &file)?;
	tokio::task::spawn_blocking(move || {
		let img = image::open(&path)
			.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?
			.to_rgba8();
		let data = arboard::ImageData {
			width: img.width() as usize,
			height: img.height() as usize,
			bytes: std::borrow::Cow::Owned(img.into_raw()),
		};
		arboard::Clipboard::new()
			.and_then(|mut c| c.set_image(data))
			.map_err(|e| AppError::unknown(format!("буфер обмена: {e}")))
	})
	.await
	.map_err(join_error)?
}

#[tauri::command]
pub async fn instance_screenshot_remove(
	app: AppHandle,
	l: L<'_>,
	id: String,
	file: String,
) -> Result<(), AppError> {
	let (_, dir) = instance(&l, &id).await?;
	let path = super::screenshot_path(&dir, &file)?;
	tokio::task::spawn_blocking(move || {
		crate::trash::delete(&path).map_err(|e| crate::content::installed::io_error(&path, &e))
	})
	.await
	.map_err(join_error)??;
	super::watch::emit(&app, super::watch::EVENT_SCREENSHOTS, &id);
	Ok(())
}

/// Открывает лог последнего запуска (`logs/latest.log`, иначе `logs/launcher-output.log`)
/// программой по умолчанию.
#[tauri::command]
pub async fn instance_log_file(app: AppHandle, l: L<'_>, id: String) -> Result<(), AppError> {
	use tauri_plugin_opener::OpenerExt;
	let (_, dir) = instance(&l, &id).await?;
	let path = ["latest.log", "launcher-output.log"]
		.iter()
		.map(|name| dir.join("logs").join(name))
		.find(|p| p.is_file())
		.ok_or_else(|| AppError::coded(ErrorCode::FileNotFound, "logs/latest.log"))?;
	app.opener()
		.open_path(path.to_string_lossy(), None::<&str>)
		.map_err(|e| AppError::unknown(e.to_string()))
}

/// Файлы логов (`logs/`) и отчёты о вылете (`crash-reports/`), новые первыми.
#[tauri::command]
pub async fn instance_logs_list(l: L<'_>, id: String) -> Result<Vec<LogFile>, AppError> {
	let (_, dir) = instance(&l, &id).await?;
	tokio::task::spawn_blocking(move || super::log_list(&dir))
		.await
		.map_err(join_error)
}

/// Содержимое файла лога: последние строки, разобранные как живой лог.
#[tauri::command]
pub async fn instance_logs_read(
	l: L<'_>,
	id: String,
	name: String,
) -> Result<LogContent, AppError> {
	let (_, dir) = instance(&l, &id).await?;
	let (path, kind) = super::log_path(&dir, &name)?;
	tokio::task::spawn_blocking(move || super::read_log(&path, kind))
		.await
		.map_err(join_error)?
}

/// Следить за папками одной сборки (`null` — перестать).
#[tauri::command]
pub async fn instance_watch(app: AppHandle, l: L<'_>, id: Option<String>) -> Result<(), AppError> {
	let generation = l.watch.begin();
	let target = match id {
		Some(id) => {
			let (_, dir) = instance(&l, &id).await?;
			Some((id, dir))
		}
		None => None,
	};
	let missing = target
		.as_ref()
		.map(|(id, _)| id.clone())
		.unwrap_or_default();
	// Тот же замок у удаления сборки: наблюдение не начнётся посреди удаления.
	let _content = match &target {
		Some((id, _)) => Some(l.content_locks.lock(id).await),
		None => None,
	};
	l.watch.set(&app, generation, target).map_err(|e| match e {
		super::watch::WatchError::Gone => AppError::coded(ErrorCode::InstanceNotFound, missing),
		other => AppError::unknown(format!("наблюдение за папкой: {other}")),
	})
}
