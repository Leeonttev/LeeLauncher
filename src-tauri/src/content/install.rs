//! Установка и обновление контента: план (версия, зависимости, конфликты), скачивание, запись в индекс.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::installed::{self, Entry, Index, SourceRef, UpdateInfo};
use super::{
	ContentFile, ContentSource, ContentVersion, DependencyKind, FileKey, ProjectHit, ProjectType,
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
	/// Зависимости, которые уже стоят из другого каталога или файлом: не ставятся.
	pub elsewhere: Vec<InstalledElsewhere>,
}

/// Зависимость, которая уже есть в сборке из другого каталога (или добавлена файлом).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledElsewhere {
	/// Зависимость в каталоге установки.
	pub project: ProjectHit,
	/// Откуда стоит копия; `None` — файл без известного каталога.
	pub installed_from: Option<SourceKind>,
	/// Ключ установленного файла.
	pub file: String,
	pub optional: bool,
}

/// Что уже стоит в сборке: по каталогу установки, по индексу и по id модов в jar.
struct Local<'a> {
	installed: &'a HashMap<(SourceKind, String), String>,
	index: &'a Index,
	mod_ids: &'a HashMap<String, Vec<String>>,
}

impl Local<'_> {
	/// Файл того же проекта из другого каталога: id мода совпал со slug зависимости
	/// или совпали slug/название/автор записи индекса.
	fn elsewhere(
		&self,
		hit: &ProjectHit,
		kind: ProjectType,
		folder: &str,
	) -> Option<(String, Option<SourceKind>)> {
		let slug = mod_id_norm(&hit.slug);
		self.index.files.iter().find_map(|(key, entry)| {
			if key.rsplit_once('/').map(|(f, _)| f) != Some(folder) {
				return None;
			}
			let from = entry.source.as_ref().map(|s| s.kind);
			if from == Some(hit.source) {
				return None;
			}
			let by_id = kind == ProjectType::Mod
				&& !slug.is_empty()
				&& self
					.mod_ids
					.get(key)
					.is_some_and(|ids| ids.iter().any(|id| mod_id_norm(id) == slug));
			let by_meta = entry.source.as_ref().is_some_and(|s| same_meta(s, hit));
			(by_id || by_meta).then(|| (key.clone(), from))
		})
	}
}

/// id мода и slug сравниваются без регистра, `_` и `-` — одно и то же.
fn mod_id_norm(id: &str) -> String {
	id.trim().to_ascii_lowercase().replace('_', "-")
}

/// Правило склейки из поиска «Все», но для записи индекса.
fn same_meta(s: &SourceRef, hit: &ProjectHit) -> bool {
	let filled = |a: Option<&str>, b: &str| {
		let a = super::search_all::norm(a.unwrap_or_default());
		!a.is_empty() && a == super::search_all::norm(b)
	};
	let slug = s
		.slug
		.as_deref()
		.is_some_and(|x| !x.is_empty() && x.eq_ignore_ascii_case(&hit.slug));
	let title = filled(s.title.as_deref(), &hit.title);
	let author = filled(s.author.as_deref(), &hit.author);
	(slug && (title || author)) || (title && author)
}

/// id модов в jar из `mods/` (ключ индекса → id). Читает файлы — вызывать вне async.
pub fn local_mod_ids(game_dir: &Path, index: &Index) -> HashMap<String, Vec<String>> {
	index
		.files
		.keys()
		.filter(|k| k.starts_with("mods/") && k.to_ascii_lowercase().ends_with(".jar"))
		.filter_map(|key| {
			let path = [true, false]
				.into_iter()
				.filter_map(|on| installed::disk_path(game_dir, key, on).ok())
				.find(|p| p.is_file())?;
			let ids = installed::mod_ids(&path);
			(!ids.is_empty()).then(|| (key.clone(), ids))
		})
		.collect()
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
	mod_ids: &HashMap<String, Vec<String>>,
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
		elsewhere: Vec::new(),
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
	let local = Local {
		installed: &installed,
		index,
		mod_ids,
	};
	resolve_dependencies(source, instance, &local, &root, req, &mut plan).await?;
	Ok(plan)
}

