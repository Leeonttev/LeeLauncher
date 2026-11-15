//! Установка и обновление контента: план (версия, зависимости, конфликты), скачивание, запись в индекс.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::installed::{self, Entry, Index, SourceRef, UpdateInfo};
use super::{
	ContentFile, ContentSource, ContentVersion, DependencyKind, ProjectHit, ProjectType,
	SourceKind, VersionFilter,
};
use crate::download::{Downloader, Job, Progress, Verify};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::instances::Instance;
use crate::meta::LoaderKind;

/// Зависимостей за одну установку не больше этого (защита от циклов и огромных деревьев).
const MAX_DEPENDENCIES: usize = 64;
const MAX_DEPTH: usize = 8;

/// Что поставить.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
	#[serde(default)]
	pub source: SourceKind,
	pub project_id: String,
	/// Конкретная версия; нет — лучшая подходящая сборке.
	#[serde(default)]
	pub version_id: Option<String>,
	/// Тип, под которым ставить (вкладка каталога); нет — по проекту.
	#[serde(default, rename = "type")]
	pub project_type: Option<ProjectType>,
	/// Датапак — в этот мир (`saves/<мир>/datapacks`); нет — общая папка `datapacks`.
	#[serde(default)]
	pub world: Option<String>,
	/// Необязательные зависимости, которые тоже поставить (id проектов).
	#[serde(default)]
	pub optional: Vec<String>,
}

/// Задача контента в списке загрузок (для повтора).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentRequest {
	Install(InstallRequest),
	/// Обновить эти файлы (ключи; `None` — все, у которых есть обновление).
	Update(Option<Vec<String>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentAction {
	Install,
	Update,
}

/// Что показывать в списке загрузок у задачи `content`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentTask {
	pub action: ContentAction,
	pub source: SourceKind,
	/// Главный проект; у обновления нескольких файлов — `null`.
	pub project_id: Option<String>,
	pub project_title: Option<String>,
	pub icon_url: Option<String>,
	/// Сколько ещё файлов кроме главного (зависимости или другие обновления).
	pub extra_count: u32,
}

/// Один файл плана.
#[derive(Debug, Clone)]
pub struct PlannedItem {
	pub hit: ProjectHit,
	pub version_id: String,
	pub version_number: String,
	pub file: ContentFile,
	/// Папка от папки игры (`mods`, `saves/<мир>/datapacks`).
	pub folder: String,
	pub dependency: bool,
	/// Заменяет этот установленный файл (ключ) — другую версию того же проекта.
	pub replaces: Option<String>,
	/// Как ставится (папка, мир — распаковка).
	pub kind: ProjectType,
	/// Скачать можно только на сайте каталога (`page_url`).
	pub site_only: bool,
	pub page_url: Option<String>,
	/// Мир (датапак) для датапака в мир.
	pub world: Option<String>,
}

impl PlannedItem {
	pub fn key(&self) -> String {
		format!("{}/{}", self.folder, self.file.file_name)
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
	pub project_id: String,
	pub title: Option<String>,
}

/// План установки.
#[derive(Debug, Clone)]
pub struct InstallPlan {
	pub target: PlannedItem,
	pub version: ContentVersion,
	/// Обязательные зависимости, которых ещё нет (и загрузчик шейдеров): поставятся вместе.
	pub required: Vec<PlannedItem>,
	/// Необязательные зависимости, которых нет: при выборе (`optional` в запросе) — тоже в `required`.
	pub optional: Vec<ProjectHit>,
	/// Обязательные зависимости без подходящей версии.
	pub missing: Vec<ProjectRef>,
	/// Установленные файлы (ключи), с которыми проект несовместим: при установке выключаются.
	pub conflicts: Vec<String>,
	/// Эта же версия уже стоит.
	pub already_installed: bool,
}

impl InstallPlan {
	/// Что скачает задача: всё, кроме файлов «только с сайта».
	pub fn items(&self) -> Vec<PlannedItem> {
		self.all().into_iter().filter(|i| !i.site_only).collect()
	}

	/// Файлы «только с сайта» (сам проект и зависимости): их пользователь скачивает браузером.
	pub fn manual(&self) -> Vec<PlannedItem> {
		self.all().into_iter().filter(|i| i.site_only).collect()
	}

