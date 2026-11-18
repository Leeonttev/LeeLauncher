//! Контент сборок. Каталоги (Modrinth, CurseForge) спрятаны за [`ContentSource`].

pub mod cache;
pub mod commands;
pub mod curseforge;
pub mod fingerprint;
pub mod install;
pub mod installed;
#[cfg(test)]
mod live_tests;
pub mod manual;
pub mod modrinth;
pub mod search_all;
pub mod worlds;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorCode};
use crate::instances::Instance;
use crate::meta::LoaderKind;

/// Событие: содержимое сборки изменилось (установка, удаление, переключение, папки на диске).
pub const EVENT_CHANGED: &str = "instance-content-changed";

/// Один запрос к API каталога вместе с повторами — с первой отправки; очередь к каталогу
/// и пауза лимита запросов сюда не входят.
pub const FETCH_DEADLINE: Duration = Duration::from_secs(10);
/// Сколько команда каталога для интерфейса ждёт ответа целиком.
pub const UI_DEADLINE: Duration = Duration::from_secs(12);
/// Один каталог при проверке обновлений и опознании файлов.
pub const SOURCE_DEADLINE: Duration = Duration::from_secs(12);
/// На сколько очередь и пауза лимита могут продлить срок [`within`]: верхний предел команды.
pub const WAIT_ALLOWANCE: Duration = Duration::from_secs(20);
/// `details` ошибки `source_unavailable`, когда каталог не уложился в срок.
pub const TIMEOUT_DETAILS: &str = "timeout";

/// Каталог не ответил вовремя.
pub fn timed_out() -> AppError {
	AppError::coded(ErrorCode::SourceUnavailable, TIMEOUT_DETAILS)
}

/// Срок запроса вышел: соединение так и не установилось — `network`, иначе — `timeout`.
pub fn deadline_error(source: &str, connect_failed: bool) -> AppError {
	if connect_failed {
		AppError::coded(ErrorCode::Network, format!("{source}: нет соединения"))
	} else {
		timed_out()
	}
}

/// Срок команды: пока запросы ждут очереди или паузы лимита ([`waiting`]), он стоит,
/// но не дальше `hard`.
struct Budget {
	state: std::sync::Mutex<BudgetState>,
	hard: Instant,
	parent: Option<Arc<Budget>>,
}

struct BudgetState {
	deadline: Instant,
	waiting: u32,
	since: Instant,
}

impl Budget {
	fn enter(&self) {
		if let Ok(mut s) = self.state.lock() {
			if s.waiting == 0 {
				s.since = Instant::now();
			}
			s.waiting += 1;
		}
		if let Some(p) = &self.parent {
			p.enter();
		}
	}

	fn leave(&self) {
		if let Ok(mut s) = self.state.lock() {
			s.waiting = s.waiting.saturating_sub(1);
			if s.waiting == 0 {
				let paused = s.since.elapsed();
				s.deadline = (s.deadline + paused).min(self.hard);
			}
		}
		if let Some(p) = &self.parent {
			p.leave();
		}
	}

	/// Когда проверить срок снова и не вышел ли он уже.
	fn next_check(&self) -> Option<Instant> {
		let now = Instant::now();
		if now >= self.hard {
			return None;
		}
		let s = self.state.lock().ok()?;
		let at = if s.waiting > 0 {
			now + s.deadline.saturating_duration_since(s.since)
		} else {
			s.deadline
		};
		if at <= now {
			return None;
		}
		Some(at.min(self.hard))
	}
}

tokio::task_local! {
	static BUDGET: Arc<Budget>;
}

/// `fut`, но не дольше `limit` (иначе `source_unavailable`/`timeout`). Очередь к каталогу
/// и пауза лимита в срок не входят, но всё вместе — не дольше `limit` + [`WAIT_ALLOWANCE`].
pub async fn within<T>(
	limit: Duration,
	fut: impl Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
	within_capped(limit, WAIT_ALLOWANCE, fut).await
}

/// [`within`] со своим запасом на ожидание (`0` — срок не продлевается).
pub async fn within_capped<T>(
	limit: Duration,
	allowance: Duration,
	fut: impl Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
	let now = Instant::now();
	let parent = BUDGET.try_with(|b| b.clone()).ok();
	let mut hard = now + limit + allowance;
	if let Some(p) = &parent {
		hard = hard.min(p.hard);
	}
	let budget = Arc::new(Budget {
		state: std::sync::Mutex::new(BudgetState {
			deadline: (now + limit).min(hard),
			waiting: 0,
			since: now,
		}),
		hard,
		parent,
	});
	let fut = BUDGET.scope(budget.clone(), fut);
	tokio::pin!(fut);
	loop {
		let Some(at) = budget.next_check() else {
			return Err(timed_out());
		};
		tokio::select! {
			result = &mut fut => return result,
			_ = tokio::time::sleep_until(at) => {}
		}
	}
}

