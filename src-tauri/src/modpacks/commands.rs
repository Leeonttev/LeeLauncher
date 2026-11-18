//! Команды модпаков, переноса и экспорта.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use super::export::{self, ExportPreview};
use super::install::{self, InspectSource, Preview, UpdatePreview, Work};
use super::launchers::{self, Category, ImportItem, ImportSource};
use super::{PackAction, PackOrigin, PackRequest, PackTask};
use crate::content::{self, ContentVersion, SourceKind, VersionFilter};
use crate::download::Progress;
use crate::downloads::{Download, Target};
use crate::error::{AppError, ErrorCode};
use crate::instances::commands::TauriEvents;
use crate::instances::install::{InstallEvents, InstallStage};
use crate::instances::{Instance, InstanceCover};
use crate::lang::Lang;
use crate::launcher::Launcher;

type L<'a> = State<'a, Arc<Launcher>>;

pub const EVENT_INSPECT_PROGRESS: &str = "modpack-inspect-progress";
pub const EVENT_SCAN_PROGRESS: &str = "import-scan-progress";
/// Обновление модпака проверяется не чаще этого (без `force`).
const UPDATE_TTL: Duration = Duration::from_secs(10 * 60);

fn join_error(err: tokio::task::JoinError) -> AppError {
	AppError::unknown(err.to_string())
}

fn emit<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
	if let Err(err) = app.emit(event, payload) {
		log::warn!("не удалось отправить {event}: {err}");
	}
}

async fn emit_instances(app: &AppHandle, l: &Launcher) {
	emit(
		app,
		crate::instances::commands::EVENT_CHANGED,
		l.instances.list().await,
	);
}

fn task(l: &Launcher, id: &str) -> Result<Download, AppError> {
	l.downloads
		.get(id)
		.ok_or_else(|| AppError::unknown("задача пропала"))
}

/// Задача в фоне; после успеха — опознание контента через каталог и `instance-content-changed`.
fn spawn(app: AppHandle, l: Arc<Launcher>, started: install::Started, work: Work) {
	let id = started.instance_id.clone();
	let events: Arc<dyn InstallEvents> = Arc::new(TauriEvents(app.clone()));
	tauri::async_runtime::spawn(async move {
		if install::run(l.clone(), started, work, events).await {
			if let Err(err) = crate::content::commands::identify(&l, &id).await.result {
				log::info!("сборка {id}: контент не опознан: {:?}", err.code);
			}
			crate::gamedata::watch::emit(&app, crate::content::EVENT_CHANGED, &id);
		}
	});
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectProgress {
	request_id: String,
	bytes_done: u64,
	bytes_total: u64,
}

/// Модпак до установки. Из каталога (`projectId`, `versionId?`) архив качается в кэш с событием
/// `modpack-inspect-progress`; с диска (`path`) — копируется в кэш и опознаётся по sha1.
#[tauri::command]
pub async fn modpack_inspect(
	app: AppHandle,
	l: L<'_>,
	request_id: Option<String>,
	source: Option<SourceKind>,
	project_id: Option<String>,
	version_id: Option<String>,
	path: Option<String>,
) -> Result<Preview, AppError> {
	let inspect = match (project_id, path) {
		(Some(project_id), None) => InspectSource::Catalog {
			source: source.unwrap_or_default(),
			project_id,
			version_id,
		},
		(None, Some(path)) => InspectSource::File(PathBuf::from(path)),
		_ => {
			return Err(AppError::coded(
				ErrorCode::InvalidValue,
				"нужен projectId или path",
			));
		}
	};
	let request_id = request_id.unwrap_or_default();
	let (attempt, cancel) = begin_inspect(&l, &request_id);
	let event_id = request_id.clone();
	let progress = Progress::new(move |snap| {
		emit(
			&app,
			EVENT_INSPECT_PROGRESS,
			InspectProgress {
				request_id: event_id.clone(),
				bytes_done: snap.bytes_done,
				bytes_total: snap.bytes_total,
			},
		);
	});
	let result = async {
		let prepared = install::prepare(&l, &inspect, progress, &cancel).await?;
		install::preview(&l, &prepared, &cancel).await
	}
	.await;
	end_inspect(&l, &request_id, attempt);
	if cancel.is_cancelled() {
		return Err(AppError::new(ErrorCode::Cancelled));
	}
	result
}

static INSPECT_ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Новая попытка `modpack_inspect` с этим `requestId`: прошлая с тем же id отменяется.
fn begin_inspect(l: &Launcher, request_id: &str) -> (u64, CancellationToken) {
	let attempt = INSPECT_ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
	let token = CancellationToken::new();
	if let Ok(mut map) = l.pack_inspects.lock()
		&& let Some((_, old)) = map.insert(request_id.to_owned(), (attempt, token.clone()))
	{
		old.cancel();
	}
	(attempt, token)
}

fn end_inspect(l: &Launcher, request_id: &str, attempt: u64) {
	if let Ok(mut map) = l.pack_inspects.lock()
		&& map.get(request_id).is_some_and(|(a, _)| *a == attempt)
	{
		map.remove(request_id);
	}
}

/// Отменяет `modpack_inspect` с этим `requestId`.
#[tauri::command]
pub async fn modpack_inspect_cancel(l: L<'_>, request_id: String) -> Result<(), AppError> {
	let token = l
		.pack_inspects
		.lock()
		.ok()
		.and_then(|map| map.get(&request_id).map(|(_, t)| t.clone()));
	if let Some(token) = token {
		log::info!("чтение модпака отменено");
		token.cancel();
	}
	Ok(())
}

/// Новая сборка из модпака и задача в списке загрузок.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackStarted {
	pub instance: Instance,
	pub download: Download,
}

