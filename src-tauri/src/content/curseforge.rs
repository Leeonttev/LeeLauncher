//! Каталог CurseForge (<https://docs.curseforge.com/rest-api/>). Ключ API — `LEELAUNCHER_CURSEFORGE_KEY`
//! при сборке или в окружении; без него каталог выключен.
//!
//! Версия проекта здесь — файл (`fileId`). Загрузчики и версии игры приходят одним списком
//! `gameVersions` (`["1.20.1", "Forge", "Client"]`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use url::Url;

use super::cache::ResponseCache;
use super::{
	BoxFuture, CategoryTag, ContentFile, ContentSource, ContentVersion, Dependency, DependencyKind,
	FileKey, GalleryImage, Links, Project, ProjectHit, ProjectType, SEARCH_LIMIT_DEFAULT,
	SearchPage, SearchQuery, SearchSort, Side, SourceKind, TeamMember, VersionChannel,
	VersionFilter, VersionSide, timestamp_ms,
};
use crate::error::{AppError, ErrorCode};

pub const API_BASE: &str = "https://api.curseforge.com";
/// Ключ API: вшитый при сборке, иначе из окружения.
pub const KEY_ENV: &str = "LEELAUNCHER_CURSEFORGE_KEY";
/// Minecraft в CurseForge.
pub const GAME_ID: u32 = 432;
/// Iris Shaders и Oculus на CurseForge.
pub const IRIS_ID: &str = "455508";
pub const OCULUS_ID: &str = "581495";
/// Откуда CurseForge отдаёт файлы (`downloadUrl`). Им тоже нужен ключ: CDN требует `x-api-key`.
pub const CDN_HOSTS: [&str; 2] = ["edge.forgecdn.net", "mediafilez.forgecdn.net"];
const SITE: &str = "https://www.curseforge.com";

/// Разделы (`classId`) Minecraft.
pub mod class {
	pub const MODS: u32 = 6;
	pub const RESOURCEPACKS: u32 = 12;
	pub const WORLDS: u32 = 17;
	pub const MODPACKS: u32 = 4471;
	pub const SHADERS: u32 = 6552;
	pub const DATAPACKS: u32 = 6945;
}

/// Категорий в поиске не больше этого (`categoryIds`).
pub const MAX_CATEGORIES: u32 = 10;
const CONCURRENCY: usize = 6;
const MAX_RATE_WAIT: Duration = Duration::from_secs(10);
const NETWORK_RETRIES: u32 = 2;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Больше 50 за страницу API не отдаёт, `index + pageSize` — не больше 10 000.
const PAGE_MAX: u32 = 50;
const RESULTS_MAX: u32 = 10_000;
/// Файлов проекта читаем не больше этого числа страниц (новые первыми).
const FILE_PAGES: u32 = 10;
/// id за один запрос `POST /v1/mods`, `/v1/mods/files`, отпечатков — за один `/v1/fingerprints`.
const BATCH: usize = 200;
const FINGERPRINT_BATCH: usize = 1000;

const TTL_SEARCH: Duration = Duration::from_secs(2 * 60);
const TTL_PROJECT: Duration = Duration::from_secs(5 * 60);
const TTL_HASHES: Duration = Duration::from_secs(10 * 60);
const TTL_TAGS: Duration = Duration::from_secs(24 * 60 * 60);

/// Ключ API: сборочный важнее переменной окружения во время работы. Пустой — нет ключа.
pub fn api_key() -> Option<String> {
	let valid = |k: &str| {
		let k = k.trim();
		(!k.is_empty()).then(|| k.to_owned())
	};
	option_env!("LEELAUNCHER_CURSEFORGE_KEY")
		.and_then(valid)
		.or_else(|| std::env::var(KEY_ENV).ok().as_deref().and_then(valid))
}

/// Значение заголовка с ключом (помечено секретным: не выводится в `Debug`).
pub fn key_header(key: &str) -> Result<reqwest::header::HeaderValue, AppError> {
	let mut value = reqwest::header::HeaderValue::from_str(key)
		.map_err(|_| AppError::unknown("curseforge: ключ API с недопустимыми символами"))?;
	value.set_sensitive(true);
	Ok(value)
}

/// Клиент API: ключ во всех запросах, без перенаправлений (ключ не уйдёт на чужой адрес).
pub fn build_client(key: &str) -> Result<reqwest::Client, AppError> {
	let mut headers = reqwest::header::HeaderMap::new();
	headers.insert("x-api-key", key_header(key)?);
	headers.insert(
		reqwest::header::ACCEPT,
		reqwest::header::HeaderValue::from_static("application/json"),
	);
	reqwest::Client::builder()
		.user_agent(crate::http::USER_AGENT)
		.default_headers(headers)
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(Duration::from_secs(5))
		.timeout(REQUEST_TIMEOUT)
		.build()
		.map_err(|e| AppError::unknown(e.to_string()))
}

/// Адрес файла с CDN CurseForge (`https://` и хост из [`CDN_HOSTS`]).
pub fn is_cdn_url(url: &str) -> bool {
	Url::parse(url).is_ok_and(|u| {
		u.scheme() == "https"
			&& u.host_str()
				.is_some_and(|h| CDN_HOSTS.iter().any(|c| h.eq_ignore_ascii_case(c)))
	})
}

fn class_of(kind: ProjectType) -> u32 {
	match kind {
		ProjectType::Mod => class::MODS,
		ProjectType::Resourcepack => class::RESOURCEPACKS,
		ProjectType::Shader => class::SHADERS,
		ProjectType::Datapack => class::DATAPACKS,
		ProjectType::Modpack => class::MODPACKS,
		ProjectType::World => class::WORLDS,
	}
}

/// Тип по разделу. Незнакомые разделы (дополнения, плагины) считаются модами.
pub fn type_of_class(class: Option<u32>) -> ProjectType {
	match class {
		Some(class::WORLDS) => ProjectType::World,
		Some(class::RESOURCEPACKS) => ProjectType::Resourcepack,
		Some(class::SHADERS) => ProjectType::Shader,
		Some(class::DATAPACKS) => ProjectType::Datapack,
		Some(class::MODPACKS) => ProjectType::Modpack,
		_ => ProjectType::Mod,
	}
}

/// `ModLoaderType` CurseForge.
fn loader_type(name: &str) -> Option<u32> {
	match name.to_ascii_lowercase().as_str() {
		"forge" => Some(1),
		"fabric" => Some(4),
		"quilt" => Some(5),
		"neoforge" => Some(6),
		_ => None,
	}
}

fn loader_name(kind: u32) -> Option<&'static str> {
	match kind {
		1 => Some("forge"),
		2 => Some("cauldron"),
		3 => Some("liteloader"),
		4 => Some("fabric"),
		5 => Some("quilt"),
		6 => Some("neoforge"),
		_ => None,
	}
}

/// Версия игры в `gameVersions` (`1.20.1`, `1.21-Snapshot`), а не загрузчик или сторона.
fn is_game_version(text: &str) -> bool {
	text.starts_with(|c: char| c.is_ascii_digit())
}

/// Загрузчики из `gameVersions`. Старые моды без отметки — `forge`: до Fabric других не было.
fn loaders_of(game_versions: &[String], class: Option<u32>) -> Vec<String> {
	let mut loaders: Vec<String> = Vec::new();
	for gv in game_versions {
		let lower = gv.to_ascii_lowercase();
		if matches!(
			lower.as_str(),
			"forge" | "fabric" | "quilt" | "neoforge" | "liteloader" | "cauldron"
		) && !loaders.contains(&lower)
		{
			loaders.push(lower);
		}
	}
	if loaders.is_empty() {
		match type_of_class(class) {
			ProjectType::Resourcepack => loaders.push("minecraft".into()),
			ProjectType::Datapack => loaders.push("datapack".into()),
			ProjectType::Mod => loaders.push("forge".into()),
			ProjectType::Shader | ProjectType::Modpack | ProjectType::World => {}
		}
	}
	loaders
}

fn numeric(id: &str) -> Option<u64> {
	(!id.is_empty() && id.len() <= 12 && id.bytes().all(|b| b.is_ascii_digit()))
		.then(|| id.parse().ok())
		.flatten()
}

fn non_empty(value: Option<String>) -> Option<String> {
	value.filter(|v| !v.trim().is_empty())
}

/// Числа CurseForge иногда приходят дробными (`1.2E+7`).
fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
	let value = Option::<f64>::deserialize(d)?.unwrap_or(0.0);
	Ok(if value.is_finite() && value > 0.0 {
		value as u64
	} else {
		0
	})
}

enum Request {
	Get(Url),
	Post(Url, Value),
}

impl Request {
	fn key(&self) -> String {
		match self {
			Self::Get(url) => format!("GET {url}"),
			Self::Post(url, body) => format!("POST {url} {body}"),
		}
	}
}