/// Ожидание вне срока команды: очередь к каталогу, пауза лимита запросов.
pub async fn waiting<T>(fut: impl Future<Output = T>) -> T {
	struct Leave(Option<Arc<Budget>>);
	impl Drop for Leave {
		fn drop(&mut self) {
			if let Some(b) = &self.0 {
				b.leave();
			}
		}
	}
	let budget = BUDGET.try_with(|b| b.clone()).ok();
	if let Some(b) = &budget {
		b.enter();
	}
	let _leave = Leave(budget);
	fut.await
}

/// Пауза лимита дольше, чем осталось команде: ждать бессмысленно — сразу `rate_limited`.
pub fn wait_exceeds_command(wait: Duration) -> bool {
	BUDGET
		.try_with(|b| Instant::now() + wait > b.hard)
		.unwrap_or(false)
}

/// Каталог, который не ответил при проверке файлов сборки.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFailure {
	pub source: SourceKind,
	pub code: ErrorCode,
	/// Часть его файлов проверена через другой каталог; `null` — ни одного.
	pub covered_by: Option<Coverage>,
}

/// Сколько файлов не ответившего каталога нашлось и проверено в другом.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
	pub source: SourceKind,
	pub files: u32,
}

/// Каталог не ответил (сеть, сбой, срок), а не «не нашёл»: его файлы можно проверить в другом.
pub fn worth_fallback(err: &AppError) -> bool {
	matches!(err.code, ErrorCode::Network | ErrorCode::SourceUnavailable)
}

/// Общая ошибка, когда не ответил ни один нужный каталог.
pub fn all_failed(mut errors: Vec<AppError>) -> AppError {
	if errors.len() == 1 {
		return errors.remove(0);
	}
	if errors.iter().all(|e| e.code == ErrorCode::Network) {
		return AppError::coded(ErrorCode::Network, "все каталоги: нет соединения");
	}
	if errors.iter().all(|e| {
		e.code == ErrorCode::SourceUnavailable && e.details.as_deref() == Some(TIMEOUT_DETAILS)
	}) {
		return timed_out();
	}
	errors
		.into_iter()
		.find(|e| e.code != ErrorCode::Network)
		.unwrap_or_else(|| AppError::coded(ErrorCode::Network, "нет соединения"))
}

/// Каталог, откуда пришёл контент.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
	#[default]
	Modrinth,
	Curseforge,
}

impl SourceKind {
	pub const ALL: [Self; 2] = [Self::Modrinth, Self::Curseforge];

	pub fn as_str(self) -> &'static str {
		match self {
			Self::Modrinth => "modrinth",
			Self::Curseforge => "curseforge",
		}
	}

	/// Второй каталог (подстраховка).
	pub fn other(self) -> Self {
		match self {
			Self::Modrinth => Self::Curseforge,
			Self::Curseforge => Self::Modrinth,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
	Mod,
	Resourcepack,
	Shader,
	Datapack,
	/// Только в каталоге; в сборку не ставится.
	Modpack,
	/// Мир (CurseForge): ставится распаковкой в `saves/`, в индекс контента не попадает.
	World,
}

impl ProjectType {
	/// Что можно поставить в сборку.
	pub const INSTALLABLE: [Self; 4] =
		[Self::Mod, Self::Resourcepack, Self::Shader, Self::Datapack];

	pub fn as_str(self) -> &'static str {
		match self {
			Self::Mod => "mod",
			Self::Resourcepack => "resourcepack",
			Self::Shader => "shader",
			Self::Datapack => "datapack",
			Self::Modpack => "modpack",
			Self::World => "world",
		}
	}

	/// Папка внутри папки игры (у модпака и мира — нет: они не файлы контента).
	pub fn folder(self) -> Option<&'static str> {
		match self {
			Self::Mod => Some("mods"),
			Self::Resourcepack => Some("resourcepacks"),
			Self::Shader => Some("shaderpacks"),
			Self::Datapack => Some("datapacks"),
			Self::Modpack | Self::World => None,
		}
	}

	/// Расширение файлов этого типа.
	pub fn extension(self) -> &'static str {
		match self {
			Self::Mod => "jar",
			Self::Modpack => "mrpack",
			_ => "zip",
		}
	}
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
	#[default]
	Relevance,
	Downloads,
	Follows,
	Newest,
	Updated,
	/// По названию (CurseForge).
	Name,
}

/// Запрос поиска.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
	/// Каталог, в котором ищем. Из запроса фронта не читается: его выбирает [`SearchParams::source`].
	#[serde(skip)]
	pub source: SourceKind,
	#[serde(rename = "type")]
	pub project_type: ProjectType,
	#[serde(default)]
	pub query: String,
	/// Любая из версий игры.
	#[serde(default)]
	pub game_versions: Vec<String>,
	/// Любой из загрузчиков (`fabric`, `forge`, `iris`, `optifine`…).
	#[serde(default)]
	pub loaders: Vec<String>,
	/// Все категории сразу.
	#[serde(default)]
	pub categories: Vec<String>,
	#[serde(default)]
	pub sort: SearchSort,
	#[serde(default)]
	pub offset: u32,
	/// 1–100, по умолчанию 20.
	#[serde(default)]
	pub limit: Option<u32>,
	/// Сборка: у каждого результата будет `install` (установлен ли, подходит ли).
	#[serde(default)]
	pub instance_id: Option<String>,
}