fn task_info(action: PackAction, link: &super::ModpackLink) -> PackTask {
	PackTask {
		action,
		title: link.title.clone(),
		icon_url: link.icon_url.clone(),
		launcher: None,
	}
}

/// Создаёт сборку из модпака (`token` из `modpack_inspect`) и ставит её в фоне (задача
/// `modpack`). `optional` — пути отмеченных необязательных файлов (нет — все).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn modpack_install(
	app: AppHandle,
	l: L<'_>,
	token: String,
	name: Option<String>,
	group: Option<String>,
	cover: Option<String>,
	optional: Option<Vec<String>>,
) -> Result<PackStarted, AppError> {
	if let Some(name) = &name {
		crate::instances::validate_name(name)?;
	}
	crate::instances::normalize_group(group.as_deref())?;
	let cover = cover.as_deref().map(InstanceCover::parse).transpose()?;
	let mut prepared = install::by_token(&l, &token).await?;
	install::check_requirements(&l, &mut prepared.requirements, true).await?;
	let optional =
		optional.unwrap_or_else(|| prepared.default_optional(&super::PackState::default()));
	let (files, _) = prepared.index.select(Some(&optional));
	super::ensure_space(
		&l.layout.instances_dir(),
		files.iter().map(|f| f.file_size).sum(),
	)?;
	let name = match name {
		Some(name) => name,
		None => install::unique_name(
			&install::instance_name(&prepared.link.title),
			&l.instances.list().await,
		),
	};
	let req = prepared.requirements.clone();
	let instance = l
		.instances
		.create(
			&name,
			&req.game_version,
			req.loader,
			cover,
			group.as_deref(),
		)
		.await?;
	let link = prepared.link.clone();
	l.instances
		.modify(&instance.id, |inst| {
			inst.modpack = Some(link);
			Ok(())
		})
		.await?;
	log::info!(
		"сборка {} из модпака {} {}",
		instance.id,
		prepared.link.title,
		prepared.link.version_number
	);
	let request = PackRequest::Install {
		token,
		optional: optional.clone(),
	};
	let info = task_info(PackAction::Install, &prepared.link);
	let started = install::begin(&l, &instance.id, request, info, true).await?;
	let download = task(&l, &started.task_id)?;
	let work = Work::Install {
		prepared,
		optional,
		repair: false,
	};
	spawn(app.clone(), l.inner().clone(), started, work);
	emit_instances(&app, &l).await;
	let instance = l.instances.get(&instance.id).await?;
	Ok(PackStarted { instance, download })
}