async fn resolve_dependencies(
	source: &dyn ContentSource,
	instance: &Instance,
	local: &Local<'_>,
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
			let installed_key = local.installed.get(&(req.source, project.clone()));
			// Тот же проект из другого каталога считается установленным.
			let elsewhere = match (installed_key, hits.get(&project)) {
				(None, Some(hit)) => {
					let dep_kind = dependency_type(hit);
					folder_for(dep_kind, req.world.as_deref())
						.ok()
						.and_then(|folder| local.elsewhere(hit, dep_kind, &folder))
						.map(|(file, from)| InstalledElsewhere {
							project: hit.clone(),
							installed_from: from,
							file,
							optional: kind == DependencyKind::Optional,
						})
				}
				_ => None,
			};
			match kind {
				DependencyKind::Incompatible => {
					let key = installed_key.or(elsewhere.as_ref().map(|e| &e.file));
					if let Some(key) = key
						&& !plan.conflicts.contains(key)
						&& plan.target.replaces.as_ref() != Some(key)
					{
						plan.conflicts.push(key.clone());
					}
				}
				DependencyKind::Optional => {
					if installed_key.is_some() || seen.contains(&project) {
						continue;
					}
					if !optional_seen.insert(project.clone()) {
						continue;
					}
					if let Some(found) = elsewhere {
						plan.elsewhere.push(found);
					} else if let Some(hit) = hits.get(&project) {
						plan.optional.push(hit.clone());
					}
				}
				DependencyKind::Required => {
					if installed_key.is_some() || !seen.insert(project.clone()) {
						continue;
					}
					if let Some(found) = elsewhere {
						plan.elsewhere.retain(|e| e.project.id != found.project.id);
						plan.elsewhere.push(found);
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
	plan.elsewhere
		.retain(|e| !required.contains(&e.project.id) && e.project.id != target);
	Ok(())
}

/// План обновления по найденным обновлениям (`instance_content_updates`). `keys` — какие файлы,
/// `None` — все с обновлением. Берутся файлы, чьё обновление ставится из каталога `kind`: своего
/// или найденного подстраховкой (`via`). Сеть: версии пачкой, проекты подстраховки пачкой.
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
			let s = e.source.as_ref()?;
			let u = e.update.as_ref().filter(|u| u.catalog(s) == kind)?;
			Some((key, e, s, u))
		})
		.collect();
	if chosen.is_empty() {
		return Ok(Vec::new());
	}
	let ids: Vec<String> = chosen.iter().map(|(.., u)| u.version_id.clone()).collect();
	let mut foreign: Vec<String> = chosen
		.iter()
		.filter(|(.., u)| u.via.is_some())
		.filter_map(|(.., u)| u.via_project_id.clone())
		.collect();
	foreign.sort();
	foreign.dedup();
	// Своим проектам без настоящего slug он берётся из каталога; не ответил — slug пустой.
	let mut unslugged: Vec<String> = chosen
		.iter()
		.filter(|(.., s, u)| u.via.is_none() && !has_slug(s))
		.map(|(.., s, _)| s.project_id.clone())
		.collect();
	unslugged.sort();
	unslugged.dedup();
	let (versions, hits, own) = tokio::join!(
		source.versions(&ids),
		project_hits(source, &foreign),
		project_hits(source, &unslugged)
	);
	let versions: HashMap<String, ContentVersion> =
		versions?.into_iter().map(|v| (v.id.clone(), v)).collect();
	let hits = hits?;
	let own = own.unwrap_or_else(|err| {
		log::warn!("slug проектов обновления: {:?}", err.code);
		HashMap::new()
	});
	let mut items = Vec::new();
	for (key, entry, src, update) in chosen {
		let Some(version) = versions.get(&update.version_id) else {
			continue;
		};
		let (project_type, _) = installed::parse_key(key)?;
		let hit = match (update.via, update.via_project_id.as_ref()) {
			(Some(via), Some(id)) => hits
				.get(id)
				.cloned()
				.map(|mut h| {
					h.project_type = project_type;
					h
				})
				.unwrap_or_else(|| update_hit(via, id, None, src, project_type)),
			(Some(_), None) => continue,
			(None, _) => {
				let slug = if has_slug(src) {
					src.slug.as_deref()
				} else {
					own.get(&src.project_id)
						.map(|h| h.slug.as_str())
						.filter(|s| !s.is_empty())
				};
				update_hit(src.kind, &src.project_id, slug, src, project_type)
			}
		};
		match planned_replacement(&hit, project_type, version, key, entry) {
			Ok(item) => items.push(item),
			Err(err) => log::warn!("обновление {key} пропущено: {:?}", err.details),
		}
	}
	Ok(items)
}