/// Что о проекте нужно для версий: раздел (тип, загрузчики по умолчанию) и страница на сайте.
#[derive(Debug, Clone, Default)]
struct ModInfo {
	class: Option<u32>,
	website: Option<String>,
}

pub struct CurseForge {
	client: reqwest::Client,
	base: Url,
	cache: ResponseCache,
	limit: Semaphore,
	blocked_until: Mutex<Option<Instant>>,
	tags_file: PathBuf,
	/// Срок одного запроса вместе с повторами.
	deadline: Duration,
}

impl CurseForge {
	/// `client` — из [`build_client`] (с ключом); `tags_file` — копия категорий на диске.
	pub fn new(client: reqwest::Client, base: &str, tags_file: PathBuf) -> Result<Self, AppError> {
		let base = Url::parse(base).map_err(|e| AppError::unknown(format!("{base}: {e}")))?;
		Ok(Self {
			client,
			base,
			cache: ResponseCache::default(),
			limit: Semaphore::new(CONCURRENCY),
			blocked_until: Mutex::new(None),
			tags_file,
			deadline: super::FETCH_DEADLINE,
		})
	}

	#[cfg(test)]
	pub fn with_deadline(mut self, deadline: Duration) -> Self {
		self.deadline = deadline;
		self
	}

	fn url(&self, segments: &[&str]) -> Url {
		let mut url = self.base.clone();
		if let Ok(mut path) = url.path_segments_mut() {
			path.pop_if_empty().push("v1").extend(segments);
		}
		url
	}

	async fn fetch(&self, req: Request, ttl: Duration) -> Result<(Arc<Vec<u8>>, bool), AppError> {
		let key = req.key();
		if let Some(body) = self.cache.fresh(&key) {
			return Ok((body, false));
		}
		let connect_failed = AtomicBool::new(false);
		let sent = self.send(&req, &connect_failed).await;
		match sent {
			Ok(body) => {
				let body = Arc::new(body);
				self.cache.put(key, ttl, body.clone());
				Ok((body, false))
			}
			Err(err) if matches!(err.code, ErrorCode::Network | ErrorCode::SourceUnavailable) => {
				match self.cache.stale(&key) {
					Some(body) => Ok((body, true)),
					None => Err(err),
				}
			}
			Err(err) => Err(err),
		}
	}

	async fn json<T: DeserializeOwned>(
		&self,
		req: Request,
		ttl: Duration,
	) -> Result<(T, bool), AppError> {
		let (body, offline) = self.fetch(req, ttl).await?;
		let value = serde_json::from_slice(&body).map_err(|e| {
			AppError::coded(
				ErrorCode::SourceUnavailable,
				format!("curseforge: ответ не разобран: {e}"),
			)
		})?;
		Ok((value, offline))
	}

	/// Ответ вида `{ "data": … }`.
	async fn data<T: DeserializeOwned>(
		&self,
		req: Request,
		ttl: Duration,
	) -> Result<(T, bool), AppError> {
		let (wrapped, offline): (Data<T>, bool) = self.json(req, ttl).await?;
		Ok((wrapped.data, offline))
	}

	async fn wait_rate(&self) -> Result<(), AppError> {
		let until = self.blocked_until.lock().ok().and_then(|b| *b);
		let Some(until) = until else { return Ok(()) };
		let now = Instant::now();
		if until <= now {
			return Ok(());
		}
		let wait = until - now;
		if wait > MAX_RATE_WAIT || super::wait_exceeds_command(wait) {
			return Err(AppError::coded(
				ErrorCode::RateLimited,
				format!("curseforge: лимит запросов, ещё {} с", wait.as_secs() + 1),
			));
		}
		tokio::time::sleep(wait).await;
		Ok(())
	}