/// Где искать: один каталог или все включённые сразу (`"all"`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchScope {
	#[default]
	Modrinth,
	Curseforge,
	All,
}

/// Аргумент `catalog_search`: запрос + где искать + курсор режима «Все».
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
	#[serde(default)]
	pub source: SearchScope,
	/// Курсор следующей порции режима «Все» (из прошлого ответа); `offset` тогда не читается.
	#[serde(default)]
	pub cursor: Option<String>,
	#[serde(flatten)]
	pub query: SearchQuery,
}

pub const SEARCH_LIMIT_DEFAULT: u32 = 20;
pub const SEARCH_LIMIT_MAX: u32 = 100;

/// Нужен ли проект на клиенте или сервере.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
	Required,
	Optional,
	Unsupported,
	#[default]
	Unknown,
}

impl Side {
	pub fn parse(text: &str) -> Self {
		match text {
			"required" => Self::Required,
			"optional" => Self::Optional,
			"unsupported" => Self::Unsupported,
			_ => Self::Unknown,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncompatibleReason {
	GameVersion,
	Loader,
	/// В ванильную сборку: моду или шейдеру нужен загрузчик модов.
	NeedsLoader,
}

/// Состояние проекта относительно сборки.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum InstallState {
	None,
	#[serde(rename_all = "camelCase")]
	Installed {
		version_id: String,
		file: String,
		enabled: bool,
	},
	#[serde(rename_all = "camelCase")]
	Update {
		version_id: String,
		latest_version_id: String,
		latest_number: String,
	},
	Incompatible {
		reason: IncompatibleReason,
	},
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectHit {
	pub source: SourceKind,
	pub id: String,
	/// Slug, а если его нет — id.
	pub slug: String,
	#[serde(rename = "type")]
	pub project_type: ProjectType,
	pub title: String,
	/// Автор (владелец команды); может быть пустым, если каталог его не дал.
	pub author: String,
	pub description: String,
	pub icon_url: Option<String>,
	pub downloads: u64,
	/// Подписчики; `null` — каталог их не считает (CurseForge).
	pub follows: Option<u64>,
	/// Автор разрешил скачивать только на сайте каталога (CurseForge `allowModDistribution: false`).
	pub site_only: bool,
	/// Страница проекта на сайте каталога.
	pub url: String,
	/// Категории (без загрузчиков).
	pub categories: Vec<String>,
	/// Unix мс; 0 — неизвестно.
	pub updated_at: u64,
	pub game_versions: Vec<String>,
	/// Загрузчики проекта (`fabric`, `iris`…).
	pub loaders: Vec<String>,
	pub client_side: Side,
	pub server_side: Side,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub install: Option<InstallState>,
	/// Режим «Все»: тот же проект в других каталогах (строка склеена из нескольких).
	pub also_on: Vec<AlsoOn>,
	/// Unix мс создания проекта (для сортировки `newest` в режиме «Все»); 0 — неизвестно.
	#[serde(skip)]
	pub created_at: u64,
}

/// Тот же проект в другом каталоге (режим «Все»).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlsoOn {
	pub source: SourceKind,
	pub id: String,
	pub slug: String,
	pub url: String,
	pub site_only: bool,
	/// Загрузки в этом каталоге (у строки — свои, не сумма).
	pub downloads: u64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub install: Option<InstallState>,
}

impl AlsoOn {
	pub fn of(hit: &ProjectHit) -> Self {
		Self {
			source: hit.source,
			id: hit.id.clone(),
			slug: hit.slug.clone(),
			url: hit.url.clone(),
			site_only: hit.site_only,
			downloads: hit.downloads,
			install: None,
		}
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
	pub hits: Vec<ProjectHit>,
	pub offset: u32,
	pub limit: u32,
	pub total: u64,
	/// Сети нет, показан сохранённый ответ.
	pub offline: bool,
	/// Режим «Все»: курсор следующей порции; `null` — конец.
	pub cursor: Option<String>,
	/// Режим «Все»: каталоги, которые не ответили (результаты — только от остальных).
	pub partial: Vec<PartialFailure>,
}

impl SearchPage {
	pub fn new(hits: Vec<ProjectHit>, offset: u32, limit: u32, total: u64, offline: bool) -> Self {
		Self {
			hits,
			offset,
			limit,
			total,
			offline,
			cursor: None,
			partial: Vec::new(),
		}
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialFailure {
	pub source: SourceKind,
	pub error: AppError,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct License {
	pub id: String,
	pub name: String,
	pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DonationLink {
	pub platform: String,
	pub url: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Links {
	/// Страница проекта на сайте каталога.
	pub site: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub source: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub wiki: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub issues: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub discord: Option<String>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub donate: Vec<DonationLink>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryImage {
	/// Полный размер.
	pub url: String,
	/// Уменьшенная копия.
	pub thumb_url: String,
	pub title: Option<String>,
	pub description: Option<String>,
	pub featured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
	pub id: String,
	pub username: String,
	pub avatar_url: Option<String>,
	pub role: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
	#[serde(flatten)]
	pub hit: ProjectHit,
	/// Описание в markdown как есть (может содержать HTML) — санитизирует фронт.
	pub body: String,
	pub gallery: Vec<GalleryImage>,
	pub links: Links,
	pub license: Option<License>,
	pub versions_count: usize,
	pub team: Vec<TeamMember>,
	/// Unix мс.
	pub published_at: u64,
	/// Что поставится в сборку `instanceId` (если просили и есть подходящая версия).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub best_version: Option<ContentVersion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionChannel {
	Release,
	Beta,
	Alpha,
}

/// Где нужен файл версии (CurseForge: метки `Client`/`Server` в списке версий).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSide {
	pub client: Side,
	pub server: Side,
}

/// Файл версии. Адрес и хеши фронту не нужны и не отдаются.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentFile {
	#[serde(skip)]
	pub url: String,
	#[serde(rename = "name")]
	pub file_name: String,
	#[serde(skip)]
	pub sha1: String,
	pub size: u64,
	pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
	Required,
	Optional,
	Incompatible,
	Embedded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
	pub project_id: Option<String>,
	pub version_id: Option<String>,
	pub kind: DependencyKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentVersion {
	#[serde(skip)]
	pub source: SourceKind,
	pub id: String,
	pub project_id: String,
	pub name: String,
	#[serde(rename = "number")]
	pub version_number: String,
	pub channel: VersionChannel,
	pub game_versions: Vec<String>,
	pub loaders: Vec<String>,
	/// Unix мс.
	pub published_at: u64,
	pub downloads: u64,
	/// Список изменений (markdown); только в `catalog_version`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub changelog: Option<String>,
	pub files: Vec<ContentFile>,
	pub dependencies: Vec<Dependency>,
	/// Автор запретил стороннюю загрузку: качать только на сайте ([`Self::page_url`]).
	pub site_only: bool,
	/// Страница файла на сайте каталога (CurseForge — сразу начинает скачивание); `null` — нет.
	pub page_url: Option<String>,
	/// Где нужен файл, если каталог это указал.
	pub side: Option<VersionSide>,
	/// `catalog_versions` со сборкой: эта версия стоит в ней.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub installed: Option<bool>,
}

impl ContentVersion {
	/// Основной файл версии (`primary`, иначе первый).
	pub fn primary_file(&self) -> Option<&ContentFile> {
		self.files
			.iter()
			.find(|f| f.primary)
			.or_else(|| self.files.first())
	}
}

/// Категория каталога для фильтра.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryTag {
	pub slug: String,
	pub project_type: String,
	/// Группа фильтров на сайте каталога (`categories`, `features`, `resolutions`, `performance impact`).
	pub header: String,
	/// SVG-иконка текстом (Modrinth); может быть пустой.
	pub icon: String,
	/// Иконка картинкой по адресу (CurseForge).
	#[serde(default)]
	pub icon_url: Option<String>,
	/// Название от каталога (у Modrinth — slug).
	#[serde(default)]
	pub name: String,
	/// slug родительской категории (CurseForge); `null` — верхний уровень.
	#[serde(default)]
	pub parent: Option<String>,
}

/// Фильтр версий проекта: любая из версий игры и любой из загрузчиков (пусто — без фильтра).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionFilter {
	pub game_versions: Vec<String>,
	pub loaders: Vec<String>,
}

impl VersionFilter {
	/// Фильтр под сборку для контента этого типа.
	pub fn for_instance(instance: &Instance, kind: ProjectType) -> Self {
		Self {
			game_versions: vec![instance.game_version.clone()],
			loaders: loaders_for(kind, instance.loader.kind, &instance.game_version),
		}
	}

	pub fn matches(&self, version: &ContentVersion) -> bool {
		self.matches_lists(&version.game_versions, &version.loaders)
	}

	pub fn matches_lists(&self, game_versions: &[String], loaders: &[String]) -> bool {
		let game = self.game_versions.is_empty()
			|| game_versions.iter().any(|v| self.game_versions.contains(v));
		let loader = self.loaders.is_empty()
			|| loaders
				.iter()
				.any(|l| self.loaders.iter().any(|x| x.eq_ignore_ascii_case(l)));
		game && loader
	}
}

/// Загрузчики каталога, подходящие сборке, для контента этого типа. Пусто — загрузчик не важен.
pub fn loaders_for(kind: ProjectType, loader: LoaderKind, game_version: &str) -> Vec<String> {
	let list: &[&str] = match kind {
		ProjectType::Mod => match loader {
			LoaderKind::Fabric => &["fabric"],
			// Quilt запускает и моды Fabric.
			LoaderKind::Quilt => &["quilt", "fabric"],
			LoaderKind::Forge => &["forge"],
			// Ранний NeoForge (1.20.1) совместим с модами Forge.
			LoaderKind::Neoforge if game_version == "1.20.1" => &["neoforge", "forge"],
			LoaderKind::Neoforge => &["neoforge"],
			LoaderKind::Vanilla => &[],
		},
		ProjectType::Datapack => &["datapack"],
		ProjectType::Resourcepack => &["minecraft"],
		// Шейдеры: любой (Iris, OptiFine, Canvas, ванильные «core shaders»).
		ProjectType::Shader | ProjectType::Modpack | ProjectType::World => &[],
	};
	list.iter().map(|s| (*s).to_owned()).collect()
}

/// Проект загрузчика шейдеров для сборки в этом каталоге: Fabric/Quilt — Iris, Forge/NeoForge — Oculus.
pub fn shader_loader_project(source: SourceKind, loader: LoaderKind) -> Option<&'static str> {
	let iris = matches!(loader, LoaderKind::Fabric | LoaderKind::Quilt);
	let oculus = matches!(loader, LoaderKind::Forge | LoaderKind::Neoforge);
	match source {
		SourceKind::Modrinth if iris => Some(modrinth::IRIS_ID),
		SourceKind::Modrinth if oculus => Some(modrinth::OCULUS_ID),
		SourceKind::Curseforge if iris => Some(curseforge::IRIS_ID),
		SourceKind::Curseforge if oculus => Some(curseforge::OCULUS_ID),
		_ => None,
	}
}

/// Загрузчик шейдеров этой сборки уже стоит — из любого каталога (чтобы не поставить второй).
pub fn shader_loader_installed(
	loader: LoaderKind,
	installed: &impl Fn(SourceKind, &str) -> bool,
) -> bool {
	SourceKind::ALL
		.into_iter()
		.any(|source| shader_loader_project(source, loader).is_some_and(|id| installed(source, id)))
}

/// Подходит ли проект сборке по его спискам версий игры и загрузчиков (без запроса версий).
pub fn compatibility(
	instance: &Instance,
	kind: ProjectType,
	game_versions: &[String],
	loaders: &[String],
) -> Option<IncompatibleReason> {
	let vanilla = instance.loader.kind == LoaderKind::Vanilla;
	if vanilla && matches!(kind, ProjectType::Mod | ProjectType::Shader) {
		return Some(IncompatibleReason::NeedsLoader);
	}
	if !game_versions.is_empty() && !game_versions.contains(&instance.game_version) {
		return Some(IncompatibleReason::GameVersion);
	}
	let filter = VersionFilter {
		game_versions: Vec::new(),
		loaders: loaders_for(kind, instance.loader.kind, &instance.game_version),
	};
	if kind == ProjectType::Mod && !loaders.is_empty() && !filter.matches_lists(&[], loaders) {
		return Some(IncompatibleReason::Loader);
	}
	None
}

/// Самая подходящая версия: первая стабильная, иначе первая бета, иначе первая (список — новые первыми).
pub fn pick_best(versions: &[ContentVersion]) -> Option<&ContentVersion> {
	[VersionChannel::Release, VersionChannel::Beta]
		.iter()
		.find_map(|ch| versions.iter().find(|v| v.channel == *ch))
		.or_else(|| versions.first())
}

/// RFC 3339 → Unix мс (0, если не разобралось).
pub fn timestamp_ms(text: &str) -> u64 {
	chrono::DateTime::parse_from_rfc3339(text)
		.ok()
		.and_then(|t| u64::try_from(t.timestamp_millis()).ok())
		.unwrap_or(0)
}

/// Установленный файл для опознания: Modrinth берёт sha1, CurseForge — отпечаток и id проекта.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileKey {
	pub sha1: String,
	pub fingerprint: Option<u32>,
	/// Проект этого каталога (для `latest`).
	pub project_id: Option<String>,
}

impl FileKey {
	pub fn sha1(sha1: impl Into<String>) -> Self {
		Self {
			sha1: sha1.into(),
			..Self::default()
		}
	}

	pub fn of_sha1s(sha1s: &[String]) -> Vec<Self> {
		sha1s.iter().map(|s| Self::sha1(s.clone())).collect()
	}
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

/// Каталог контента. Новые каталоги (CurseForge) реализуют этот интерфейс и добавляются в [`Sources`].
pub trait ContentSource: Send + Sync {
	fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxFuture<'a, SearchPage>;
	/// Проект по id или slug, с командой авторов и всеми версиями (`versionsCount`).
	fn project<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Project>;
	/// Проект по slug в разделе этого типа (у CurseForge slug уникален только внутри раздела).
	fn project_by_slug<'a>(&'a self, slug: &'a str, kind: ProjectType) -> BoxFuture<'a, Project> {
		let _ = kind;
		self.project(slug)
	}
	/// Проекты по id или slug (без отсутствующих), с авторами.
	fn projects<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ProjectHit>>;
	/// Версии проекта, новые первыми.
	fn project_versions<'a>(
		&'a self,
		id: &'a str,
		filter: &'a VersionFilter,
	) -> BoxFuture<'a, Vec<ContentVersion>>;
	fn version<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ContentVersion>;
	/// Версии по id (без отсутствующих).
	fn versions<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ContentVersion>>;
	/// Категории (кэш сутки, без сети — с диска); `bool` — без сети.
	fn categories(&self) -> BoxFuture<'_, (Vec<CategoryTag>, bool)>;
	/// Определение файлов (Modrinth — по sha1, CurseForge — по отпечатку): sha1 → версия.
	/// Неизвестных в ответе нет.
	fn identify<'a>(
		&'a self,
		files: &'a [FileKey],
	) -> BoxFuture<'a, HashMap<String, ContentVersion>>;
	/// Последние подходящие версии для файлов этого каталога: sha1 → версия (та же — обновления нет).
	fn latest<'a>(
		&'a self,
		files: &'a [FileKey],
		filter: &'a VersionFilter,
	) -> BoxFuture<'a, HashMap<String, ContentVersion>>;
}

/// Выключенный каталог (CurseForge без ключа): на всё отвечает `source_disabled`.
pub struct Disabled;

impl Disabled {
	fn fail<'a, T: Send + 'a>(&self) -> BoxFuture<'a, T> {
		let err = AppError::coded(ErrorCode::SourceDisabled, curseforge::NO_KEY);
		Box::pin(async move { Err(err) })
	}
}

impl ContentSource for Disabled {
	fn search<'a>(&'a self, _: &'a SearchQuery) -> BoxFuture<'a, SearchPage> {
		self.fail()
	}
	fn project<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Project> {
		self.fail()
	}
	fn projects<'a>(&'a self, _: &'a [String]) -> BoxFuture<'a, Vec<ProjectHit>> {
		self.fail()
	}
	fn project_versions<'a>(
		&'a self,
		_: &'a str,
		_: &'a VersionFilter,
	) -> BoxFuture<'a, Vec<ContentVersion>> {
		self.fail()
	}
	fn version<'a>(&'a self, _: &'a str) -> BoxFuture<'a, ContentVersion> {
		self.fail()
	}
	fn versions<'a>(&'a self, _: &'a [String]) -> BoxFuture<'a, Vec<ContentVersion>> {
		self.fail()
	}
	fn categories(&self) -> BoxFuture<'_, (Vec<CategoryTag>, bool)> {
		self.fail()
	}
	fn identify<'a>(&'a self, _: &'a [FileKey]) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
		self.fail()
	}
	fn latest<'a>(
		&'a self,
		_: &'a [FileKey],
		_: &'a VersionFilter,
	) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
		self.fail()
	}
}