/// Настоящий slug: не пустой и не id проекта (так его записывали старые обновления).
fn has_slug(s: &SourceRef) -> bool {
	s.slug
		.as_deref()
		.is_some_and(|x| !x.is_empty() && x != s.project_id)
}

/// Проекты по id одной пачкой (пусто — без запроса).
async fn project_hits(
	source: &dyn ContentSource,
	ids: &[String],
) -> Result<HashMap<String, ProjectHit>, AppError> {
	if ids.is_empty() {
		return Ok(HashMap::new());
	}
	Ok(source
		.projects(ids)
		.await?
		.into_iter()
		.map(|h| (h.id.clone(), h))
		.collect())
}

/// Проект обновления из того, что известно по индексу (каталог проект не отдал).
fn update_hit(
	kind: SourceKind,
	id: &str,
	slug: Option<&str>,
	src: &SourceRef,
	project_type: ProjectType,
) -> ProjectHit {
	ProjectHit {
		source: kind,
		id: id.to_owned(),
		slug: slug.unwrap_or_default().to_owned(),
		project_type,
		title: src.title.clone().unwrap_or_else(|| id.to_owned()),
		author: src.author.clone().unwrap_or_default(),
		description: String::new(),
		icon_url: src.icon_url.clone(),
		downloads: 0,
		follows: None,
		site_only: false,
		url: super::project_url(kind, id),
		categories: Vec::new(),
		updated_at: 0,
		game_versions: Vec::new(),
		loaders: Vec::new(),
		client_side: super::Side::Unknown,
		server_side: super::Side::Unknown,
		install: None,
		also_on: Vec::new(),
		created_at: 0,
	}
}

/// Новая версия на место установленного файла `key`.
fn planned_replacement(
	hit: &ProjectHit,
	project_type: ProjectType,
	version: &ContentVersion,
	key: &str,
	entry: &Entry,
) -> Result<PlannedItem, AppError> {
	let mut hit = hit.clone();
	hit.site_only = version.site_only;
	let folder = key.rsplit_once('/').map(|(f, _)| f).unwrap_or_default();
	let mut item = planned(&hit, project_type, version, folder, entry.dependency)?;
	item.replaces = Some(key.to_owned());
	Ok(item)
}

/// Файл не ответившего каталога в другом: версия там сейчас и последняя подходящая
/// (`None` — подходящей для сборки нет).
#[derive(Debug, Clone)]
pub struct Covered {
	pub current: ContentVersion,
	pub latest: Option<ContentVersion>,
}

impl Covered {
	/// Обновление оттуда, если последняя версия не та, что стоит.
	pub fn update(&self) -> Option<UpdateInfo> {
		let v = self.newer()?;
		Some(UpdateInfo {
			version_id: v.id.clone(),
			version_number: v.version_number.clone(),
			site_only: v.site_only,
			via: Some(v.source),
			via_project_id: Some(v.project_id.clone()),
		})
	}

	fn newer(&self) -> Option<&ContentVersion> {
		self.latest.as_ref().filter(|v| v.id != self.current.id)
	}
}

