//! Каталог Modrinth (API v2, `https://api.modrinth.com/v2`). Свой User-Agent по правилам Modrinth,
//! ограничение частоты по заголовкам `X-Ratelimit-*`, кэш ответов в памяти, теги — ещё и на диске.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use url::Url;

use super::cache::ResponseCache;
use super::{
	BoxFuture, CategoryTag, ContentFile, ContentSource, ContentVersion, Dependency, DependencyKind,
	DonationLink, FileKey, GalleryImage, License, Links, Project, ProjectHit, ProjectType,
	SEARCH_LIMIT_DEFAULT, SEARCH_LIMIT_MAX, SearchPage, SearchQuery, SearchSort, Side, SourceKind,
	TeamMember, VersionChannel, VersionFilter, timestamp_ms,
};
use crate::error::{AppError, ErrorCode};

pub const API_BASE: &str = "https://api.modrinth.com";
/// Iris Shaders (Fabric/Quilt) и Oculus (Forge/NeoForge) — загрузчики шейдеров.
pub const IRIS_ID: &str = "YL57xq9U";
pub const OCULUS_ID: &str = "GchcoXML";
const SITE: &str = "https://modrinth.com";
/// Modrinth просит User-Agent вида `автор/проект/версия (контакт)`.
pub const USER_AGENT: &str = concat!(
	"leeonttev/LeeLauncher/",
	env!("CARGO_PKG_VERSION"),
	" (https://github.com/leeonttev/LeeLauncher)"
);

/// Одновременных запросов к API.
const CONCURRENCY: usize = 6;
/// Сколько готовы подождать сброса лимита, прежде чем вернуть `rate_limited`.
const MAX_RATE_WAIT: Duration = Duration::from_secs(10);
const NETWORK_RETRIES: u32 = 2;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Сколько id/хешей за один запрос.
const BATCH: usize = 100;

const TTL_SEARCH: Duration = Duration::from_secs(2 * 60);
const TTL_PROJECT: Duration = Duration::from_secs(5 * 60);
const TTL_HASHES: Duration = Duration::from_secs(10 * 60);
const TTL_TAGS: Duration = Duration::from_secs(24 * 60 * 60);

/// Имена загрузчиков в списке категорий Modrinth (в поиске они идут вместе с категориями).
const LOADER_NAMES: &[&str] = &[
	"fabric",
	"forge",
	"neoforge",
	"quilt",
	"liteloader",
	"modloader",
	"rift",
	"babric",
	"bta-babric",
	"java-agent",
	"legacy-fabric",
	"nilloader",
	"ornithe",
	"datapack",
	"minecraft",
	"iris",
	"optifine",
	"canvas",
	"vanilla",
	"bukkit",
	"folia",
	"paper",
	"purpur",
	"spigot",
	"sponge",
	"bungeecord",
	"velocity",
	"waterfall",
	"geyser",
];

fn is_loader(name: &str) -> bool {
	LOADER_NAMES.contains(&name)
}

