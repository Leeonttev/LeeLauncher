//! Команды Tauri: версии и загрузчики, сборки, установка, Java, настройки.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use super::install::{
	self, EVENT_FINISHED, EVENT_JAVA_PROGRESS, EVENT_PROGRESS, InstallEvents, InstallFinished,
	InstallMode, InstallProgress,
};
use super::{Instance, InstancePatch, Loader};
use crate::download::Progress;
use crate::downloads::{Download, Target};
use crate::error::{AppError, ErrorCode};
use crate::java::{self, JavaInstall};
use crate::lang::Lang;
use crate::launcher::Launcher;
use crate::meta::model::VersionType;
use crate::meta::{GameVersions, Listed, LoaderKind, LoaderVersionEntry};
use crate::settings::{Settings, SettingsPatch};
use crate::sysmem::{self, SystemMemory};

pub const EVENT_CHANGED: &str = "instances-changed";

type L<'a> = State<'a, Arc<Launcher>>;

fn emit<T: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
	if let Err(err) = app.emit(event, payload) {
		log::warn!("не удалось отправить {event}: {err}");
	}
}

async fn emit_changed(app: &AppHandle, l: &Launcher) {
	emit(app, EVENT_CHANGED, l.instances.list().await);
	// Название или версия идущей сборки могли смениться.
	crate::presence::refresh(app);
}

pub(crate) struct TauriEvents(pub AppHandle);

impl InstallEvents for TauriEvents {
	fn progress(&self, progress: InstallProgress) {
		emit(&self.0, EVENT_PROGRESS, progress);
	}

	fn changed(&self, instances: Vec<Instance>) {
		emit(&self.0, EVENT_CHANGED, instances);
	}

	fn finished(&self, finished: InstallFinished) {
		emit(&self.0, EVENT_FINISHED, finished);
	}
}

#[tauri::command]
pub async fn minecraft_versions(
	l: L<'_>,
	kinds: Option<Vec<VersionType>>,
	refresh: Option<bool>,
) -> Result<GameVersions, AppError> {
	l.meta
		.game_versions(
			kinds.as_deref(),
			refresh.unwrap_or(false),
			&CancellationToken::new(),
		)
		.await
}

#[tauri::command]
pub async fn loader_game_versions(
	l: L<'_>,
	loader: LoaderKind,
) -> Result<Listed<String>, AppError> {
	l.meta
		.loader_game_versions(loader, &CancellationToken::new())
		.await
}

#[tauri::command]
pub async fn loader_versions(
	l: L<'_>,
	loader: LoaderKind,
	game_version: String,
) -> Result<Listed<LoaderVersionEntry>, AppError> {
	l.meta
		.loader_versions(loader, &game_version, &CancellationToken::new())
		.await
}

/// Версия игры есть в манифесте, загрузчик её поддерживает (и указанная версия загрузчика существует).
pub(crate) async fn check_version(
	l: &Launcher,
	game_version: &str,
	loader: &Loader,
) -> Result<(), AppError> {
	let cancel = CancellationToken::new();
	l.meta.find_version(game_version, &cancel).await?;
	if loader.kind == LoaderKind::Vanilla {
		return Ok(());
	}
	let versions = l
		.meta
		.loader_versions(loader.kind, game_version, &cancel)
		.await?
		.items;
	let ok = match &loader.version {
		None => !versions.is_empty(),
		Some(v) => versions.iter().any(|e| &e.id == v),
	};
	if ok {
		Ok(())
	} else {
		Err(AppError::coded(
			ErrorCode::LoaderNotFound,
			format!(
				"{} {} для Minecraft {game_version}",
				loader.kind.as_str(),
				loader.version.as_deref().unwrap_or("(любая)")
			),
		))
	}
}

#[tauri::command]
pub async fn instances_list(l: L<'_>) -> Result<Vec<Instance>, AppError> {
	Ok(l.instances.list().await)
}

#[tauri::command]
pub async fn instance_create(
	app: AppHandle,
	l: L<'_>,
	name: String,
	game_version: String,
	loader: Loader,
	cover: Option<String>,
	group: Option<String>,
) -> Result<Instance, AppError> {
	super::validate_name(&name)?;
	super::normalize_group(group.as_deref())?;
	let cover = cover
		.as_deref()
		.map(super::InstanceCover::parse)
		.transpose()?;
	check_version(&l, &game_version, &loader).await?;
	let instance = l
		.instances
		.create(&name, &game_version, loader, cover, group.as_deref())
		.await?;
	log::info!("создана сборка {} ({})", instance.id, instance.game_version);
	emit_changed(&app, &l).await;
	Ok(instance)
}

