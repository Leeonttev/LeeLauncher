//! Команды Tauri каталога (`catalog_*`) и контента сборок (`content_install`, `instance_content*`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};
use tokio_util::sync::CancellationToken;

use super::install::{
	self, ContentAction, ContentRequest, ContentTask, InstallPlan, InstallRequest,
	InstalledElsewhere, PlannedItem, ProjectRef,
};
use super::installed::{self, ContentItem, Detected, FileRef, Index, UpdateInfo};
use super::manual::ManualFile;
use super::{
	AllInfo, AlsoOn, CategoryTag, ContentSource, ContentVersion, EVENT_CHANGED, FileKey,
	InstallState, Project, ProjectHit, ProjectType, SearchPage, SearchParams, SearchScope,
	SourceFailure, SourceInfo, SourceKind, VersionChannel, VersionFilter,
};
use crate::download::Progress;
use crate::downloads::{Download, Target};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::gamedata::watch;
use crate::instances::Instance;
use crate::launcher::Launcher;

type L<'a> = State<'a, Arc<Launcher>>;

/// Обновления проверяются не чаще этого (без `force`).
const UPDATES_TTL: Duration = Duration::from_secs(10 * 60);
/// План установки: проект, версии и дерево зависимостей.
const RESOLVE_DEADLINE: Duration = Duration::from_secs(30);

fn emit_changed(app: &AppHandle, instance_id: &str) {
	watch::emit(app, EVENT_CHANGED, instance_id);
}

fn join_error(err: tokio::task::JoinError) -> AppError {
	AppError::unknown(err.to_string())
}

fn is_offline_error(err: &AppError) -> bool {
	matches!(
		err.code,
		ErrorCode::Network | ErrorCode::SourceUnavailable | ErrorCode::RateLimited
	)
}

/// Под замком контента: иначе запись после удаления заново создаст `instances/<id>/`.
pub(crate) fn ensure_exists(l: &Launcher, instance_id: &str) -> Result<(), AppError> {
	if l.layout
		.instance_dir(instance_id)
		.join(crate::instances::FILE_NAME)
		.is_file()
	{
		Ok(())
	} else {
		Err(AppError::coded(
			ErrorCode::InstanceNotFound,
			instance_id.to_owned(),
		))
	}
}

/// Сверяет индекс с папками и сохраняет, если изменился. Вызывать под замком контента сборки.
pub(crate) async fn sync_index(
	l: &Launcher,
	instance_id: &str,
) -> Result<(Index, Vec<ContentItem>), AppError> {
	ensure_exists(l, instance_id)?;
	let game_dir = l.layout.game_dir(instance_id);
	let path = installed::index_path(&l.layout.instance_dir(instance_id));
	let instance_dir = l.layout.instance_dir(instance_id);
	tokio::task::spawn_blocking(move || {
		let mut index = installed::load_index(&path)?;
		let (mut list, changed) = installed::sync(&game_dir, &mut index);
		if changed {
			installed::save_index(&path, &index)?;
		}
		crate::modpacks::mark_from_modpack(&instance_dir, &mut list);
		Ok((index, list))
	})
	.await
	.map_err(join_error)?
}

async fn synced(l: &Launcher, instance_id: &str) -> Result<(Index, Vec<ContentItem>), AppError> {
	let _guard = l.content_locks.lock(instance_id).await;
	sync_index(l, instance_id).await
}

/// Состояние проекта относительно сборки (по установленному и спискам версий/загрузчиков проекта).
fn install_state(
	hit: &ProjectHit,
	instance: &Instance,
	items: &[ContentItem],
	worlds: &super::worlds::WorldsState,
) -> InstallState {
	if hit.project_type == ProjectType::World {
		if let Some((folder, w)) = worlds
			.worlds
			.iter()
			.find(|(_, w)| w.source == hit.source && w.project_id == hit.id)
		{
			return InstallState::Installed {
				version_id: w.version_id.clone(),
				file: folder.clone(),
				enabled: true,
			};
		}
		return match super::compatibility(instance, hit.project_type, &hit.game_versions, &[]) {
			Some(reason) => InstallState::Incompatible { reason },
			None => InstallState::None,
		};
	}
	let found = items.iter().find(|i| {
		i.project
			.as_ref()
			.is_some_and(|p| p.source == hit.source && p.id == hit.id)
	});
	if let Some(item) = found
		&& let Some(project) = &item.project
	{
		return match &item.update {
			Some(update) => InstallState::Update {
				version_id: project.version_id.clone(),
				latest_version_id: update.version_id.clone(),
				latest_number: update.number.clone(),
			},
			None => InstallState::Installed {
				version_id: project.version_id.clone(),
				file: item.file.clone(),
				enabled: item.enabled,
			},
		};
	}
	match super::compatibility(instance, hit.project_type, &hit.game_versions, &hit.loaders) {
		Some(reason) => InstallState::Incompatible { reason },
		None => InstallState::None,
	}
}

struct Context {
	instance: Instance,
	items: Vec<ContentItem>,
	worlds: super::worlds::WorldsState,
}

impl Context {
	fn state(&self, hit: &ProjectHit) -> InstallState {
		install_state(hit, &self.instance, &self.items, &self.worlds)
	}
}

async fn instance_items(
	l: &Launcher,
	instance_id: Option<&str>,
) -> Result<Option<Context>, AppError> {
	let Some(id) = instance_id else {
		return Ok(None);
	};
	let instance = l.instances.get(id).await?;
	let (_, items) = synced(l, id).await?;
	let worlds = super::worlds::load(&l.layout.instance_dir(id));
	Ok(Some(Context {
		instance,
		items,
		worlds,
	}))
}

/// Каталоги и что они умеют (без сети).
#[tauri::command]
pub async fn catalog_sources(l: L<'_>) -> Result<Vec<SourceInfo>, AppError> {
	Ok(l.sources.info())
}

/// Режим поиска «Все»: участвующие каталоги, типы, сортировки (без сети).
#[tauri::command]
pub async fn catalog_all_info(l: L<'_>) -> Result<AllInfo, AppError> {
	Ok(l.sources.all_info())
}

/// Поиск в каталоге или во всех сразу (`source: "all"`). С `instanceId` у каждого результата
/// (и у каждого `alsoOn`) есть `install`.
#[tauri::command]
pub async fn catalog_search(l: L<'_>, params: SearchParams) -> Result<SearchPage, AppError> {
	let SearchParams {
		source,
		cursor,
		mut query,
	} = params;
	let single = match source {
		SearchScope::Modrinth => Some(SourceKind::Modrinth),
		SearchScope::Curseforge => Some(SourceKind::Curseforge),
		SearchScope::All => None,
	};
	if let Some(kind) = single {
		query.source = kind;
		let context = instance_items(&l, query.instance_id.as_deref()).await?;
		let mut page =
			super::within(super::UI_DEADLINE, l.sources.get(kind).search(&query)).await?;
		if let Some(ctx) = context {
			for hit in &mut page.hits {
				hit.install = Some(ctx.state(hit));
			}
		}
		return Ok(page);
	}
	super::search_all::check_query(&query)?;
	let sources = l.sources.for_all(query.project_type);
	if sources.is_empty() {
		return Err(AppError::coded(
			ErrorCode::InvalidValue,
			format!("type: {}", query.project_type.as_str()),
		));
	}
	let context = instance_items(&l, query.instance_id.as_deref()).await?;
	let mut page = super::within(
		super::UI_DEADLINE,
		super::search_all::search(sources, &query, cursor.as_deref()),
	)
	.await?;
	let hits = std::mem::take(&mut page.rows)
		.into_iter()
		.map(|row| {
			let mut hit = row.hit;
			hit.also_on = row
				.twins
				.iter()
				.map(|twin| {
					let mut also = AlsoOn::of(twin);
					also.install = context.as_ref().map(|ctx| ctx.state(twin));
					also
				})
				.collect();
			if let Some(ctx) = &context {
				hit.install = Some(ctx.state(&hit));
			}
			hit
		})
		.collect();
	Ok(page.into_search_page(hits))
}