	/// Лимиты API в документации не описаны: на 429 ждём `Retry-After` (иначе минуту).
	fn note_limited(&self, headers: &reqwest::header::HeaderMap) {
		let retry = headers
			.get("retry-after")
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.trim().parse::<u64>().ok())
			.unwrap_or(60)
			.clamp(1, 120);
		if let Ok(mut blocked) = self.blocked_until.lock() {
			*blocked = Some(Instant::now() + Duration::from_secs(retry));
		}
	}

	async fn send(&self, req: &Request, connect_failed: &AtomicBool) -> Result<Vec<u8>, AppError> {
		// Очередь и пауза лимита — вне срока запроса (но в сроке команды, см. `super::within`).
		let _permit = super::waiting(self.limit.acquire())
			.await
			.map_err(|_| AppError::unknown("curseforge: семафор закрыт"))?;
		let mut network_attempt = 0;
		let mut waited_429 = false;
		// Срок — только у отправки и повторов: считается с первой отправки.
		let mut left = self.deadline;
		loop {
			super::waiting(self.wait_rate()).await?;
			let started = tokio::time::Instant::now();
			let step = tokio::time::timeout(left, async {
				let builder = match req {
					Request::Get(url) => self.client.get(url.clone()),
					Request::Post(url, body) => self.client.post(url.clone()).json(body),
				};
				let resp = match builder.send().await {
					Ok(resp) => {
						connect_failed.store(false, Ordering::Relaxed);
						resp
					}
					Err(err) => {
						if err.is_connect() {
							connect_failed.store(true, Ordering::Relaxed);
						}
						if (err.is_connect() || err.is_timeout())
							&& network_attempt < NETWORK_RETRIES
						{
							network_attempt += 1;
							tokio::time::sleep(Duration::from_millis(
								500 * u64::from(network_attempt),
							))
							.await;
							return Ok(None);
						}
						return Err(AppError::coded(
							ErrorCode::Network,
							format!("curseforge: {}", err.without_url()),
						));
					}
				};
				let status = resp.status().as_u16();
				match status {
					200..=299 => {
						return resp.bytes().await.map(|b| Some(b.to_vec())).map_err(|e| {
							AppError::coded(
								ErrorCode::Network,
								format!("curseforge: {}", e.without_url()),
							)
						});
					}
					429 => {
						self.note_limited(resp.headers());
						if waited_429 {
							return Err(AppError::coded(
								ErrorCode::RateLimited,
								"curseforge: HTTP 429",
							));
						}
						waited_429 = true;
					}
					401 | 403 => {
						log::warn!("curseforge: HTTP {status} — ключ API не принят");
						return Err(AppError::coded(ErrorCode::SourceDisabled, DENIED));
					}
					404 => {
						return Err(AppError::coded(
							ErrorCode::ContentNotFound,
							format!("curseforge: HTTP 404 {}", resp.url().path()),
						));
					}
					500..=599 => {
						return Err(AppError::coded(
							ErrorCode::SourceUnavailable,
							format!("curseforge: HTTP {status}"),
						));
					}
					_ => {
						let body = resp.text().await.unwrap_or_default();
						let body: String = body.chars().take(300).collect();
						return Err(AppError::unknown(format!(
							"curseforge: HTTP {status}: {body}"
						)));
					}
				}
				Ok(None)
			})
			.await;
			left = left.saturating_sub(started.elapsed());
			match step {
				Ok(Ok(Some(body))) => return Ok(body),
				Ok(Ok(None)) => {}
				Ok(Err(err)) => return Err(err),
				Err(_) => {
					return Err(super::deadline_error(
						"curseforge",
						connect_failed.load(Ordering::Relaxed),
					));
				}
			}
		}
	}

	/// Числовой id проекта: число как есть, slug — через поиск `slug=` (с разделом — однозначно).
	async fn mod_id(&self, id: &str) -> Result<u64, AppError> {
		self.mod_id_in(id, None).await
	}

	async fn mod_id_in(&self, id: &str, kind: Option<ProjectType>) -> Result<u64, AppError> {
		if let Some(n) = numeric(id) {
			return Ok(n);
		}
		let mut url = self.url(&["mods", "search"]);
		url.query_pairs_mut()
			.append_pair("gameId", &GAME_ID.to_string())
			.append_pair("slug", id);
		if let Some(kind) = kind {
			url.query_pairs_mut()
				.append_pair("classId", &class_of(kind).to_string());
		}
		let (found, _): (Vec<RawMod>, bool) = self.data(Request::Get(url), TTL_PROJECT).await?;
		found
			.first()
			.map(|m| m.id)
			.ok_or_else(|| AppError::coded(ErrorCode::ContentNotFound, format!("curseforge: {id}")))
	}

	async fn search_impl(&self, q: &SearchQuery) -> Result<SearchPage, AppError> {
		let limit = q.limit.unwrap_or(SEARCH_LIMIT_DEFAULT).clamp(1, PAGE_MAX);
		if q.offset.saturating_add(limit) > RESULTS_MAX {
			return Ok(SearchPage::new(
				Vec::new(),
				q.offset,
				limit,
				u64::from(RESULTS_MAX),
				false,
			));
		}
		let category_ids = if q.categories.is_empty() {
			Vec::new()
		} else {
			self.category_ids(class_of(q.project_type), &q.categories)
				.await
		};
		let url = self.search_url(q, limit, &category_ids);
		let (raw, offline): (RawSearch, bool) = self.json(Request::Get(url), TTL_SEARCH).await?;
		Ok(SearchPage::new(
			raw.data
				.into_iter()
				.map(|m| m.hit(Some(q.project_type)))
				.collect(),
			raw.pagination.index,
			raw.pagination.page_size.max(limit),
			raw.pagination.total_count.min(u64::from(RESULTS_MAX)),
			offline,
		))
	}

	fn search_url(&self, q: &SearchQuery, limit: u32, category_ids: &[u32]) -> Url {
		let mut url = self.url(&["mods", "search"]);
		{
			let mut pairs = url.query_pairs_mut();
			pairs
				.append_pair("gameId", &GAME_ID.to_string())
				.append_pair("classId", &class_of(q.project_type).to_string());
			let text = q.query.trim();
			if !text.is_empty() {
				pairs.append_pair("searchFilter", text);
			}
			let order = if q.sort == SearchSort::Name {
				"asc"
			} else {
				"desc"
			};
			pairs
				.append_pair("sortField", &sort_field(q.sort).to_string())
				.append_pair("sortOrder", order);
			let versions: Vec<&str> = q
				.game_versions
				.iter()
				.map(|v| v.trim())
				.filter(|v| !v.is_empty())
				.take(4)
				.collect();
			match versions.as_slice() {
				[] => {}
				[one] => {
					pairs.append_pair("gameVersion", one);
				}
				many => {
					pairs.append_pair("gameVersions", &json!(many).to_string());
				}
			}
			let mut loaders: Vec<u32> = q.loaders.iter().filter_map(|l| loader_type(l)).collect();
			loaders.sort_unstable();
			loaders.dedup();
			if !loaders.is_empty() {
				pairs.append_pair("modLoaderTypes", &json!(loaders).to_string());
			}
			if !category_ids.is_empty() {
				pairs.append_pair("categoryIds", &json!(category_ids).to_string());
			}
			pairs
				.append_pair("index", &q.offset.to_string())
				.append_pair("pageSize", &limit.to_string());
		}
		url
	}

	/// slug категорий раздела → id (незнакомые пропускаются; не больше 10 — ограничение API).
	async fn category_ids(&self, class: u32, slugs: &[String]) -> Vec<u32> {
		let all = match self.raw_categories().await {
			Ok((all, _)) => all,
			Err(err) => {
				log::warn!("curseforge: категории для поиска: {:?}", err.code);
				return Vec::new();
			}
		};
		slugs
			.iter()
			.filter_map(|slug| {
				let found = all
					.iter()
					.find(|c| c.class_id == Some(class) && &c.slug == slug)
					.map(|c| c.id);
				if found.is_none() {
					log::info!("curseforge: категории {slug} нет в разделе {class}");
				}
				found
			})
			.take(10)
			.collect()
	}

	async fn raw_mod(&self, id: u64) -> Result<RawMod, AppError> {
		let url = self.url(&["mods", &id.to_string()]);
		Ok(self.data(Request::Get(url), TTL_PROJECT).await?.0)
	}

	/// Проекты пачкой (`POST /v1/mods`).
	async fn raw_mods(&self, ids: &[u64]) -> Result<Vec<RawMod>, AppError> {
		let mut out = Vec::new();
		for chunk in ids.chunks(BATCH) {
			let body = json!({ "modIds": chunk, "filterPcOnly": true });
			let (mut mods, _): (Vec<RawMod>, bool) = self
				.data(Request::Post(self.url(&["mods"]), body), TTL_PROJECT)
				.await?;
			out.append(&mut mods);
		}
		Ok(out)
	}

	/// Сведения о проектах для версий. Не получилось — пусто (версии всё равно полезны).
	async fn infos(&self, ids: impl IntoIterator<Item = u64>) -> HashMap<u64, ModInfo> {
		let mut ids: Vec<u64> = ids.into_iter().collect();
		ids.sort_unstable();
		ids.dedup();
		if ids.is_empty() {
			return HashMap::new();
		}
		match self.raw_mods(&ids).await {
			Ok(mods) => mods.into_iter().map(|m| (m.id, m.info())).collect(),
			Err(err) => {
				log::warn!("curseforge: проекты версий: {:?}", err.code);
				HashMap::new()
			}
		}
	}

	async fn project_impl(&self, id: &str, kind: Option<ProjectType>) -> Result<Project, AppError> {
		let id = self.mod_id_in(id, kind).await?;
		let sid = id.to_string();
		let description = self.data::<String>(
			Request::Get(self.url(&["mods", &sid, "description"])),
			TTL_PROJECT,
		);
		let files = {
			let mut url = self.url(&["mods", &sid, "files"]);
			url.query_pairs_mut().append_pair("pageSize", "1");
			self.json::<RawFiles>(Request::Get(url), TTL_PROJECT)
		};
		let (raw, description, files) = tokio::join!(self.raw_mod(id), description, files);
		let raw = raw?;
		let body = match description {
			Ok((body, _)) => body,
			Err(err) => {
				log::warn!("curseforge: описание {id}: {:?}", err.code);
				raw.summary.clone()
			}
		};
		let versions_count = files.map_or(0, |(f, _)| f.pagination.total_count as usize);
		Ok(raw.into_project(body, versions_count))
	}

	async fn projects_impl(&self, ids: &[String]) -> Result<Vec<ProjectHit>, AppError> {
		let mut numbers = Vec::new();
		for id in ids {
			match numeric(id) {
				Some(n) => numbers.push(n),
				None => match self.mod_id(id).await {
					Ok(n) => numbers.push(n),
					Err(err) if err.code == ErrorCode::ContentNotFound => {}
					Err(err) => return Err(err),
				},
			}
		}
		numbers.sort_unstable();
		numbers.dedup();
		Ok(self
			.raw_mods(&numbers)
			.await?
			.into_iter()
			.map(|m| m.hit(None))
			.collect())
	}

	async fn project_versions_impl(
		&self,
		id: &str,
		filter: &VersionFilter,
	) -> Result<Vec<ContentVersion>, AppError> {
		let id = self.mod_id(id).await?;
		let info = match self.raw_mod(id).await {
			Ok(m) => m.info(),
			Err(err) if err.code == ErrorCode::ContentNotFound => return Err(err),
			Err(err) => {
				log::warn!("curseforge: проект {id}: {:?}", err.code);
				ModInfo::default()
			}
		};
		let game_version = match filter.game_versions.as_slice() {
			[one] => Some(one.clone()),
			_ => None,
		};
		let mut types: Vec<u32> = filter
			.loaders
			.iter()
			.filter_map(|l| loader_type(l))
			.collect();
		types.dedup();
		let loader = match (&game_version, types.as_slice()) {
			(Some(_), [one]) if filter.loaders.len() == 1 => Some(*one),
			_ => None,
		};
		let mut versions = Vec::new();
		let mut index = 0u32;
		for _ in 0..FILE_PAGES {
			let mut url = self.url(&["mods", &id.to_string(), "files"]);
			{
				let mut pairs = url.query_pairs_mut();
				if let Some(gv) = &game_version {
					pairs.append_pair("gameVersion", gv);
				}
				if let Some(loader) = loader {
					pairs.append_pair("modLoaderType", &loader.to_string());
				}
				pairs
					.append_pair("index", &index.to_string())
					.append_pair("pageSize", &PAGE_MAX.to_string());
			}
			let (page, _): (RawFiles, bool) = self.json(Request::Get(url), TTL_PROJECT).await?;
			let got = page.data.len() as u32;
			versions.extend(
				page.data
					.into_iter()
					.filter_map(|f| f.into_version(Some(&info)))
					.filter(|v| filter.matches(v)),
			);
			index += got;
			if got < PAGE_MAX || u64::from(index) >= page.pagination.total_count {
				break;
			}
		}
		versions.sort_by_key(|a| std::cmp::Reverse(a.published_at));
		Ok(versions)
	}

	/// Файлы по id пачкой (`POST /v1/mods/files`) с разделом и страницей проекта.
	async fn versions_impl(&self, ids: &[String]) -> Result<Vec<ContentVersion>, AppError> {
		let mut numbers: Vec<u64> = ids.iter().filter_map(|i| numeric(i)).collect();
		numbers.sort_unstable();
		numbers.dedup();
		let mut files: Vec<RawFile> = Vec::new();
		for chunk in numbers.chunks(BATCH) {
			let body = json!({ "fileIds": chunk });
			let (mut got, _): (Vec<RawFile>, bool) = self
				.data(
					Request::Post(self.url(&["mods", "files"]), body),
					TTL_PROJECT,
				)
				.await?;
			files.append(&mut got);
		}
		let infos = self.infos(files.iter().map(|f| f.mod_id)).await;
		Ok(files
			.into_iter()
			.filter_map(|f| {
				let info = infos.get(&f.mod_id).cloned();
				f.into_version(info.as_ref())
			})
			.collect())
	}

	async fn version_impl(&self, id: &str) -> Result<ContentVersion, AppError> {
		let mut version = self
			.versions_impl(std::slice::from_ref(&id.to_owned()))
			.await?
			.into_iter()
			.next()
			.ok_or_else(|| {
				AppError::coded(ErrorCode::ContentNotFound, format!("curseforge: {id}"))
			})?;
		let url = self.url(&[
			"mods",
			&version.project_id,
			"files",
			&version.id,
			"changelog",
		]);
		match self.data::<String>(Request::Get(url), TTL_PROJECT).await {
			Ok((changelog, _)) => version.changelog = non_empty(Some(changelog)),
			Err(err) => log::warn!("curseforge: изменения {id}: {:?}", err.code),
		}
		Ok(version)
	}

	async fn raw_categories(&self) -> Result<(Vec<RawCategory>, bool), AppError> {
		let mut url = self.url(&["categories"]);
		url.query_pairs_mut()
			.append_pair("gameId", &GAME_ID.to_string());
		match self
			.data::<Vec<RawCategory>>(Request::Get(url), TTL_TAGS)
			.await
		{
			Ok((categories, offline)) => {
				if !offline {
					self.save_tags(&categories);
				}
				Ok((categories, offline))
			}
			Err(err) if !matches!(err.code, ErrorCode::Network | ErrorCode::SourceUnavailable) => {
				Err(err)
			}
			Err(err) => {
				log::warn!("curseforge: категории без сети: {:?}", err.code);
				match self.load_tags() {
					Some(stored) => Ok((stored, true)),
					None => Err(AppError::new(ErrorCode::Network)),
				}
			}
		}
	}

	async fn categories_impl(&self) -> Result<(Vec<CategoryTag>, bool), AppError> {
		let (all, offline) = self.raw_categories().await?;
		Ok((categories_of(all), offline))
	}

	fn save_tags(&self, tags: &[RawCategory]) {
		let result = serde_json::to_vec(tags)
			.map_err(std::io::Error::other)
			.and_then(|json| crate::fs_util::write_atomic(&self.tags_file, &json));
		if let Err(err) = result {
			log::warn!("не удалось сохранить категории CurseForge: {err}");
		}
	}

	fn load_tags(&self) -> Option<Vec<RawCategory>> {
		let bytes = std::fs::read(&self.tags_file).ok()?;
		serde_json::from_slice(&bytes).ok()
	}

	/// Опознание по отпечаткам (`POST /v1/fingerprints/432`). Совпадение принимается, только если
	/// совпал и sha1 (отпечаток не видит пробельных байтов).
	async fn identify_impl(
		&self,
		files: &[FileKey],
	) -> Result<HashMap<String, ContentVersion>, AppError> {
		let mut by_fp: HashMap<u32, Vec<&FileKey>> = HashMap::new();
		for f in files {
			if let Some(fp) = f.fingerprint {
				by_fp.entry(fp).or_default().push(f);
			}
		}
		let mut fps: Vec<u32> = by_fp.keys().copied().collect();
		fps.sort_unstable();
		let mut matched: Vec<RawFile> = Vec::new();
		for chunk in fps.chunks(FINGERPRINT_BATCH) {
			let body = json!({ "fingerprints": chunk });
			let url = self.url(&["fingerprints", &GAME_ID.to_string()]);
			let (result, _): (RawFingerprints, bool) =
				self.data(Request::Post(url, body), TTL_HASHES).await?;
			matched.extend(result.exact_matches.into_iter().map(|m| m.file));
		}
		let infos = self.infos(matched.iter().map(|f| f.mod_id)).await;
		let mut out = HashMap::new();
		for file in matched {
			let Some(keys) = u32::try_from(file.file_fingerprint)
				.ok()
				.and_then(|fp| by_fp.get(&fp))
			else {
				continue;
			};
			let cf_sha1 = file.sha1();
			let info = infos.get(&file.mod_id).cloned();
			let Some(version) = file.into_version(info.as_ref()) else {
				continue;
			};
			for key in keys {
				let same = cf_sha1
					.as_deref()
					.is_none_or(|s| s.eq_ignore_ascii_case(&key.sha1));
				if same {
					out.insert(key.sha1.to_ascii_lowercase(), version.clone());
				}
			}
		}
		Ok(out)
	}

	/// Последние подходящие файлы проектов (по `latestFilesIndexes`; новее — больший `fileId`).
	async fn latest_impl(
		&self,
		files: &[FileKey],
		filter: &VersionFilter,
	) -> Result<HashMap<String, ContentVersion>, AppError> {
		let mut projects: Vec<u64> = files
			.iter()
			.filter_map(|f| f.project_id.as_deref().and_then(numeric))
			.collect();
		projects.sort_unstable();
		projects.dedup();
		if projects.is_empty() {
			return Ok(HashMap::new());
		}
		let mods = self.raw_mods(&projects).await?;
		let mut picked: HashMap<u64, u64> = HashMap::new();
		let mut infos: HashMap<u64, ModInfo> = HashMap::new();
		for m in &mods {
			infos.insert(m.id, m.info());
			if let Some(file) = pick_latest(&m.latest_files_indexes, filter) {
				picked.insert(m.id, file);
			}
		}
		let ids: Vec<String> = picked.values().map(|f| f.to_string()).collect();
		let mut by_project: HashMap<String, ContentVersion> = HashMap::new();
		for chunk in ids.chunks(BATCH) {
			let body =
				json!({ "fileIds": chunk.iter().filter_map(|i| numeric(i)).collect::<Vec<_>>() });
			let (got, _): (Vec<RawFile>, bool) = self
				.data(
					Request::Post(self.url(&["mods", "files"]), body),
					TTL_HASHES,
				)
				.await?;
			for f in got {
				let info = infos.get(&f.mod_id).cloned();
				if let Some(v) = f.into_version(info.as_ref()) {
					by_project.insert(v.project_id.clone(), v);
				}
			}
		}
		Ok(files
			.iter()
			.filter_map(|f| {
				let v = by_project.get(f.project_id.as_deref()?)?;
				Some((f.sha1.to_ascii_lowercase(), v.clone()))
			})
			.collect())
	}
}