/// Подстраховка: файлы не ответившего каталога (ключ индекса и [`FileKey`] с sha1 и отпечатком)
/// опознаются в `alt` и проверяются там же — по запросу пачкой на опознание и на последние версии
/// каждого типа. Файлов, которых в `alt` нет, в ответе нет.
pub async fn fallback_latest(
	alt: &dyn ContentSource,
	instance: &Instance,
	files: &[(String, FileKey)],
) -> Result<HashMap<String, Covered>, AppError> {
	if files.is_empty() {
		return Ok(HashMap::new());
	}
	let keys: Vec<FileKey> = files
		.iter()
		.map(|(_, k)| FileKey {
			project_id: None,
			..k.clone()
		})
		.collect();
	let found = alt.identify(&keys).await?;
	let mut groups: HashMap<ProjectType, Vec<FileKey>> = HashMap::new();
	for (key, k) in files {
		let (Some(v), Ok((kind, _))) = (found.get(&k.sha1), installed::parse_key(key)) else {
			continue;
		};
		groups.entry(kind).or_default().push(FileKey {
			project_id: Some(v.project_id.clone()),
			..k.clone()
		});
	}
	let mut latest = HashMap::new();
	for (kind, keys) in groups {
		let filter = update_filter(instance, kind);
		latest.extend(alt.latest(&keys, &filter).await?);
	}
	Ok(files
		.iter()
		.filter_map(|(key, k)| {
			let current = found.get(&k.sha1)?.clone();
			let latest = latest
				.get(&k.sha1)
				.filter(|v| v.project_id == current.project_id && v.source == current.source)
				.cloned();
			Some((key.clone(), Covered { current, latest }))
		})
		.collect())
}

/// План обновления через подстраховку: что в `alt` новее, ставится оттуда (источник файла
/// меняется на `alt`). Второе значение — сколько файлов нашлось и проверено в `alt`.
pub async fn plan_fallback(
	alt: &dyn ContentSource,
	instance: &Instance,
	index: &Index,
	files: &[(String, FileKey)],
) -> Result<(Vec<PlannedItem>, usize), AppError> {
	let covered = fallback_latest(alt, instance, files).await?;
	let mut ids: Vec<String> = covered
		.values()
		.filter_map(|c| Some(c.newer()?.project_id.clone()))
		.collect();
	ids.sort();
	ids.dedup();
	let hits = project_hits(alt, &ids).await?;
	let mut items = Vec::new();
	for (key, _) in files {
		let Some(version) = covered.get(key).and_then(Covered::newer) else {
			continue;
		};
		let Some((entry, src)) = index
			.files
			.get(key)
			.and_then(|e| Some((e, e.source.as_ref()?)))
		else {
			continue;
		};
		let (project_type, _) = installed::parse_key(key)?;
		let hit = hits
			.get(&version.project_id)
			.cloned()
			.map(|mut h| {
				h.project_type = project_type;
				h
			})
			.unwrap_or_else(|| {
				update_hit(version.source, &version.project_id, None, src, project_type)
			});
		match planned_replacement(&hit, project_type, version, key, entry) {
			Ok(item) => items.push(item),
			Err(err) => log::warn!("обновление {key} пропущено: {:?}", err.details),
		}
	}
	Ok((items, covered.len()))
}