#[tauri::command]
pub async fn instance_update(
	app: AppHandle,
	l: L<'_>,
	id: String,
	patch: InstancePatch,
) -> Result<Instance, AppError> {
	let current = l.instances.get(&id).await?;
	if patch.changes_version(&current) {
		let game = patch
			.game_version
			.as_deref()
			.unwrap_or(&current.game_version);
		let loader = patch.loader.as_ref().unwrap_or(&current.loader);
		check_version(&l, game, loader).await?;
	}
	let instance = l.instances.update(&id, patch).await?;
	emit_changed(&app, &l).await;
	Ok(instance)
}

#[tauri::command]
pub async fn instance_remove(app: AppHandle, l: L<'_>, id: String) -> Result<(), AppError> {
	{
		// Под замком контента: задача контента либо уже видна, либо не найдёт сборку.
		let _content = l.content_locks.lock(&id).await;
		if l.downloads.has_active_content(&id) {
			return Err(AppError::coded(ErrorCode::InstanceBusy, id));
		}
		l.watch.stop_if(&id);
		l.manual.forget(&id);
		l.instances.remove(&id).await?;
	}
	l.games.forget(&id);
	l.downloads.forget_instance(&id);
	log::info!("сборка {id} удалена");
	emit_changed(&app, &l).await;
	Ok(())
}

#[tauri::command]
pub async fn instance_duplicate(
	app: AppHandle,
	l: L<'_>,
	id: String,
	name: String,
) -> Result<Instance, AppError> {
	let instance = l.instances.duplicate(&id, &name).await?;
	log::info!("сборка {id} скопирована в {}", instance.id);
	emit_changed(&app, &l).await;
	Ok(instance)
}

/// Переименовывает группу у всех её сборок (совпала с другой группой — объединяет).
/// Возвращает изменённые сборки.
#[tauri::command]
pub async fn instance_group_rename(
	app: AppHandle,
	l: L<'_>,
	from: String,
	to: String,
) -> Result<Vec<Instance>, AppError> {
	let to = super::normalize_group(Some(&to))?
		.ok_or_else(|| AppError::coded(ErrorCode::InvalidName, "group"))?;
	let (changed, error) = l.instances.set_group_of(&from, Some(to)).await;
	if !changed.is_empty() {
		emit_changed(&app, &l).await;
	}
	error.map_or(Ok(changed), Err)
}

/// Снимает группу у всех её сборок; сами сборки остаются. Возвращает изменённые сборки.
#[tauri::command]
pub async fn instance_group_remove(
	app: AppHandle,
	l: L<'_>,
	name: String,
) -> Result<Vec<Instance>, AppError> {
	let (changed, error) = l.instances.set_group_of(&name, None).await;
	if !changed.is_empty() {
		emit_changed(&app, &l).await;
	}
	error.map_or(Ok(changed), Err)
}

#[tauri::command]
pub async fn instance_install(
	app: AppHandle,
	l: L<'_>,
	id: String,
	mode: Option<InstallMode>,
) -> Result<(), AppError> {
	start_install(app, l.inner().clone(), &id, mode.unwrap_or_default()).await?;
	Ok(())
}

/// Начинает установку в фоне и возвращает id задачи в списке загрузок.
async fn start_install(
	app: AppHandle,
	l: Arc<Launcher>,
	id: &str,
	mode: InstallMode,
) -> Result<String, AppError> {
	let started = install::begin(&l, id, mode).await?;
	let task_id = started.task_id.clone();
	let events: Arc<dyn InstallEvents> = Arc::new(TauriEvents(app));
	tauri::async_runtime::spawn(install::run(l, started, events));
	Ok(task_id)
}

#[tauri::command]
pub async fn instance_install_cancel(l: L<'_>, id: String) -> Result<(), AppError> {
	if l.instances.cancel_task(&id) {
		log::info!("установка {id}: отмена");
	}
	Ok(())
}