/// Повтор задачи `modpack`/`import`/`export` (`download_retry`). `repair` — у прошлой были
/// предупреждения: докачать (докопировать) только недостающее.
pub async fn retry(
	app: AppHandle,
	l: Arc<Launcher>,
	instance_id: String,
	request: PackRequest,
	repair: bool,
) -> Result<Download, AppError> {
	let instance = l.instances.get(&instance_id).await?;
	let fresh = instance.installed.is_none();
	match request {
		PackRequest::Install { token, optional } => {
			let mut prepared = install::by_token(&l, &token).await?;
			install::check_requirements(&l, &mut prepared.requirements, true).await?;
			let info = task_info(PackAction::Install, &prepared.link);
			let request = PackRequest::Install {
				token,
				optional: optional.clone(),
			};
			let started = install::begin(&l, &instance_id, request, info, fresh).await?;
			let download = task(&l, &started.task_id)?;
			let work = Work::Install {
				prepared,
				optional,
				repair,
			};
			spawn(app, l, started, work);
			Ok(download)
		}
		PackRequest::Update { token, optional } => {
			start_update(app, l, &instance_id, &token, Some(optional), None, repair).await
		}
		PackRequest::Import { item, include } => {
			let candidate = resolve_item(item.clone()).await?;
			let info = import_info(&candidate);
			let request = PackRequest::Import {
				item,
				include: include.clone(),
			};
			let started = install::begin(&l, &instance_id, request, info, fresh).await?;
			let download = task(&l, &started.task_id)?;
			spawn(app, l, started, Work::Import { candidate, include });
			Ok(download)
		}
		PackRequest::Export {
			name,
			version,
			include,
			path,
		} => start_export(l, instance, name, version, include, path).await,
	}
}

/// Новая версия модпака (`modpack_check_update`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModpackUpdate {
	/// Лучшая версия проекта (стабильная, иначе бета, иначе последняя), без списка изменений.
	pub latest: Option<ContentVersion>,
	/// `latest` новее установленной.
	pub available: bool,
	pub current_version_id: Option<String>,
	/// Unix, мс.
	pub checked_at: u64,
}

fn catalog_project(instance: &Instance) -> Result<(SourceKind, String), AppError> {
	match &instance.modpack {
		Some(link) if link.source != PackOrigin::File => link
			.project_id
			.clone()
			.zip(install::source_of(link.source))
			.map(|(p, s)| (s, p))
			.ok_or_else(|| AppError::coded(ErrorCode::ModpackNotLinked, instance.id.clone())),
		_ => Err(AppError::coded(
			ErrorCode::ModpackNotLinked,
			instance.id.clone(),
		)),
	}
}

/// Проверяет новую версию модпака в каталоге (не чаще раза в 10 минут без `force`).
/// Модпак не из каталога — `modpack_not_linked`.
#[tauri::command]
pub async fn modpack_check_update(
	l: L<'_>,
	id: String,
	force: Option<bool>,
) -> Result<ModpackUpdate, AppError> {
	let instance = l.instances.get(&id).await?;
	let (source, project) = catalog_project(&instance)?;
	let current = instance.modpack.as_ref().and_then(|m| m.version_id.clone());
	if !force.unwrap_or(false)
		&& let Some(cached) = l.pack_updates.lock().ok().and_then(|m| {
			m.get(&id)
				.filter(|(t, u)| t.elapsed() < UPDATE_TTL && u.current_version_id == current)
				.map(|(_, u)| u.clone())
		}) {
		return Ok(cached);
	}
	let versions = l
		.sources
		.get(source)
		.project_versions(&project, &VersionFilter::default())
		.await?;
	let mut latest = content::pick_best(&versions).cloned();
	let installed = current
		.as_ref()
		.and_then(|v| versions.iter().find(|x| &x.id == v));
	let available = match (&latest, installed) {
		(Some(latest), Some(installed)) => {
			latest.id != installed.id && latest.published_at > installed.published_at
		}
		(Some(latest), None) => current.as_ref() != Some(&latest.id),
		_ => false,
	};
	if let Some(v) = &mut latest {
		v.changelog = None;
	}
	let update = ModpackUpdate {
		latest,
		available,
		current_version_id: current,
		checked_at: super::now_ms(),
	};
	if let Ok(mut m) = l.pack_updates.lock() {
		m.insert(id, (Instant::now(), update.clone()));
	}
	Ok(update)
}