	fn all(&self) -> Vec<PlannedItem> {
		let mut items = Vec::with_capacity(1 + self.required.len());
		if !self.already_installed {
			items.push(self.target.clone());
		}
		items.extend(self.required.iter().cloned());
		items
	}
}

/// Установленные проекты: (каталог, id проекта) → ключ файла.
fn installed_projects(index: &Index) -> HashMap<(SourceKind, String), String> {
	index
		.files
		.iter()
		.filter_map(|(key, e)| {
			let s = e.source.as_ref()?;
			Some(((s.kind, s.project_id.clone()), key.clone()))
		})
		.collect()
}

fn planned(
	hit: &ProjectHit,
	kind: ProjectType,
	version: &ContentVersion,
	folder: &str,
	dependency: bool,
) -> Result<PlannedItem, AppError> {
	let file = version.primary_file().cloned().ok_or_else(|| {
		AppError::coded(
			ErrorCode::ContentIncompatible,
			format!("{}: нет файла", version.id),
		)
	})?;
	let ext_ok = file
		.file_name
		.rsplit_once('.')
		.is_some_and(|(_, e)| e.eq_ignore_ascii_case(kind.extension()));
	if !version.site_only && file.url.is_empty() {
		return Err(AppError::coded(
			ErrorCode::DownloadFailed,
			format!("{}: у файла нет адреса", version.id),
		));
	}
	if !fs_util::is_safe_file_name(&file.file_name)
		|| file.file_name.ends_with(installed::DISABLED_SUFFIX)
		|| !ext_ok
	{
		return Err(AppError::coded(
			ErrorCode::ContentIncompatible,
			format!("{}: файл {}", version.id, file.file_name),
		));
	}
	Ok(PlannedItem {
		hit: hit.clone(),
		version_id: version.id.clone(),
		version_number: version.version_number.clone(),
		file,
		folder: folder.to_owned(),
		dependency,
		replaces: None,
		kind,
		site_only: version.site_only,
		page_url: version.page_url.clone(),
		world: folder
			.strip_prefix("saves/")
			.and_then(|r| r.strip_suffix("/datapacks"))
			.map(str::to_owned),
	})
}

/// Тип зависимости: модпак как зависимость — мод, остальное как у проекта.
fn dependency_type(hit: &ProjectHit) -> ProjectType {
	match hit.project_type {
		ProjectType::Modpack => ProjectType::Mod,
		other => other,
	}
}

fn folder_for(kind: ProjectType, world: Option<&str>) -> Result<String, AppError> {
	if kind == ProjectType::World {
		return Ok(WORLDS_FOLDER.to_owned());
	}
	installed::folder_key(kind, world.filter(|_| kind == ProjectType::Datapack))
}

/// Папка миров: мир ставится распаковкой архива в `saves/<имя>` ([`super::worlds`]).
pub const WORLDS_FOLDER: &str = "saves";

/// Нет подходящей версии.
fn incompatible(instance: &Instance, id: &str, filter: &VersionFilter) -> AppError {
	AppError::coded(
		ErrorCode::ContentIncompatible,
		format!(
			"{id}: {} {}",
			instance.game_version,
			filter.loaders.join("/")
		),
	)
}

/// Строит план установки. Сеть: проект, версии, зависимости. `index` — после [`installed::sync`].
pub async fn resolve(
	source: &dyn ContentSource,
	instance: &Instance,
	game_dir: &Path,
	index: &Index,
	req: &InstallRequest,
) -> Result<InstallPlan, AppError> {
	super::check_ref(&req.project_id)?;
	if let Some(v) = &req.version_id {
		super::check_ref(v)?;
	}
	for o in &req.optional {
		super::check_ref(o)?;
	}
	let hit = source
		.projects(std::slice::from_ref(&req.project_id))
		.await?
		.into_iter()
		.next()
		.ok_or_else(|| AppError::coded(ErrorCode::ContentNotFound, req.project_id.clone()))?;
	let kind = req.project_type.unwrap_or(hit.project_type);
	if kind == ProjectType::Modpack {
		return Err(AppError::coded(ErrorCode::ContentIncompatible, "modpack"));
	}
	let vanilla = instance.loader.kind == LoaderKind::Vanilla;
	if vanilla && matches!(kind, ProjectType::Mod | ProjectType::Shader) {
		return Err(AppError::coded(
			ErrorCode::ContentIncompatible,
			"needs_loader",
		));
	}
	let world = req
		.world
		.as_deref()
		.filter(|_| kind == ProjectType::Datapack);
	let folder = folder_for(kind, world)?;
	if let Some(world) = world
		&& !game_dir.join("saves").join(world).is_dir()
	{
		return Err(AppError::coded(
			ErrorCode::FileNotFound,
			format!("saves/{world}"),
		));
	}
	let filter = VersionFilter::for_instance(instance, kind);
	let version = match &req.version_id {
		Some(id) => {
			let version = source.version(id).await?;
			if version.project_id != hit.id {
				return Err(AppError::coded(
					ErrorCode::InvalidValue,
					format!("версия {id} не из проекта {}", hit.id),
				));
			}
			// Загрузчик для модов важен всегда; версию игры пользователь мог выбрать сам.
			let loader_filter = VersionFilter {
				game_versions: Vec::new(),
				loaders: filter.loaders.clone(),
			};
			if !loader_filter.matches(&version) {
				return Err(incompatible(instance, id, &filter));
			}
			version
		}
		None => {
			let versions = source.project_versions(&hit.id, &filter).await?;
			super::pick_best(&versions)
				.cloned()
				.ok_or_else(|| incompatible(instance, &hit.id, &filter))?
		}
	};

	let installed = installed_projects(index);
	let mut target = planned(&hit, kind, &version, &folder, false)?;
	let mut already_installed = false;
	// Миры не заменяются: «установить ещё раз» — вторая копия.
	if kind != ProjectType::World
		&& let Some(key) = installed.get(&(req.source, hit.id.clone()))
	{
		let same = index
			.files
			.get(key)
			.and_then(|e| e.source.as_ref())
			.is_some_and(|s| s.version_id == version.id);
		if same && *key == target.key() {
			already_installed = true;
		} else {
			target.replaces = Some(key.clone());
		}
	}

	let mut plan = InstallPlan {
		target,
		version: version.clone(),
		required: Vec::new(),
		optional: Vec::new(),
		missing: Vec::new(),
		conflicts: Vec::new(),
		already_installed,
	};
	// Загрузчик шейдеров — как обязательная зависимость; подходит уже стоящий из любого каталога.
	let mut root = version;
	let has = |source: SourceKind, id: &str| installed.contains_key(&(source, id.to_owned()));
	if kind == ProjectType::Shader
		&& let Some(loader) = super::shader_loader_project(req.source, instance.loader.kind)
		&& !super::shader_loader_installed(instance.loader.kind, &has)
	{
		root.dependencies.push(super::Dependency {
			project_id: Some(loader.to_owned()),
			version_id: None,
			kind: DependencyKind::Required,
		});
	}
	resolve_dependencies(source, instance, &installed, &root, req, &mut plan).await?;
	Ok(plan)
}

async fn resolve_dependencies(
	source: &dyn ContentSource,
	instance: &Instance,
	installed: &HashMap<(SourceKind, String), String>,
	root: &ContentVersion,
	req: &InstallRequest,
	plan: &mut InstallPlan,
) -> Result<(), AppError> {
	let mut seen: HashSet<String> = HashSet::from([root.project_id.clone()]);
	let mut optional_seen = HashSet::new();
	let mut level: VecDeque<ContentVersion> = VecDeque::from([root.clone()]);
	for depth in 0..MAX_DEPTH {
		if level.is_empty() || plan.required.len() >= MAX_DEPENDENCIES {
			break;
		}
		let deps: Vec<_> = level
			.drain(..)
			.flat_map(|v| v.dependencies)
			.filter(|d| d.kind != DependencyKind::Embedded)
			.collect();
		let version_ids: Vec<String> = deps
			.iter()
			.filter_map(|d| d.version_id.clone())
			.filter(|id| super::check_ref(id).is_ok())
			.collect();
		let pinned: HashMap<String, ContentVersion> = if version_ids.is_empty() {
			HashMap::new()
		} else {
			source
				.versions(&version_ids)
				.await?
				.into_iter()
				.map(|v| (v.id.clone(), v))
				.collect()
		};
		let mut wanted: Vec<(DependencyKind, String, Option<ContentVersion>)> = Vec::new();
		for dep in deps {
			let version = dep
				.version_id
				.as_ref()
				.and_then(|id| pinned.get(id))
				.cloned();
			let project = dep
				.project_id
				.clone()
				.or_else(|| version.as_ref().map(|v| v.project_id.clone()));
			if let Some(project) = project.filter(|p| super::check_ref(p).is_ok()) {
				// Выбранные необязательные зависимости самого проекта ставятся как обязательные.
				let kind = if depth == 0
					&& dep.kind == DependencyKind::Optional
					&& req.optional.contains(&project)
				{
					DependencyKind::Required
				} else {
					dep.kind
				};
				wanted.push((kind, project, version));
			}
		}
		let mut ids: Vec<String> = wanted.iter().map(|(_, p, _)| p.clone()).collect();
		ids.sort();
		ids.dedup();
		let hits: HashMap<String, ProjectHit> = if ids.is_empty() {
			HashMap::new()
		} else {
			source
				.projects(&ids)
				.await?
				.into_iter()
				.flat_map(|h| [(h.id.clone(), h.clone()), (h.slug.clone(), h)])
				.collect()
		};
		let reference = |id: &str| ProjectRef {
			project_id: id.to_owned(),
			title: hits.get(id).map(|h| h.title.clone()),
		};

		for (kind, project, version) in wanted {
			// Каталог мог ответить по slug: дальше — настоящий id.
			let project = hits.get(&project).map_or(project, |h| h.id.clone());
			let installed_key = installed.get(&(req.source, project.clone()));
			match kind {
				DependencyKind::Incompatible => {
					if let Some(key) = installed_key
						&& !plan.conflicts.contains(key)
						&& plan.target.replaces.as_ref() != Some(key)
					{
						plan.conflicts.push(key.clone());
					}
				}
				DependencyKind::Optional => {
					if installed_key.is_none()
						&& !seen.contains(&project)
						&& optional_seen.insert(project.clone())
						&& let Some(hit) = hits.get(&project)
					{
						plan.optional.push(hit.clone());
					}
				}
				DependencyKind::Required => {
					if installed_key.is_some() || !seen.insert(project.clone()) {
						continue;
					}
					if plan.required.len() >= MAX_DEPENDENCIES {
						break;
					}
					let Some(hit) = hits.get(&project) else {
						plan.missing.push(reference(&project));
						continue;
					};
					let dep_kind = dependency_type(hit);
					let filter = VersionFilter::for_instance(instance, dep_kind);
					let version = match version {
						Some(v) => Some(v),
						None => {
							let list = source.project_versions(&project, &filter).await?;
							super::pick_best(&list).cloned()
						}
					};
					let item = version.as_ref().and_then(|v| {
						let folder = folder_for(dep_kind, req.world.as_deref()).ok()?;
						planned(hit, dep_kind, v, &folder, true).ok()
					});
					match (item, version) {
						(Some(item), Some(version)) => {
							plan.required.push(item);
							level.push_back(version);
						}
						_ => plan.missing.push(reference(&project)),
					}
				}
				DependencyKind::Embedded => {}
			}
		}
	}
	// То, что уже ставится, не нужно как «необязательное».
	let target = plan.target.hit.id.clone();
	let required: HashSet<String> = plan.required.iter().map(|d| d.hit.id.clone()).collect();
	plan.optional
		.retain(|o| !required.contains(&o.id) && o.id != target);
	Ok(())
}

/// План обновления по найденным обновлениям (`instance_content_updates`). `keys` — какие файлы,
/// `None` — все с обновлением. Сеть: версии пачкой.
pub async fn plan_update(
	source: &dyn ContentSource,
	kind: SourceKind,
	index: &Index,
	keys: Option<&[String]>,
) -> Result<Vec<PlannedItem>, AppError> {
	let chosen: Vec<(&String, &Entry, &SourceRef, &UpdateInfo)> = index
		.files
		.iter()
		.filter(|(key, _)| keys.is_none_or(|k| k.contains(key)))
		.filter_map(|(key, e)| {
			let s = e.source.as_ref().filter(|s| s.kind == kind)?;
			Some((key, e, s, e.update.as_ref()?))
		})
		.collect();
	if chosen.is_empty() {
		return Ok(Vec::new());
	}
	let ids: Vec<String> = chosen.iter().map(|(.., u)| u.version_id.clone()).collect();
	let versions: HashMap<String, ContentVersion> = source
		.versions(&ids)
		.await?
		.into_iter()
		.map(|v| (v.id.clone(), v))
		.collect();
	let mut items = Vec::new();
	for (key, entry, src, update) in chosen {
		let Some(version) = versions.get(&update.version_id) else {
			continue;
		};
		let (kind, _) = installed::parse_key(key)?;
		let folder = key.rsplit_once('/').map(|(f, _)| f).unwrap_or_default();
		let hit = ProjectHit {
			source: src.kind,
			id: src.project_id.clone(),
			slug: src.project_id.clone(),
			project_type: kind,
			title: src.title.clone().unwrap_or_else(|| src.project_id.clone()),
			author: src.author.clone().unwrap_or_default(),
			description: String::new(),
			icon_url: src.icon_url.clone(),
			downloads: 0,
			follows: None,
			site_only: version.site_only,
			url: super::project_url(src.kind, &src.project_id),
			categories: Vec::new(),
			updated_at: 0,
			game_versions: Vec::new(),
			loaders: Vec::new(),
			client_side: super::Side::Unknown,
			server_side: super::Side::Unknown,
			install: None,
			also_on: Vec::new(),
			created_at: 0,
		};
		match planned(&hit, kind, version, folder, entry.dependency) {
			Ok(mut item) => {
				item.replaces = Some(key.clone());
				items.push(item);
			}
			Err(err) => log::warn!("обновление {key} пропущено: {:?}", err.details),
		}
	}
	Ok(items)
}

/// Фильтр «последних версий» для проверки обновлений: у шейдеров загрузчик нужен явно.
pub fn update_filter(instance: &Instance, kind: ProjectType) -> VersionFilter {
	let mut filter = VersionFilter::for_instance(instance, kind);
	if filter.loaders.is_empty() {
		filter.loaders = ["iris", "optifine", "canvas", "vanilla"]
			.map(str::to_owned)
			.to_vec();
	}
	filter
}

fn modified_ms(path: &Path) -> u64 {
	std::fs::metadata(path)
		.and_then(|m| m.modified())
		.ok()
		.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
		.map_or(0, |d| d.as_millis() as u64)
}

/// Качает план во временную `.content-tmp-*`, чтобы не трогать установленное, пока всё не скачано.
pub async fn download(
	dl: &Downloader,
	stage_root: &Path,
	items: &[PlannedItem],
	progress: std::sync::Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<std::path::PathBuf, AppError> {
	let staging = stage_root.join(format!(".content-tmp-{:08x}", rand::random::<u32>()));
	let jobs: Vec<Job> = items
		.iter()
		.enumerate()
		.map(|(i, item)| Job {
			url: item.file.url.clone(),
			path: staging.join(i.to_string()),
			sha1: Some(item.file.sha1.clone()),
			size: (item.file.size > 0).then_some(item.file.size),
		})
		.collect();
	if let Err(err) = dl.files(jobs, Verify::Hash, progress, cancel).await {
		let _ = std::fs::remove_dir_all(&staging);
		return Err(err.into());
	}
	Ok(staging)
}

/// Что сделал [`apply`] помимо файлов контента.
#[derive(Debug, Default)]
pub struct Applied {
	/// Поставленные миры: папка в `saves/` и файл плана.
	pub worlds: Vec<(String, PlannedItem)>,
	/// Зависимости, которые не поставлены: мод с тем же id уже есть (из другого каталога).
	pub skipped: Vec<String>,
}

/// id модов в `mods/` (включённых и выключенных), кроме файлов, которые план заменяет.
fn installed_mod_ids(game_dir: &Path, items: &[PlannedItem]) -> HashSet<String> {
	let replaced: HashSet<String> = items.iter().filter_map(|i| i.replaces.clone()).collect();
	let mut ids = HashSet::new();
	let Ok(entries) = std::fs::read_dir(game_dir.join("mods")) else {
		return ids;
	};
	for entry in entries.flatten() {
		let name = entry.file_name().to_string_lossy().into_owned();
		let (base, _) = installed::split_disabled(&name);
		if !base.to_ascii_lowercase().ends_with(".jar")
			|| replaced.contains(&format!("mods/{base}"))
		{
			continue;
		}
		ids.extend(installed::mod_ids(&entry.path()));
	}
	ids
}

/// Переносит скачанное на место: старые версии — в корзину, конфликты выключаются.
/// Под замком контента сборки.
pub fn apply(
	game_dir: &Path,
	staging: &Path,
	index: &mut Index,
	items: &[PlannedItem],
	conflicts: &[String],
) -> Result<Applied, AppError> {
	let mut applied = Applied::default();
	// id модов, которые уже стоят (из любого каталога): зависимость с таким же id не ставим.
	let has_mod_deps = items
		.iter()
		.any(|i| i.dependency && i.kind == ProjectType::Mod);
	let installed_ids = if has_mod_deps {
		installed_mod_ids(game_dir, items)
	} else {
		HashSet::new()
	};
	for (i, item) in items.iter().enumerate() {
		let from = staging.join(i.to_string());
		if item.kind == ProjectType::World {
			let folder = super::worlds::install(
				&game_dir.join(WORLDS_FOLDER),
				&from,
				&item.hit.title,
				super::worlds::Limits::DEFAULT,
			)?;
			applied.worlds.push((folder, item.clone()));
			continue;
		}
		if item.dependency && item.kind == ProjectType::Mod && item.replaces.is_none() {
			let ids = installed::mod_ids(&from);
			if let Some(id) = ids.iter().find(|id| installed_ids.contains(*id)) {
				log::info!(
					"зависимость {} не ставится: мод {id} уже есть в сборке",
					item.file.file_name
				);
				applied.skipped.push(item.hit.id.clone());
				continue;
			}
		}
		let key = item.key();
		let mut disabled = false;
		let mut dependency = item.dependency;
		let mut olds = vec![key.clone()];
		if let Some(old) = &item.replaces {
			if let Some(entry) = index.files.get(old) {
				dependency = entry.dependency;
			}
			olds.push(old.clone());
		}
		olds.dedup();
		for old in &olds {
			let on = installed::disk_path(game_dir, old, true)?;
			let off = installed::disk_path(game_dir, old, false)?;
			if Some(old) == item.replaces.as_ref() {
				disabled = off.exists() && !on.exists();
			}
			for path in [on, off] {
				if path.exists() {
					crate::trash::delete(&path).map_err(|e| installed::io_error(&path, &e))?;
				}
			}
			index.files.remove(old);
		}
		let path = installed::disk_path(game_dir, &key, !disabled)?;
		if let Some(dir) = path.parent() {
			std::fs::create_dir_all(dir)
				.map_err(|e| AppError::storage(format!("{}: {e}", dir.display())))?;
		}
		std::fs::rename(&from, &path).map_err(|e| installed::io_error(&path, &e))?;
		let size = std::fs::metadata(&path)
			.map(|m| m.len())
			.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?;
		index.files.insert(
			key,
			Entry {
				sha1: item.file.sha1.clone(),
				size,
				modified: modified_ms(&path),
				source: Some(SourceRef {
					kind: item.hit.source,
					project_id: item.hit.id.clone(),
					version_id: item.version_id.clone(),
					title: Some(item.hit.title.clone()),
					icon_url: item.hit.icon_url.clone(),
					version_number: Some(item.version_number.clone()),
					author: Some(item.hit.author.clone()).filter(|a| !a.is_empty()),
				}),
				checked_at: None,
				update: None,
				dependency,
				fingerprint: None,
			},
		);
	}
	for key in conflicts {
		let on = installed::disk_path(game_dir, key, true)?;
		let off = installed::disk_path(game_dir, key, false)?;
		if on.exists() && !off.exists() {
			std::fs::rename(&on, &off).map_err(|e| installed::io_error(&on, &e))?;
		}
	}
	Ok(applied)
}

/// Записывает результат опознания в индекс. `true` — индекс изменился.
pub fn apply_identified(
	index: &mut Index,
	asked: &[String],
	found: &HashMap<String, ContentVersion>,
	hits: &HashMap<(SourceKind, String), ProjectHit>,
	fingerprints: &HashMap<String, u32>,
	now: i64,
) -> bool {
	let mut changed = false;
	for entry in index.files.values_mut() {
		if entry.fingerprint.is_none()
			&& let Some(fp) = fingerprints.get(&entry.sha1)
		{
			entry.fingerprint = Some(*fp);
			changed = true;
		}
		if entry.source.is_none() {
			if let Some(version) = found.get(&entry.sha1) {
				entry.source = Some(SourceRef {
					kind: version.source,
					project_id: version.project_id.clone(),
					version_id: version.id.clone(),
					title: None,
					icon_url: None,
					version_number: Some(version.version_number.clone()),
					author: None,
				});
				entry.checked_at = None;
				changed = true;
			} else if asked.contains(&entry.sha1) {
				entry.checked_at = Some(now);
				changed = true;
			}
		}
		if let Some(source) = entry.source.as_mut()
			&& (source.title.is_none() || source.author.is_none())
			&& let Some(hit) = hits.get(&(source.kind, source.project_id.clone()))
		{
			source.title = Some(hit.title.clone());
			source.icon_url = hit.icon_url.clone();
			source.author = Some(hit.author.clone());
			changed = true;
		}
	}
	changed
}

/// Записывает найденные обновления: sha1 → последняя версия (та же — обновления нет).
pub fn apply_updates(
	index: &mut Index,
	latest: &HashMap<String, ContentVersion>,
	checked: &HashSet<String>,
) {
	for (key, entry) in index.files.iter_mut() {
		if !checked.contains(key) {
			continue;
		}
		let Some(source) = &entry.source else {
			continue;
		};
		entry.update = latest
			.get(&entry.sha1)
			.filter(|v| {
				v.id != source.version_id
					&& v.project_id == source.project_id
					&& v.source == source.kind
			})
			.map(|v| UpdateInfo {
				version_id: v.id.clone(),
				version_number: v.version_number.clone(),
				site_only: v.site_only,
			});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::content::FileKey;
	use crate::content::tests::instance;
	use crate::content::{
		BoxFuture, CategoryTag, Dependency, Project, SearchPage, SearchQuery, Side, VersionChannel,
	};
	use std::sync::Mutex;

	fn file(name: &str) -> ContentFile {
		ContentFile {
			url: format!("https://cdn.modrinth.com/{name}"),
			file_name: name.into(),
			sha1: crate::download::hex_sha1(name.as_bytes()),
			size: 0,
			primary: true,
		}
	}

	fn version(project: &str, id: &str, deps: &[(&str, DependencyKind)]) -> ContentVersion {
		let mut v = crate::content::tests::version(id, VersionChannel::Release, "1.20.1", "fabric");
		v.project_id = project.into();
		v.files = vec![file(&format!("{project}-{id}.jar"))];
		v.dependencies = deps
			.iter()
			.map(|(p, kind)| Dependency {
				project_id: Some((*p).into()),
				version_id: None,
				kind: *kind,
			})
			.collect();
		v
	}

	fn hit(id: &str, kind: ProjectType) -> ProjectHit {
		ProjectHit {
			source: SourceKind::Modrinth,
			id: id.into(),
			slug: id.into(),
			project_type: kind,
			title: format!("Title {id}"),
			author: "me".into(),
			description: String::new(),
			icon_url: None,
			downloads: 0,
			follows: None,
			site_only: false,
			url: String::new(),
			categories: vec![],
			updated_at: 0,
			game_versions: vec![],
			loaders: vec![],
			client_side: Side::Unknown,
			server_side: Side::Unknown,
			install: None,
			also_on: Vec::new(),
			created_at: 0,
		}
	}

	/// Каталог в памяти: проекты и их версии (новые первыми).
	#[derive(Default)]
	struct Fake {
		projects: HashMap<String, Vec<ContentVersion>>,
		types: HashMap<String, ProjectType>,
		requests: Mutex<Vec<String>>,
	}

	impl Fake {
		fn add(&mut self, v: ContentVersion) {
			self.projects
				.entry(v.project_id.clone())
				.or_default()
				.push(v);
		}
		fn all(&self) -> impl Iterator<Item = &ContentVersion> {
			self.projects.values().flatten()
		}
	}

	fn no<'a, T: Send + 'a>() -> BoxFuture<'a, T> {
		Box::pin(async { Err(AppError::unknown("нет")) })
	}

	impl ContentSource for Fake {
		fn search<'a>(&'a self, _: &'a SearchQuery) -> BoxFuture<'a, SearchPage> {
			no()
		}
		fn project<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Project> {
			no()
		}
		fn projects<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ProjectHit>> {
			Box::pin(async move {
				Ok(ids
					.iter()
					.filter(|id| self.projects.contains_key(*id))
					.map(|id| hit(id, *self.types.get(id).unwrap_or(&ProjectType::Mod)))
					.collect())
			})
		}
		fn project_versions<'a>(
			&'a self,
			id: &'a str,
			filter: &'a VersionFilter,
		) -> BoxFuture<'a, Vec<ContentVersion>> {
			Box::pin(async move {
				self.requests.lock().unwrap().push(format!("versions {id}"));
				let list = self
					.projects
					.get(id)
					.ok_or_else(|| AppError::new(ErrorCode::ContentNotFound))?;
				Ok(list.iter().filter(|v| filter.matches(v)).cloned().collect())
			})
		}
		fn version<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ContentVersion> {
			Box::pin(async move {
				self.all()
					.find(|v| v.id == id)
					.cloned()
					.ok_or_else(|| AppError::new(ErrorCode::ContentNotFound))
			})
		}
		fn versions<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ContentVersion>> {
			Box::pin(async move {
				Ok(self
					.all()
					.filter(|v| ids.contains(&v.id))
					.cloned()
					.collect())
			})
		}
		fn categories(&self) -> BoxFuture<'_, (Vec<CategoryTag>, bool)> {
			no()
		}
		fn identify<'a>(
			&'a self,
			_: &'a [FileKey],
		) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
			no()
		}
		fn latest<'a>(
			&'a self,
			_: &'a [FileKey],
			_: &'a VersionFilter,
		) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
			no()
		}
	}

	fn request(project: &str) -> InstallRequest {
		serde_json::from_value(serde_json::json!({ "projectId": project })).unwrap()
	}

	fn installed_entry(project: &str, version: &str) -> Entry {
		Entry {
			sha1: "x".into(),
			source: Some(SourceRef {
				kind: SourceKind::Modrinth,
				project_id: project.into(),
				version_id: version.into(),
				title: None,
				icon_url: None,
				version_number: None,
				author: None,
			}),
			..Entry::default()
		}
	}

	#[tokio::test]
	async fn resolves_dependencies_and_conflicts() {
		use DependencyKind::*;
		let mut fake = Fake::default();
		fake.add(version(
			"sodium",
			"s2",
			&[
				("fapi", Required),
				("indium", Optional),
				("optifine", Incompatible),
				("emb", Embedded),
			],
		));
		fake.add(version(
			"fapi",
			"f1",
			&[("lib", Required), ("sodium", Required)],
		));
		fake.add(version("lib", "l1", &[("gone", Required)]));
		fake.add(version("indium", "i1", &[]));
		fake.add(version("optifine", "o1", &[]));
		let mut forge_only = version("forgeonly", "x1", &[]);
		forge_only.loaders = vec!["forge".into()];
		fake.add(forge_only);

		let dir = tempfile::tempdir().unwrap();
		let fabric = instance(LoaderKind::Fabric, "1.20.1");
		let mut index = Index::default();
		let plan = resolve(&fake, &fabric, dir.path(), &index, &request("sodium"))
			.await
			.unwrap();
		assert_eq!(plan.target.key(), "mods/sodium-s2.jar");
		assert!(!plan.already_installed && plan.conflicts.is_empty());
		let deps: Vec<&str> = plan.required.iter().map(|d| d.hit.id.as_str()).collect();
		assert_eq!(deps, vec!["fapi", "lib"]);
		assert!(plan.required.iter().all(|d| d.dependency));
		assert_eq!(plan.optional.len(), 1);
		assert_eq!(plan.optional[0].title, "Title indium");
		assert_eq!(plan.missing.len(), 1);
		assert_eq!(plan.missing[0].project_id, "gone");
		assert_eq!(plan.items().len(), 3);

		// Выбранная необязательная зависимость ставится.
		let mut req = request("sodium");
		req.optional = vec!["indium".into()];
		let plan = resolve(&fake, &fabric, dir.path(), &index, &req)
			.await
			.unwrap();
		assert!(plan.required.iter().any(|d| d.hit.id == "indium"));
		assert!(plan.optional.is_empty());

		// Уже стоят: fapi (не ставится снова), optifine (конфликт), старая версия sodium (заменяется).
		index
			.files
			.insert("mods/fapi.jar".into(), installed_entry("fapi", "f0"));
		index
			.files
			.insert("mods/of.jar".into(), installed_entry("optifine", "o1"));
		index.files.insert(
			"mods/sodium-old.jar".into(),
			installed_entry("sodium", "s1"),
		);
		let plan = resolve(&fake, &fabric, dir.path(), &index, &request("sodium"))
			.await
			.unwrap();
		assert!(plan.required.is_empty());
		assert_eq!(plan.target.replaces.as_deref(), Some("mods/sodium-old.jar"));
		assert_eq!(plan.conflicts, vec!["mods/of.jar".to_owned()]);

		// Та же версия тем же файлом — уже установлено.
		index
			.files
			.insert("mods/sodium-s2.jar".into(), installed_entry("sodium", "s2"));
		index.files.remove("mods/sodium-old.jar");
		let plan = resolve(&fake, &fabric, dir.path(), &index, &request("sodium"))
			.await
			.unwrap();
		assert!(plan.already_installed && plan.items().is_empty());

		// Нет версии под загрузчик; моды в ванильную сборку; чужая версия; нет проекта; нет мира.
		let err = resolve(&fake, &fabric, dir.path(), &index, &request("forgeonly"))
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::ContentIncompatible);
		let vanilla = instance(LoaderKind::Vanilla, "1.20.1");
		let err = resolve(&fake, &vanilla, dir.path(), &index, &request("sodium"))
			.await
			.unwrap_err();
		assert_eq!(
			(err.code, err.details.as_deref()),
			(ErrorCode::ContentIncompatible, Some("needs_loader"))
		);
		let mut req = request("sodium");
		req.version_id = Some("f1".into());
		let err = resolve(&fake, &fabric, dir.path(), &index, &req)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::InvalidValue);
		let err = resolve(&fake, &fabric, dir.path(), &index, &request("none"))
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::ContentNotFound);
		let mut req = request("sodium");
		req.project_type = Some(ProjectType::Datapack);
		req.world = Some("Нет такого".into());
		let err = resolve(&fake, &fabric, dir.path(), &index, &req)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::FileNotFound);
	}

	#[tokio::test]
	async fn shader_brings_its_loader() {
		let mut fake = Fake::default();
		let mut shader = version("bsl", "b1", &[]);
		shader.loaders = vec!["iris".into()];
		shader.files = vec![file("bsl.zip")];
		fake.add(shader);
		fake.types.insert("bsl".into(), ProjectType::Shader);
		fake.add(version(crate::content::modrinth::IRIS_ID, "iris1", &[]));
		let dir = tempfile::tempdir().unwrap();
		let fabric = instance(LoaderKind::Fabric, "1.20.1");
		let plan = resolve(
			&fake,
			&fabric,
			dir.path(),
			&Index::default(),
			&request("bsl"),
		)
		.await
		.unwrap();
		assert_eq!(plan.target.key(), "shaderpacks/bsl.zip");
		assert_eq!(plan.required.len(), 1);
		assert_eq!(
			plan.required[0].key(),
			format!("mods/{}-iris1.jar", crate::content::modrinth::IRIS_ID)
		);

		// Iris уже стоит — ничего не добавляется.
		let mut index = Index::default();
		index.files.insert(
			"mods/iris.jar".into(),
			installed_entry(crate::content::modrinth::IRIS_ID, "iris0"),
		);
		let plan = resolve(&fake, &fabric, dir.path(), &index, &request("bsl"))
			.await
			.unwrap();
		assert!(plan.required.is_empty());
	}

	#[test]
	fn applies_install_and_identification() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path();
		let staging = game.join(".content-tmp-1");
		std::fs::create_dir_all(game.join("mods")).unwrap();
		std::fs::create_dir_all(&staging).unwrap();
		std::fs::write(staging.join("0"), b"new").unwrap();
		std::fs::write(game.join("mods/of.jar"), b"of").unwrap();
		let mut index = Index::default();
		let item = planned(
			&hit("a", ProjectType::Mod),
			ProjectType::Mod,
			&version("a", "2", &[]),
			"mods",
			true,
		)
		.unwrap();
		apply(
			game,
			&staging,
			&mut index,
			&[item],
			&["mods/of.jar".to_owned()],
		)
		.unwrap();
		assert!(game.join("mods/a-2.jar").exists());
		assert!(game.join("mods/of.jar.disabled").exists() && !game.join("mods/of.jar").exists());
		let entry = &index.files["mods/a-2.jar"];
		assert!(entry.dependency);
		assert_eq!(entry.size, 3);
		let src = entry.source.as_ref().unwrap();
		assert_eq!(
			(src.version_id.as_str(), src.author.as_deref()),
			("2", Some("me"))
		);

		// Расширение файла не по типу и небезопасное имя не ставятся.
		let mut bad = version("a", "3", &[]);
		bad.files[0].file_name = "../x.jar".into();
		assert!(
			planned(
				&hit("a", ProjectType::Mod),
				ProjectType::Mod,
				&bad,
				"mods",
				false
			)
			.is_err()
		);
		bad.files[0].file_name = "x.zip".into();
		assert!(
			planned(
				&hit("a", ProjectType::Mod),
				ProjectType::Mod,
				&bad,
				"mods",
				false
			)
			.is_err()
		);

		let sha1 = index.files["mods/a-2.jar"].sha1.clone();
		let mut latest = HashMap::new();
		latest.insert(sha1, version("a", "3", &[]));
		let checked = HashSet::from(["mods/a-2.jar".to_owned()]);
		apply_updates(&mut index, &latest, &checked);
		assert_eq!(
			index.files["mods/a-2.jar"]
				.update
				.as_ref()
				.unwrap()
				.version_id,
			"3"
		);
		apply_updates(&mut index, &HashMap::new(), &checked);
		assert!(index.files["mods/a-2.jar"].update.is_none());

		let unknown = |sha1: &str| Entry {
			sha1: sha1.into(),
			..Entry::default()
		};
		index.files.insert("mods/u.jar".into(), unknown("s-known"));
		index
			.files
			.insert("mods/v.jar".into(), unknown("s-unknown"));
		let mut found = HashMap::new();
		found.insert("s-known".to_owned(), version("b", "b1", &[]));
		let hits = HashMap::from([(
			(SourceKind::Modrinth, "b".to_owned()),
			hit("b", ProjectType::Mod),
		)]);
		let asked = vec!["s-known".to_owned(), "s-unknown".to_owned()];
		let fps = HashMap::from([("s-unknown".to_owned(), 42u32)]);
		assert!(apply_identified(
			&mut index, &asked, &found, &hits, &fps, 77
		));
		assert_eq!(index.files["mods/v.jar"].fingerprint, Some(42));
		let u = index.files["mods/u.jar"].source.as_ref().unwrap();
		assert_eq!(
			(u.project_id.as_str(), u.title.as_deref()),
			("b", Some("Title b"))
		);
		assert_eq!(index.files["mods/v.jar"].checked_at, Some(77));
	}

	#[tokio::test]
	async fn plans_updates() {
		let mut fake = Fake::default();
		fake.add(version("a", "2", &[]));
		let mut index = Index::default();
		let mut e = installed_entry("a", "1");
		e.update = Some(UpdateInfo {
			version_id: "2".into(),
			version_number: "2".into(),
			site_only: false,
		});
		index.files.insert("mods/a-1.jar".into(), e);
		index
			.files
			.insert("mods/b.jar".into(), installed_entry("b", "1"));
		let items = plan_update(&fake, SourceKind::Modrinth, &index, None)
			.await
			.unwrap();
		assert_eq!(items.len(), 1);
		assert_eq!(items[0].replaces.as_deref(), Some("mods/a-1.jar"));
		assert_eq!(items[0].key(), "mods/a-2.jar");
		let none = plan_update(
			&fake,
			SourceKind::Modrinth,
			&index,
			Some(&["mods/b.jar".to_owned()]),
		)
		.await
		.unwrap();
		assert!(none.is_empty());
	}
}