/// Все каталоги. CurseForge без ключа API выключен: вместо него [`Disabled`].
pub struct Sources {
	modrinth: Arc<dyn ContentSource>,
	curseforge: Option<Arc<dyn ContentSource>>,
}

impl Sources {
	pub fn new(modrinth: modrinth::Modrinth, curseforge: Option<curseforge::CurseForge>) -> Self {
		Self {
			modrinth: Arc::new(modrinth),
			curseforge: curseforge.map(|c| Arc::new(c) as Arc<dyn ContentSource>),
		}
	}

	pub fn get(&self, kind: SourceKind) -> Arc<dyn ContentSource> {
		match kind {
			SourceKind::Modrinth => self.modrinth.clone(),
			SourceKind::Curseforge => match &self.curseforge {
				Some(c) => c.clone(),
				None => Arc::new(Disabled),
			},
		}
	}

	pub fn enabled(&self, kind: SourceKind) -> bool {
		match kind {
			SourceKind::Modrinth => true,
			SourceKind::Curseforge => self.curseforge.is_some(),
		}
	}

	/// Включённые каталоги, где есть этот тип, в порядке переключателя (для режима «Все»).
	pub fn for_all(&self, kind: ProjectType) -> Vec<(SourceKind, Arc<dyn ContentSource>)> {
		self.info()
			.into_iter()
			.filter(|s| s.enabled && s.types.contains(&kind))
			.map(|s| (s.id, self.get(s.id)))
			.collect()
	}