/// Папка игры в проводнике; `sub` — подпапка, `select` — выделить файл.
#[tauri::command]
pub async fn instance_open_folder(
	app: AppHandle,
	l: L<'_>,
	id: String,
	sub: Option<String>,
	select: Option<String>,
) -> Result<(), AppError> {
	use tauri_plugin_opener::OpenerExt;
	l.instances.get(&id).await?;
	let game_dir = l.instances.game_dir(&id);
	let dir = match &sub {
		Some(sub) => crate::content::commands::sub_folder(&game_dir, sub)?,
		None => game_dir,
	};
	if let Some(file) = select {
		if !crate::fs_util::is_safe_file_name(&file) {
			return Err(AppError::coded(
				ErrorCode::InvalidValue,
				format!("select: {file}"),
			));
		}
		let path = dir.join(&file);
		if !path.exists() {
			return Err(AppError::coded(ErrorCode::FileNotFound, file));
		}
		return app
			.opener()
			.reveal_item_in_dir(&path)
			.map_err(|e| AppError::unknown(e.to_string()));
	}
	crate::app::open_dir(&app, &dir)
}

/// Всегда ищет заново («Найти снова») и обновляет кэш, которым пользуется запуск игры.
#[tauri::command]
pub async fn java_list(l: L<'_>) -> Result<Vec<JavaInstall>, AppError> {
	Ok(l.java_found.refresh(&l.layout.java_dir()).await)
}

#[tauri::command]
pub async fn java_check(l: L<'_>, path: String) -> Result<JavaInstall, AppError> {
	java::check(PathBuf::from(path), l.layout.java_dir()).await
}