/// Сторона по меткам `Client`/`Server`: нет ни одной — неизвестно (`null`).
fn side_of(game_versions: &[String]) -> Option<VersionSide> {
	let has = |name: &str| game_versions.iter().any(|v| v.eq_ignore_ascii_case(name));
	let (client, server) = (has("client"), has("server"));
	if !client && !server {
		return None;
	}
	let side = |yes: bool| {
		if yes {
			Side::Required
		} else {
			Side::Unsupported
		}
	};
	Some(VersionSide {
		client: side(client),
		server: side(server),
	})
}

/// `details` у `source_disabled`: нет ключа API в сборке / ключ не принят (401/403).
pub const NO_KEY: &str = "no_key";
pub const DENIED: &str = "denied";

/// Последний подходящий файл: версия игры из фильтра, загрузчик из фильтра (у ресурспаков,
/// шейдеров и т. п. загрузчика нет — подходит любой); новее — больший `fileId`.
fn pick_latest(indexes: &[RawFileIndex], filter: &VersionFilter) -> Option<u64> {
	indexes
		.iter()
		.filter(|i| {
			filter.game_versions.is_empty() || filter.game_versions.contains(&i.game_version)
		})
		.filter(|i| {
			let name = i.mod_loader.and_then(loader_name);
			filter.loaders.is_empty()
				|| name.is_none()
				|| filter
					.loaders
					.iter()
					.any(|l| Some(l.to_ascii_lowercase().as_str()) == name)
		})
		.map(|i| i.file_id)
		.max()
}

fn sort_field(sort: SearchSort) -> u32 {
	// ModsSearchSortField: 2 Popularity, 3 LastUpdated, 6 TotalDownloads, 11 ReleasedDate,
	// 12 Rating. «По релевантности» в API нет — ближе всего популярность.
	match sort {
		SearchSort::Relevance => 2,
		SearchSort::Downloads => 6,
		SearchSort::Follows => 12,
		SearchSort::Newest => 11,
		SearchSort::Updated => 3,
		SearchSort::Name => 4,
	}
}