pub fn build_client() -> reqwest::Result<reqwest::Client> {
	reqwest::Client::builder()
		.user_agent(USER_AGENT)
		.connect_timeout(Duration::from_secs(15))
		.timeout(REQUEST_TIMEOUT)
		.build()
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

pub struct Modrinth {
	client: reqwest::Client,
	base: Url,
	cache: ResponseCache,
	limit: Semaphore,
	/// Лимит исчерпан до этого момента.
	blocked_until: Mutex<Option<Instant>>,
	tags_file: PathBuf,
}

impl Modrinth {
	/// `tags_file` — копия тегов на диске для работы без сети.
	pub fn new(client: reqwest::Client, base: &str, tags_file: PathBuf) -> Result<Self, AppError> {
		let base = Url::parse(base).map_err(|e| AppError::unknown(format!("{base}: {e}")))?;
		Ok(Self {
			client,
			base,
			cache: ResponseCache::default(),
			limit: Semaphore::new(CONCURRENCY),
			blocked_until: Mutex::new(None),
			tags_file,
		})
	}

	fn url(&self, segments: &[&str]) -> Url {
		let mut url = self.base.clone();
		if let Ok(mut path) = url.path_segments_mut() {
			path.pop_if_empty().push("v2").extend(segments);
		}
		url
	}

	/// Ответ из кэша или сети. Сеть или сервер недоступны — сохранённый ответ (`true` — устаревший).
	async fn fetch(&self, req: Request, ttl: Duration) -> Result<(Arc<Vec<u8>>, bool), AppError> {
		let key = req.key();
		if let Some(body) = self.cache.fresh(&key) {
			return Ok((body, false));
		}
		match self.send(&req).await {
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
				format!("modrinth: ответ не разобран: {e}"),
			)
		})?;
		Ok((value, offline))
	}

	async fn wait_rate(&self) -> Result<(), AppError> {
		let until = self.blocked_until.lock().ok().and_then(|b| *b);
		let Some(until) = until else { return Ok(()) };
		let now = Instant::now();
		if until <= now {
			return Ok(());
		}
		let wait = until - now;
		if wait > MAX_RATE_WAIT {
			return Err(AppError::coded(
				ErrorCode::RateLimited,
				format!("modrinth: лимит запросов, ещё {} с", wait.as_secs() + 1),
			));
		}
		tokio::time::sleep(wait).await;
		Ok(())
	}

	fn note_rate(&self, headers: &reqwest::header::HeaderMap, limited: bool) {
		let number = |name: &str| {
			headers
				.get(name)
				.and_then(|v| v.to_str().ok())
				.and_then(|v| v.trim().parse::<u64>().ok())
		};
		let remaining = number("x-ratelimit-remaining");
		if !limited && remaining != Some(0) {
			return;
		}
		let reset = number("x-ratelimit-reset")
			.or_else(|| number("retry-after"))
			.unwrap_or(60)
			.clamp(1, 60);
		if let Ok(mut blocked) = self.blocked_until.lock() {
			*blocked = Some(Instant::now() + Duration::from_secs(reset));
		}
	}

	async fn send(&self, req: &Request) -> Result<Vec<u8>, AppError> {
		let _permit = self
			.limit
			.acquire()
			.await
			.map_err(|_| AppError::unknown("modrinth: семафор закрыт"))?;
		let mut network_attempt = 0;
		let mut waited_429 = false;
		loop {
			self.wait_rate().await?;
			let builder = match req {
				Request::Get(url) => self.client.get(url.clone()),
				Request::Post(url, body) => self.client.post(url.clone()).json(body),
			};
			let resp = match builder.send().await {
				Ok(resp) => resp,
				Err(err) => {
					if (err.is_connect() || err.is_timeout()) && network_attempt < NETWORK_RETRIES {
						network_attempt += 1;
						tokio::time::sleep(Duration::from_millis(500 * u64::from(network_attempt)))
							.await;
						continue;
					}
					return Err(AppError::coded(
						ErrorCode::Network,
						format!("modrinth: {}", err.without_url()),
					));
				}
			};
			let status = resp.status().as_u16();
			self.note_rate(resp.headers(), status == 429);
			match status {
				200..=299 => {
					return resp.bytes().await.map(|b| b.to_vec()).map_err(|e| {
						AppError::coded(
							ErrorCode::Network,
							format!("modrinth: {}", e.without_url()),
						)
					});
				}
				429 if !waited_429 => {
					waited_429 = true;
					continue;
				}
				429 => {
					return Err(AppError::coded(
						ErrorCode::RateLimited,
						"modrinth: HTTP 429",
					));
				}
				404 => {
					return Err(AppError::coded(
						ErrorCode::ContentNotFound,
						format!("modrinth: HTTP 404 {}", resp.url().path()),
					));
				}
				500..=599 | 410 => {
					return Err(AppError::coded(
						ErrorCode::SourceUnavailable,
						format!("modrinth: HTTP {status}"),
					));
				}
				_ => {
					let body = resp.text().await.unwrap_or_default();
					let body: String = body.chars().take(300).collect();
					return Err(AppError::unknown(format!(
						"modrinth: HTTP {status}: {body}"
					)));
				}
			}
		}
	}

	async fn search_impl(&self, q: &SearchQuery) -> Result<SearchPage, AppError> {
		let limit = q
			.limit
			.unwrap_or(SEARCH_LIMIT_DEFAULT)
			.clamp(1, SEARCH_LIMIT_MAX);
		let mut url = self.url(&["search"]);
		{
			let mut pairs = url.query_pairs_mut();
			let text = q.query.trim();
			if !text.is_empty() {
				pairs.append_pair("query", text);
			}
			pairs.append_pair("facets", &facets(q).to_string());
			pairs.append_pair("index", sort_name(q.sort));
			pairs.append_pair("offset", &q.offset.to_string());
			pairs.append_pair("limit", &limit.to_string());
		}
		let (raw, offline): (RawSearch, bool) = self.json(Request::Get(url), TTL_SEARCH).await?;
		Ok(SearchPage::new(
			raw.hits
				.into_iter()
				.map(|h| h.into_hit(q.project_type))
				.collect(),
			raw.offset,
			raw.limit,
			raw.total_hits,
			offline,
		))
	}

	async fn project_impl(&self, id: &str) -> Result<Project, AppError> {
		let project =
			self.json::<RawProject>(Request::Get(self.url(&["project", id])), TTL_PROJECT);
		let members = self.json::<Vec<RawMember>>(
			Request::Get(self.url(&["project", id, "members"])),
			TTL_PROJECT,
		);
		let (project, members) = tokio::join!(project, members);
		let (project, _) = project?;
		// Без команды проект всё равно показываем.
		let mut team = match members {
			Ok((members, _)) => members,
			Err(err) => {
				log::warn!("modrinth: команда проекта {id}: {:?}", err.code);
				Vec::new()
			}
		};
		team.sort_by_key(|m| m.ordering);
		Ok(project.into_project(team))
	}

	/// Проекты пачкой с авторами (вторым запросом — команды проектов).
	async fn projects_impl(&self, ids: &[String]) -> Result<Vec<ProjectHit>, AppError> {
		let raw: Vec<RawProject> = self.batch("projects", ids, TTL_PROJECT).await?;
		let mut teams: Vec<String> = raw
			.iter()
			.map(|p| p.team.clone())
			.filter(|t| !t.is_empty())
			.collect();
		teams.sort();
		teams.dedup();
		let authors: HashMap<String, String> = if teams.is_empty() {
			HashMap::new()
		} else {
			match self
				.batch::<Vec<RawMember>>("teams", &teams, TTL_PROJECT)
				.await
			{
				Ok(lists) => lists
					.into_iter()
					.filter_map(|members| {
						let team = members.first()?.team_id.clone();
						Some((team, owner(&members)?))
					})
					.collect(),
				Err(err) => {
					// Без авторов список всё равно полезен.
					log::warn!("modrinth: команды проектов: {:?}", err.code);
					HashMap::new()
				}
			}
		};
		Ok(raw
			.into_iter()
			.map(|p| {
				let author = authors.get(&p.team).cloned().unwrap_or_default();
				p.hit(author)
			})
			.collect())
	}

	async fn batch<T: DeserializeOwned>(
		&self,
		what: &str,
		ids: &[String],
		ttl: Duration,
	) -> Result<Vec<T>, AppError> {
		let mut out = Vec::new();
		for chunk in ids.chunks(BATCH) {
			let mut url = self.url(&[what]);
			url.query_pairs_mut()
				.append_pair("ids", &serde_json::to_string(chunk).unwrap_or_default());
			let (mut items, _): (Vec<T>, bool) = self.json(Request::Get(url), ttl).await?;
			out.append(&mut items);
		}
		Ok(out)
	}

	async fn project_versions_impl(
		&self,
		id: &str,
		filter: &VersionFilter,
	) -> Result<Vec<ContentVersion>, AppError> {
		let mut url = self.url(&["project", id, "version"]);
		{
			let mut pairs = url.query_pairs_mut();
			if !filter.loaders.is_empty() {
				pairs.append_pair("loaders", &json!(filter.loaders).to_string());
			}
			if !filter.game_versions.is_empty() {
				pairs.append_pair("game_versions", &json!(filter.game_versions).to_string());
			}
		}
		let (raw, _): (Vec<RawVersion>, bool) = self.json(Request::Get(url), TTL_PROJECT).await?;
		Ok(raw
			.into_iter()
			.filter_map(RawVersion::into_version)
			.collect())
	}

	async fn hashes(
		&self,
		sha1s: &[String],
		filter: Option<&VersionFilter>,
	) -> Result<HashMap<String, ContentVersion>, AppError> {
		let mut out = HashMap::new();
		for chunk in sha1s.chunks(BATCH * 5) {
			let (url, body) = match filter {
				None => (
					self.url(&["version_files"]),
					json!({ "hashes": chunk, "algorithm": "sha1" }),
				),
				Some(f) => (
					self.url(&["version_files", "update"]),
					json!({
						"hashes": chunk,
						"algorithm": "sha1",
						"loaders": f.loaders,
						"game_versions": f.game_versions,
					}),
				),
			};
			let (raw, _): (HashMap<String, RawVersion>, bool) =
				self.json(Request::Post(url, body), TTL_HASHES).await?;
			out.extend(
				raw.into_iter()
					.filter_map(|(hash, v)| Some((hash.to_ascii_lowercase(), v.into_version()?))),
			);
		}
		Ok(out)
	}

	async fn categories_impl(&self) -> Result<(Vec<CategoryTag>, bool), AppError> {
		let fresh = match self
			.json::<Vec<RawCategory>>(Request::Get(self.url(&["tag", "category"])), TTL_TAGS)
			.await
		{
			Ok((categories, offline)) => Some((StoredTags { categories }, offline)),
			Err(err) if !matches!(err.code, ErrorCode::Network | ErrorCode::SourceUnavailable) => {
				return Err(err);
			}
			Err(err) => {
				log::warn!("modrinth: категории без сети: {:?}", err.code);
				None
			}
		};
		let (stored, offline) = match fresh {
			Some((stored, offline)) => {
				if !offline {
					self.save_tags(&stored);
				}
				(stored, offline)
			}
			None => match self.load_tags() {
				Some(stored) => (stored, true),
				None => return Err(AppError::new(ErrorCode::Network)),
			},
		};
		Ok((stored.into_categories(), offline))
	}

	fn save_tags(&self, tags: &StoredTags) {
		let result = serde_json::to_vec(tags)
			.map_err(std::io::Error::other)
			.and_then(|json| crate::fs_util::write_atomic(&self.tags_file, &json));
		if let Err(err) = result {
			log::warn!("не удалось сохранить теги Modrinth: {err}");
		}
	}

	fn load_tags(&self) -> Option<StoredTags> {
		let bytes = std::fs::read(&self.tags_file).ok()?;
		serde_json::from_slice(&bytes).ok()
	}
}