/// Что изменит обновление до модпака `token` (из `modpack_inspect` — версия каталога или файл).
#[tauri::command]
pub async fn modpack_update_preview(
	l: L<'_>,
	id: String,
	token: String,
	optional: Option<Vec<String>>,
) -> Result<UpdatePreview, AppError> {
	let instance = l.instances.get(&id).await?;
	if instance.modpack.is_none() {
		return Err(AppError::coded(ErrorCode::ModpackNotLinked, id));
	}
	let prepared = install::by_token(&l, &token).await?;
	install::update_preview(&l, &instance, &prepared, optional).await
}

/// Обновляет (или откатывает) модпак сборки до `token` в фоне: задача `modpack`, `mode: "update"`.
/// `backup` — сначала копия сборки «{name} (копия)». Игра запущена — `instance_busy`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn modpack_update(
	app: AppHandle,
	l: L<'_>,
	id: String,
	token: String,
	optional: Option<Vec<String>>,
	backup: Option<bool>,
	lang: Option<Lang>,
) -> Result<Download, AppError> {
	let backup = backup.unwrap_or(false).then(|| lang.unwrap_or_default());
	start_update(app, l.inner().clone(), &id, &token, optional, backup, false).await
}

async fn start_update(
	app: AppHandle,
	l: Arc<Launcher>,
	id: &str,
	token: &str,
	optional: Option<Vec<String>>,
	backup: Option<Lang>,
	repair: bool,
) -> Result<Download, AppError> {
	let instance = l.instances.get(id).await?;
	if instance.modpack.is_none() {
		return Err(AppError::coded(ErrorCode::ModpackNotLinked, id.to_owned()));
	}
	let mut prepared = install::by_token(&l, token).await?;
	install::check_requirements(&l, &mut prepared.requirements, true).await?;
	let optional = match optional {
		Some(o) => o,
		None => {
			let dir = l.layout.instance_dir(id);
			let old = tokio::task::spawn_blocking(move || super::load_state(&dir))
				.await
				.map_err(join_error)??
				.unwrap_or_default();
			prepared.default_optional(&old)
		}
	};
	let backup = match backup {
		Some(lang) => {
			let suffix = match lang {
				Lang::Ru => " (копия)",
				Lang::En => " (copy)",
			};
			let base: String = instance
				.name
				.chars()
				.take(64 - suffix.chars().count())
				.collect();
			Some(install::unique_name(
				&format!("{}{suffix}", base.trim_end()),
				&l.instances.list().await,
			))
		}
		None => None,
	};
	let info = task_info(PackAction::Update, &prepared.link);
	let request = PackRequest::Update {
		token: token.to_owned(),
		optional: optional.clone(),
	};
	let started = install::begin(&l, id, request, info, false).await?;
	let download = task(&l, &started.task_id)?;
	if let Ok(mut m) = l.pack_updates.lock() {
		m.remove(id);
	}
	let work = Work::Update {
		prepared,
		optional,
		backup,
		repair,
	};
	spawn(app, l, started, work);
	Ok(download)
}

/// Отвязывает сборку от модпака: она становится обычной (файлы не трогаются).
#[tauri::command]
pub async fn modpack_unlink(app: AppHandle, l: L<'_>, id: String) -> Result<Instance, AppError> {
	// Под замком списка вместе с записью; задача обновления связь обратно не запишет (`set_link`).
	let instance = l
		.instances
		.modify(&id, |inst| {
			if l.instances.is_installing(&inst.id) {
				return Err(AppError::coded(ErrorCode::InstanceBusy, inst.id.clone()));
			}
			inst.modpack = None;
			Ok(())
		})
		.await?;
	let state = super::state_path(&l.layout.instance_dir(&id));
	if let Err(err) = std::fs::remove_file(&state)
		&& err.kind() != std::io::ErrorKind::NotFound
	{
		log::warn!("{}: {err}", state.display());
	}
	if let Ok(mut m) = l.pack_updates.lock() {
		m.remove(&id);
	}
	emit_instances(&app, &l).await;
	crate::gamedata::watch::emit(&app, crate::content::EVENT_CHANGED, &id);
	Ok(instance)
}

pub(crate) fn dialog_title(lang: Lang, ru: &str, en: &str) -> String {
	match lang {
		Lang::Ru => ru.to_owned(),
		Lang::En => en.to_owned(),
	}
}