	/// Режим «Все» (`catalog_all_info`): объединение типов включённых каталогов и общие сортировки.
	pub fn all_info(&self) -> AllInfo {
		let enabled: Vec<SourceInfo> = self.info().into_iter().filter(|s| s.enabled).collect();
		let mut types: Vec<ProjectType> = Vec::new();
		for t in enabled.iter().flat_map(|s| &s.types) {
			if !types.contains(t) {
				types.push(*t);
			}
		}
		AllInfo {
			enabled: enabled.len() > 1,
			sources: enabled.iter().map(|s| s.id).collect(),
			types,
			sorts: search_all::SORTS.to_vec(),
			categories: false,
		}
	}

	/// Что умеют каталоги (`catalog_sources`), в порядке переключателя.
	pub fn info(&self) -> Vec<SourceInfo> {
		use ProjectType::*;
		use SearchSort::*;
		vec![
			SourceInfo {
				id: SourceKind::Modrinth,
				enabled: true,
				types: vec![Mod, Resourcepack, Shader, Datapack, Modpack],
				sorts: vec![Relevance, Downloads, Follows, Newest, Updated],
				shader_loaders: true,
				max_categories: None,
				has_follows: true,
			},
			SourceInfo {
				id: SourceKind::Curseforge,
				enabled: self.enabled(SourceKind::Curseforge),
				types: vec![Mod, Resourcepack, Shader, Datapack, World, Modpack],
				sorts: vec![Relevance, Downloads, Updated, Newest, Name],
				shader_loaders: false,
				max_categories: Some(curseforge::MAX_CATEGORIES),
				has_follows: false,
			},
		]
	}
}