impl ContentSource for Modrinth {
	fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxFuture<'a, SearchPage> {
		Box::pin(self.search_impl(query))
	}

	fn project<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Project> {
		Box::pin(self.project_impl(id))
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
		Box::pin(async move {
			let (raw, _): (RawVersion, bool) = self
				.json(Request::Get(self.url(&["version", id])), TTL_PROJECT)
				.await?;
			raw.into_version().ok_or_else(|| {
				AppError::coded(
					ErrorCode::ContentIncompatible,
					format!("{id}: нет файлов с sha1"),
				)
			})
		})
	}

	fn versions<'a>(&'a self, ids: &'a [String]) -> BoxFuture<'a, Vec<ContentVersion>> {
		Box::pin(async move {
			let raw: Vec<RawVersion> = self.batch("versions", ids, TTL_PROJECT).await?;
			Ok(raw
				.into_iter()
				.filter_map(RawVersion::into_version)
				.collect())
		})
	}

	fn categories(&self) -> BoxFuture<'_, (Vec<CategoryTag>, bool)> {
		Box::pin(self.categories_impl())
	}

	fn identify<'a>(
		&'a self,
		files: &'a [FileKey],
	) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
		Box::pin(async move { self.hashes(&sha1s_of(files), None).await })
	}

	fn latest<'a>(
		&'a self,
		files: &'a [FileKey],
		filter: &'a VersionFilter,
	) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
		Box::pin(async move { self.hashes(&sha1s_of(files), Some(filter)).await })
	}
}