/// Системное окно выбора java.exe; выбранное сразу проверяется.
#[tauri::command]
pub async fn java_pick_file(
	app: AppHandle,
	l: L<'_>,
	lang: Option<Lang>,
) -> Result<Option<JavaInstall>, AppError> {
	use tauri::Manager;
	use tauri_plugin_dialog::DialogExt;

	let mut dialog = app
		.dialog()
		.file()
		.set_title(java::pick_title(lang.unwrap_or_default()));
	if cfg!(windows) {
		dialog = dialog.add_filter("Java (java.exe, javaw.exe)", &["exe"]);
	}
	if let Some(dir) = java::pick_start_dir() {
		dialog = dialog.set_directory(dir);
	}
	if let Some(main) = app.get_webview_window(crate::MAIN_WINDOW) {
		dialog = dialog.set_parent(&main);
	}
	let (tx, rx) = tokio::sync::oneshot::channel();
	dialog.pick_file(move |file| {
		let _ = tx.send(file);
	});
	let Some(file) = rx.await.ok().flatten() else {
		return Ok(None);
	};
	let path = file
		.into_path()
		.map_err(|e| AppError::coded(ErrorCode::JavaInvalid, e.to_string()))?;
	java::check(path, l.layout.java_dir()).await.map(Some)
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct JavaProgress {
	major: u32,
	percent: f32,
	bytes_done: u64,
	bytes_total: u64,
}

#[tauri::command]
pub async fn java_install(app: AppHandle, l: L<'_>, major: u32) -> Result<JavaInstall, AppError> {
	start_java(app, l.inner().clone(), major).1.await
}

/// Ставит загрузку Java в список загрузок и возвращает id задачи и саму загрузку
/// (её ждёт `java_install` или запускает в фоне `download_retry`).
fn start_java(
	app: AppHandle,
	l: Arc<Launcher>,
	major: u32,
) -> (
	String,
	impl std::future::Future<Output = Result<JavaInstall, AppError>> + Send + 'static,
) {
	static ATTEMPT: AtomicU64 = AtomicU64::new(0);
	let attempt = ATTEMPT.fetch_add(1, Ordering::Relaxed);
	let cancel = CancellationToken::new();
	// Повторный запрос той же версии заменяет прошлый: прошлый отменяется.
	if let Ok(mut installs) = l.java_installs.lock()
		&& let Some((_, previous)) = installs.insert(major, (attempt, cancel.clone()))
	{
		previous.cancel();
	}
	let task_id = l.downloads.begin(Target::Java { major }, cancel.clone());
	let downloads = l.downloads.clone();
	let tracked_id = task_id.clone();
	let progress = Progress::new(move |snap| {
		downloads.progress(&tracked_id, None, None, snap);
		let percent = if snap.bytes_total > 0 {
			snap.bytes_done as f32 * 100.0 / snap.bytes_total as f32
		} else {
			0.0
		};
		emit(
			&app,
			EVENT_JAVA_PROGRESS,
			JavaProgress {
				major,
				percent: percent.min(100.0),
				bytes_done: snap.bytes_done,
				bytes_total: snap.bytes_total,
			},
		);
	});
	let id = task_id.clone();
	let run = async move {
		let result = async {
			let _guard = tokio::select! {
				_ = cancel.cancelled() => return Err(AppError::new(ErrorCode::Cancelled)),
				g = l.java_lock.lock() => g,
			};
			java::runtime::install(
				&l.meta,
				&l.dl,
				&l.layout.java_dir(),
				None,
				major,
				progress.clone(),
				&cancel,
			)
			.await
		}
		.await;
		if let Ok(mut installs) = l.java_installs.lock()
			&& installs.get(&major).is_some_and(|(a, _)| *a == attempt)
		{
			installs.remove(&major);
		}
		if result.is_ok() {
			progress.flush();
		}
		l.downloads
			.finish(&id, result.as_ref().map(|_| ()).map_err(Clone::clone));
		let java = result?;
		l.java_found.invalidate().await;
		log::info!("Java {} скачана: {}", java.version, java.path);
		Ok(java)
	};
	(task_id, run)
}

#[tauri::command]
pub async fn java_install_cancel(l: L<'_>, major: u32) -> Result<(), AppError> {
	let token = l
		.java_installs
		.lock()
		.ok()
		.and_then(|i| i.get(&major).map(|(_, t)| t.clone()));
	if let Some(token) = token {
		log::info!("скачивание Java {major}: отмена");
		token.cancel();
	}
	Ok(())
}

#[tauri::command]
pub async fn settings_get(l: L<'_>) -> Result<Settings, AppError> {
	Ok(l.settings.get())
}

#[tauri::command]
pub async fn settings_update(
	app: AppHandle,
	l: L<'_>,
	patch: SettingsPatch,
) -> Result<Settings, AppError> {
	let settings = l.settings.update(patch)?;
	l.dl.set_source(settings.download_source);
	crate::presence::refresh(&app);
	Ok(settings)
}

#[tauri::command]
pub async fn system_memory() -> Result<SystemMemory, AppError> {
	Ok(sysmem::system_memory())
}

/// Список загрузок: идущие и последние завершённые, новые первыми.
#[tauri::command]
pub async fn downloads_list(l: L<'_>) -> Result<Vec<Download>, AppError> {
	Ok(l.downloads.list())
}

/// Отмена загрузки по id задачи (то же, что `instance_install_cancel` / `java_install_cancel`).
#[tauri::command]
pub async fn download_cancel(l: L<'_>, id: String) -> Result<(), AppError> {
	if l.downloads.cancel(&id) {
		log::info!("загрузка {id}: отмена");
	}
	Ok(())
}

/// Повтор завершённой задачи с теми же параметрами.
#[tauri::command]
pub async fn download_retry(app: AppHandle, l: L<'_>, id: String) -> Result<Download, AppError> {
	let repair = l.downloads.get(&id).is_some_and(|t| t.warnings.is_some());
	let target = l
		.downloads
		.retry_target(&id)
		.ok_or_else(|| AppError::coded(ErrorCode::InvalidValue, format!("download: {id}")))?;
	let task_id = match target {
		Target::Instance { id, mode } => start_install(app, l.inner().clone(), &id, mode).await?,
		Target::Content {
			instance_id,
			request,
			..
		} => match crate::content::commands::retry(app, l.inner().clone(), instance_id, request)
			.await?
		{
			Some(task) => task.id,
			None => {
				return Err(AppError::coded(ErrorCode::InvalidValue, "нечего обновлять"));
			}
		},
		Target::Pack {
			instance_id,
			request,
			..
		} => {
			crate::modpacks::commands::retry(app, l.inner().clone(), instance_id, request, repair)
				.await?
				.id
		}
		Target::Java { major } => {
			let (task_id, run) = start_java(app, l.inner().clone(), major);
			tauri::async_runtime::spawn(async move {
				if let Err(err) = run.await {
					log::warn!("повтор скачивания Java {major}: {:?}", err.code);
				}
			});
			task_id
		}
	};
	l.downloads
		.get(&task_id)
		.ok_or_else(|| AppError::unknown("задача пропала"))
}

/// Убирает из списка все завершённые загрузки.
#[tauri::command]
pub async fn downloads_clear(l: L<'_>) -> Result<(), AppError> {
	l.downloads.clear();
	Ok(())
}

/// Убирает из списка одну завершённую загрузку (идущую — нет).
#[tauri::command]
pub async fn download_dismiss(l: L<'_>, id: String) -> Result<(), AppError> {
	l.downloads.dismiss(&id);
	Ok(())
}
