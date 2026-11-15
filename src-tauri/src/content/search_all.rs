//! Поиск по всем каталогам одним списком. Позиции каждого каталога и уже склеенные дубли
//! хранятся в курсоре, своего состояния у ядра нет.

use std::collections::VecDeque;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use super::{
	ContentSource, PartialFailure, ProjectHit, SEARCH_LIMIT_DEFAULT, SEARCH_LIMIT_MAX, SearchPage,
	SearchQuery, SearchSort, SourceKind,
};
use crate::error::{AppError, ErrorCode};

/// Сортировки, общие для всех каталогов.
pub const SORTS: [SearchSort; 4] = [
	SearchSort::Relevance,
	SearchSort::Downloads,
	SearchSort::Updated,
	SearchSort::Newest,
];

/// Окно выдачи каждого каталога — вдвое больше порции (не больше 50): двойники ищутся и чуть
/// дальше показанного.
const WINDOW_MAX: u32 = 50;
/// Сколько раз за одну порцию можно дозапросить один каталог (буфер кончился раньше порции).
const MAX_FETCHES: u32 = 4;
/// Сколько позиций «уже показано впереди» хранить на каталог.
const MAX_SKIP: usize = 256;
const MAX_CURSOR_LEN: usize = 8 * 1024;
const CURSOR_VERSION: u32 = 1;

/// Строка результата: проект и его двойники в других каталогах.
pub struct Row {
	pub hit: ProjectHit,
	pub twins: Vec<ProjectHit>,
}

pub struct AllPage {
	pub rows: Vec<Row>,
	pub total: u64,
	pub offline: bool,
	pub cursor: Option<String>,
	pub partial: Vec<PartialFailure>,
	pub limit: u32,
	/// Сколько строк показано до этой порции (для `offset` ответа).
	pub shown: u32,
}