/// Возможности каталога для интерфейса.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
	pub id: SourceKind,
	/// `false` — каталог выключен в этой сборке (нет ключа API).
	pub enabled: bool,
	pub types: Vec<ProjectType>,
	pub sorts: Vec<SearchSort>,
	/// Есть ли у шейдеров фильтр загрузчика.
	pub shader_loaders: bool,
	pub max_categories: Option<u32>,
	pub has_follows: bool,
}

/// Возможности режима «Все» для интерфейса.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllInfo {
	/// `true` — включено больше одного каталога (иначе «Все» = единственный каталог).
	pub enabled: bool,
	/// Каталоги, которые участвуют.
	pub sources: Vec<SourceKind>,
	/// Объединение типов (мир ищется только там, где он есть).
	pub types: Vec<ProjectType>,
	pub sorts: Vec<SearchSort>,
	/// Категорий в режиме «Все» нет.
	pub categories: bool,
}

/// Страница проекта на сайте каталога по id (когда slug неизвестен).
pub fn project_url(source: SourceKind, id: &str) -> String {
	match source {
		SourceKind::Modrinth => format!("https://modrinth.com/project/{id}"),
		SourceKind::Curseforge => format!("https://www.curseforge.com/projects/{id}"),
	}
}

/// Id или slug проекта/версии из запроса фронта: только `A–Z a–z 0–9 _ -`, до 64 символов.
pub fn check_ref(id: &str) -> Result<(), AppError> {
	let ok = !id.is_empty()
		&& id.len() <= 64
		&& id
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
	if ok {
		Ok(())
	} else {
		Err(AppError::coded(
			ErrorCode::InvalidValue,
			format!("id: {id}"),
		))
	}
}