fn sha1s_of(files: &[FileKey]) -> Vec<String> {
	let mut sha1s: Vec<String> = files
		.iter()
		.map(|f| f.sha1.clone())
		.filter(|s| !s.is_empty())
		.collect();
	sha1s.sort();
	sha1s.dedup();
	sha1s
}

fn sort_name(sort: SearchSort) -> &'static str {
	match sort {
		SearchSort::Relevance => "relevance",
		SearchSort::Downloads => "downloads",
		SearchSort::Follows => "follows",
		SearchSort::Newest => "newest",
		SearchSort::Updated => "updated",
		// По названию Modrinth не сортирует (в `catalog_sources` этой сортировки у него нет).
		SearchSort::Name => "relevance",
	}
}

/// Фильтры поиска: внутри массива — «или», между массивами — «и».
fn facets(q: &SearchQuery) -> Value {
	let mut facets = vec![json!([format!("project_type:{}", q.project_type.as_str())])];
	let any = |prefix: &str, values: &[String]| {
		let values: Vec<String> = values
			.iter()
			.map(|v| v.trim())
			.filter(|v| !v.is_empty())
			.map(|v| format!("{prefix}:{v}"))
			.collect();
		(!values.is_empty()).then(|| json!(values))
	};
	facets.extend(any("versions", &q.game_versions));
	facets.extend(any("categories", &q.loaders));
	for category in &q.categories {
		facets.extend(any("categories", std::slice::from_ref(category)));
	}
	Value::Array(facets)
}

fn non_empty(value: Option<String>) -> Option<String> {
	value.filter(|v| !v.trim().is_empty())
}

fn project_type(raw: &str, loaders: &[String]) -> ProjectType {
	match raw {
		"resourcepack" => ProjectType::Resourcepack,
		"shader" => ProjectType::Shader,
		"datapack" => ProjectType::Datapack,
		"modpack" => ProjectType::Modpack,
		_ if !loaders.is_empty() && loaders.iter().all(|l| l == "datapack") => {
			ProjectType::Datapack
		}
		_ => ProjectType::Mod,
	}
}

fn page_url(kind: ProjectType, slug: &str) -> String {
	format!("{SITE}/{}/{slug}", kind.as_str())
}

/// Автор проекта: владелец команды, иначе первый участник.
fn owner(members: &[RawMember]) -> Option<String> {
	members
		.iter()
		.find(|m| m.role.eq_ignore_ascii_case("owner"))
		.or_else(|| members.iter().min_by_key(|m| m.ordering))
		.map(|m| m.user.username.clone())
}

#[derive(Deserialize)]
struct RawSearch {
	hits: Vec<RawHit>,
	#[serde(default)]
	offset: u32,
	#[serde(default)]
	limit: u32,
	#[serde(default)]
	total_hits: u64,
}