pub(crate) fn with_parent<R: tauri::Runtime>(
	app: &AppHandle<R>,
	dialog: tauri_plugin_dialog::FileDialogBuilder<R>,
) -> tauri_plugin_dialog::FileDialogBuilder<R> {
	use tauri::Manager;
	match app.get_webview_window(crate::MAIN_WINDOW) {
		Some(main) => dialog.set_parent(&main),
		None => dialog,
	}
}

fn picked(file: Option<tauri_plugin_dialog::FilePath>) -> Result<Option<String>, AppError> {
	let Some(file) = file else {
		return Ok(None);
	};
	let path = file
		.into_path()
		.map_err(|e| AppError::coded(ErrorCode::FileNotFound, e.to_string()))?;
	Ok(Some(path.to_string_lossy().into_owned()))
}

/// Системное окно выбора файла модпака (`.mrpack`, `.zip`). `null` — закрыли без выбора.
#[tauri::command]
pub async fn modpack_pick_file(
	app: AppHandle,
	lang: Option<Lang>,
) -> Result<Option<String>, AppError> {
	use tauri_plugin_dialog::DialogExt;
	let lang = lang.unwrap_or_default();
	let mut dialog = app
		.dialog()
		.file()
		.set_title(dialog_title(lang, "Выберите модпак", "Choose a modpack"))
		.add_filter(
			dialog_title(
				lang,
				"Модпак (*.mrpack, *.zip)",
				"Modpack (*.mrpack, *.zip)",
			),
			&["mrpack", "zip"],
		);
	if let Some(dir) = dirs::download_dir() {
		dialog = dialog.set_directory(dir);
	}
	let (tx, rx) = tokio::sync::oneshot::channel();
	with_parent(&app, dialog).pick_file(move |file| {
		let _ = tx.send(file);
	});
	picked(rx.await.ok().flatten())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
	source: ImportSource,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
	pub sources: Vec<ImportSource>,
	/// Свободно на диске с папкой данных; `null` — не узнать.
	pub free_bytes: Option<u64>,
}

/// Помечает уже перенесённые (по `Instance.importedFrom`).
fn mark_imported(source: &mut ImportSource, instances: &[Instance]) {
	for c in &mut source.items {
		let from = c.imported_from();
		c.imported = instances
			.iter()
			.any(|i| i.imported_from.as_ref() == Some(&from));
	}
}

/// Загрузчика такой версии (или для этой версии игры) нет в метаданных — `old_loader`.
/// Без сети и кэша метаданных не проверяется.
async fn mark_unavailable(l: &Launcher, source: &mut ImportSource) {
	let cancel = CancellationToken::new();
	for c in &mut source.items {
		let (Some(game), Some(loader), None) = (&c.game_version, &c.loader, &c.unsupported) else {
			continue;
		};
		if loader.kind == crate::meta::LoaderKind::Vanilla {
			continue;
		}
		let Ok(list) = l.meta.loader_versions(loader.kind, game, &cancel).await else {
			continue;
		};
		let known = match &loader.version {
			Some(v) => {
				let long = format!("{game}-{v}");
				list.items.iter().any(|x| &x.id == v || x.id == long)
			}
			None => !list.items.is_empty(),
		};
		if !known {
			c.unsupported = Some(launchers::Unsupported {
				reason: launchers::UnsupportedReason::OldLoader,
				detail: Some(format!(
					"{} {}",
					loader.kind.as_str(),
					loader.version.as_deref().unwrap_or_default()
				)),
			});
		}
	}
}