#[cfg(test)]
pub mod tests {
	use super::*;
	use crate::instances::Loader;

	#[tokio::test]
	async fn within_stops_while_waiting() {
		let started = std::time::Instant::now();
		let result = within_capped(Duration::from_millis(200), Duration::from_secs(2), async {
			waiting(tokio::time::sleep(Duration::from_millis(500))).await;
			Ok::<_, AppError>(())
		})
		.await;
		assert!(result.is_ok());
		assert!(started.elapsed() >= Duration::from_millis(500));
		// Без ожидания срок действует как прежде.
		let err = within(Duration::from_millis(100), async {
			tokio::time::sleep(Duration::from_millis(400)).await;
			Ok::<_, AppError>(())
		})
		.await
		.unwrap_err();
		assert_eq!(err.details.as_deref(), Some(TIMEOUT_DETAILS));
		// Верхний предел: ждать дольше запаса нельзя.
		let err = within_capped(
			Duration::from_millis(100),
			Duration::from_millis(200),
			async {
				waiting(tokio::time::sleep(Duration::from_secs(5))).await;
				Ok::<_, AppError>(())
			},
		)
		.await
		.unwrap_err();
		assert_eq!(err.code, ErrorCode::SourceUnavailable);
	}

	pub fn version(id: &str, channel: VersionChannel, game: &str, loader: &str) -> ContentVersion {
		ContentVersion {
			source: SourceKind::Modrinth,
			id: id.into(),
			project_id: "p".into(),
			name: id.into(),
			version_number: id.into(),
			channel,
			game_versions: vec![game.into()],
			loaders: vec![loader.into()],
			published_at: 0,
			downloads: 0,
			changelog: None,
			files: vec![],
			dependencies: vec![],
			site_only: false,
			page_url: None,
			side: None,
			installed: None,
		}
	}

	pub fn instance(loader: LoaderKind, game: &str) -> Instance {
		let mut i: Instance = serde_json::from_value(serde_json::json!({
			"id": "abc", "name": "x", "gameVersion": game,
			"loader": { "kind": "vanilla", "version": null },
			"createdAt": "2026-01-01T00:00:00Z", "installState": "installed"
		}))
		.unwrap();
		i.loader = Loader {
			kind: loader,
			version: None,
		};
		i
	}

	#[test]
	fn best_version_and_filter() {
		let list = vec![
			version("a", VersionChannel::Alpha, "1.20.1", "fabric"),
			version("b", VersionChannel::Beta, "1.20.1", "fabric"),
			version("r", VersionChannel::Release, "1.20.1", "fabric"),
		];
		assert_eq!(pick_best(&list).unwrap().id, "r");
		assert_eq!(pick_best(&list[..2]).unwrap().id, "b");
		assert_eq!(pick_best(&list[..1]).unwrap().id, "a");
		assert!(pick_best(&[]).is_none());

		let filter = VersionFilter {
			game_versions: vec!["1.20.1".into()],
			loaders: loaders_for(ProjectType::Mod, LoaderKind::Quilt, "1.20.1"),
		};
		assert!(filter.matches(&list[0]));
		assert!(!filter.matches(&version("x", VersionChannel::Release, "1.20.1", "forge")));
		assert!(!filter.matches(&version("x", VersionChannel::Release, "1.19", "fabric")));
		assert!(VersionFilter::default().matches(&list[0]));
		assert_eq!(
			loaders_for(ProjectType::Mod, LoaderKind::Neoforge, "1.20.1"),
			vec!["neoforge", "forge"]
		);
		assert_eq!(
			loaders_for(ProjectType::Mod, LoaderKind::Neoforge, "1.21.1"),
			vec!["neoforge"]
		);
		assert!(loaders_for(ProjectType::Shader, LoaderKind::Fabric, "1.21").is_empty());
	}