fn categories_of(all: Vec<RawCategory>) -> Vec<CategoryTag> {
	let slugs: HashMap<u32, String> = all.iter().map(|c| (c.id, c.slug.clone())).collect();
	let mut tags: Vec<CategoryTag> = all
		.into_iter()
		.filter(|c| !c.is_class.unwrap_or(false))
		.filter_map(|c| {
			let class = c.class_id?;
			let kind = match class {
				class::MODS => ProjectType::Mod,
				class::RESOURCEPACKS => ProjectType::Resourcepack,
				class::SHADERS => ProjectType::Shader,
				class::DATAPACKS => ProjectType::Datapack,
				class::MODPACKS => ProjectType::Modpack,
				class::WORLDS => ProjectType::World,
				_ => return None,
			};
			// Родитель — другая категория (не сам раздел).
			let parent = c
				.parent_category_id
				.filter(|p| *p != class)
				.and_then(|p| slugs.get(&p).cloned());
			Some(CategoryTag {
				name: if c.name.is_empty() {
					c.slug.clone()
				} else {
					c.name
				},
				slug: c.slug,
				project_type: kind.as_str().to_owned(),
				header: "categories".into(),
				icon: String::new(),
				icon_url: non_empty(c.icon_url),
				parent,
			})
		})
		.collect();
	tags.sort_by(|a, b| (&a.project_type, &a.name).cmp(&(&b.project_type, &b.name)));
	tags
}

impl ContentSource for CurseForge {
	fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxFuture<'a, SearchPage> {
		Box::pin(self.search_impl(query))
	}

	fn project<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Project> {
		Box::pin(self.project_impl(id, None))
	}

	fn project_by_slug<'a>(&'a self, slug: &'a str, kind: ProjectType) -> BoxFuture<'a, Project> {
		Box::pin(self.project_impl(slug, Some(kind)))
	}

	fn projects<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ProjectHit>> {
		Box::pin(self.projects_impl(ids))
	}

	fn project_versions<'a>(
		&'a self,
		id: &'a str,
		filter: &'a VersionFilter,
	) -> BoxFuture<'a, Vec<ContentVersion>> {
		Box::pin(self.project_versions_impl(id, filter))
	}

	fn version<'a>(&'a self, id: &'a str) -> BoxFuture<'a, ContentVersion> {
		Box::pin(self.version_impl(id))
	}

	fn versions<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ContentVersion>> {
		Box::pin(self.versions_impl(ids))
	}

	fn categories(&self) -> BoxFuture<'_, (Vec<CategoryTag>, bool)> {
		Box::pin(self.categories_impl())
	}

	fn identify<'a>(
		&'a self,
		files: &'a [FileKey],
	) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
		Box::pin(self.identify_impl(files))
	}

	fn latest<'a>(
		&'a self,
		files: &'a [FileKey],
		filter: &'a VersionFilter,
	) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
		Box::pin(self.latest_impl(files, filter))
	}
}