/// Ищет сборки других лаунчеров (с `paths` — только там). `quick` — без размеров.
#[tauri::command]
pub async fn import_scan(
	app: AppHandle,
	l: L<'_>,
	paths: Option<Vec<String>>,
	quick: Option<bool>,
) -> Result<ScanResult, AppError> {
	let quick = quick.unwrap_or(false);
	let instances = l.instances.list().await;
	let mut sources = Vec::new();
	match paths {
		Some(paths) => {
			for path in paths {
				let dir = PathBuf::from(&path);
				let found = tokio::task::spawn_blocking(move || launchers::scan_path(&dir, quick))
					.await
					.map_err(join_error)?;
				let mut source = match found {
					Ok((launcher, root, items)) => ImportSource {
						id: format!("{launcher:?}:{}", root.display()).to_lowercase(),
						launcher,
						root: root.to_string_lossy().into_owned(),
						error: None,
						items,
					},
					Err(error) => ImportSource {
						id: format!("folder:{path}").to_lowercase(),
						launcher: launchers::LauncherKind::Folder,
						root: path,
						error: Some(error),
						items: Vec::new(),
					},
				};
				mark_imported(&mut source, &instances);
				if !quick {
					mark_unavailable(&l, &mut source).await;
				}
				emit(
					&app,
					EVENT_SCAN_PROGRESS,
					ScanProgress {
						source: source.clone(),
					},
				);
				sources.push(source);
			}
		}
		None => {
			for (launcher, root) in launchers::default_roots() {
				let dir = root.clone();
				let found =
					tokio::task::spawn_blocking(move || launchers::scan(launcher, &dir, quick))
						.await
						.map_err(join_error)?;
				let (items, error) = match found {
					Ok(items) => (items, None),
					Err(err) => (Vec::new(), Some(err)),
				};
				if items.is_empty() && error.is_none() {
					continue;
				}
				let mut source = ImportSource {
					id: format!("{launcher:?}:{}", root.display()).to_lowercase(),
					launcher,
					root: root.to_string_lossy().into_owned(),
					error,
					items,
				};
				mark_imported(&mut source, &instances);
				if !quick {
					mark_unavailable(&l, &mut source).await;
				}
				emit(
					&app,
					EVENT_SCAN_PROGRESS,
					ScanProgress {
						source: source.clone(),
					},
				);
				sources.push(source);
			}
		}
	}
	Ok(ScanResult {
		sources,
		free_bytes: super::free_space(l.layout.root()),
	})
}

/// Системное окно выбора папки (лаунчера или сборки). `null` — закрыли без выбора.
#[tauri::command]
pub async fn import_pick_folder(
	app: AppHandle,
	lang: Option<Lang>,
) -> Result<Option<String>, AppError> {
	use tauri_plugin_dialog::DialogExt;
	let lang = lang.unwrap_or_default();
	let dialog = app.dialog().file().set_title(dialog_title(
		lang,
		"Выберите папку сборки или лаунчера",
		"Choose an instance or launcher folder",
	));
	let (tx, rx) = tokio::sync::oneshot::channel();
	with_parent(&app, dialog).pick_folder(move |dir| {
		let _ = tx.send(dir);
	});
	picked(rx.await.ok().flatten())
}

fn import_info(candidate: &launchers::Candidate) -> PackTask {
	PackTask {
		action: PackAction::Import,
		title: candidate.name.clone(),
		icon_url: candidate.icon.clone(),
		launcher: Some(candidate.launcher),
	}
}

async fn resolve_item(item: ImportItem) -> Result<launchers::Candidate, AppError> {
	tokio::task::spawn_blocking(move || {
		let mut c = launchers::resolve(&item)?;
		c.measure();
		Ok(c)
	})
	.await
	.map_err(join_error)?
}