#[derive(Deserialize)]
struct RawHit {
	project_id: String,
	#[serde(default)]
	slug: Option<String>,
	title: String,
	#[serde(default)]
	description: String,
	#[serde(default)]
	author: String,
	#[serde(default)]
	categories: Vec<String>,
	#[serde(default)]
	display_categories: Vec<String>,
	#[serde(default)]
	versions: Vec<String>,
	#[serde(default)]
	downloads: u64,
	#[serde(default)]
	follows: u64,
	#[serde(default)]
	icon_url: Option<String>,
	#[serde(default)]
	date_modified: String,
	#[serde(default)]
	date_created: String,
	#[serde(default)]
	client_side: String,
	#[serde(default)]
	server_side: String,
}

impl RawHit {
	fn into_hit(self, kind: ProjectType) -> ProjectHit {
		let shown = if self.display_categories.is_empty() {
			&self.categories
		} else {
			&self.display_categories
		};
		ProjectHit {
			source: SourceKind::Modrinth,
			categories: shown.iter().filter(|c| !is_loader(c)).cloned().collect(),
			loaders: self
				.categories
				.iter()
				.filter(|c| is_loader(c))
				.cloned()
				.collect(),
			url: page_url(
				kind,
				&non_empty(self.slug.clone()).unwrap_or_else(|| self.project_id.clone()),
			),
			slug: non_empty(self.slug).unwrap_or_else(|| self.project_id.clone()),
			id: self.project_id,
			project_type: kind,
			title: self.title,
			description: self.description,
			author: self.author,
			icon_url: non_empty(self.icon_url),
			downloads: self.downloads,
			follows: Some(self.follows),
			site_only: false,
			game_versions: self.versions,
			updated_at: timestamp_ms(&self.date_modified),
			client_side: Side::parse(&self.client_side),
			server_side: Side::parse(&self.server_side),
			install: None,
			also_on: Vec::new(),
			created_at: timestamp_ms(&self.date_created),
		}
	}
}

#[derive(Deserialize)]
struct RawLicense {
	id: String,
	#[serde(default)]
	name: String,
	#[serde(default)]
	url: Option<String>,
}

#[derive(Deserialize)]
struct RawDonation {
	#[serde(default)]
	platform: String,
	url: String,
}

#[derive(Deserialize)]
struct RawGallery {
	url: String,
	#[serde(default)]
	raw_url: Option<String>,
	#[serde(default)]
	featured: bool,
	#[serde(default)]
	title: Option<String>,
	#[serde(default)]
	description: Option<String>,
	#[serde(default)]
	ordering: i64,
}

#[derive(Deserialize)]
struct RawProject {
	id: String,
	#[serde(default)]
	slug: Option<String>,
	#[serde(default)]
	project_type: String,
	title: String,
	#[serde(default)]
	description: String,
	#[serde(default)]
	body: String,
	#[serde(default)]
	categories: Vec<String>,
	#[serde(default)]
	loaders: Vec<String>,
	#[serde(default)]
	game_versions: Vec<String>,
	#[serde(default)]
	downloads: u64,
	#[serde(default)]
	followers: u64,
	#[serde(default)]
	license: Option<RawLicense>,
	#[serde(default)]
	issues_url: Option<String>,
	#[serde(default)]
	source_url: Option<String>,
	#[serde(default)]
	wiki_url: Option<String>,
	#[serde(default)]
	discord_url: Option<String>,
	#[serde(default)]
	donation_urls: Option<Vec<RawDonation>>,
	#[serde(default)]
	icon_url: Option<String>,
	#[serde(default)]
	gallery: Vec<RawGallery>,
	#[serde(default)]
	published: String,
	#[serde(default)]
	updated: String,
	#[serde(default)]
	team: String,
	#[serde(default)]
	client_side: String,
	#[serde(default)]
	server_side: String,
	#[serde(default)]
	versions: Vec<String>,
}

impl RawProject {
	fn hit(&self, author: String) -> ProjectHit {
		let kind = project_type(&self.project_type, &self.loaders);
		ProjectHit {
			source: SourceKind::Modrinth,
			id: self.id.clone(),
			slug: non_empty(self.slug.clone()).unwrap_or_else(|| self.id.clone()),
			url: page_url(
				kind,
				&non_empty(self.slug.clone()).unwrap_or_else(|| self.id.clone()),
			),
			project_type: kind,
			title: self.title.clone(),
			author,
			description: self.description.clone(),
			icon_url: non_empty(self.icon_url.clone()),
			downloads: self.downloads,
			follows: Some(self.followers),
			site_only: false,
			categories: self
				.categories
				.iter()
				.filter(|c| !is_loader(c))
				.cloned()
				.collect(),
			updated_at: timestamp_ms(&self.updated),
			game_versions: self.game_versions.clone(),
			loaders: self.loaders.clone(),
			client_side: Side::parse(&self.client_side),
			server_side: Side::parse(&self.server_side),
			install: None,
			also_on: Vec::new(),
			created_at: timestamp_ms(&self.published),
		}
	}