#[derive(Deserialize)]
struct Data<T> {
	data: T,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawPagination {
	#[serde(default)]
	index: u32,
	#[serde(default)]
	page_size: u32,
	#[serde(default)]
	total_count: u64,
}

#[derive(Deserialize)]
struct RawSearch {
	#[serde(default)]
	data: Vec<RawMod>,
	#[serde(default)]
	pagination: RawPagination,
}

#[derive(Deserialize)]
struct RawFiles {
	#[serde(default)]
	data: Vec<RawFile>,
	#[serde(default)]
	pagination: RawPagination,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawLinks {
	#[serde(default)]
	website_url: Option<String>,
	#[serde(default)]
	wiki_url: Option<String>,
	#[serde(default)]
	issues_url: Option<String>,
	#[serde(default)]
	source_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAuthor {
	#[serde(default)]
	id: u64,
	#[serde(default)]
	name: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawAsset {
	#[serde(default)]
	title: Option<String>,
	#[serde(default)]
	description: Option<String>,
	#[serde(default)]
	thumbnail_url: Option<String>,
	#[serde(default)]
	url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RawCategory {
	id: u32,
	#[serde(default)]
	name: String,
	#[serde(default)]
	slug: String,
	#[serde(default)]
	parent_category_id: Option<u32>,
	#[serde(default)]
	icon_url: Option<String>,
	#[serde(default)]
	is_class: Option<bool>,
	#[serde(default)]
	class_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFileIndex {
	#[serde(default)]
	game_version: String,
	file_id: u64,
	#[serde(default)]
	mod_loader: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMod {
	id: u64,
	#[serde(default)]
	name: String,
	#[serde(default)]
	slug: String,
	#[serde(default)]
	links: Option<RawLinks>,
	#[serde(default)]
	summary: String,
	#[serde(default, deserialize_with = "lenient_u64")]
	download_count: u64,
	#[serde(default)]
	categories: Vec<RawCategory>,
	#[serde(default)]
	class_id: Option<u32>,
	#[serde(default)]
	authors: Vec<RawAuthor>,
	#[serde(default)]
	logo: Option<RawAsset>,
	#[serde(default)]
	screenshots: Vec<RawAsset>,
	#[serde(default)]
	latest_files: Vec<RawFile>,
	#[serde(default)]
	latest_files_indexes: Vec<RawFileIndex>,
	#[serde(default)]
	date_created: String,
	#[serde(default)]
	date_modified: String,
	#[serde(default)]
	date_released: String,
	#[serde(default)]
	allow_mod_distribution: Option<bool>,
}

impl RawMod {
	fn website(&self) -> Option<String> {
		self.links
			.as_ref()
			.and_then(|l| non_empty(l.website_url.clone()))
			.filter(|u| u.starts_with("https://"))
	}

	fn info(&self) -> ModInfo {
		ModInfo {
			class: self.class_id,
			website: self.website(),
		}
	}

	fn hit(&self, kind: Option<ProjectType>) -> ProjectHit {
		let project_type = kind.unwrap_or_else(|| type_of_class(self.class_id));
		let mut game_versions: Vec<String> = Vec::new();
		let mut loaders: Vec<String> = Vec::new();
		let mut add_version = |v: &str| {
			if is_game_version(v) && !game_versions.iter().any(|x| x == v) {
				game_versions.push(v.to_owned());
			}
		};
		for index in &self.latest_files_indexes {
			add_version(&index.game_version);
			if let Some(name) = index.mod_loader.and_then(loader_name)
				&& !loaders.iter().any(|l| l == name)
			{
				loaders.push(name.to_owned());
			}
		}
		for file in &self.latest_files {
			for v in &file.game_versions {
				add_version(v);
			}
			for l in loaders_of(&file.game_versions, Some(class_of(project_type))) {
				if !loaders.contains(&l) {
					loaders.push(l);
				}
			}
		}
		if loaders.is_empty() {
			loaders = loaders_of(&[], Some(class_of(project_type)));
		}
		ProjectHit {
			source: SourceKind::Curseforge,
			id: self.id.to_string(),
			slug: if self.slug.is_empty() {
				self.id.to_string()
			} else {
				self.slug.clone()
			},
			project_type,
			title: self.name.clone(),
			author: self
				.authors
				.first()
				.map(|a| a.name.clone())
				.unwrap_or_default(),
			description: self.summary.clone(),
			icon_url: self.logo.as_ref().and_then(|l| {
				non_empty(l.thumbnail_url.clone()).or_else(|| non_empty(l.url.clone()))
			}),
			downloads: self.download_count,
			follows: None,
			site_only: self.allow_mod_distribution == Some(false),
			url: self
				.website()
				.unwrap_or_else(|| format!("{SITE}/projects/{}", self.id)),
			categories: self
				.categories
				.iter()
				.filter(|c| !c.is_class.unwrap_or(false))
				.map(|c| c.slug.clone())
				.filter(|s| !s.is_empty())
				.collect(),
			updated_at: [&self.date_released, &self.date_modified]
				.iter()
				.map(|d| timestamp_ms(d))
				.find(|t| *t > 0)
				.unwrap_or(0),
			game_versions,
			loaders,
			client_side: Side::Unknown,
			server_side: Side::Unknown,
			install: None,
			also_on: Vec::new(),
			created_at: timestamp_ms(&self.date_created),
		}
	}

	fn into_project(self, body: String, versions_count: usize) -> Project {
		let hit = self.hit(None);
		let links = self.links.unwrap_or_default();
		Project {
			links: Links {
				site: non_empty(links.website_url)
					.unwrap_or_else(|| format!("{SITE}/projects/{}", self.id)),
				source: non_empty(links.source_url),
				wiki: non_empty(links.wiki_url),
				issues: non_empty(links.issues_url),
				discord: None,
				donate: Vec::new(),
			},
			hit,
			body,
			gallery: self
				.screenshots
				.into_iter()
				.filter_map(|s| {
					let url = non_empty(s.url)?;
					Some(GalleryImage {
						thumb_url: non_empty(s.thumbnail_url).unwrap_or_else(|| url.clone()),
						url,
						title: non_empty(s.title),
						description: non_empty(s.description),
						featured: false,
					})
				})
				.collect(),
			license: None,
			versions_count,
			team: self
				.authors
				.into_iter()
				.map(|a| TeamMember {
					id: a.id.to_string(),
					username: a.name,
					avatar_url: None,
					role: "Author".into(),
				})
				.collect(),
			published_at: timestamp_ms(&self.date_created),
			best_version: None,
		}
	}
}

#[derive(Debug, Deserialize)]
struct RawHash {
	#[serde(default)]
	value: String,
	#[serde(default)]
	algo: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDependency {
	mod_id: u64,
	#[serde(default)]
	relation_type: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFile {
	id: u64,
	mod_id: u64,
	#[serde(default)]
	display_name: String,
	#[serde(default)]
	file_name: String,
	#[serde(default)]
	release_type: u8,
	#[serde(default)]
	hashes: Vec<RawHash>,
	#[serde(default)]
	file_date: String,
	#[serde(default, deserialize_with = "lenient_u64")]
	file_length: u64,
	#[serde(default, deserialize_with = "lenient_u64")]
	download_count: u64,
	#[serde(default)]
	download_url: Option<String>,
	#[serde(default)]
	game_versions: Vec<String>,
	#[serde(default)]
	dependencies: Vec<RawDependency>,
	#[serde(default)]
	is_server_pack: Option<bool>,
	#[serde(default, deserialize_with = "lenient_u64")]
	file_fingerprint: u64,
}

impl RawFile {
	fn sha1(&self) -> Option<String> {
		self.hashes
			.iter()
			.find(|h| h.algo == 1 && crate::fs_util::is_sha1_hex(&h.value))
			.map(|h| h.value.to_ascii_lowercase())
	}

	/// Файл как версия проекта. Серверные сборки модпаков и файлы без sha1 (проверить нечем)
	/// пропускаются. Нет адреса — версия `blocked` (автор запретил стороннюю загрузку).
	fn into_version(self, info: Option<&ModInfo>) -> Option<ContentVersion> {
		if self.is_server_pack == Some(true) {
			return None;
		}
		let sha1 = self.sha1()?;
		let class = info.and_then(|i| i.class);
		let url = non_empty(self.download_url.clone()).unwrap_or_default();
		let url = if url.is_empty() || is_cdn_url(&url) {
			url
		} else {
			log::warn!("curseforge: файл {} не с CDN CurseForge", self.id);
			String::new()
		};
		// Адрес не с CDN — тоже «только с сайта».
		let site_only = url.is_empty();
		let page_url = match info.and_then(|i| i.website.as_deref()) {
			Some(site) => format!("{}/download/{}", site.trim_end_matches('/'), self.id),
			None => format!("{SITE}/projects/{}", self.mod_id),
		};
		let side = side_of(&self.game_versions);
		let name = if self.display_name.trim().is_empty() {
			self.file_name.clone()
		} else {
			self.display_name.clone()
		};
		Some(ContentVersion {
			source: SourceKind::Curseforge,
			id: self.id.to_string(),
			project_id: self.mod_id.to_string(),
			version_number: name.clone(),
			name,
			channel: match self.release_type {
				2 => VersionChannel::Beta,
				3 => VersionChannel::Alpha,
				_ => VersionChannel::Release,
			},
			loaders: loaders_of(&self.game_versions, class),
			game_versions: self
				.game_versions
				.iter()
				.filter(|v| is_game_version(v))
				.cloned()
				.collect(),
			published_at: timestamp_ms(&self.file_date),
			downloads: self.download_count,
			changelog: None,
			files: vec![ContentFile {
				url,
				file_name: self.file_name,
				sha1,
				size: self.file_length,
				primary: true,
			}],
			dependencies: self
				.dependencies
				.into_iter()
				.filter_map(|d| {
					let kind = match d.relation_type {
						1 | 6 => DependencyKind::Embedded,
						2 => DependencyKind::Optional,
						3 => DependencyKind::Required,
						5 => DependencyKind::Incompatible,
						// 4 — «инструмент»: для игры не нужен.
						_ => return None,
					};
					Some(Dependency {
						project_id: Some(d.mod_id.to_string()),
						version_id: None,
						kind,
					})
				})
				.collect(),
			site_only,
			page_url: Some(page_url),
			side,
			installed: None,
		})
	}
}

#[derive(Deserialize)]
struct RawMatch {
	file: RawFile,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFingerprints {
	#[serde(default)]
	exact_matches: Vec<RawMatch>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::io::{AsyncReadExt, AsyncWriteExt};

	const SHA1: &str = "da39a3ee5e6b4b0d3255bfef95601890afd80709";

	fn file_json(id: u64, mod_id: u64, url: Option<&str>) -> Value {
		json!({
			"id": id, "gameId": 432, "modId": mod_id, "isAvailable": true,
			"displayName": format!("Mod {id}"), "fileName": format!("mod-{id}.jar"),
			"releaseType": 2, "fileStatus": 4,
			"hashes": [ { "value": "0cc175b9c0f1b6a831c399e269772661", "algo": 2 },
				{ "value": SHA1.to_uppercase(), "algo": 1 } ],
			"fileDate": "2024-01-01T00:00:00.000Z", "fileLength": 1234, "downloadCount": 5.0,
			"downloadUrl": url, "gameVersions": ["1.20.1", "Forge", "NeoForge", "Client", "Java 17"],
			"sortableGameVersions": [],
			"dependencies": [ { "modId": 2, "relationType": 3 }, { "modId": 3, "relationType": 2 },
				{ "modId": 4, "relationType": 5 }, { "modId": 5, "relationType": 1 },
				{ "modId": 6, "relationType": 4 } ],
			"isServerPack": false, "fileFingerprint": 3_376_380_438u64, "modules": []
		})
	}

	fn mod_json(id: u64, class: u32) -> Value {
		json!({
			"id": id, "gameId": 432, "name": "Jei", "slug": "jei",
			"links": { "websiteUrl": "https://www.curseforge.com/minecraft/mc-mods/jei/", "wikiUrl": "",
				"issuesUrl": "https://github.com/x/issues", "sourceUrl": null },
			"summary": "Items", "status": 4, "downloadCount": 3.5e8, "isFeatured": false,
			"primaryCategoryId": 1, "categories": [ { "id": 421, "slug": "library-api", "isClass": false, "classId": 6 } ],
			"classId": class, "authors": [ { "id": 7, "name": "mezz", "url": "" } ],
			"logo": { "id": 1, "modId": id, "title": "", "description": "", "thumbnailUrl": "https://media.forgecdn.net/t.png", "url": "https://media.forgecdn.net/l.png" },
			"screenshots": [ { "title": "Shot", "description": "", "thumbnailUrl": "https://t", "url": "https://u" } ],
			"mainFileId": 10, "latestFiles": [ file_json(10, id, Some("https://edge.forgecdn.net/files/1/10/mod-10.jar")) ],
			"latestFilesIndexes": [
				{ "gameVersion": "1.20.1", "fileId": 10, "filename": "a", "releaseType": 1, "modLoader": 1 },
				{ "gameVersion": "1.20.1", "fileId": 12, "filename": "b", "releaseType": 2, "modLoader": 6 },
				{ "gameVersion": "1.19.2", "fileId": 9, "filename": "c", "releaseType": 1, "modLoader": 1 }
			],
			"dateCreated": "2020-01-01T00:00:00Z", "dateModified": "2024-02-01T00:00:00Z",
			"dateReleased": "2024-01-01T00:00:00Z", "allowModDistribution": true, "thumbsUpCount": 9
		})
	}

	#[test]
	fn parses_file_as_version() {
		let raw: RawFile = serde_json::from_value(file_json(
			10,
			1,
			Some("https://edge.forgecdn.net/files/1/10/mod-10.jar"),
		))
		.unwrap();
		let info = ModInfo {
			class: Some(class::MODS),
			website: Some("https://www.curseforge.com/minecraft/mc-mods/jei/".into()),
		};
		let v = raw.into_version(Some(&info)).unwrap();
		assert_eq!((v.id.as_str(), v.project_id.as_str()), ("10", "1"));
		assert_eq!(v.channel, VersionChannel::Beta);
		assert_eq!(v.game_versions, vec!["1.20.1"]);
		assert_eq!(v.loaders, vec!["forge", "neoforge"]);
		assert_eq!(v.files[0].sha1, SHA1);
		assert_eq!(v.files[0].size, 1234);
		assert_eq!(v.downloads, 5);
		assert!(!v.site_only);
		assert_eq!(
			v.page_url.as_deref(),
			Some("https://www.curseforge.com/minecraft/mc-mods/jei/download/10")
		);
		let kinds: Vec<_> = v.dependencies.iter().map(|d| d.kind).collect();
		assert_eq!(
			kinds,
			vec![
				DependencyKind::Required,
				DependencyKind::Optional,
				DependencyKind::Incompatible,
				DependencyKind::Embedded
			]
		);
		assert_eq!(v.published_at, 1_704_067_200_000);

		// Запрет распространения: адреса нет, версия помечена, ссылка — на страницу.
		let raw: RawFile = serde_json::from_value(file_json(11, 1, None)).unwrap();
		let v = raw.into_version(None).unwrap();
		assert!(v.site_only);
		assert!(v.files[0].url.is_empty());
		assert_eq!(
			v.page_url.as_deref(),
			Some("https://www.curseforge.com/projects/1")
		);

		// Адрес не с CDN (чужой хост или http) — тоже «только с сайта», ключ туда не уходит.
		for url in [
			"https://evil.example/mod-12.jar",
			"http://edge.forgecdn.net/files/1/12/mod-12.jar",
		] {
			let raw: RawFile = serde_json::from_value(file_json(12, 1, Some(url))).unwrap();
			let v = raw.into_version(None).unwrap();
			assert!(v.site_only, "{url}");
			assert!(v.files[0].url.is_empty());
		}
		let json = serde_json::to_value(&v).unwrap();
		assert_eq!(json["siteOnly"], true);

		// Адрес не с CDN CurseForge не берём.
		let raw: RawFile =
			serde_json::from_value(file_json(12, 1, Some("https://evil.example.com/a.jar")))
				.unwrap();
		assert!(raw.into_version(None).unwrap().files[0].url.is_empty());
		let raw: RawFile =
			serde_json::from_value(file_json(12, 1, Some("http://edge.forgecdn.net/a.jar")))
				.unwrap();
		assert!(raw.into_version(None).unwrap().files[0].url.is_empty());

		// Без sha1 и серверная сборка — не версия.
		let mut j = file_json(13, 1, None);
		j["hashes"] = json!([{ "value": "x", "algo": 2 }]);
		assert!(
			serde_json::from_value::<RawFile>(j)
				.unwrap()
				.into_version(None)
				.is_none()
		);
		let mut j = file_json(14, 1, None);
		j["isServerPack"] = json!(true);
		assert!(
			serde_json::from_value::<RawFile>(j)
				.unwrap()
				.into_version(None)
				.is_none()
		);

		// Ресурспак без загрузчиков — `minecraft`, старый мод без отметки — `forge`.
		let mut j = file_json(15, 1, None);
		j["gameVersions"] = json!(["1.12.2"]);
		let rp = ModInfo {
			class: Some(class::RESOURCEPACKS),
			website: None,
		};
		let raw: RawFile = serde_json::from_value(j.clone()).unwrap();
		assert_eq!(
			raw.into_version(Some(&rp)).unwrap().loaders,
			vec!["minecraft"]
		);
		let raw: RawFile = serde_json::from_value(j).unwrap();
		assert_eq!(raw.into_version(None).unwrap().loaders, vec!["forge"]);
	}

	#[test]
	fn parses_mod_as_project() {
		let raw: RawMod = serde_json::from_value(mod_json(238222, class::MODS)).unwrap();
		let hit = raw.hit(None);
		assert_eq!(hit.source, SourceKind::Curseforge);
		assert_eq!((hit.id.as_str(), hit.slug.as_str()), ("238222", "jei"));
		assert_eq!(hit.project_type, ProjectType::Mod);
		assert_eq!(hit.author, "mezz");
		assert_eq!(hit.downloads, 350_000_000);
		assert_eq!(hit.follows, None);
		assert!(!hit.site_only);
		assert_eq!(hit.url, "https://www.curseforge.com/minecraft/mc-mods/jei/");
		assert_eq!(hit.categories, vec!["library-api"]);
		assert_eq!(hit.game_versions, vec!["1.20.1", "1.19.2"]);
		assert_eq!(hit.loaders, vec!["forge", "neoforge"]);
		assert_eq!(
			hit.icon_url.as_deref(),
			Some("https://media.forgecdn.net/t.png")
		);
		assert_eq!(hit.updated_at, 1_704_067_200_000);

		let p = raw.into_project("<p>Body</p>".into(), 42);
		assert_eq!(
			p.links.site,
			"https://www.curseforge.com/minecraft/mc-mods/jei/"
		);
		assert_eq!(
			p.links.issues.as_deref(),
			Some("https://github.com/x/issues")
		);
		assert!(p.links.wiki.is_none());
		assert_eq!(p.versions_count, 42);
		assert_eq!(p.team[0].username, "mezz");
		assert_eq!(p.gallery[0].title.as_deref(), Some("Shot"));
		assert_eq!(p.body, "<p>Body</p>");

		let raw: RawMod = serde_json::from_value(mod_json(1, class::SHADERS)).unwrap();
		assert_eq!(raw.hit(None).project_type, ProjectType::Shader);
		assert_eq!(type_of_class(Some(class::WORLDS)), ProjectType::World);
		assert_eq!(type_of_class(Some(5)), ProjectType::Mod);
	}

	#[test]
	fn picks_latest_and_maps_categories() {
		let raw: RawMod = serde_json::from_value(mod_json(1, class::MODS)).unwrap();
		let filter = |g: &[&str], l: &[&str]| VersionFilter {
			game_versions: g.iter().map(|s| (*s).to_owned()).collect(),
			loaders: l.iter().map(|s| (*s).to_owned()).collect(),
		};
		let idx = &raw.latest_files_indexes;
		assert_eq!(pick_latest(idx, &filter(&["1.20.1"], &["forge"])), Some(10));
		assert_eq!(
			pick_latest(idx, &filter(&["1.20.1"], &["neoforge", "forge"])),
			Some(12)
		);
		assert_eq!(pick_latest(idx, &filter(&["1.19.2"], &[])), Some(9));
		assert_eq!(pick_latest(idx, &filter(&["1.21"], &["forge"])), None);
		assert_eq!(pick_latest(idx, &filter(&["1.20.1"], &["fabric"])), None);

		let cats: Vec<RawCategory> = serde_json::from_value(json!([
			{ "id": 6, "slug": "mc-mods", "isClass": true, "classId": null },
			{ "id": 421, "slug": "library-api", "isClass": false, "classId": 6, "iconUrl": "https://i.png", "parentCategoryId": 6 },
			{ "id": 4843, "name": "Automation", "slug": "technology-automation", "isClass": false, "classId": 6, "parentCategoryId": 412 },
			{ "id": 412, "name": "Technology", "slug": "technology", "isClass": false, "classId": 6, "parentCategoryId": 6 },
			{ "id": 248, "name": "Adventure", "slug": "adventure", "isClass": false, "classId": 17, "parentCategoryId": 17 },
			{ "id": 4473, "name": "Magic", "slug": "magic", "isClass": false, "classId": 4471, "parentCategoryId": 4471 }
		]))
		.unwrap();
		let tags = categories_of(cats);
		let kinds: Vec<&str> = tags.iter().map(|t| t.project_type.as_str()).collect();
		assert_eq!(kinds, vec!["mod", "mod", "mod", "modpack", "world"]);
		let by = |slug: &str| tags.iter().find(|t| t.slug == slug).unwrap();
		assert_eq!(by("library-api").parent, None);
		assert_eq!(by("library-api").icon_url.as_deref(), Some("https://i.png"));
		assert_eq!(by("technology-automation").name, "Automation");
		assert_eq!(
			by("technology-automation").parent.as_deref(),
			Some("technology")
		);
		assert_eq!(by("adventure").parent, None);
	}

	#[test]
	fn builds_search_url() {
		let cf = CurseForge::new(
			reqwest::Client::new(),
			"https://api.curseforge.com",
			PathBuf::from("x.json"),
		)
		.unwrap();
		let q: SearchQuery = serde_json::from_value(json!({
			"source": "curseforge", "type": "mod", "query": "jei",
			"gameVersions": ["1.20.1"], "loaders": ["quilt", "fabric", "iris"],
			"sort": "downloads", "offset": 40, "limit": 100
		}))
		.unwrap();
		let url = cf.search_url(&q, 50, &[421]);
		let pairs: HashMap<String, String> = url.query_pairs().into_owned().collect();
		assert_eq!(url.path(), "/v1/mods/search");
		assert_eq!(pairs["gameId"], "432");
		assert_eq!(pairs["classId"], "6");
		assert_eq!(pairs["searchFilter"], "jei");
		assert_eq!(pairs["sortField"], "6");
		assert_eq!(pairs["sortOrder"], "desc");
		assert_eq!(pairs["gameVersion"], "1.20.1");
		assert_eq!(pairs["modLoaderTypes"], "[4,5]");
		assert_eq!(pairs["categoryIds"], "[421]");
		assert_eq!(
			(pairs["index"].as_str(), pairs["pageSize"].as_str()),
			("40", "50")
		);

		let q: SearchQuery = serde_json::from_value(json!({
			"type": "shader", "gameVersions": ["1.20.1", "1.20.4"]
		}))
		.unwrap();
		let pairs: HashMap<String, String> = cf
			.search_url(&q, 20, &[])
			.query_pairs()
			.into_owned()
			.collect();
		assert_eq!(pairs["classId"], "6552");
		assert_eq!(pairs["gameVersions"], r#"["1.20.1","1.20.4"]"#);
		assert!(!pairs.contains_key("modLoaderTypes") && !pairs.contains_key("searchFilter"));
	}

	#[test]
	fn key_and_cdn() {
		assert!(is_cdn_url("https://edge.forgecdn.net/files/1/2/a.jar"));
		assert!(is_cdn_url(
			"https://mediafilez.forgecdn.net/files/1/2/a.jar"
		));
		assert!(!is_cdn_url("https://forgecdn.net.evil.com/a.jar"));
		assert!(!is_cdn_url("http://edge.forgecdn.net/a.jar"));
		let value = key_header("$2a$10$abc").unwrap();
		assert!(value.is_sensitive());
		assert!(key_header("bad\nkey").is_err());
		assert_eq!(numeric("238222"), Some(238_222));
		assert_eq!(numeric("jei"), None);
	}

	/// Локальный сервер с заготовленными ответами; запоминает запросы.
	async fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		let seen = Arc::new(Mutex::new(Vec::new()));
		let log = seen.clone();
		tokio::spawn(async move {
			for response in responses {
				let Ok((mut socket, _)) = listener.accept().await else {
					return;
				};
				let mut buf = vec![0u8; 64 * 1024];
				let n = socket.read(&mut buf).await.unwrap_or(0);
				let head = String::from_utf8_lossy(&buf[..n]).into_owned();
				let first = head.lines().next().unwrap_or_default().to_owned();
				let key = head
					.lines()
					.find(|l| l.to_ascii_lowercase().starts_with("x-api-key:"))
					.unwrap_or_default()
					.to_owned();
				log.lock().unwrap().push(format!("{first} | {key}"));
				let _ = socket.write_all(response.as_bytes()).await;
				let _ = socket.shutdown().await;
			}
		});
		(format!("http://{addr}"), seen)
	}

	fn response(status: &str, body: &str) -> String {
		format!(
			"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
			body.len()
		)
	}

	fn client(base: &str) -> CurseForge {
		let dir = std::env::temp_dir().join(format!("ll-cf-{}", rand::random::<u32>()));
		CurseForge::new(
			build_client("test-key").unwrap(),
			base,
			dir.join("cats.json"),
		)
		.unwrap()
	}

	/// Сервер, который принимает соединение и молчит.
	async fn silent() -> String {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		tokio::spawn(async move {
			let mut held = Vec::new();
			while let Ok((socket, _)) = listener.accept().await {
				held.push(socket);
			}
		});
		format!("http://{addr}")
	}

	#[tokio::test]
	async fn silent_server_times_out() {
		let base = silent().await;
		let cf = client(&base).with_deadline(Duration::from_millis(500));
		let started = Instant::now();
		let q: SearchQuery = serde_json::from_value(json!({ "type": "mod" })).unwrap();
		let err = cf.search_impl(&q).await.unwrap_err();
		assert!(started.elapsed() < Duration::from_secs(3));
		assert_eq!(err.code, ErrorCode::SourceUnavailable);
		assert_eq!(
			err.details.as_deref(),
			Some(crate::content::TIMEOUT_DETAILS)
		);
	}

	#[tokio::test]
	async fn identifies_by_fingerprint_and_maps_errors() {
		let matches = json!({ "data": {
			"isCacheBuilt": true,
			"exactMatches": [ { "id": 1, "file": file_json(10, 1, None), "latestFiles": [] } ],
			"exactFingerprints": [3_376_380_438u64], "partialMatches": [],
			"partialMatchFingerprints": {}, "installedFingerprints": [], "unmatchedFingerprints": [7]
		}});
		let mods = json!({ "data": [ mod_json(1, class::MODS) ] });
		let (base, seen) = serve(vec![
			response("200 OK", &matches.to_string()),
			response("200 OK", &mods.to_string()),
			response("403 Forbidden", "{}"),
			response("404 Not Found", "{}"),
		])
		.await;
		let cf = client(&base);
		let files = vec![
			FileKey {
				sha1: SHA1.into(),
				fingerprint: Some(3_376_380_438),
				project_id: None,
			},
			// Тот же отпечаток, другой sha1 — не совпадение.
			FileKey {
				sha1: "0000000000000000000000000000000000000000".into(),
				fingerprint: Some(3_376_380_438),
				project_id: None,
			},
			FileKey::sha1("ffffffffffffffffffffffffffffffffffffffff"),
		];
		let found = cf.identify_impl(&files).await.unwrap();
		assert_eq!(found.len(), 1);
		let v = &found[SHA1];
		assert_eq!(v.source, SourceKind::Curseforge);
		assert!(v.site_only);
		assert_eq!(
			v.page_url.as_deref(),
			Some("https://www.curseforge.com/minecraft/mc-mods/jei/download/10")
		);
		{
			let seen = seen.lock().unwrap();
			assert!(
				seen[0].starts_with("POST /v1/fingerprints/432 "),
				"{}",
				seen[0]
			);
			assert!(seen[0].ends_with("x-api-key: test-key"), "{}", seen[0]);
			assert!(seen[1].starts_with("POST /v1/mods "));
		}
		let err = cf.version_impl("5").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::SourceDisabled);
		assert_eq!(err.details.as_deref(), Some(DENIED));
		let err = cf.raw_mod(77).await.unwrap_err();
		assert_eq!(err.code, ErrorCode::ContentNotFound);
	}

	#[tokio::test]
	async fn latest_and_versions() {
		let mods = json!({ "data": [ mod_json(1, class::MODS) ] });
		let files = json!({ "data": [ file_json(12, 1, Some("https://edge.forgecdn.net/files/1/12/mod-12.jar")) ] });
		let page = json!({ "data": [
			file_json(20, 1, Some("https://edge.forgecdn.net/files/1/20/a.jar")),
			file_json(21, 1, Some("https://edge.forgecdn.net/files/1/21/b.jar"))
		], "pagination": { "index": 0, "pageSize": 50, "resultCount": 2, "totalCount": 2 } });
		let (base, seen) = serve(vec![
			response("200 OK", &mods.to_string()),
			response("200 OK", &files.to_string()),
			response(
				"200 OK",
				&json!({ "data": mod_json(1, class::MODS) }).to_string(),
			),
			response("200 OK", &page.to_string()),
		])
		.await;
		let cf = client(&base);
		let filter = VersionFilter {
			game_versions: vec!["1.20.1".into()],
			loaders: vec!["neoforge".into()],
		};
		let key = FileKey {
			sha1: "abc".into(),
			fingerprint: None,
			project_id: Some("1".into()),
		};
		let found = cf.latest_impl(&[key], &filter).await.unwrap();
		assert_eq!(found["abc"].id, "12");

		let versions = cf.project_versions_impl("1", &filter).await.unwrap();
		assert_eq!(versions.len(), 2);
		let seen = seen.lock().unwrap();
		assert!(
			seen[3].starts_with(
				"GET /v1/mods/1/files?gameVersion=1.20.1&modLoaderType=6&index=0&pageSize=50 "
			),
			"{}",
			seen[3]
		);
	}

	/// Живая проверка — только с ключом в окружении: `cargo test live_curseforge -- --ignored --nocapture`.
	#[tokio::test]
	#[ignore]
	async fn live_curseforge() {
		let Some(key) = api_key() else {
			eprintln!("нет {KEY_ENV} — пропуск");
			return;
		};
		let dir = tempfile::tempdir().unwrap();
		let cf = CurseForge::new(
			build_client(&key).unwrap(),
			API_BASE,
			dir.path().join("cats.json"),
		)
		.unwrap();
		let q: SearchQuery = serde_json::from_value(json!({
			"source": "curseforge", "type": "mod", "query": "jei", "gameVersions": ["1.20.1"],
			"loaders": ["forge"]
		}))
		.unwrap();
		let page = cf.search_impl(&q).await.unwrap();
		eprintln!("найдено {}", page.total);
		assert!(!page.hits.is_empty());
		let project = cf.project_impl("238222", None).await.unwrap();
		assert_eq!(project.hit.slug, "jei");
		let filter = VersionFilter {
			game_versions: vec!["1.20.1".into()],
			loaders: vec!["forge".into()],
		};
		let versions = cf.project_versions_impl("238222", &filter).await.unwrap();
		let best = super::super::pick_best(&versions).unwrap();
		eprintln!("лучшая: {} {:?}", best.version_number, best.files[0].url);
		let (cats, _) = cf.categories_impl().await.unwrap();
		assert!(cats.iter().any(|c| c.project_type == "mod"));
		let iris = cf
			.projects_impl(&[IRIS_ID.to_owned(), OCULUS_ID.to_owned()])
			.await
			.unwrap();
		assert_eq!(iris.len(), 2);
	}
}