/// Перенос: новая сборка и задача `import` на каждую. Всё проверяется до начала.
#[tauri::command]
pub async fn import_run(
	app: AppHandle,
	l: L<'_>,
	items: Vec<ImportItem>,
	include: Vec<Category>,
	group: Option<String>,
) -> Result<Vec<Download>, AppError> {
	crate::instances::normalize_group(group.as_deref())?;
	let mut planned = Vec::new();
	let mut need = 0u64;
	for item in items {
		let mut candidate = resolve_item(item.clone()).await?;
		if let Some(problem) = &candidate.unsupported {
			return Err(AppError::coded(
				ErrorCode::InvalidValue,
				format!("{:?}: {}", problem.reason, candidate.path),
			));
		}
		let game_version = match candidate.game_version.clone() {
			Some(v) => v,
			// «Последний выпуск» официального лаунчера.
			None if candidate.profile.is_some() => {
				l.meta
					.game_versions(None, false, &CancellationToken::new())
					.await?
					.latest
					.release
			}
			None => {
				return Err(AppError::coded(
					ErrorCode::VersionNotFound,
					candidate.path.clone(),
				));
			}
		};
		let mut req = super::Requirements {
			game_version,
			loader: candidate
				.loader
				.clone()
				.unwrap_or(crate::instances::Loader {
					kind: crate::meta::LoaderKind::Vanilla,
					version: None,
				}),
		};
		install::check_requirements(&l, &mut req, false)
			.await
			.map_err(|err| {
				if err.code == ErrorCode::LoaderNotFound {
					AppError::coded(
						ErrorCode::InvalidValue,
						format!("OldLoader: {}", candidate.path),
					)
				} else {
					err
				}
			})?;
		candidate.game_version = Some(req.game_version.clone());
		need += include
			.iter()
			.map(|c| candidate.sizes.get(c).copied().unwrap_or(0))
			.sum::<u64>();
		planned.push((item, candidate, req));
	}
	super::ensure_space(&l.layout.instances_dir(), need)?;

	let mut downloads = Vec::new();
	for (item, candidate, req) in planned {
		let name = install::unique_name(
			&install::instance_name(&candidate.name),
			&l.instances.list().await,
		);
		let instance = l
			.instances
			.create(&name, &req.game_version, req.loader, None, group.as_deref())
			.await?;
		let from = candidate.imported_from();
		let (memory, jvm_args, modpack) = (
			candidate.memory,
			candidate.jvm_args.clone(),
			candidate.modpack.clone(),
		);
		l.instances
			.modify(&instance.id, |inst| {
				inst.imported_from = Some(from);
				inst.memory = memory;
				inst.jvm_args = jvm_args;
				inst.modpack = modpack;
				Ok(())
			})
			.await?;
		log::info!(
			"перенос {:?} «{}» → сборка {}",
			candidate.launcher,
			candidate.name,
			instance.id
		);
		let info = import_info(&candidate);
		let request = PackRequest::Import {
			item,
			include: include.clone(),
		};
		let started = install::begin(&l, &instance.id, request, info, true).await?;
		downloads.push(task(&l, &started.task_id)?);
		let work = Work::Import {
			candidate,
			include: include.clone(),
		};
		spawn(app.clone(), l.inner().clone(), started, work);
	}
	emit_instances(&app, &l).await;
	Ok(downloads)
}

/// Что войдёт в `.mrpack`: категории, моды ссылками и файлами.
#[tauri::command]
pub async fn instance_export_preview(l: L<'_>, id: String) -> Result<ExportPreview, AppError> {
	let instance = l.instances.get(&id).await?;
	export::preview(&l, &instance).await
}

/// Системное окно «Сохранить как» для `.mrpack` (папка «Загрузки»). `null` — закрыли.
#[tauri::command]
pub async fn instance_export_pick_path(
	app: AppHandle,
	l: L<'_>,
	id: String,
	lang: Option<Lang>,
	file_name: Option<String>,
) -> Result<Option<String>, AppError> {
	use tauri_plugin_dialog::DialogExt;
	let instance = l.instances.get(&id).await?;
	let lang = lang.unwrap_or_default();
	let name: String = file_name
		.unwrap_or_else(|| format!("{}.mrpack", instance.name))
		.chars()
		.map(|c| {
			if r#"/\:*?"<>|"#.contains(c) || c.is_control() {
				'_'
			} else {
				c
			}
		})
		.collect();
	let mut dialog = app
		.dialog()
		.file()
		.set_title(dialog_title(lang, "Сохранить модпак", "Save modpack"))
		.add_filter("Modrinth (*.mrpack)", &["mrpack"])
		.set_file_name(name);
	if let Some(dir) = dirs::download_dir() {
		dialog = dialog.set_directory(dir);
	}
	let (tx, rx) = tokio::sync::oneshot::channel();
	with_parent(&app, dialog).save_file(move |file| {
		let _ = tx.send(file);
	});
	let Some(path) = picked(rx.await.ok().flatten())? else {
		return Ok(None);
	};
	let mut path = PathBuf::from(path);
	// Если расширение дописали мы, это уже другой файл: замену его никто не подтверждал.
	let mut confirmed = true;
	if path
		.extension()
		.is_none_or(|e| !e.eq_ignore_ascii_case("mrpack"))
	{
		path.set_extension("mrpack");
		confirmed = !path.exists();
	}
	if let Ok(mut paths) = l.export_paths.lock() {
		paths.insert(path.clone(), confirmed);
	}
	Ok(Some(path.to_string_lossy().into_owned()))
}