	fn into_project(self, team: Vec<RawMember>) -> Project {
		let hit = self.hit(owner(&team).unwrap_or_default());
		let mut gallery = self.gallery;
		gallery.sort_by_key(|g| g.ordering);
		Project {
			links: Links {
				site: page_url(hit.project_type, &hit.slug),
				issues: non_empty(self.issues_url),
				source: non_empty(self.source_url),
				wiki: non_empty(self.wiki_url),
				discord: non_empty(self.discord_url),
				donate: self
					.donation_urls
					.unwrap_or_default()
					.into_iter()
					.map(|d| DonationLink {
						platform: d.platform,
						url: d.url,
					})
					.collect(),
			},
			hit,
			body: self.body,
			versions_count: self.versions.len(),
			published_at: timestamp_ms(&self.published),
			best_version: None,
			license: self.license.map(|l| License {
				name: if l.name.is_empty() {
					l.id.clone()
				} else {
					l.name
				},
				id: l.id,
				url: non_empty(l.url),
			}),
			gallery: gallery
				.into_iter()
				.map(|g| GalleryImage {
					url: non_empty(g.raw_url).unwrap_or_else(|| g.url.clone()),
					thumb_url: g.url,
					title: non_empty(g.title),
					description: non_empty(g.description),
					featured: g.featured,
				})
				.collect(),
			team: team
				.into_iter()
				.map(|m| TeamMember {
					id: m.user.id,
					username: m.user.username,
					avatar_url: non_empty(m.user.avatar_url),
					role: m.role,
				})
				.collect(),
		}
	}
}