/// Записывает итог подстраховки: ключ → обновление из другого каталога (`None` — новее там нет).
/// Новые отпечатки CurseForge запоминаются.
pub fn apply_fallback(
	index: &mut Index,
	covered: &HashMap<String, Option<UpdateInfo>>,
	fingerprints: &HashMap<String, u32>,
) {
	for (key, entry) in index.files.iter_mut() {
		if entry.fingerprint.is_none()
			&& let Some(fp) = fingerprints.get(&entry.sha1)
		{
			entry.fingerprint = Some(*fp);
		}
		if entry.source.is_none() {
			continue;
		}
		match covered.get(key) {
			Some(Some(update)) => entry.update = Some(update.clone()),
			// Во втором новее нет: прежнее обновление оттуда устарело, своё не трогаем.
			Some(None) if entry.update.as_ref().is_some_and(|u| u.via.is_some()) => {
				entry.update = None;
			}
			_ => {}
		}
	}
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
					slug: Some(item.hit.slug.clone()).filter(|s| !s.is_empty()),
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
					slug: None,
				});
				entry.checked_at = None;
				changed = true;
			} else if asked.contains(&entry.sha1) {
				entry.checked_at = Some(now);
				changed = true;
			}
		}
		if let Some(source) = entry.source.as_mut()
			&& let Some(hit) = hits.get(&(source.kind, source.project_id.clone()))
		{
			if source.title.is_none() || source.author.is_none() {
				source.title = Some(hit.title.clone());
				source.icon_url = hit.icon_url.clone();
				source.author = Some(hit.author.clone());
				changed = true;
			}
			// Старые обновления писали в slug id проекта — заменяется настоящим.
			if !has_slug(source)
				&& !hit.slug.is_empty()
				&& source.slug.as_deref() != Some(&hit.slug)
			{
				source.slug = Some(hit.slug.clone());
				changed = true;
			}
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
				via: None,
				via_project_id: None,
			});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
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
		source: SourceKind,
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
					.map(|id| {
						let mut h = hit(id, *self.types.get(id).unwrap_or(&ProjectType::Mod));
						h.source = self.source;
						h
					})
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
				slug: None,
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
		let plan = resolve(
			&fake,
			&fabric,
			dir.path(),
			&index,
			&HashMap::new(),
			&request("sodium"),
		)
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
		let plan = resolve(&fake, &fabric, dir.path(), &index, &HashMap::new(), &req)
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
		let plan = resolve(
			&fake,
			&fabric,
			dir.path(),
			&index,
			&HashMap::new(),
			&request("sodium"),
		)
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
		let plan = resolve(
			&fake,
			&fabric,
			dir.path(),
			&index,
			&HashMap::new(),
			&request("sodium"),
		)
		.await
		.unwrap();
		assert!(plan.already_installed && plan.items().is_empty());

		// Нет версии под загрузчик; моды в ванильную сборку; чужая версия; нет проекта; нет мира.
		let err = resolve(
			&fake,
			&fabric,
			dir.path(),
			&index,
			&HashMap::new(),
			&request("forgeonly"),
		)
		.await
		.unwrap_err();
		assert_eq!(err.code, ErrorCode::ContentIncompatible);
		let vanilla = instance(LoaderKind::Vanilla, "1.20.1");
		let err = resolve(
			&fake,
			&vanilla,
			dir.path(),
			&index,
			&HashMap::new(),
			&request("sodium"),
		)
		.await
		.unwrap_err();
		assert_eq!(
			(err.code, err.details.as_deref()),
			(ErrorCode::ContentIncompatible, Some("needs_loader"))
		);
		let mut req = request("sodium");
		req.version_id = Some("f1".into());
		let err = resolve(&fake, &fabric, dir.path(), &index, &HashMap::new(), &req)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::InvalidValue);
		let err = resolve(
			&fake,
			&fabric,
			dir.path(),
			&index,
			&HashMap::new(),
			&request("none"),
		)
		.await
		.unwrap_err();
		assert_eq!(err.code, ErrorCode::ContentNotFound);
		let mut req = request("sodium");
		req.project_type = Some(ProjectType::Datapack);
		req.world = Some("Нет такого".into());
		let err = resolve(&fake, &fabric, dir.path(), &index, &HashMap::new(), &req)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::FileNotFound);
	}

	fn jar(path: &Path, mod_id: &str) {
		use std::io::Write;
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
		zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
			.unwrap();
		write!(zip, r#"{{"id":"{mod_id}"}}"#).unwrap();
		zip.finish().unwrap();
	}

	#[tokio::test]
	async fn dependency_from_other_catalog_counts_as_installed() {
		use DependencyKind::*;
		let mut fake = Fake {
			source: SourceKind::Curseforge,
			..Fake::default()
		};
		fake.add(version(
			"create-fabric",
			"c1",
			&[
				("fabric-api", Required),
				("cloth-config", Required),
				("sodium", Required),
				("modmenu", Optional),
			],
		));
		fake.add(version("fabric-api", "f1", &[]));
		fake.add(version("cloth-config", "cc1", &[]));
		fake.add(version("sodium", "s1", &[]));
		fake.add(version("modmenu", "mm1", &[]));

		let dir = tempfile::tempdir().unwrap();
		let fabric = instance(LoaderKind::Fabric, "1.20.1");
		let mut index = Index::default();
		// Fabric API с Modrinth старой записью (без slug и названия): узнаётся по id мода в jar.
		jar(&dir.path().join("mods/fabric-api-0.92.jar"), "fabric-api");
		index.files.insert(
			"mods/fabric-api-0.92.jar".into(),
			installed_entry("P7dR8mSH", "mr1"),
		);
		// Cloth Config с Modrinth: slug и автор совпали.
		let mut cloth = installed_entry("9s6osm5g", "mr2");
		if let Some(s) = cloth.source.as_mut() {
			s.slug = Some("cloth-config".into());
			s.title = Some("Cloth Config v13".into());
			s.author = Some("me".into());
		}
		index.files.insert("mods/cloth-config.jar".into(), cloth);
		// Только slug — мало: Sodium ставится.
		let mut other = installed_entry("AANobbMI", "mr3");
		if let Some(s) = other.source.as_mut() {
			s.slug = Some("sodium".into());
			s.title = Some("Another".into());
			s.author = Some("someone".into());
		}
		index.files.insert("mods/other.jar".into(), other);
		// Mod Menu добавлен файлом.
		jar(&dir.path().join("mods/modmenu.jar"), "modmenu");
		index
			.files
			.insert("mods/modmenu.jar".into(), Entry::default());

		let mod_ids = local_mod_ids(dir.path(), &index);
		let req: InstallRequest = serde_json::from_value(serde_json::json!({
			"projectId": "create-fabric",
			"source": "curseforge",
		}))
		.unwrap();
		let plan = resolve(&fake, &fabric, dir.path(), &index, &mod_ids, &req)
			.await
			.unwrap();
		let required: Vec<&str> = plan.required.iter().map(|i| i.hit.id.as_str()).collect();
		assert_eq!(required, ["sodium"]);
		assert!(plan.optional.is_empty());
		let elsewhere: Vec<(&str, Option<SourceKind>, &str, bool)> = plan
			.elsewhere
			.iter()
			.map(|e| {
				(
					e.project.id.as_str(),
					e.installed_from,
					e.file.as_str(),
					e.optional,
				)
			})
			.collect();
		assert_eq!(
			elsewhere,
			[
				(
					"fabric-api",
					Some(SourceKind::Modrinth),
					"mods/fabric-api-0.92.jar",
					false
				),
				(
					"cloth-config",
					Some(SourceKind::Modrinth),
					"mods/cloth-config.jar",
					false
				),
				("modmenu", None, "mods/modmenu.jar", true),
			]
		);
		let asked = fake.requests.lock().unwrap().clone();
		assert!(!asked.iter().any(|r| r.contains("fabric-api")));

		// Из того же каталога копия не «другая»: Modrinth-установка видит её как обычную.
		let mr = Fake {
			source: SourceKind::Modrinth,
			..std::mem::take(&mut fake)
		};
		let mut req = req;
		req.source = SourceKind::Modrinth;
		let plan = resolve(&mr, &fabric, dir.path(), &index, &mod_ids, &req)
			.await
			.unwrap();
		assert!(
			plan.elsewhere
				.iter()
				.all(|e| e.project.id != "cloth-config")
		);
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
			&HashMap::new(),
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
		let plan = resolve(
			&fake,
			&fabric,
			dir.path(),
			&index,
			&HashMap::new(),
			&request("bsl"),
		)
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
			via: None,
			via_project_id: None,
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

	/// Второй каталог для подстраховки (как CurseForge): узнаёт файл по отпечатку и sha1,
	/// последние версии — по проекту с учётом фильтра сборки.
	#[derive(Default)]
	struct Alt {
		/// Отпечаток, sha1 и версия, что сейчас стоит.
		files: Vec<(u32, String, ContentVersion)>,
		/// Версии проектов, новые первыми.
		projects: HashMap<String, Vec<ContentVersion>>,
		calls: Mutex<Vec<&'static str>>,
	}

	impl ContentSource for Alt {
		fn search<'a>(&'a self, _: &'a SearchQuery) -> BoxFuture<'a, SearchPage> {
			no()
		}
		fn project<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Project> {
			no()
		}
		fn projects<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ProjectHit>> {
			Box::pin(async move {
				self.calls.lock().unwrap().push("projects");
				Ok(ids
					.iter()
					.filter(|id| self.projects.contains_key(*id))
					.map(|id| {
						let mut h = hit(id, ProjectType::Mod);
						h.source = SourceKind::Curseforge;
						h.slug = format!("slug-{id}");
						h
					})
					.collect())
			})
		}
		fn project_versions<'a>(
			&'a self,
			_: &'a str,
			_: &'a VersionFilter,
		) -> BoxFuture<'a, Vec<ContentVersion>> {
			no()
		}
		fn version<'a>(&'a self, _: &'a str) -> BoxFuture<'a, ContentVersion> {
			no()
		}
		fn versions<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ContentVersion>> {
			Box::pin(async move {
				Ok(self
					.projects
					.values()
					.flatten()
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
			keys: &'a [FileKey],
		) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
			Box::pin(async move {
				self.calls.lock().unwrap().push("identify");
				Ok(keys
					.iter()
					.filter_map(|k| {
						let (_, _, v) = self
							.files
							.iter()
							.find(|(fp, sha1, _)| Some(*fp) == k.fingerprint && *sha1 == k.sha1)?;
						Some((k.sha1.clone(), v.clone()))
					})
					.collect())
			})
		}
		fn latest<'a>(
			&'a self,
			keys: &'a [FileKey],
			filter: &'a VersionFilter,
		) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
			Box::pin(async move {
				self.calls.lock().unwrap().push("latest");
				Ok(keys
					.iter()
					.filter_map(|k| {
						let list = self.projects.get(k.project_id.as_deref()?)?;
						let v = list.iter().find(|v| filter.matches(v))?;
						Some((k.sha1.clone(), v.clone()))
					})
					.collect())
			})
		}
	}

	fn cf_version(project: &str, id: &str, game: &str) -> ContentVersion {
		let mut v = version(project, id, &[]);
		v.source = SourceKind::Curseforge;
		v.game_versions = vec![game.into()];
		v.files[0].url = format!("https://edge.forgecdn.net/{id}.jar");
		v
	}

	/// Файл с Modrinth `mods/a-1.jar`, его отпечаток 7; на CurseForge он — проект 100.
	fn fallback_setup() -> (Alt, Index, Vec<(String, FileKey)>) {
		let sha1 = "aa".repeat(20);
		let mut alt = Alt::default();
		alt.files
			.push((7, sha1.clone(), cf_version("100", "cf1", "1.20.1")));
		alt.projects.insert(
			"100".into(),
			vec![
				cf_version("100", "cf3", "1.21.1"),
				cf_version("100", "cf2", "1.20.1"),
				cf_version("100", "cf1", "1.20.1"),
			],
		);
		let mut index = Index::default();
		let mut e = installed_entry("a", "1");
		e.sha1 = sha1.clone();
		e.update = Some(UpdateInfo {
			version_id: "2".into(),
			version_number: "2".into(),
			site_only: false,
			via: None,
			via_project_id: None,
		});
		index.files.insert("mods/a-1.jar".into(), e);
		let files = vec![(
			"mods/a-1.jar".to_owned(),
			FileKey {
				sha1,
				fingerprint: Some(7),
				project_id: None,
			},
		)];
		(alt, index, files)
	}

	#[tokio::test]
	async fn fallback_finds_update_on_other_catalog() {
		let (alt, mut index, files) = fallback_setup();
		let inst = instance(LoaderKind::Fabric, "1.20.1");
		let covered = fallback_latest(&alt, &inst, &files).await.unwrap();
		// Одна пачка на опознание и одна на последние версии.
		assert_eq!(*alt.calls.lock().unwrap(), ["identify", "latest"]);
		// cf3 — для другой версии игры: обновление — cf2.
		let update = covered["mods/a-1.jar"].update().unwrap();
		assert_eq!(update.version_id, "cf2");
		assert_eq!(update.via, Some(SourceKind::Curseforge));
		assert_eq!(update.via_project_id.as_deref(), Some("100"));

		let found = HashMap::from([("mods/a-1.jar".to_owned(), Some(update))]);
		apply_fallback(&mut index, &found, &HashMap::from([("aa".repeat(20), 7)]));
		let entry = &index.files["mods/a-1.jar"];
		assert_eq!(entry.fingerprint, Some(7));
		assert_eq!(
			entry.update.as_ref().unwrap().via,
			Some(SourceKind::Curseforge)
		);
		// Свой источник у файла прежний.
		assert_eq!(entry.source.as_ref().unwrap().kind, SourceKind::Modrinth);

		// Обновление ставит второй каталог, у Modrinth в плане его нет.
		let modrinth = Fake::default();
		let none = plan_update(&modrinth, SourceKind::Modrinth, &index, None)
			.await
			.unwrap();
		assert!(none.is_empty());
		let items = plan_update(&alt, SourceKind::Curseforge, &index, None)
			.await
			.unwrap();
		assert_eq!(items.len(), 1);
		assert_eq!(items[0].hit.source, SourceKind::Curseforge);
		assert_eq!(items[0].hit.id, "100");
		assert_eq!(items[0].replaces.as_deref(), Some("mods/a-1.jar"));

		// Установка меняет источник файла на CurseForge (со slug).
		let dir = tempfile::tempdir().unwrap();
		let staging = dir.path().join(".content-tmp-1");
		std::fs::create_dir_all(&staging).unwrap();
		std::fs::write(staging.join("0"), b"new").unwrap();
		apply(dir.path(), &staging, &mut index, &items, &[]).unwrap();
		assert!(!index.files.contains_key("mods/a-1.jar"));
		let entry = &index.files["mods/100-cf2.jar"];
		let src = entry.source.as_ref().unwrap();
		assert_eq!(src.kind, SourceKind::Curseforge);
		assert_eq!(
			(src.project_id.as_str(), src.version_id.as_str()),
			("100", "cf2")
		);
		assert_eq!(src.slug.as_deref(), Some("slug-100"));
		assert!(entry.update.is_none());
	}

	#[tokio::test]
	async fn fallback_plans_install_from_other_catalog() {
		let (alt, index, files) = fallback_setup();
		let inst = instance(LoaderKind::Fabric, "1.20.1");
		let (items, found) = plan_fallback(&alt, &inst, &index, &files).await.unwrap();
		assert_eq!(found, 1);
		assert_eq!(items.len(), 1);
		assert_eq!(items[0].version_id, "cf2");
		assert_eq!(items[0].hit.source, SourceKind::Curseforge);
		assert_eq!(items[0].hit.slug, "slug-100");
		assert_eq!(items[0].replaces.as_deref(), Some("mods/a-1.jar"));
	}

	#[tokio::test]
	async fn fallback_misses_unknown_files() {
		let (mut alt, index, mut files) = fallback_setup();
		let inst = instance(LoaderKind::Fabric, "1.20.1");
		// Отпечаток совпал, sha1 — нет: это другой файл.
		alt.files[0].1 = "bb".repeat(20);
		let covered = fallback_latest(&alt, &inst, &files).await.unwrap();
		assert!(covered.is_empty());
		// Нечего проверять — последние версии не спрашиваются.
		assert_eq!(*alt.calls.lock().unwrap(), ["identify"]);
		let (items, found) = plan_fallback(&alt, &inst, &index, &files).await.unwrap();
		assert!(items.is_empty() && found == 0);

		// Нашёлся, но новее нет: проверен, обновления нет.
		alt.files[0].1 = files[0].1.sha1.clone();
		alt.projects
			.get_mut("100")
			.unwrap()
			.retain(|v| v.id != "cf2");
		let covered = fallback_latest(&alt, &inst, &files).await.unwrap();
		assert!(covered["mods/a-1.jar"].update().is_none());
		// Своё обновление (с Modrinth) при этом остаётся.
		let mut index = index;
		let none = HashMap::from([("mods/a-1.jar".to_owned(), None)]);
		apply_fallback(&mut index, &none, &HashMap::new());
		assert_eq!(
			index.files["mods/a-1.jar"]
				.update
				.as_ref()
				.unwrap()
				.version_id,
			"2"
		);

		// Без отпечатка CurseForge файл не узнаёт.
		files[0].1.fingerprint = None;
		assert!(
			fallback_latest(&alt, &inst, &files)
				.await
				.unwrap()
				.is_empty()
		);
	}
}