/// Экспорт сборки в `.mrpack` по пути из `instance_export_pick_path`: задача `export`
/// (`packing`, по готовности — `outputPath`). Во время игры можно.
#[tauri::command]
pub async fn instance_export(
	l: L<'_>,
	id: String,
	name: String,
	version: String,
	include: Vec<Category>,
	path: String,
) -> Result<Download, AppError> {
	let instance = l.instances.get(&id).await?;
	start_export(
		l.inner().clone(),
		instance,
		name,
		version,
		include,
		PathBuf::from(path),
	)
	.await
}

async fn start_export(
	l: Arc<Launcher>,
	instance: Instance,
	name: String,
	version: String,
	include: Vec<Category>,
	path: PathBuf,
) -> Result<Download, AppError> {
	let name = name.trim().to_owned();
	let version = version.trim().to_owned();
	if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
		return Err(AppError::new(ErrorCode::InvalidName));
	}
	if version.is_empty() || version.chars().count() > 32 || version.chars().any(char::is_control) {
		return Err(AppError::coded(ErrorCode::InvalidValue, "version"));
	}
	let bad_path = || AppError::coded(ErrorCode::InvalidValue, format!("path: {}", path.display()));
	if !path.is_absolute() || path.parent().is_none_or(|p| !p.is_dir()) {
		return Err(bad_path());
	}
	if path
		.extension()
		.is_none_or(|e| !e.eq_ignore_ascii_case("mrpack"))
	{
		return Err(bad_path());
	}
	// Только путь из `instance_export_pick_path`.
	let may_replace = l
		.export_paths
		.lock()
		.ok()
		.and_then(|paths| paths.get(&path).copied());
	match may_replace {
		None => return Err(bad_path()),
		Some(false) if path.exists() => return Err(bad_path()),
		_ => {}
	}
	if instance.install_state != crate::instances::InstallState::Installed {
		return Err(AppError::coded(ErrorCode::NotInstalled, instance.id));
	}
	let id = instance.id.clone();
	let cancel = CancellationToken::new();
	let info = PackTask {
		action: PackAction::Export,
		title: format!("{name} {version}"),
		icon_url: None,
		launcher: None,
	};
	let request = PackRequest::Export {
		name: name.clone(),
		version: version.clone(),
		include: include.clone(),
		path: path.clone(),
	};
	let task_id = {
		// Удаление сборки ждёт тот же замок и видит задачу экспорта.
		let _content = l.content_locks.lock(&id).await;
		l.instances.get(&id).await?;
		l.downloads.begin(
			Target::Pack {
				instance_id: id.clone(),
				request,
				info,
			},
			cancel.clone(),
		)
	};
	let download = task(&l, &task_id)?;
	tauri::async_runtime::spawn(async move {
		let downloads = l.downloads.clone();
		let tracked = task_id.clone();
		let progress = Progress::new(move |snap| {
			let percent = (snap.files_total > 0)
				.then(|| snap.files_done as f64 * 100.0 / snap.files_total as f64);
			downloads.progress(&tracked, Some(InstallStage::Packing), percent, snap);
		});
		progress.flush();
		let result = export::export(
			&l, &instance, &name, &version, &include, &path, progress, &cancel,
		)
		.await;
		match &result {
			Ok(()) => {
				log::info!("сборка {id} экспортирована");
				let out = path.to_string_lossy().into_owned();
				l.downloads
					.annotate(&task_id, |t| t.output_path = Some(out));
			}
			Err(err) => log::warn!("экспорт {id}: {:?} {:?}", err.code, err.details),
		}
		l.downloads.finish(&task_id, result);
	});
	Ok(download)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::paths::Layout;

	#[tokio::test]
	async fn inspect_with_same_request_cancels_previous() {
		let dir = tempfile::tempdir().unwrap();
		let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
		let (first, a) = begin_inspect(&l, "dialog");
		let (second, b) = begin_inspect(&l, "dialog");
		assert!(a.is_cancelled() && !b.is_cancelled());
		// Поздний конец первой попытки не убирает вторую.
		end_inspect(&l, "dialog", first);
		assert!(l.pack_inspects.lock().unwrap().contains_key("dialog"));
		let (_, other) = begin_inspect(&l, "other");
		assert!(!other.is_cancelled());
		end_inspect(&l, "dialog", second);
		assert!(!l.pack_inspects.lock().unwrap().contains_key("dialog"));
	}
}