#[derive(Deserialize)]
struct RawUser {
	id: String,
	username: String,
	#[serde(default)]
	avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct RawMember {
	#[serde(default)]
	team_id: String,
	user: RawUser,
	#[serde(default)]
	role: String,
	#[serde(default)]
	ordering: i64,
}

#[derive(Deserialize)]
struct RawHashes {
	#[serde(default)]
	sha1: Option<String>,
}

#[derive(Deserialize)]
struct RawFile {
	hashes: RawHashes,
	url: String,
	filename: String,
	#[serde(default)]
	primary: bool,
	#[serde(default)]
	size: u64,
}

#[derive(Deserialize)]
struct RawDependency {
	#[serde(default)]
	version_id: Option<String>,
	#[serde(default)]
	project_id: Option<String>,
	#[serde(default)]
	dependency_type: String,
}

#[derive(Deserialize)]
struct RawVersion {
	id: String,
	project_id: String,
	#[serde(default)]
	name: String,
	#[serde(default)]
	version_number: String,
	#[serde(default)]
	changelog: Option<String>,
	#[serde(default)]
	dependencies: Vec<RawDependency>,
	#[serde(default)]
	game_versions: Vec<String>,
	#[serde(default)]
	version_type: String,
	#[serde(default)]
	loaders: Vec<String>,
	#[serde(default)]
	date_published: String,
	#[serde(default)]
	downloads: u64,
	#[serde(default)]
	files: Vec<RawFile>,
}

impl RawVersion {
	/// Файлы без sha1 отбрасываются (проверить их нечем); версия без файлов — `None`.
	fn into_version(self) -> Option<ContentVersion> {
		let files: Vec<ContentFile> = self
			.files
			.into_iter()
			.filter_map(|f| {
				let sha1 = f.hashes.sha1.filter(|h| crate::fs_util::is_sha1_hex(h))?;
				Some(ContentFile {
					url: f.url,
					file_name: f.filename,
					sha1: sha1.to_ascii_lowercase(),
					size: f.size,
					primary: f.primary,
				})
			})
			.collect();
		if files.is_empty() {
			return None;
		}
		Some(ContentVersion {
			source: SourceKind::Modrinth,
			id: self.id,
			project_id: self.project_id,
			name: self.name,
			version_number: self.version_number,
			channel: match self.version_type.as_str() {
				"beta" => VersionChannel::Beta,
				"alpha" => VersionChannel::Alpha,
				_ => VersionChannel::Release,
			},
			game_versions: self.game_versions,
			loaders: self.loaders,
			published_at: timestamp_ms(&self.date_published),
			downloads: self.downloads,
			changelog: non_empty(self.changelog),
			installed: None,
			site_only: false,
			page_url: None,
			side: None,
			files,
			dependencies: self
				.dependencies
				.into_iter()
				.map(|d| Dependency {
					project_id: non_empty(d.project_id),
					version_id: non_empty(d.version_id),
					kind: match d.dependency_type.as_str() {
						"required" => DependencyKind::Required,
						"incompatible" => DependencyKind::Incompatible,
						"embedded" => DependencyKind::Embedded,
						_ => DependencyKind::Optional,
					},
				})
				.collect(),
		})
	}
}

#[derive(Serialize, Deserialize)]
struct RawCategory {
	#[serde(default)]
	icon: String,
	name: String,
	#[serde(default)]
	project_type: String,
	#[serde(default)]
	header: String,
}

#[derive(Serialize, Deserialize)]
struct StoredTags {
	categories: Vec<RawCategory>,
}

impl StoredTags {
	fn into_categories(self) -> Vec<CategoryTag> {
		self.categories
			.into_iter()
			.map(|c| CategoryTag {
				name: c.name.clone(),
				slug: c.name,
				project_type: c.project_type,
				header: c.header,
				icon: c.icon,
				icon_url: None,
				parent: None,
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::io::{AsyncReadExt, AsyncWriteExt};

	fn query(json: Value) -> SearchQuery {
		serde_json::from_value(json).unwrap()
	}

	#[test]
	fn builds_facets() {
		let q = query(json!({
			"type": "mod",
			"gameVersions": ["1.20.1"],
			"loaders": ["fabric", "quilt"],
			"categories": ["optimization", "utility"],
		}));
		assert_eq!(
			facets(&q),
			json!([
				["project_type:mod"],
				["versions:1.20.1"],
				["categories:fabric", "categories:quilt"],
				["categories:optimization"],
				["categories:utility"],
			])
		);
		let q = query(json!({ "type": "shader", "gameVersions": [" "] }));
		assert_eq!(facets(&q), json!([["project_type:shader"]]));
	}

	#[test]
	fn parses_version_and_project() {
		let raw: RawVersion = serde_json::from_value(json!({
			"id": "v1", "project_id": "p1", "name": "Sodium 0.5", "version_number": "0.5.8",
			"version_type": "beta", "game_versions": ["1.20.1"], "loaders": ["fabric"],
			"date_published": "2024-01-01T00:00:00Z", "downloads": 5,
			"dependencies": [
				{ "project_id": "P2", "dependency_type": "required" },
				{ "version_id": "V3", "project_id": null, "dependency_type": "incompatible" },
				{ "file_name": "x.jar", "dependency_type": "weird" }
			],
			"files": [
				{ "hashes": { "sha1": "DA39A3EE5E6B4B0D3255BFEF95601890AFD80709", "sha512": "x" },
				  "url": "https://cdn.modrinth.com/a.jar", "filename": "a.jar", "primary": false, "size": 3 },
				{ "hashes": {}, "url": "https://cdn.modrinth.com/b.jar", "filename": "b.jar", "primary": true, "size": 3 }
			]
		}))
		.unwrap();
		let v = raw.into_version().unwrap();
		assert_eq!(v.channel, VersionChannel::Beta);
		assert_eq!(v.files.len(), 1);
		assert_eq!(v.files[0].sha1, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
		assert_eq!(v.primary_file().unwrap().file_name, "a.jar");
		assert_eq!(v.dependencies[0].kind, DependencyKind::Required);
		assert_eq!(v.dependencies[1].kind, DependencyKind::Incompatible);
		assert_eq!(v.dependencies[2].kind, DependencyKind::Optional);
		let json = serde_json::to_value(&v).unwrap();
		assert_eq!(json["files"][0]["name"], "a.jar");
		assert_eq!(json["publishedAt"], 1_704_067_200_000u64);
		assert_eq!(json["channel"], "beta");

		let raw: RawProject = serde_json::from_value(json!({
			"id": "p1", "slug": "dp", "project_type": "mod", "title": "Pack", "loaders": ["datapack"],
			"icon_url": "", "license": { "id": "MIT", "name": "" },
			"gallery": [
				{ "url": "b", "featured": false, "ordering": 2 },
				{ "url": "a", "raw_url": "a-raw", "featured": true, "ordering": 1 }
			]
		}))
		.unwrap();
		let team: Vec<RawMember> = serde_json::from_value(json!([
			{ "team_id": "t", "user": { "id": "u2", "username": "second" }, "role": "Dev", "ordering": 0 },
			{ "team_id": "t", "user": { "id": "u1", "username": "boss" }, "role": "Owner", "ordering": 1 }
		]))
		.unwrap();
		let p = raw.into_project(team);
		assert_eq!(p.hit.project_type, ProjectType::Datapack);
		assert_eq!(p.hit.author, "boss");
		assert_eq!(p.links.site, "https://modrinth.com/datapack/dp");
		assert_eq!(p.hit.icon_url, None);
		assert_eq!(p.license.as_ref().unwrap().name, "MIT");
		assert_eq!(p.gallery[0].url, "a-raw");
		assert_eq!(p.gallery[0].thumb_url, "a");
		assert_eq!(p.gallery[1].url, "b");
		let json = serde_json::to_value(&p).unwrap();
		assert_eq!(json["type"], "datapack");
		assert_eq!(json["versionsCount"], 0);
		assert!(json.get("hit").is_none() && json.get("bestVersion").is_none());

		let hit: RawHit = serde_json::from_value(json!({
			"project_id": "x", "title": "T", "categories": ["fabric", "optimization"],
			"display_categories": ["fabric", "optimization"]
		}))
		.unwrap();
		let hit = hit.into_hit(ProjectType::Mod);
		assert_eq!(hit.categories, vec!["optimization"]);
		assert_eq!(hit.loaders, vec!["fabric"]);
	}

	/// Локальный сервер: отвечает по очереди заданными ответами и запоминает запросы.
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
				let mut buf = vec![0u8; 16 * 1024];
				let n = socket.read(&mut buf).await.unwrap_or(0);
				let head = String::from_utf8_lossy(&buf[..n]);
				log.lock()
					.unwrap()
					.push(head.lines().next().unwrap_or_default().to_owned());
				let _ = socket.write_all(response.as_bytes()).await;
				let _ = socket.shutdown().await;
			}
		});
		(format!("http://{addr}"), seen)
	}