	#[test]
	fn compatibility_reasons() {
		let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
		let fabric = instance(LoaderKind::Fabric, "1.20.1");
		let vanilla = instance(LoaderKind::Vanilla, "1.20.1");
		assert_eq!(
			compatibility(&fabric, ProjectType::Mod, &s(&["1.20.1"]), &s(&["fabric"])),
			None
		);
		assert_eq!(
			compatibility(&fabric, ProjectType::Mod, &s(&["1.21"]), &s(&["fabric"])),
			Some(IncompatibleReason::GameVersion)
		);
		assert_eq!(
			compatibility(&fabric, ProjectType::Mod, &s(&["1.20.1"]), &s(&["forge"])),
			Some(IncompatibleReason::Loader)
		);
		assert_eq!(
			compatibility(
				&vanilla,
				ProjectType::Shader,
				&s(&["1.20.1"]),
				&s(&["iris"])
			),
			Some(IncompatibleReason::NeedsLoader)
		);
		assert_eq!(
			compatibility(
				&vanilla,
				ProjectType::Resourcepack,
				&s(&["1.20.1"]),
				&s(&["minecraft"])
			),
			None
		);
		assert_eq!(
			shader_loader_project(SourceKind::Modrinth, LoaderKind::Neoforge),
			Some(modrinth::OCULUS_ID)
		);
		assert_eq!(
			shader_loader_project(SourceKind::Curseforge, LoaderKind::Quilt),
			Some(curseforge::IRIS_ID)
		);
		assert_eq!(
			shader_loader_project(SourceKind::Curseforge, LoaderKind::Vanilla),
			None
		);
		// Iris из Modrinth уже стоит — для шейдера из CurseForge второй не нужен.
		assert!(shader_loader_installed(LoaderKind::Fabric, &|s, id| {
			s == SourceKind::Modrinth && id == modrinth::IRIS_ID
		}));
		assert!(!shader_loader_installed(LoaderKind::Forge, &|s, id| {
			s == SourceKind::Modrinth && id == modrinth::IRIS_ID
		}));
	}

	#[test]
	fn refs_types_and_json() {
		assert!(check_ref("AANobbMI").is_ok() && check_ref("fabric-api").is_ok());
		for bad in ["", "../x", "a b", "a/b", "a?b"] {
			assert!(check_ref(bad).is_err(), "{bad}");
		}
		let t: ProjectType = serde_json::from_str("\"shader\"").unwrap();
		assert_eq!(t.folder(), Some("shaderpacks"));
		assert_eq!(ProjectType::Modpack.folder(), None);
		let q: SearchQuery = serde_json::from_value(serde_json::json!({ "type": "mod" })).unwrap();
		assert_eq!(q.source, SourceKind::Modrinth);
		assert_eq!(q.sort, SearchSort::Relevance);

		let state = InstallState::Update {
			version_id: "a".into(),
			latest_version_id: "b".into(),
			latest_number: "2".into(),
		};
		let json = serde_json::to_value(&state).unwrap();
		assert_eq!(json["kind"], "update");
		assert_eq!(json["latestVersionId"], "b");
		let json = serde_json::to_value(InstallState::Incompatible {
			reason: IncompatibleReason::NeedsLoader,
		})
		.unwrap();
		assert_eq!(
			json,
			serde_json::json!({ "kind": "incompatible", "reason": "needs_loader" })
		);
		assert_eq!(timestamp_ms("2024-01-01T00:00:00Z"), 1_704_067_200_000);
		assert_eq!(timestamp_ms("x"), 0);

		let mut v = version("a", VersionChannel::Release, "1.20.1", "fabric");
		v.files.push(ContentFile {
			url: "u".into(),
			file_name: "a.jar".into(),
			sha1: "s".into(),
			size: 1,
			primary: true,
		});
		let json = serde_json::to_value(&v).unwrap();
		assert_eq!(json["number"], "a");
		assert_eq!(
			json["files"][0],
			serde_json::json!({ "name": "a.jar", "size": 1, "primary": true })
		);
		assert!(json.get("source").is_none() && json.get("installed").is_none());
		assert_eq!(json["siteOnly"], false);
		assert!(json["pageUrl"].is_null());
		let s: SourceKind = serde_json::from_value(serde_json::json!("curseforge")).unwrap();
		assert_eq!(s, SourceKind::Curseforge);
	}
}