/// Категории для фильтра этого типа (кэш на сутки, без сети — сохранённые).
#[tauri::command]
pub async fn catalog_categories(
	l: L<'_>,
	source: Option<SourceKind>,
	project_type: ProjectType,
) -> Result<Vec<CategoryTag>, AppError> {
	let source = l.sources.get(source.unwrap_or_default());
	let (all, _) = super::within(super::UI_DEADLINE, source.categories()).await?;
	let of = |kind: &str| -> Vec<CategoryTag> {
		all.iter()
			.filter(|c| c.project_type == kind)
			.cloned()
			.collect()
	};
	let list = of(project_type.as_str());
	// У датапаков в каталоге свои категории могут не прийти — тогда категории модов.
	Ok(
		if list.is_empty() && project_type == ProjectType::Datapack {
			of("mod")
		} else {
			list
		},
	)
}

/// Проект. С `instanceId` — `install` и `bestVersion` (что поставится в сборку).
#[tauri::command]
pub async fn catalog_project(
	l: L<'_>,
	source: Option<SourceKind>,
	id: String,
	instance_id: Option<String>,
	project_type: Option<ProjectType>,
) -> Result<Project, AppError> {
	super::check_ref(&id)?;
	let started = tokio::time::Instant::now();
	let source = l.sources.get(source.unwrap_or_default());
	let mut project = super::within(super::UI_DEADLINE, async {
		match project_type {
			Some(kind) => source.project_by_slug(&id, kind).await,
			None => source.project(&id).await,
		}
	})
	.await?;
	if let Some(kind) = project_type.filter(|k| *k != ProjectType::Modpack) {
		project.hit.project_type = kind;
	}
	if let Some(ctx) = instance_items(&l, instance_id.as_deref()).await? {
		let state = ctx.state(&project.hit);
		let instance = &ctx.instance;
		if !matches!(state, InstallState::Incompatible { .. }) {
			let filter = VersionFilter::for_instance(instance, project.hit.project_type);
			let left = super::UI_DEADLINE.saturating_sub(started.elapsed());
			match super::within(left, source.project_versions(&project.hit.id, &filter)).await {
				Ok(versions) => project.best_version = super::pick_best(&versions).cloned(),
				Err(err) => log::warn!("версии {}: {:?}", project.hit.id, err.code),
			}
		}
		project.hit.install = Some(state);
	}
	if let Some(v) = &mut project.best_version {
		v.changelog = None;
	}
	Ok(project)
}

/// Версии проекта, новые первыми, без списка изменений. С `instanceId` — `installed` у версии,
/// которая стоит в сборке.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn catalog_versions(
	l: L<'_>,
	source: Option<SourceKind>,
	project_id: String,
	game_version: Option<String>,
	loader: Option<String>,
	channel: Option<VersionChannel>,
	instance_id: Option<String>,
) -> Result<Vec<ContentVersion>, AppError> {
	super::check_ref(&project_id)?;
	let filter = VersionFilter {
		game_versions: game_version.into_iter().collect(),
		loaders: loader.into_iter().collect(),
	};
	let source = source.unwrap_or_default();
	let catalog = l.sources.get(source);
	let mut versions = super::within(
		super::UI_DEADLINE,
		catalog.project_versions(&project_id, &filter),
	)
	.await?;
	if let Some(channel) = channel {
		versions.retain(|v| v.channel == channel);
	}
	let installed: HashSet<String> = match instance_items(&l, instance_id.as_deref()).await? {
		Some(ctx) => ctx
			.items
			.iter()
			.filter_map(|i| i.project.as_ref())
			.filter(|p| p.source == source)
			.map(|p| p.version_id.clone())
			.chain(
				ctx.worlds
					.worlds
					.values()
					.filter(|w| w.source == source)
					.map(|w| w.version_id.clone()),
			)
			.collect(),
		None => HashSet::new(),
	};
	for v in &mut versions {
		v.changelog = None;
		if instance_id.is_some() {
			v.installed = Some(installed.contains(&v.id));
		}
	}
	Ok(versions)
}

/// Версия со списком изменений.
#[tauri::command]
pub async fn catalog_version(
	l: L<'_>,
	source: Option<SourceKind>,
	version_id: String,
) -> Result<ContentVersion, AppError> {
	super::check_ref(&version_id)?;
	let source = l.sources.get(source.unwrap_or_default());
	super::within(super::UI_DEADLINE, source.version(&version_id)).await
}

/// Что покажет диалог установки.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveView {
	pub version: ContentVersion,
	pub project: ProjectHit,
	/// Поставятся вместе (обязательные зависимости, загрузчик шейдеров).
	pub required: Vec<ProjectHit>,
	/// Можно выбрать (`optional` в `content_install`).
	pub optional: Vec<ProjectHit>,
	/// Обязательные зависимости без подходящей версии.
	pub missing: Vec<ProjectRef>,
	/// Установленное, что несовместимо и будет выключено.
	pub conflicts: Vec<ContentItem>,
	pub already_installed: bool,
	/// Файлы «только с сайта» среди проекта и его зависимостей.
	pub manual: Vec<ManualFile>,
	/// Зависимости, которые уже стоят из другого каталога: не скачиваются.
	pub installed_elsewhere: Vec<InstalledElsewhere>,
}

async fn plan(
	l: &Launcher,
	instance: &Instance,
	request: &InstallRequest,
) -> Result<(InstallPlan, Vec<ContentItem>), AppError> {
	let (index, items) = synced(l, &instance.id).await?;
	let game_dir = l.layout.game_dir(&instance.id);
	let mod_ids = {
		let (dir, idx) = (game_dir.clone(), index.clone());
		tokio::task::spawn_blocking(move || install::local_mod_ids(&dir, &idx))
			.await
			.map_err(join_error)?
	};
	let source = l.sources.get(request.source);
	let plan = super::within(
		RESOLVE_DEADLINE,
		install::resolve(
			source.as_ref(),
			instance,
			&game_dir,
			&index,
			&mod_ids,
			request,
		),
	)
	.await?;
	Ok((plan, items))
}

/// План установки: версия, зависимости, конфликты (без скачивания).
#[tauri::command]
pub async fn catalog_resolve(
	l: L<'_>,
	instance_id: String,
	request: InstallRequest,
) -> Result<ResolveView, AppError> {
	let instance = l.instances.get(&instance_id).await?;
	let (plan, items) = plan(&l, &instance, &request).await?;
	let mut version = plan.version.clone();
	version.changelog = None;
	Ok(ResolveView {
		version,
		project: plan.target.hit.clone(),
		required: plan.required.iter().map(|i| i.hit.clone()).collect(),
		optional: plan.optional.clone(),
		missing: plan.missing.clone(),
		conflicts: items
			.into_iter()
			.filter(|i| plan.conflicts.contains(&i.key))
			.collect(),
		already_installed: plan.already_installed,
		manual: plan.manual().iter().map(ManualFile::from_planned).collect(),
		installed_elsewhere: plan.elsewhere.clone(),
	})
}

/// Задача на то, что можно скачать, и файлы «только с сайта».
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
	pub download: Option<Download>,
	pub manual: Vec<ManualFile>,
	/// Каталоги, которые не ответили (только у `instance_content_update`): их файлы не обновлены.
	pub failed_sources: Vec<SourceFailure>,
}