impl AllPage {
	pub fn into_search_page(self, rows: Vec<ProjectHit>) -> SearchPage {
		let mut page = SearchPage::new(rows, self.shown, self.limit, self.total, self.offline);
		page.cursor = self.cursor;
		page.partial = self.partial;
		page
	}
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LaneState {
	source: SourceKind,
	/// Первая позиция в выдаче каталога, которую ещё не показали.
	next: u32,
	/// Позиции дальше `next`, уже показанные (склеены с другой строкой).
	#[serde(default)]
	skip: Vec<u32>,
	#[serde(default)]
	done: bool,
	#[serde(default)]
	total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cursor {
	v: u32,
	shown: u32,
	lanes: Vec<LaneState>,
}

fn bad_cursor(what: &str) -> AppError {
	AppError::coded(ErrorCode::InvalidValue, format!("cursor: {what}"))
}

fn encode(cursor: &Cursor) -> Result<String, AppError> {
	let json = serde_json::to_vec(cursor).map_err(|e| AppError::unknown(e.to_string()))?;
	Ok(URL_SAFE_NO_PAD.encode(json))
}

fn decode(text: &str) -> Result<Cursor, AppError> {
	if text.len() > MAX_CURSOR_LEN {
		return Err(bad_cursor("too long"));
	}
	let bytes = URL_SAFE_NO_PAD
		.decode(text.trim())
		.map_err(|_| bad_cursor("base64"))?;
	let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| bad_cursor("json"))?;
	if cursor.v != CURSOR_VERSION {
		return Err(bad_cursor("version"));
	}
	Ok(cursor)
}

/// Строка для сравнения: нижний регистр, только буквы и цифры.
fn norm(text: &str) -> String {
	text.chars()
		.filter(|c| c.is_alphanumeric())
		.flat_map(char::to_lowercase)
		.collect()
}

/// Один проект в двух каталогах: slug и ещё название или автор, либо название и автор.
pub fn same_project(a: &ProjectHit, b: &ProjectHit) -> bool {
	if a.source == b.source || a.project_type != b.project_type {
		return false;
	}
	let filled = |x: &str, y: &str| {
		let (x, y) = (norm(x), norm(y));
		!x.is_empty() && x == y
	};
	let slug = !a.slug.is_empty() && a.slug.eq_ignore_ascii_case(&b.slug);
	let title = filled(&a.title, &b.title);
	let author = filled(&a.author, &b.author);
	(slug && (title || author)) || (title && author)
}

fn key(sort: SearchSort, hit: &ProjectHit) -> u64 {
	match sort {
		SearchSort::Downloads => hit.downloads,
		SearchSort::Updated => hit.updated_at,
		SearchSort::Newest => hit.created_at,
		_ => 0,
	}
}

/// Порядок главного каталога в склеенной строке: Modrinth первым.
fn rank(source: SourceKind) -> usize {
	SourceKind::ALL
		.iter()
		.position(|s| *s == source)
		.unwrap_or(usize::MAX)
}

struct Lane {
	source: Arc<dyn ContentSource>,
	state: LaneState,
	buf: VecDeque<(u32, ProjectHit)>,
	/// Позиция после последней полученной.
	fetched: u32,
	exhausted: bool,
	failed: bool,
	fetches: u32,
	/// Позиции, убранные из буфера склейкой (не с головы).
	removed: Vec<u32>,
	offline: bool,
}

impl Lane {
	fn blocked(&self) -> bool {
		self.buf.is_empty() && !self.exhausted && !self.failed
	}
}

async fn fetch_page(
	source: Arc<dyn ContentSource>,
	mut query: SearchQuery,
	kind: SourceKind,
	offset: u32,
	limit: u32,
) -> Result<SearchPage, AppError> {
	query.source = kind;
	query.offset = offset;
	query.limit = Some(limit);
	query.categories.clear();
	source.search(&query).await
}

fn absorb(
	lane: &mut Lane,
	result: Result<SearchPage, AppError>,
	partial: &mut Vec<PartialFailure>,
) {
	lane.fetches += 1;
	match result {
		Ok(page) => {
			let offset = lane.fetched;
			let got = page.hits.len() as u32;
			lane.offline |= page.offline;
			lane.state.total = page.total;
			for (i, hit) in page.hits.into_iter().enumerate() {
				let pos = offset + i as u32;
				if !lane.state.skip.contains(&pos) {
					lane.buf.push_back((pos, hit));
				}
			}
			lane.fetched = offset + got;
			lane.exhausted = got == 0 || u64::from(lane.fetched) >= page.total;
		}
		Err(err) => {
			log::warn!(
				"поиск «Все»: {} не ответил: {:?}",
				lane.state.source.as_str(),
				err.code
			);
			lane.failed = true;
			partial.push(PartialFailure {
				source: lane.state.source,
				error: err,
			});
		}
	}
}

/// Общий код, когда не ответил ни один каталог.
fn combined_error(mut errors: Vec<AppError>) -> AppError {
	if errors.len() == 1 {
		return errors.remove(0);
	}
	if errors.iter().all(|e| e.code == ErrorCode::Network) {
		return AppError::coded(ErrorCode::Network, "все каталоги: нет соединения");
	}
	let details: Vec<String> = errors
		.iter()
		.map(|e| format!("{:?}: {}", e.code, e.message))
		.collect();
	AppError::coded(ErrorCode::SourceUnavailable, details.join("; "))
}

/// Проверка запроса режима «Все»: только общие сортировки, без категорий.
pub fn check_query(query: &SearchQuery) -> Result<(), AppError> {
	if !SORTS.contains(&query.sort) {
		return Err(AppError::coded(
			ErrorCode::InvalidValue,
			"sort: в режиме «Все» только relevance, downloads, updated, newest",
		));
	}
	if !query.categories.is_empty() {
		return Err(AppError::coded(
			ErrorCode::InvalidValue,
			"categories: в режиме «Все» категорий нет",
		));
	}
	Ok(())
}

/// Порция поиска по каталогам `sources` (в порядке [`SourceKind::ALL`]; только те, где есть этот тип).
pub async fn search(
	sources: Vec<(SourceKind, Arc<dyn ContentSource>)>,
	query: &SearchQuery,
	cursor: Option<&str>,
) -> Result<AllPage, AppError> {
	check_query(query)?;
	let limit = query
		.limit
		.unwrap_or(SEARCH_LIMIT_DEFAULT)
		.clamp(1, SEARCH_LIMIT_MAX);
	let window = limit.saturating_mul(2).min(WINDOW_MAX);
	let saved = match cursor {
		Some(text) => Some(decode(text)?),
		None => None,
	};
	let shown = saved.as_ref().map(|c| c.shown).unwrap_or(0);
	let mut lanes: Vec<Lane> = sources
		.into_iter()
		.map(|(kind, source)| {
			let mut state = saved
				.as_ref()
				.and_then(|c| c.lanes.iter().find(|l| l.source == kind).cloned())
				.unwrap_or_default();
			state.source = kind;
			state.skip.retain(|p| *p >= state.next);
			Lane {
				source,
				fetched: state.next,
				exhausted: state.done,
				state,
				buf: VecDeque::new(),
				failed: false,
				fetches: 0,
				removed: Vec::new(),
				offline: false,
			}
		})
		.collect();
	lanes.sort_by_key(|l| rank(l.state.source));

	let mut partial = Vec::new();
	let mut errors = Vec::new();

	// Первые запросы — параллельно.
	let mut set = JoinSet::new();
	for (i, lane) in lanes.iter().enumerate() {
		if !lane.exhausted {
			let fut = fetch_page(
				lane.source.clone(),
				query.clone(),
				lane.state.source,
				lane.fetched,
				window,
			);
			set.spawn(async move { (i, fut.await) });
		}
	}
	let asked = set.len();
	while let Some(joined) = set.join_next().await {
		let (i, result) = joined.map_err(|e| AppError::unknown(e.to_string()))?;
		if let Err(err) = &result {
			errors.push(err.clone());
		}
		absorb(&mut lanes[i], result, &mut partial);
	}
	if asked > 0 && errors.len() == asked {
		return Err(combined_error(errors));
	}

	let mut rows: Vec<Row> = Vec::new();
	while rows.len() < limit as usize {
		// Буфер кончился, а каталог ещё не исчерпан — дозапрос (иначе не понять порядок).
		for lane in lanes.iter_mut() {
			while lane.blocked() && lane.fetches < MAX_FETCHES {
				let result = fetch_page(
					lane.source.clone(),
					query.clone(),
					lane.state.source,
					lane.fetched,
					window,
				)
				.await;
				absorb(lane, result, &mut partial);
			}
		}
		if lanes.iter().any(Lane::blocked) {
			break;
		}
		let pick = lanes
			.iter()
			.enumerate()
			.filter_map(|(i, l)| l.buf.front().map(|(pos, hit)| (i, *pos, hit)))
			.min_by(|(ia, pa, ha), (ib, pb, hb)| {
				let by_field = if query.sort == SearchSort::Relevance {
					std::cmp::Ordering::Equal
				} else {
					key(query.sort, hb).cmp(&key(query.sort, ha))
				};
				by_field.then(pa.cmp(pb)).then(ia.cmp(ib))
			})
			.map(|(i, _, _)| i);
		let Some(i) = pick else {
			break;
		};
		let Some((_, hit)) = lanes[i].buf.pop_front() else {
			break;
		};
		if rows.iter().any(|r| {
			std::iter::once(&r.hit)
				.chain(&r.twins)
				.any(|h| h.source == hit.source && h.id == hit.id)
		}) {
			continue;
		}
		let mut group = vec![hit];
		for (j, other) in lanes.iter_mut().enumerate() {
			if j == i {
				continue;
			}
			if let Some(at) = other
				.buf
				.iter()
				.position(|(_, h)| same_project(&group[0], h))
				&& let Some((pos, twin)) = other.buf.remove(at)
			{
				if at > 0 {
					other.removed.push(pos);
				}
				group.push(twin);
			}
		}
		group.sort_by_key(|h| rank(h.source));
		let mut group = group.into_iter();
		let Some(main) = group.next() else {
			continue;
		};
		rows.push(Row {
			hit: main,
			twins: group.collect(),
		});
	}

	let mut states = Vec::with_capacity(lanes.len());
	let mut total = 0u64;
	let mut offline = false;
	for lane in lanes {
		offline |= lane.offline;
		total = total.saturating_add(lane.state.total);
		let next = lane.buf.front().map(|(p, _)| *p).unwrap_or(lane.fetched);
		let mut skip: Vec<u32> = lane
			.state
			.skip
			.iter()
			.chain(&lane.removed)
			.copied()
			.filter(|p| *p >= next)
			.collect();
		skip.sort_unstable();
		skip.dedup();
		skip.truncate(MAX_SKIP);
		states.push(LaneState {
			source: lane.state.source,
			next,
			skip,
			done: lane.exhausted && lane.buf.is_empty() && !lane.failed,
			total: lane.state.total,
		});
	}
	let shown_now = shown.saturating_add(rows.len() as u32);
	let cursor = if states.iter().all(|s| s.done) {
		None
	} else {
		Some(encode(&Cursor {
			v: CURSOR_VERSION,
			shown: shown_now,
			lanes: states,
		})?)
	};
	Ok(AllPage {
		rows,
		total,
		offline,
		cursor,
		partial,
		limit,
		shown,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::content::{
		BoxFuture, CategoryTag, ContentVersion, FileKey, Project, ProjectType, Side, VersionFilter,
	};
	use std::collections::HashMap;
	use std::sync::Mutex;

	struct Fake {
		kind: SourceKind,
		hits: Vec<ProjectHit>,
		fail: Option<ErrorCode>,
		calls: Mutex<u32>,
	}

	impl Fake {
		fn new(kind: SourceKind, hits: Vec<ProjectHit>) -> Arc<Self> {
			Arc::new(Self {
				kind,
				hits,
				fail: None,
				calls: Mutex::new(0),
			})
		}
		fn failing(kind: SourceKind, code: ErrorCode) -> Arc<Self> {
			Arc::new(Self {
				kind,
				hits: Vec::new(),
				fail: Some(code),
				calls: Mutex::new(0),
			})
		}
	}

	fn nope<'a, T: Send + 'a>() -> BoxFuture<'a, T> {
		Box::pin(async { Err(AppError::unknown("fake")) })
	}

	impl ContentSource for Fake {
		fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxFuture<'a, SearchPage> {
			Box::pin(async move {
				if let Ok(mut c) = self.calls.lock() {
					*c += 1;
				}
				assert_eq!(q.source, self.kind);
				if let Some(code) = self.fail {
					return Err(AppError::coded(code, "fake"));
				}
				let mut list = self.hits.clone();
				match q.sort {
					SearchSort::Downloads => list.sort_by_key(|h| std::cmp::Reverse(h.downloads)),
					SearchSort::Updated => list.sort_by_key(|h| std::cmp::Reverse(h.updated_at)),
					SearchSort::Newest => list.sort_by_key(|h| std::cmp::Reverse(h.created_at)),
					_ => {}
				}
				let limit = q.limit.unwrap_or(20).min(7) as usize;
				let hits: Vec<ProjectHit> = list
					.iter()
					.skip(q.offset as usize)
					.take(limit)
					.cloned()
					.collect();
				Ok(SearchPage::new(
					hits,
					q.offset,
					limit as u32,
					list.len() as u64,
					false,
				))
			})
		}
		fn project<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Project> {
			nope()
		}
		fn projects<'a>(&'a self, _: &'a [String]) -> BoxFuture<'a, Vec<ProjectHit>> {
			nope()
		}
		fn project_versions<'a>(
			&'a self,
			_: &'a str,
			_: &'a VersionFilter,
		) -> BoxFuture<'a, Vec<ContentVersion>> {
			nope()
		}
		fn version<'a>(&'a self, _: &'a str) -> BoxFuture<'a, ContentVersion> {
			nope()
		}
		fn versions<'a>(&'a self, _: &'a [String]) -> BoxFuture<'a, Vec<ContentVersion>> {
			nope()
		}
		fn categories(&self) -> BoxFuture<'_, (Vec<CategoryTag>, bool)> {
			nope()
		}
		fn identify<'a>(
			&'a self,
			_: &'a [FileKey],
		) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
			nope()
		}
		fn latest<'a>(
			&'a self,
			_: &'a [FileKey],
			_: &'a VersionFilter,
		) -> BoxFuture<'a, HashMap<String, ContentVersion>> {
			nope()
		}
	}

	fn hit(source: SourceKind, id: &str, title: &str, author: &str, n: u64) -> ProjectHit {
		ProjectHit {
			source,
			id: id.into(),
			slug: id.to_lowercase(),
			project_type: ProjectType::Mod,
			title: title.into(),
			author: author.into(),
			description: String::new(),
			icon_url: None,
			downloads: n,
			follows: None,
			site_only: false,
			url: String::new(),
			categories: Vec::new(),
			updated_at: n * 10,
			game_versions: Vec::new(),
			loaders: Vec::new(),
			client_side: Side::Unknown,
			server_side: Side::Unknown,
			install: None,
			also_on: Vec::new(),
			created_at: 1000 - n,
		}
	}

	const MR: SourceKind = SourceKind::Modrinth;
	const CF: SourceKind = SourceKind::Curseforge;

	fn mr_list(n: u64) -> Vec<ProjectHit> {
		(0..n)
			.map(|i| {
				hit(
					MR,
					&format!("m{i}"),
					&format!("Modrinth {i}"),
					"a",
					100 - i * 3,
				)
			})
			.collect()
	}
	fn cf_list(n: u64) -> Vec<ProjectHit> {
		(0..n)
			.map(|i| hit(CF, &format!("c{i}"), &format!("Curse {i}"), "b", 99 - i * 2))
			.collect()
	}

	fn query(sort: SearchSort, limit: u32) -> SearchQuery {
		let mut q: SearchQuery = serde_json::from_value(serde_json::json!({ "type": "mod" }))
			.unwrap_or_else(|e| panic!("{e}"));
		q.sort = sort;
		q.limit = Some(limit);
		q
	}

	fn pair(mr: Arc<Fake>, cf: Arc<Fake>) -> Vec<(SourceKind, Arc<dyn ContentSource>)> {
		vec![
			(CF, cf as Arc<dyn ContentSource>),
			(MR, mr as Arc<dyn ContentSource>),
		]
	}

	/// Все порции подряд: (порции, сколько запросов).
	async fn all_pages(
		sources: impl Fn() -> Vec<(SourceKind, Arc<dyn ContentSource>)>,
		q: &SearchQuery,
	) -> Vec<Vec<Row>> {
		let mut cursor: Option<String> = None;
		let mut pages = Vec::new();
		for _ in 0..50 {
			let page = search(sources(), q, cursor.as_deref())
				.await
				.unwrap_or_else(|e| panic!("{e:?}"));
			pages.push(page.rows);
			match page.cursor {
				Some(c) => cursor = Some(c),
				None => return pages,
			}
		}
		panic!("курсор не кончился");
	}

	fn ids(rows: &[Row]) -> Vec<String> {
		rows.iter().map(|r| r.hit.id.clone()).collect()
	}

	#[tokio::test]
	async fn merges_by_field_without_gaps() {
		let (mr, cf) = (Fake::new(MR, mr_list(23)), Fake::new(CF, cf_list(17)));
		for sort in [
			SearchSort::Downloads,
			SearchSort::Updated,
			SearchSort::Newest,
		] {
			let q = query(sort, 6);
			let pages = all_pages(|| pair(mr.clone(), cf.clone()), &q).await;
			let rows: Vec<&Row> = pages.iter().flatten().collect();
			assert_eq!(rows.len(), 40, "{sort:?}");
			let mut seen: Vec<&str> = rows.iter().map(|r| r.hit.id.as_str()).collect();
			let keys: Vec<u64> = rows.iter().map(|r| key(sort, &r.hit)).collect();
			assert!(keys.windows(2).all(|w| w[0] >= w[1]), "{sort:?}: {keys:?}");
			seen.sort_unstable();
			seen.dedup();
			assert_eq!(seen.len(), 40);
			assert!(pages[..pages.len() - 1].iter().all(|p| !p.is_empty()));
		}
	}

	#[tokio::test]
	async fn relevance_alternates() {
		let (mr, cf) = (Fake::new(MR, mr_list(5)), Fake::new(CF, cf_list(3)));
		let q = query(SearchSort::Relevance, 3);
		let pages = all_pages(|| pair(mr.clone(), cf.clone()), &q).await;
		let all: Vec<String> = pages.iter().flat_map(|p| ids(p)).collect();
		assert_eq!(all, ["m0", "c0", "m1", "c1", "m2", "c2", "m3", "m4"]);
		assert_eq!(ids(&pages[0]), ["m0", "c0", "m1"]);
	}

	#[tokio::test]
	async fn merges_twins_and_keeps_doubtful() {
		let mut mr = mr_list(4);
		mr[0] = hit(MR, "AANobbMI", "Sodium", "jellysquid3", 90);
		mr[0].slug = "sodium".into();
		mr[2] = hit(MR, "u6dRKJwZ", "Just Enough Items", "mezz", 50);
		mr[2].slug = "jei".into();
		mr[3] = hit(MR, "zzz", "Backpacks", "alice", 40);
		mr[3].slug = "backpacks".into();
		let mut cf = cf_list(6);
		// Двойник JEI далеко впереди: склеится на первой порции, позиция уйдёт в `skip`.
		cf[5] = hit(CF, "238222", "Just Enough Items (JEI)", "mezz", 20);
		cf[5].slug = "jei".into();
		cf[1] = hit(CF, "394468", "Sodium", "jellysquid3_", 80);
		cf[1].slug = "sodium".into();
		// Тот же slug, но другие название и автор — не склеиваем.
		cf[2] = hit(CF, "777", "Better Backpacks", "bob", 70);
		cf[2].slug = "backpacks".into();
		let (mr, cf) = (Fake::new(MR, mr), Fake::new(CF, cf));
		let q = query(SearchSort::Relevance, 6);
		let pages = all_pages(|| pair(mr.clone(), cf.clone()), &q).await;
		assert_eq!(
			ids(&pages[0]),
			["AANobbMI", "c0", "m1", "u6dRKJwZ", "777", "zzz"]
		);
		let rows: Vec<&Row> = pages.iter().flatten().collect();
		let sodium = rows.iter().find(|r| r.hit.id == "AANobbMI").unwrap();
		assert_eq!(sodium.twins.len(), 1);
		assert_eq!(sodium.twins[0].id, "394468");
		let jei = rows.iter().find(|r| r.hit.id == "u6dRKJwZ").unwrap();
		assert_eq!(jei.twins[0].id, "238222");
		assert!(rows.iter().any(|r| r.hit.id == "zzz" && r.twins.is_empty()));
		assert!(rows.iter().any(|r| r.hit.id == "777"));
		// 4 + 6 − 2 склеенных, без повторов.
		assert_eq!(rows.len(), 8);
		let mut all: Vec<&str> = rows
			.iter()
			.flat_map(|r| std::iter::once(&r.hit).chain(&r.twins))
			.map(|h| h.id.as_str())
			.collect();
		all.sort_unstable();
		all.dedup();
		assert_eq!(all.len(), 10);
	}

	#[tokio::test]
	async fn twin_first_from_curseforge_shows_modrinth_row() {
		let mut mr = mr_list(3);
		mr[2] = hit(MR, "m-jei", "JEI", "mezz", 1);
		mr[2].slug = "jei".into();
		let mut cf = cf_list(2);
		cf[0] = hit(CF, "c-jei", "JEI", "mezz", 99);
		cf[0].slug = "jei".into();
		let (mr, cf) = (Fake::new(MR, mr), Fake::new(CF, cf));
		let q = query(SearchSort::Downloads, 3);
		let pages = all_pages(|| pair(mr.clone(), cf.clone()), &q).await;
		let rows: Vec<&Row> = pages.iter().flatten().collect();
		// Двойник CurseForge встал раньше: строка — Modrinth, на месте CurseForge.
		assert_eq!(rows[1].hit.id, "m-jei");
		assert_eq!(rows[1].hit.source, MR);
		assert_eq!(rows[1].twins[0].id, "c-jei");
		assert_eq!(rows.len(), 4);
	}

	#[test]
	fn doubtful_matches() {
		let a = hit(MR, "x", "Sodium", "jelly", 1);
		let mut b = hit(CF, "y", "Sodium", "someone", 1);
		b.slug = "sodium-fork".into();
		// Только название — мало.
		assert!(!same_project(&a, &b));
		b.author = "Jelly".into();
		assert!(same_project(&a, &b));
		let mut c = hit(CF, "x", "Other", "other", 1);
		c.slug = "x".into();
		// Только slug — мало.
		assert!(!same_project(&a, &c));
		let mut d = a.clone();
		d.source = CF;
		d.project_type = ProjectType::Resourcepack;
		assert!(!same_project(&a, &d));
	}

	#[tokio::test]
	async fn one_source_failed() {
		let mr = Fake::new(MR, mr_list(3));
		let cf = Fake::failing(CF, ErrorCode::SourceDisabled);
		let page = search(pair(mr, cf), &query(SearchSort::Downloads, 10), None)
			.await
			.unwrap_or_else(|e| panic!("{e:?}"));
		assert_eq!(ids(&page.rows), ["m0", "m1", "m2"]);
		assert_eq!(page.partial.len(), 1);
		assert_eq!(page.partial[0].source, CF);
		// Упавший каталог спросим снова со следующей порции.
		assert!(page.cursor.is_some());
	}

	#[tokio::test]
	async fn both_failed() {
		let r = search(
			pair(
				Fake::failing(MR, ErrorCode::Network),
				Fake::failing(CF, ErrorCode::Network),
			),
			&query(SearchSort::Relevance, 10),
			None,
		)
		.await;
		assert_eq!(r.err().map(|e| e.code), Some(ErrorCode::Network));
		let r = search(
			pair(
				Fake::failing(MR, ErrorCode::Network),
				Fake::failing(CF, ErrorCode::RateLimited),
			),
			&query(SearchSort::Relevance, 10),
			None,
		)
		.await;
		assert_eq!(r.err().map(|e| e.code), Some(ErrorCode::SourceUnavailable));
	}

	#[tokio::test]
	async fn single_source_same_format() {
		let mr = Fake::new(MR, mr_list(9));
		let q = query(SearchSort::Relevance, 4);
		let pages = all_pages(|| vec![(MR, mr.clone() as Arc<dyn ContentSource>)], &q).await;
		let all: Vec<String> = pages.iter().flat_map(|p| ids(p)).collect();
		assert_eq!(all.len(), 9);
		assert_eq!(all[0], "m0");
		assert_eq!(all[8], "m8");
	}

	#[tokio::test]
	async fn rejects_categories_sort_and_bad_cursor() {
		let mr = || vec![(MR, Fake::new(MR, mr_list(1)) as Arc<dyn ContentSource>)];
		let mut q = query(SearchSort::Relevance, 4);
		q.categories = vec!["magic".into()];
		assert_eq!(
			search(mr(), &q, None).await.err().map(|e| e.code),
			Some(ErrorCode::InvalidValue)
		);
		let q = query(SearchSort::Follows, 4);
		assert_eq!(
			search(mr(), &q, None).await.err().map(|e| e.code),
			Some(ErrorCode::InvalidValue)
		);
		let q = query(SearchSort::Relevance, 4);
		assert_eq!(
			search(mr(), &q, Some("!!")).await.err().map(|e| e.code),
			Some(ErrorCode::InvalidValue)
		);
	}
}