	fn response(status: &str, headers: &str, body: &str) -> String {
		format!(
			"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
			body.len()
		)
	}

	fn client(base: &str) -> Modrinth {
		let dir = std::env::temp_dir().join(format!("ll-tags-{}", rand::random::<u32>()));
		Modrinth::new(build_client().unwrap(), base, dir.join("tags.json")).unwrap()
	}

	#[tokio::test]
	async fn caches_and_maps_errors() {
		let body =
			r#"{"hits":[{"project_id":"a","title":"A"}],"offset":0,"limit":20,"total_hits":1}"#;
		let (base, seen) = serve(vec![
			response("200 OK", "X-Ratelimit-Remaining: 299\r\n", body),
			response("404 Not Found", "", "{}"),
			response("503 Service Unavailable", "", "{}"),
		])
		.await;
		let m = client(&base);
		let q = query(json!({ "type": "mod", "query": "sodium" }));
		let page = m.search_impl(&q).await.unwrap();
		assert_eq!((page.total, page.offline), (1, false));
		assert_eq!(page.hits[0].id, "a");
		// Второй раз — из кэша, без запроса.
		m.search_impl(&q).await.unwrap();
		assert_eq!(seen.lock().unwrap().len(), 1);
		assert!(seen.lock().unwrap()[0].starts_with("GET /v2/search?query=sodium&facets="));

		let err = m
			.project_versions_impl("nope", &VersionFilter::default())
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::ContentNotFound);
		let err = m.version("v").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::SourceUnavailable);
	}

	#[tokio::test]
	async fn rate_limit_waits_or_fails() {
		let (base, seen) = serve(vec![
			response(
				"429 Too Many Requests",
				"X-Ratelimit-Remaining: 0\r\nX-Ratelimit-Reset: 1\r\n",
				"{}",
			),
			response("200 OK", "", "[]"),
			response("429 Too Many Requests", "X-Ratelimit-Reset: 50\r\n", "{}"),
		])
		.await;
		let m = client(&base);
		let started = Instant::now();
		let versions = m.versions(&["a".to_owned()]).await.unwrap();
		assert!(versions.is_empty());
		assert!(started.elapsed() >= Duration::from_millis(900));
		assert_eq!(seen.lock().unwrap().len(), 2);
		let err = m.versions(&["b".to_owned()]).await.unwrap_err();
		assert_eq!(err.code, ErrorCode::RateLimited);
		// Лимит исчерпан на 50 с — следующий запрос сразу `rate_limited`, без сети.
		let err = m.version("c").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::RateLimited);
		assert_eq!(seen.lock().unwrap().len(), 3);
	}

	#[tokio::test]
	async fn offline_uses_stale_and_disk_tags() {
		let dir = tempfile::tempdir().unwrap();
		let tags = dir.path().join("tags.json");
		let stored = StoredTags {
			categories: vec![RawCategory {
				icon: String::new(),
				name: "magic".into(),
				project_type: "mod".into(),
				header: "categories".into(),
			}],
		};
		std::fs::write(&tags, serde_json::to_vec(&stored).unwrap()).unwrap();
		// Порт, где никто не слушает.
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		drop(listener);
		let m = Modrinth::new(build_client().unwrap(), &base, tags).unwrap();
		let (categories, offline) = m.categories_impl().await.unwrap();
		assert!(offline);
		assert_eq!(categories[0].slug, "magic");

		let q = query(json!({ "type": "mod" }));
		let key = Request::Get({
			let mut url = m.url(&["search"]);
			url.query_pairs_mut()
				.append_pair("facets", &facets(&q).to_string())
				.append_pair("index", "relevance")
				.append_pair("offset", "0")
				.append_pair("limit", "20");
			url
		})
		.key();
		m.cache.put(
			key,
			Duration::ZERO,
			Arc::new(br#"{"hits":[],"offset":0,"limit":20,"total_hits":0}"#.to_vec()),
		);
		let page = m.search_impl(&q).await.unwrap();
		assert!(page.offline);
		let err = m.version("x").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::Network);
	}
}