/// Ставит проект с зависимостями в фоне. Файлы «только с сайта» попадают в ждущие после задачи;
/// если скачивать нечего, их передаёт фронт в `instance_manual_begin`.
#[tauri::command]
pub async fn content_install(
	app: AppHandle,
	l: L<'_>,
	instance_id: String,
	request: InstallRequest,
) -> Result<Started, AppError> {
	start_install(app, l.inner().clone(), instance_id, request).await
}

async fn start_install(
	app: AppHandle,
	l: Arc<Launcher>,
	instance_id: String,
	request: InstallRequest,
) -> Result<Started, AppError> {
	let instance = l.instances.get(&instance_id).await?;
	let (plan, _) = plan(&l, &instance, &request).await?;
	let items = plan.items();
	let manual: Vec<ManualFile> = plan.manual().iter().map(ManualFile::from_planned).collect();
	if items.is_empty() {
		if !manual.is_empty() {
			return Ok(Started {
				download: None,
				manual,
				failed_sources: Vec::new(),
			});
		}
		return Err(AppError::coded(
			ErrorCode::AlreadyInstalled,
			plan.target.file.file_name.clone(),
		));
	}
	let info = ContentTask {
		action: ContentAction::Install,
		source: request.source,
		project_id: Some(plan.target.hit.id.clone()),
		project_title: Some(plan.target.hit.title.clone()),
		icon_url: plan.target.hit.icon_url.clone(),
		extra_count: items.len().saturating_sub(1) as u32,
	};
	let download = start_task(
		app,
		l,
		instance_id,
		ContentRequest::Install(request),
		info,
		items,
		plan.conflicts,
		manual.clone(),
	)
	.await?;
	Ok(Started {
		download: Some(download),
		manual,
		failed_sources: Vec::new(),
	})
}

/// Пока идёт задача модпака, контент руками не трогаем. Вызывать под замком контента сборки.
fn ensure_no_pack(l: &Launcher, id: &str) -> Result<(), AppError> {
	if l.downloads.has_active_pack(id) {
		return Err(AppError::coded(ErrorCode::InstanceBusy, id.to_owned()));
	}
	Ok(())
}

/// Регистрирует задачу контента под замком сборки и запускает в фоне.
#[allow(clippy::too_many_arguments)]
async fn start_task(
	app: AppHandle,
	l: Arc<Launcher>,
	instance_id: String,
	request: ContentRequest,
	info: ContentTask,
	items: Vec<PlannedItem>,
	conflicts: Vec<String>,
	manual: Vec<ManualFile>,
) -> Result<Download, AppError> {
	let _guard = l.content_locks.lock(&instance_id).await;
	ensure_exists(&l, &instance_id)?;
	ensure_no_pack(&l, &instance_id)?;
	if l.instances.is_running(&instance_id)
		&& (!conflicts.is_empty() || items.iter().any(|i| i.replaces.is_some()))
	{
		return Err(AppError::coded(ErrorCode::FileInUse, instance_id));
	}
	let cancel = CancellationToken::new();
	let task_id = l.downloads.begin(
		Target::Content {
			instance_id: instance_id.clone(),
			request,
			info,
		},
		cancel.clone(),
	);
	let downloads = l.downloads.clone();
	let tracked = task_id.clone();
	let progress = Progress::new(move |snap| downloads.progress(&tracked, None, None, snap));
	let id = task_id.clone();
	let task = l.clone();
	tauri::async_runtime::spawn(async move {
		let result = run_task(
			&task,
			&instance_id,
			&items,
			&conflicts,
			progress.clone(),
			&cancel,
		)
		.await;
		if result.is_ok() {
			progress.flush();
			if !manual.is_empty() {
				save_manual(&task, &instance_id, &id, &manual).await;
			}
		}
		match &result {
			Ok(()) => log::info!("контент {instance_id}: поставлено файлов: {}", items.len()),
			Err(err) => log::warn!("контент {instance_id}: {:?} {:?}", err.code, err.details),
		}
		task.downloads.finish(&id, result);
		emit_changed(&app, &instance_id);
	});
	l.downloads
		.get(&task_id)
		.ok_or_else(|| AppError::unknown("задача пропала"))
}

/// Файлы «только с сайта» — в ждущие сборки и в предупреждение задачи.
async fn save_manual(l: &Launcher, instance_id: &str, task_id: &str, manual: &[ManualFile]) {
	let _guard = l.content_locks.lock(instance_id).await;
	if ensure_exists(l, instance_id).is_err() {
		return;
	}
	let dir = l.layout.instance_dir(instance_id);
	let files = manual.to_vec();
	match tokio::task::spawn_blocking(move || super::manual::add_waiting(&dir, &files)).await {
		Ok(Ok(_)) => {
			let manual = manual.to_vec();
			l.downloads.annotate(task_id, |t| {
				t.warnings = Some(crate::downloads::Warnings {
					failed: Vec::new(),
					manual,
				});
			});
		}
		Ok(Err(err)) => log::warn!("ждущие файлы {instance_id}: {:?}", err.code),
		Err(err) => log::warn!("ждущие файлы {instance_id}: {err}"),
	}
}

async fn run_task(
	l: &Launcher,
	instance_id: &str,
	items: &[PlannedItem],
	conflicts: &[String],
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<(), AppError> {
	let instance_dir = l.layout.instance_dir(instance_id);
	let staging = install::download(&l.dl, &instance_dir, items, progress, cancel).await?;
	let result = async {
		// Сборку могли удалить, пока качалось.
		l.instances.get(instance_id).await?;
		let _guard = l.content_locks.lock(instance_id).await;
		let (mut index, _) = sync_index(l, instance_id).await?;
		let game_dir = l.layout.game_dir(instance_id);
		let items = items.to_vec();
		let conflicts = conflicts.to_vec();
		let path = installed::index_path(&instance_dir);
		let stage = staging.clone();
		let dir = instance_dir.clone();
		tokio::task::spawn_blocking(move || {
			let applied = install::apply(&game_dir, &stage, &mut index, &items, &conflicts);
			// Даже если что-то не перенеслось, записываем то, что уже на месте.
			installed::save_index(&path, &index)?;
			let applied = applied?;
			if !applied.worlds.is_empty() {
				let mut state = super::worlds::load(&dir);
				for (folder, item) in &applied.worlds {
					state.worlds.insert(
						folder.clone(),
						super::worlds::WorldSource {
							source: item.hit.source,
							project_id: item.hit.id.clone(),
							version_id: item.version_id.clone(),
						},
					);
				}
				super::worlds::save(&dir, &state)?;
			}
			Ok(())
		})
		.await
		.map_err(join_error)?
	}
	.await;
	let _ = std::fs::remove_dir_all(&staging);
	result
}

/// Повтор задачи `content` из списка загрузок (план строится заново).
pub async fn retry(
	app: AppHandle,
	l: Arc<Launcher>,
	instance_id: String,
	request: ContentRequest,
) -> Result<Option<Download>, AppError> {
	match request {
		ContentRequest::Install(request) => {
			Ok(start_install(app, l, instance_id, request).await?.download)
		}
		ContentRequest::Update(keys) => Ok(start_update(app, l, instance_id, keys).await?.download),
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentList {
	pub items: Vec<ContentItem>,
	/// Последнее обращение к каталогам (определение файлов, проверка обновлений) не удалось ни у одного.
	pub offline: bool,
	/// Каталоги, которые не ответили в последний раз, хотя другие ответили.
	pub failed_sources: Vec<SourceFailure>,
}

/// Контент сборки без сети. Неизвестные файлы опознаются в фоне.
#[tauri::command]
pub async fn instance_content(
	app: AppHandle,
	l: L<'_>,
	id: String,
) -> Result<ContentList, AppError> {
	l.instances.get(&id).await?;
	let (index, items) = synced(&l, &id).await?;
	reconcile_manual(&l, &id, &index).await;
	if needs_identify(&index) {
		spawn_identify(app, l.inner().clone(), id.clone());
	}
	Ok(ContentList {
		items,
		offline: l.content_locks.is_offline(&id),
		failed_sources: l.content_locks.failed(&id),
	})
}

/// Ждущие файлы, которые уже лежат в сборке (положили руками), убираются из списка.
async fn reconcile_manual(l: &Launcher, id: &str, index: &Index) {
	let dir = l.layout.instance_dir(id);
	if !super::manual::state_path(&dir).is_file() {
		return;
	}
	let present: HashSet<String> = index.files.values().map(|e| e.sha1.clone()).collect();
	let _guard = l.content_locks.lock(id).await;
	let d = dir.clone();
	match tokio::task::spawn_blocking(move || super::manual::reconcile(&d, &present)).await {
		Ok(Ok(true)) if super::manual::load(&dir).is_empty() => l.downloads.clear_manual(id),
		Ok(Err(err)) => log::warn!("ждущие файлы {id}: {:?}", err.code),
		_ => {}
	}
}

fn needs_identify(index: &Index) -> bool {
	!installed::unidentified(index, installed::now_secs()).is_empty()
		|| index
			.files
			.values()
			.any(|e| e.source.as_ref().is_some_and(incomplete))
}

/// Записи не хватает данных проекта: названия, автора или slug (старые обновления писали в slug id).
fn incomplete(s: &installed::SourceRef) -> bool {
	s.title.is_none() || s.author.is_none() || s.slug.as_deref() == Some(s.project_id.as_str())
}

fn spawn_identify(app: AppHandle, l: Arc<Launcher>, instance_id: String) {
	if !l.content_locks.begin_identify(&instance_id) {
		return;
	}
	tauri::async_runtime::spawn(async move {
		let done = identify(&l, &instance_id).await;
		l.content_locks.end_identify(&instance_id);
		let changed = match done.result {
			Ok(changed) => changed,
			Err(err) => {
				log::warn!(
					"контент {instance_id}: определение файлов не удалось: {:?} {}",
					err.code,
					err.details.unwrap_or_default()
				);
				false
			}
		};
		if changed || done.state_changed {
			emit_changed(&app, &instance_id);
		}
	});
}

/// Ответы каталогов при опознании и проверке обновлений: кто не ответил.
#[derive(Default)]
struct Failures {
	needed: HashSet<SourceKind>,
	errors: Vec<(SourceKind, AppError)>,
	/// Не ответивший каталог → кто проверил часть его файлов и сколько.
	covered: HashMap<SourceKind, (SourceKind, u32)>,
}

impl Failures {
	fn need(&mut self, kind: SourceKind) {
		self.needed.insert(kind);
	}

	fn fail(&mut self, kind: SourceKind, err: AppError) {
		log::warn!("каталог {}: {:?}", kind.as_str(), err.code);
		self.needed.insert(kind);
		if !self.errors.iter().any(|(k, _)| *k == kind) {
			self.errors.push((kind, err));
		}
	}

	fn failed(&self, kind: SourceKind) -> bool {
		self.errors.iter().any(|(k, _)| *k == kind)
	}

	/// Каталоги, чьи файлы стоит проверить через другой: не ответили, а другой включён и цел.
	fn fallbacks(&self, enabled: impl Fn(SourceKind) -> bool) -> Vec<(SourceKind, SourceKind)> {
		self.errors
			.iter()
			.filter(|(_, err)| super::worth_fallback(err))
			.map(|(kind, _)| (*kind, kind.other()))
			.filter(|(_, alt)| enabled(*alt) && !self.failed(*alt))
			.collect()
	}

	/// Подстраховка ответила: `alt` нашёл `files` файлов не ответившего `kind`.
	fn cover(&mut self, kind: SourceKind, alt: SourceKind, files: usize) {
		self.needed.insert(alt);
		self.covered
			.insert(kind, (alt, u32::try_from(files).unwrap_or(u32::MAX)));
	}

	/// Не ответил ни один нужный каталог — общая ошибка.
	fn all_failed(&self) -> Option<AppError> {
		(!self.errors.is_empty() && self.errors.len() >= self.needed.len())
			.then(|| super::all_failed(self.errors.iter().map(|(_, e)| e.clone()).collect()))
	}

	fn list(&self) -> Vec<SourceFailure> {
		let mut list: Vec<SourceFailure> = self
			.errors
			.iter()
			.map(|(source, err)| SourceFailure {
				source: *source,
				code: err.code,
				covered_by: self
					.covered
					.get(source)
					.filter(|(_, files)| *files > 0)
					.map(|(by, files)| super::Coverage {
						source: *by,
						files: *files,
					}),
			})
			.collect();
		list.sort_by_key(|f| SourceKind::ALL.iter().position(|k| *k == f.source));
		list
	}
}

/// Опознаёт файлы без источника: Modrinth по sha1, затем CurseForge по отпечатку. Каждый каталог
/// ждём не дольше [`super::SOURCE_DEADLINE`]; не ответивший — во втором значении, а ошибка —
/// только если не ответил ни один.
///
/// Итог по каталогам (`offline`, `failedSources` из `instance_content`) записывается при любом
/// исходе, если каталоги спрашивали: прежний список не остаётся висеть после новой ошибки.
pub(crate) async fn identify(l: &Launcher, instance_id: &str) -> Identified {
	let mut failures = Failures::default();
	let result = identify_with(l, instance_id, &mut failures).await;
	let state_changed = record_checked(l, instance_id, &result, &failures);
	Identified {
		result,
		state_changed,
	}
}

pub(crate) struct Identified {
	/// `true` — индекс изменился.
	pub result: Result<bool, AppError>,
	/// Изменились `offline` или `failedSources` сборки.
	pub state_changed: bool,
}

/// Записывает, какие каталоги не ответили. Каталоги не спрашивали и вышла ошибка — ничего
/// не известно, прежнее остаётся. Не ответил ни один и все — без сети — `offline`.
fn record_checked(
	l: &Launcher,
	instance_id: &str,
	result: &Result<bool, AppError>,
	failures: &Failures,
) -> bool {
	if failures.needed.is_empty() && result.is_err() {
		return false;
	}
	let offline =
		failures.all_failed().is_some() && failures.errors.iter().all(|(_, e)| is_offline_error(e));
	let failed = if offline { Vec::new() } else { failures.list() };
	let was_offline = l.content_locks.is_offline(instance_id);
	l.content_locks.set_offline(instance_id, offline);
	let failed_changed = l.content_locks.set_failed(instance_id, failed);
	was_offline != offline || failed_changed
}

async fn identify_with(
	l: &Launcher,
	instance_id: &str,
	failures: &mut Failures,
) -> Result<bool, AppError> {
	let (asked, untitled, known_fp, paths) = {
		let (index, _) = synced(l, instance_id).await?;
		let mut untitled: HashMap<SourceKind, Vec<String>> = HashMap::new();
		for s in index.files.values().filter_map(|e| e.source.as_ref()) {
			if incomplete(s) {
				untitled
					.entry(s.kind)
					.or_default()
					.push(s.project_id.clone());
			}
		}
		let known_fp: HashMap<String, u32> = index
			.files
			.values()
			.filter_map(|e| Some((e.sha1.clone(), e.fingerprint?)))
			.collect();
		let paths: HashMap<String, String> = index
			.files
			.iter()
			.map(|(k, e)| (e.sha1.clone(), k.clone()))
			.collect();
		(
			installed::unidentified(&index, installed::now_secs()),
			untitled,
			known_fp,
			paths,
		)
	};
	let mut found = HashMap::new();
	if !asked.is_empty() {
		failures.need(SourceKind::Modrinth);
		let modrinth = l.sources.get(SourceKind::Modrinth);
		match super::within(
			super::SOURCE_DEADLINE,
			modrinth.identify(&FileKey::of_sha1s(&asked)),
		)
		.await
		{
			Ok(list) => found = list,
			Err(err) => failures.fail(SourceKind::Modrinth, err),
		}
	}
	// Не узнал Modrinth — CurseForge по отпечатку (отпечаток считается один раз и хранится).
	let mut fingerprints: HashMap<String, u32> = HashMap::new();
	let rest: Vec<String> = asked
		.iter()
		.filter(|s| !found.contains_key(*s))
		.cloned()
		.collect();
	if !rest.is_empty() && l.sources.enabled(SourceKind::Curseforge) {
		failures.need(SourceKind::Curseforge);
		fingerprints = fingerprints_of(l, instance_id, &rest, &known_fp, &paths).await;
		let keys: Vec<FileKey> = rest
			.iter()
			.filter_map(|sha1| {
				Some(FileKey {
					sha1: sha1.clone(),
					fingerprint: Some(*fingerprints.get(sha1)?),
					project_id: None,
				})
			})
			.collect();
		let curseforge = l.sources.get(SourceKind::Curseforge);
		match super::within(super::SOURCE_DEADLINE, curseforge.identify(&keys)).await {
			Ok(cf) => found.extend(cf),
			Err(err) => failures.fail(SourceKind::Curseforge, err),
		}
	}
	let mut by_source: HashMap<SourceKind, Vec<String>> = untitled;
	for v in found.values() {
		by_source
			.entry(v.source)
			.or_default()
			.push(v.project_id.clone());
	}
	let mut hits: HashMap<(SourceKind, String), ProjectHit> = HashMap::new();
	for (kind, mut ids) in by_source {
		ids.sort();
		ids.dedup();
		if ids.is_empty() || !l.sources.enabled(kind) || failures.failed(kind) {
			continue;
		}
		failures.need(kind);
		let source = l.sources.get(kind);
		match super::within(super::SOURCE_DEADLINE, source.projects(&ids)).await {
			Ok(list) => hits.extend(list.into_iter().map(|h| ((h.source, h.id.clone()), h))),
			Err(err) => failures.fail(kind, err),
		}
	}
	if let Some(err) = failures.all_failed() {
		return Err(err);
	}
	// Кто-то не ответил — «не найдено» не запоминаем, иначе файл не проверится сутки.
	let checked: &[String] = if failures.errors.is_empty() {
		&asked
	} else {
		&[]
	};
	let _guard = l.content_locks.lock(instance_id).await;
	let (mut index, _) = sync_index(l, instance_id).await?;
	let changed = install::apply_identified(
		&mut index,
		checked,
		&found,
		&hits,
		&fingerprints,
		installed::now_secs(),
	);
	if changed {
		let path = installed::index_path(&l.layout.instance_dir(instance_id));
		tokio::task::spawn_blocking(move || installed::save_index(&path, &index))
			.await
			.map_err(join_error)??;
	}
	Ok(changed)
}

/// Отпечатки CurseForge файлов с этими sha1 (уже известные — из индекса, остальные — по файлам).
async fn fingerprints_of(
	l: &Launcher,
	instance_id: &str,
	sha1s: &[String],
	known: &HashMap<String, u32>,
	paths: &HashMap<String, String>,
) -> HashMap<String, u32> {
	let game_dir = l.layout.game_dir(instance_id);
	let jobs: Vec<(String, Option<u32>, Option<String>)> = sha1s
		.iter()
		.map(|s| (s.clone(), known.get(s).copied(), paths.get(s).cloned()))
		.collect();
	tokio::task::spawn_blocking(move || {
		let mut out = HashMap::new();
		for (sha1, known, key) in jobs {
			if let Some(fp) = known {
				out.insert(sha1, fp);
				continue;
			}
			let Some(key) = key else { continue };
			let path = [true, false]
				.into_iter()
				.filter_map(|on| installed::disk_path(&game_dir, &key, on).ok())
				.find(|p| p.is_file());
			if let Some(fp) = path.and_then(|p| super::fingerprint::of_file(&p).ok()) {
				out.insert(sha1, fp);
			}
		}
		out
	})
	.await
	.unwrap_or_default()
}

/// Файлы, выбранные `pick`, для подстраховки в `alt`. CurseForge узнаёт файл по отпечатку:
/// если его нет в индексе, он считается по файлу. Второе значение — отпечатки (sha1 → отпечаток).
async fn fallback_files(
	l: &Launcher,
	instance_id: &str,
	index: &Index,
	alt: SourceKind,
	pick: impl Fn(&String, &installed::Entry, &installed::SourceRef) -> bool,
) -> (Vec<(String, FileKey)>, HashMap<String, u32>) {
	let chosen: Vec<(&String, &installed::Entry)> = index
		.files
		.iter()
		.filter(|(key, e)| e.source.as_ref().is_some_and(|s| pick(key, e, s)))
		.collect();
	let mut fingerprints = HashMap::new();
	if alt == SourceKind::Curseforge && !chosen.is_empty() {
		let sha1s: Vec<String> = chosen.iter().map(|(_, e)| e.sha1.clone()).collect();
		let known: HashMap<String, u32> = chosen
			.iter()
			.filter_map(|(_, e)| Some((e.sha1.clone(), e.fingerprint?)))
			.collect();
		let paths: HashMap<String, String> = chosen
			.iter()
			.map(|(key, e)| (e.sha1.clone(), (*key).clone()))
			.collect();
		fingerprints = fingerprints_of(l, instance_id, &sha1s, &known, &paths).await;
	}
	let files = chosen
		.into_iter()
		.map(|(key, e)| {
			(
				key.clone(),
				FileKey {
					sha1: e.sha1.clone(),
					fingerprint: e.fingerprint.or_else(|| fingerprints.get(&e.sha1).copied()),
					project_id: None,
				},
			)
		})
		.collect();
	(files, fingerprints)
}

/// Список контента после проверки обновлений.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentUpdates {
	pub items: Vec<ContentItem>,
	/// Каталоги, которые не ответили: их файлы в этот раз не проверены.
	pub failed_sources: Vec<SourceFailure>,
}

type UpdateGroups = Vec<(ProjectType, Vec<(String, FileKey)>)>;

/// Последние версии файлов одного каталога: найденное и проверенные ключи. Ошибка — каталог
/// не ответил; то, что он успел проверить до неё, тоже отбрасывается.
async fn latest_of(
	source: Arc<dyn ContentSource>,
	instance: &Instance,
	groups: UpdateGroups,
) -> Result<(HashMap<String, ContentVersion>, HashSet<String>), AppError> {
	let mut latest = HashMap::new();
	let mut checked = HashSet::new();
	for (kind, files) in groups {
		let keys: Vec<FileKey> = files.iter().map(|(_, k)| k.clone()).collect();
		let filter = install::update_filter(instance, kind);
		latest.extend(source.latest(&keys, &filter).await?);
		checked.extend(files.into_iter().map(|(k, _)| k));
	}
	Ok((latest, checked))
}

/// Проверяет обновления контента (не чаще раза в 10 минут без `force`). Каждый каталог — отдельно
/// и не дольше [`super::SOURCE_DEADLINE`]; ошибка — только если не ответил ни один нужный.
#[tauri::command]
pub async fn instance_content_updates(
	app: AppHandle,
	l: L<'_>,
	id: String,
	force: Option<bool>,
) -> Result<ContentUpdates, AppError> {
	let instance = l.instances.get(&id).await?;
	if !force.unwrap_or(false) && l.content_locks.updates_fresh(&id, UPDATES_TTL) {
		return Ok(ContentUpdates {
			items: synced(&l, &id).await?.1,
			failed_sources: l.content_locks.failed(&id),
		});
	}
	let (index, _) = synced(&l, &id).await?;
	let mut groups: HashMap<SourceKind, HashMap<ProjectType, Vec<(String, FileKey)>>> =
		HashMap::new();
	for (key, entry) in &index.files {
		if let Some(s) = &entry.source
			&& l.sources.enabled(s.kind)
			&& let Ok((kind, _)) = installed::parse_key(key)
		{
			groups
				.entry(s.kind)
				.or_default()
				.entry(kind)
				.or_default()
				.push((
					key.clone(),
					FileKey {
						sha1: entry.sha1.clone(),
						fingerprint: entry.fingerprint,
						project_id: Some(s.project_id.clone()),
					},
				));
		}
	}
	let mut take = |kind: SourceKind| -> Option<UpdateGroups> {
		let list: UpdateGroups = groups.remove(&kind)?.into_iter().collect();
		(!list.is_empty()).then_some(list)
	};
	let (modrinth, curseforge) = (take(SourceKind::Modrinth), take(SourceKind::Curseforge));
	let check = |kind: SourceKind, groups: Option<UpdateGroups>| {
		let source = l.sources.get(kind);
		let instance = &instance;
		async move {
			let groups = groups?;
			Some(super::within(super::SOURCE_DEADLINE, latest_of(source, instance, groups)).await)
		}
	};
	let (from_modrinth, from_curseforge) = tokio::join!(
		check(SourceKind::Modrinth, modrinth),
		check(SourceKind::Curseforge, curseforge)
	);
	let mut failures = Failures::default();
	let mut latest = HashMap::new();
	let mut checked = HashSet::new();
	for (kind, result) in [
		(SourceKind::Modrinth, from_modrinth),
		(SourceKind::Curseforge, from_curseforge),
	] {
		match result {
			None => {}
			Some(Ok((found, keys))) => {
				failures.need(kind);
				latest.extend(found);
				checked.extend(keys);
			}
			Some(Err(err)) => failures.fail(kind, err),
		}
	}
	// Каталог не ответил (не «не нашёл») — его файлы проверяются во втором, одной пачкой.
	let mut covered: HashMap<String, Option<UpdateInfo>> = HashMap::new();
	let mut fingerprints = HashMap::new();
	for (kind, alt) in failures.fallbacks(|k| l.sources.enabled(k)) {
		let (files, fps) = fallback_files(&l, &id, &index, alt, |_, _, s| s.kind == kind).await;
		if files.is_empty() {
			continue;
		}
		let source = l.sources.get(alt);
		let check = install::fallback_latest(source.as_ref(), &instance, &files);
		match super::within(super::SOURCE_DEADLINE, check).await {
			Ok(found) => {
				failures.cover(kind, alt, found.len());
				covered.extend(found.into_iter().map(|(key, c)| (key, c.update())));
				fingerprints.extend(fps);
			}
			Err(err) => log::warn!(
				"подстраховка {} через {}: {:?}",
				kind.as_str(),
				alt.as_str(),
				err.code
			),
		}
	}
	if let Some(err) = failures.all_failed() {
		if is_offline_error(&err) {
			l.content_locks.set_offline(&id, true);
			l.content_locks.set_failed(&id, Vec::new());
		}
		return Err(err);
	}
	l.content_locks.set_offline(&id, false);
	let failed_sources = failures.list();
	l.content_locks.set_failed(&id, failed_sources.clone());
	if failed_sources.is_empty() {
		l.content_locks.mark_updates_checked(&id);
	}
	let items = {
		let _guard = l.content_locks.lock(&id).await;
		let (mut index, _) = sync_index(&l, &id).await?;
		install::apply_updates(&mut index, &latest, &checked);
		install::apply_fallback(&mut index, &covered, &fingerprints);
		let game_dir = l.layout.game_dir(&id);
		let path = installed::index_path(&l.layout.instance_dir(&id));
		tokio::task::spawn_blocking(move || {
			installed::save_index(&path, &index)?;
			Ok::<_, AppError>(installed::sync(&game_dir, &mut index).0)
		})
		.await
		.map_err(join_error)??
	};
	emit_changed(&app, &id);
	Ok(ContentUpdates {
		items,
		failed_sources,
	})
}

/// Игра запущена — файлы контента заняты.
fn ensure_not_running(l: &Launcher, id: &str) -> Result<(), AppError> {
	if l.instances.is_running(id) {
		Err(AppError::coded(ErrorCode::FileInUse, id.to_owned()))
	} else {
		Ok(())
	}
}

/// Обновляет `files` (или все). Обновления «только с сайта» сразу идут в ждущие.
#[tauri::command]
pub async fn instance_content_update(
	app: AppHandle,
	l: L<'_>,
	id: String,
	files: Option<Vec<FileRef>>,
) -> Result<Started, AppError> {
	let keys = files
		.map(|f| f.iter().map(FileRef::key).collect::<Result<Vec<_>, _>>())
		.transpose()?;
	start_update(app, l.inner().clone(), id, keys).await
}

async fn start_update(
	app: AppHandle,
	l: Arc<Launcher>,
	instance_id: String,
	keys: Option<Vec<String>>,
) -> Result<Started, AppError> {
	let instance = l.instances.get(&instance_id).await?;
	ensure_not_running(&l, &instance_id)?;
	let (index, _) = synced(&l, &instance_id).await?;
	let mut all = Vec::new();
	let mut failures = Failures::default();
	// Каталоги планируются одновременно; не ответивший уходит в `failedSources`.
	let plan = |kind: SourceKind| {
		let enabled = l.sources.enabled(kind);
		let source = l.sources.get(kind);
		let (index, keys) = (&index, keys.as_deref());
		async move {
			if !enabled {
				return None;
			}
			let plan = install::plan_update(source.as_ref(), kind, index, keys);
			Some(super::within(super::SOURCE_DEADLINE, plan).await)
		}
	};
	let (from_modrinth, from_curseforge) =
		tokio::join!(plan(SourceKind::Modrinth), plan(SourceKind::Curseforge));
	for (kind, result) in [
		(SourceKind::Modrinth, from_modrinth),
		(SourceKind::Curseforge, from_curseforge),
	] {
		let Some(result) = result else { continue };
		failures.need(kind);
		match result {
			Ok(items) => all.extend(items),
			Err(err) => failures.fail(kind, err),
		}
	}
	// Обновления не ответившего каталога ищутся во втором; найденное ставится оттуда.
	for (kind, alt) in failures.fallbacks(|k| l.sources.enabled(k)) {
		let (files, _) = fallback_files(&l, &instance_id, &index, alt, |key, e, s| {
			keys.as_deref().is_none_or(|k| k.contains(key))
				&& e.update.as_ref().is_some_and(|u| u.catalog(s) == kind)
		})
		.await;
		if files.is_empty() {
			continue;
		}
		let source = l.sources.get(alt);
		let plan = install::plan_fallback(source.as_ref(), &instance, &index, &files);
		match super::within(super::SOURCE_DEADLINE, plan).await {
			Ok((items, found)) => {
				failures.cover(kind, alt, found);
				all.extend(items);
			}
			Err(err) => log::warn!(
				"подстраховка {} через {}: {:?}",
				kind.as_str(),
				alt.as_str(),
				err.code
			),
		}
	}
	if all.is_empty()
		&& let Some(err) = failures.all_failed()
	{
		return Err(err);
	}
	let failed_sources = failures.list();
	let (manual_items, items): (Vec<PlannedItem>, Vec<PlannedItem>) =
		all.into_iter().partition(|i| i.site_only);
	let manual: Vec<ManualFile> = manual_items.iter().map(ManualFile::from_planned).collect();
	if !manual.is_empty() {
		let _guard = l.content_locks.lock(&instance_id).await;
		ensure_exists(&l, &instance_id)?;
		let dir = l.layout.instance_dir(&instance_id);
		let files = manual.clone();
		tokio::task::spawn_blocking(move || super::manual::add_waiting(&dir, &files))
			.await
			.map_err(join_error)??;
	}
	let Some(first) = items.first() else {
		return Ok(Started {
			download: None,
			manual,
			failed_sources,
		});
	};
	let single = items.len() == 1;
	let info = ContentTask {
		action: ContentAction::Update,
		source: first.hit.source,
		project_id: single.then(|| first.hit.id.clone()),
		project_title: Some(first.hit.title.clone()),
		icon_url: first.hit.icon_url.clone(),
		extra_count: items.len().saturating_sub(1) as u32,
	};
	let download = start_task(
		app,
		l,
		instance_id,
		ContentRequest::Update(keys),
		info,
		items,
		Vec::new(),
		Vec::new(),
	)
	.await?;
	Ok(Started {
		download: Some(download),
		manual,
		failed_sources,
	})
}

/// Включает или выключает файл (переименование в `.disabled` и обратно).
#[tauri::command]
pub async fn instance_content_toggle(
	app: AppHandle,
	l: L<'_>,
	id: String,
	item: FileRef,
	enabled: bool,
) -> Result<ContentItem, AppError> {
	l.instances.get(&id).await?;
	let key = item.key()?;
	ensure_not_running(&l, &id)?;
	let game_dir = l.layout.game_dir(&id);
	let from = installed::disk_path(&game_dir, &key, !enabled)?;
	let to = installed::disk_path(&game_dir, &key, enabled)?;
	let items = {
		let _guard = l.content_locks.lock(&id).await;
		ensure_exists(&l, &id)?;
		ensure_no_pack(&l, &id)?;
		match (from.exists(), to.exists()) {
			(false, true) => {}
			(true, false) => {
				std::fs::rename(&from, &to).map_err(|e| installed::io_error(&from, &e))?
			}
			(true, true) => return Err(AppError::coded(ErrorCode::AlreadyInstalled, key)),
			(false, false) => return Err(AppError::coded(ErrorCode::FileNotFound, key)),
		}
		sync_index(&l, &id).await?.1
	};
	emit_changed(&app, &id);
	items
		.into_iter()
		.find(|c| c.key == key)
		.ok_or_else(|| AppError::coded(ErrorCode::FileNotFound, key))
}

/// Удаляет файлы в корзину. Уже пропавшие пропускаются.
#[tauri::command]
pub async fn instance_content_remove(
	app: AppHandle,
	l: L<'_>,
	id: String,
	files: Vec<FileRef>,
) -> Result<(), AppError> {
	l.instances.get(&id).await?;
	let keys = files
		.iter()
		.map(FileRef::key)
		.collect::<Result<Vec<_>, _>>()?;
	ensure_not_running(&l, &id)?;
	let game_dir = l.layout.game_dir(&id);
	let result = {
		let _guard = l.content_locks.lock(&id).await;
		ensure_no_pack(&l, &id)?;
		let result = tokio::task::spawn_blocking(move || {
			let mut first_error = None;
			for key in &keys {
				for enabled in [true, false] {
					let Ok(path) = installed::disk_path(&game_dir, key, enabled) else {
						continue;
					};
					if path.exists()
						&& let Err(err) = crate::trash::delete(&path)
					{
						log::warn!("не удалось удалить {}: {err}", path.display());
						first_error.get_or_insert_with(|| installed::io_error(&path, &err));
					}
				}
			}
			first_error.map_or(Ok(()), Err)
		})
		.await
		.map_err(join_error)?;
		sync_index(&l, &id).await?;
		result
	};
	emit_changed(&app, &id);
	result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedReason {
	/// Не `.jar`/`.zip`, битый архив или тип не подходит к файлу.
	Type,
	Modpack,
	/// Модпак CurseForge (пока не поддерживается).
	Curseforge,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsupported {
	pub path: String,
	pub reason: UnsupportedReason,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddFilesResult {
	pub added: Vec<ContentItem>,
	pub unsupported: Vec<Unsupported>,
	/// `.zip`, тип которых не понять: спросить пользователя и вызвать ещё раз с `projectType`.
	pub needs_type: Vec<String>,
}

enum Outcome {
	Added(String),
	Unsupported(UnsupportedReason),
	NeedsType,
}

/// Файлы с диска — в папку по типу; одноимённый старый уходит в корзину.
#[tauri::command]
pub async fn instance_content_add_files(
	app: AppHandle,
	l: L<'_>,
	id: String,
	paths: Vec<String>,
	project_type: Option<ProjectType>,
	world: Option<String>,
) -> Result<AddFilesResult, AppError> {
	l.instances.get(&id).await?;
	let game_dir = l.layout.game_dir(&id);
	if let Some(world) = &world {
		crate::gamedata::world_dir(&game_dir, world)?;
	}
	let (outcomes, error) = {
		let _guard = l.content_locks.lock(&id).await;
		ensure_exists(&l, &id)?;
		ensure_no_pack(&l, &id)?;
		let game = game_dir.clone();
		tokio::task::spawn_blocking(move || {
			let mut outcomes = Vec::new();
			for p in paths {
				match add_one(&game, Path::new(&p), project_type, world.as_deref()) {
					Ok(outcome) => outcomes.push((p, outcome)),
					// Ошибка записи — дальше не идём, уже добавленное остаётся.
					Err(err) => return (outcomes, Some(err)),
				}
			}
			(outcomes, None)
		})
		.await
		.map_err(join_error)?
	};
	let mut result = AddFilesResult {
		added: Vec::new(),
		unsupported: Vec::new(),
		needs_type: Vec::new(),
	};
	let mut keys = Vec::new();
	for (path, outcome) in outcomes {
		match outcome {
			Outcome::Added(key) => keys.push(key),
			Outcome::Unsupported(reason) => result.unsupported.push(Unsupported { path, reason }),
			Outcome::NeedsType => result.needs_type.push(path),
		}
	}
	if !keys.is_empty() {
		let (index, items) = synced(&l, &id).await?;
		if needs_identify(&index) {
			spawn_identify(app.clone(), l.inner().clone(), id.clone());
		}
		emit_changed(&app, &id);
		result.added = items
			.into_iter()
			.filter(|c| keys.contains(&c.key))
			.collect();
	}
	match error {
		Some(err) => Err(err),
		None => Ok(result),
	}
}

fn add_one(
	game_dir: &Path,
	src: &Path,
	project_type: Option<ProjectType>,
	world: Option<&str>,
) -> Result<Outcome, AppError> {
	let name = src
		.file_name()
		.and_then(|n| n.to_str())
		.map(str::to_owned)
		.unwrap_or_default();
	if !src.is_absolute() || !src.is_file() {
		return Err(AppError::coded(
			ErrorCode::FileNotFound,
			src.display().to_string(),
		));
	}
	if !fs_util::is_safe_file_name(&name) || name.ends_with(installed::DISABLED_SUFFIX) {
		return Ok(Outcome::Unsupported(UnsupportedReason::Type));
	}
	let ext_fits = |kind: ProjectType| {
		name.rsplit_once('.')
			.is_some_and(|(_, e)| e.eq_ignore_ascii_case(kind.extension()))
	};
	let kind = match (project_type, installed::detect_type(src)) {
		(_, Detected::CurseforgeModpack) => {
			return Ok(Outcome::Unsupported(UnsupportedReason::Curseforge));
		}
		(_, Detected::Modpack) | (Some(ProjectType::Modpack), _) => {
			return Ok(Outcome::Unsupported(UnsupportedReason::Modpack));
		}
		(_, Detected::Unsupported) => return Ok(Outcome::Unsupported(UnsupportedReason::Type)),
		(Some(kind), _) if ext_fits(kind) => kind,
		(Some(_), _) => return Ok(Outcome::Unsupported(UnsupportedReason::Type)),
		(None, Detected::Kind(kind)) => kind,
		(None, Detected::NeedsType) => return Ok(Outcome::NeedsType),
	};
	let world = world.filter(|_| kind == ProjectType::Datapack);
	let key = format!("{}/{name}", installed::folder_key(kind, world)?);
	let dest = installed::disk_path(game_dir, &key, true)?;
	installed::import_file(src, &dest)?;
	Ok(Outcome::Added(key))
}

/// Подпапка игры от фронта — только из известных (`mods`, `saves/<мир>/datapacks`…).
pub fn sub_folder(game_dir: &Path, sub: &str) -> Result<PathBuf, AppError> {
	const ALLOWED: &[&str] = &[
		"mods",
		"resourcepacks",
		"shaderpacks",
		"datapacks",
		"saves",
		"screenshots",
		"logs",
		"crash-reports",
		"config",
	];
	let first = sub.split('/').next().unwrap_or_default();
	let ok = ALLOWED.contains(&first) && sub.split('/').all(fs_util::is_safe_file_name);
	ok.then(|| fs_util::safe_join(game_dir, sub))
		.flatten()
		.ok_or_else(|| AppError::coded(ErrorCode::InvalidValue, format!("sub: {sub}")))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::content::installed::{ItemProject, ItemUpdate};
	use crate::content::tests::instance;
	use crate::content::{IncompatibleReason, Side};
	use crate::meta::LoaderKind;

	fn hit(id: &str, versions: &[&str], loaders: &[&str]) -> ProjectHit {
		ProjectHit {
			source: SourceKind::Modrinth,
			id: id.into(),
			slug: id.into(),
			project_type: ProjectType::Mod,
			title: id.into(),
			author: String::new(),
			description: String::new(),
			icon_url: None,
			downloads: 0,
			follows: None,
			site_only: false,
			url: String::new(),
			categories: vec![],
			updated_at: 0,
			game_versions: versions.iter().map(|s| (*s).to_owned()).collect(),
			loaders: loaders.iter().map(|s| (*s).to_owned()).collect(),
			client_side: Side::Unknown,
			server_side: Side::Unknown,
			install: None,
			also_on: Vec::new(),
			created_at: 0,
		}
	}

	fn item(project: &str, update: bool) -> ContentItem {
		ContentItem {
			key: format!("mods/{project}.jar"),
			file: format!("{project}.jar"),
			project_type: ProjectType::Mod,
			enabled: true,
			size: 1,
			world: None,
			project: Some(ItemProject {
				source: SourceKind::Modrinth,
				id: project.into(),
				version_id: "v1".into(),
				title: project.into(),
				author: String::new(),
				icon_url: None,
				version_number: "1".into(),
				url: None,
			}),
			update: update.then(|| ItemUpdate {
				version_id: "v2".into(),
				number: "2".into(),
				site_only: false,
				via: None,
			}),
			from_modpack: false,
			sha1: String::new(),
		}
	}

	#[test]
	fn install_states() {
		let fabric = instance(LoaderKind::Fabric, "1.20.1");
		let items = vec![item("a", false), item("b", true)];
		assert_eq!(
			install_state(
				&hit("a", &["1.20.1"], &["fabric"]),
				&fabric,
				&items,
				&Default::default()
			),
			InstallState::Installed {
				version_id: "v1".into(),
				file: "a.jar".into(),
				enabled: true
			}
		);
		assert!(matches!(
			install_state(&hit("b", &[], &[]), &fabric, &items, &Default::default()),
			InstallState::Update { latest_version_id, .. } if latest_version_id == "v2"
		));
		assert_eq!(
			install_state(
				&hit("c", &["1.20.1"], &["fabric"]),
				&fabric,
				&items,
				&Default::default()
			),
			InstallState::None
		);
		assert_eq!(
			install_state(
				&hit("c", &["1.20.1"], &["forge"]),
				&fabric,
				&items,
				&Default::default()
			),
			InstallState::Incompatible {
				reason: IncompatibleReason::Loader
			}
		);
	}

	#[test]
	fn sub_folders_and_add_files() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path().join("game");
		assert_eq!(sub_folder(&game, "mods").unwrap(), game.join("mods"));
		assert_eq!(
			sub_folder(&game, "saves/Мир/datapacks").unwrap(),
			game.join("saves/Мир/datapacks")
		);
		for bad in ["", "..", "saves/../..", "options.txt", "C:/x", "mods//a"] {
			assert!(sub_folder(&game, bad).is_err(), "{bad}");
		}

		let src = dir.path().join("in");
		std::fs::create_dir_all(&src).unwrap();
		std::fs::write(src.join("m.jar"), b"jar").unwrap();
		std::fs::write(src.join("x.zip"), b"not zip").unwrap();
		std::fs::write(src.join("p.mrpack"), b"x").unwrap();
		assert!(matches!(
			add_one(&game, &src.join("m.jar"), None, None).unwrap(),
			Outcome::Added(k) if k == "mods/m.jar"
		));
		assert!(game.join("mods/m.jar").exists());
		assert!(matches!(
			add_one(&game, &src.join("x.zip"), None, None).unwrap(),
			Outcome::Unsupported(UnsupportedReason::Type)
		));
		assert!(matches!(
			add_one(&game, &src.join("p.mrpack"), None, None).unwrap(),
			Outcome::Unsupported(UnsupportedReason::Modpack)
		));
		assert!(matches!(
			add_one(&game, &src.join("m.jar"), Some(ProjectType::Shader), None).unwrap(),
			Outcome::Unsupported(UnsupportedReason::Type)
		));
		assert_eq!(
			add_one(&game, Path::new("relative.jar"), None, None)
				.err()
				.unwrap()
				.code,
			ErrorCode::FileNotFound
		);
	}

	#[test]
	fn failures_fall_back_only_when_silent() {
		let both = |_: SourceKind| true;
		let mut f = Failures::default();
		f.need(SourceKind::Curseforge);
		f.fail(SourceKind::Modrinth, super::super::timed_out());
		assert_eq!(
			f.fallbacks(both),
			[(SourceKind::Modrinth, SourceKind::Curseforge)]
		);
		assert!(f.fallbacks(|k| k == SourceKind::Modrinth).is_empty());

		f.cover(SourceKind::Modrinth, SourceKind::Curseforge, 3);
		assert!(f.all_failed().is_none());
		let list = f.list();
		assert_eq!(list.len(), 1);
		assert_eq!(
			list[0].covered_by,
			Some(super::super::Coverage {
				source: SourceKind::Curseforge,
				files: 3,
			})
		);
		let json = serde_json::to_value(&list[0]).unwrap();
		assert_eq!(
			json,
			serde_json::json!({
				"source": "modrinth",
				"code": "source_unavailable",
				"coveredBy": { "source": "curseforge", "files": 3 }
			})
		);

		// «Не найдено» — ответ каталога, а не молчание.
		let mut f = Failures::default();
		f.fail(
			SourceKind::Modrinth,
			AppError::new(ErrorCode::ContentNotFound),
		);
		assert!(f.fallbacks(both).is_empty());

		// Второй тоже не ответил — подстраховки нет, ничего не найдено — `coveredBy: null`.
		let mut f = Failures::default();
		f.fail(SourceKind::Modrinth, AppError::new(ErrorCode::Network));
		f.fail(SourceKind::Curseforge, AppError::new(ErrorCode::Network));
		assert!(f.fallbacks(both).is_empty());
		f.cover(SourceKind::Modrinth, SourceKind::Curseforge, 0);
		assert!(f.list().iter().all(|s| s.covered_by.is_none()));
	}
}
