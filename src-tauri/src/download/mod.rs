//! Загрузка файлов: параллельно с ограничением, проверка sha1, повторы, зеркала, отмена, прогресс.

pub mod mirrors;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub use self::mirrors::DownloadSource;
use crate::error::{AppError, ErrorCode};

/// Сколько файлов качается одновременно.
pub const CONCURRENCY: usize = 16;
/// Попыток на один адрес.
const ATTEMPTS_PER_URL: u32 = 3;
/// Скорость ниже этой за окно [`SLOW_WINDOW`] — переходим на следующий адрес (если он есть).
const SLOW_BYTES_PER_SEC: u64 = 16 * 1024;
const SLOW_WINDOW: Duration = Duration::from_secs(20);
/// Прогресс отправляется не чаще этого.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// HTTP-клиент для файлов: без общего тайм-аута (большие файлы качаются долго), но с тайм-аутом чтения.
pub fn build_client() -> reqwest::Result<reqwest::Client> {
	build_client_with(&[])
}

fn host_in(url: &url::Url, hosts: &[&str]) -> bool {
	url.host_str()
		.is_some_and(|h| hosts.iter().any(|x| h.eq_ignore_ascii_case(x)))
}

/// Заголовок с секретом уходит только по `https` к одному из `hosts`.
fn secret_allowed(url: &url::Url, hosts: &[&str]) -> bool {
	url.scheme() == "https" && host_in(url, hosts)
}

/// С хостов секретного заголовка перенаправляем только на них же и только по `https`.
fn redirect_allowed(previous: &[url::Url], target: &url::Url, secret_hosts: &[&str]) -> bool {
	let from_secret = previous.iter().any(|u| host_in(u, secret_hosts));
	!from_secret || secret_allowed(target, secret_hosts)
}

/// С секретным заголовком для хостов. reqwest переносит заголовки при перенаправлении,
/// поэтому с этих хостов на чужие и на `http` не переходим.
pub fn build_client_with(
	secret_hosts: &'static [&'static str],
) -> reqwest::Result<reqwest::Client> {
	let policy = reqwest::redirect::Policy::custom(move |attempt| {
		if attempt.previous().len() >= 10 {
			return attempt.error("слишком много перенаправлений");
		}
		if !redirect_allowed(attempt.previous(), attempt.url(), secret_hosts) {
			attempt.error("перенаправление с CDN на другой хост или на http")
		} else {
			attempt.follow()
		}
	});
	reqwest::Client::builder()
		.user_agent(crate::http::USER_AGENT)
		.connect_timeout(Duration::from_secs(15))
		.read_timeout(Duration::from_secs(30))
		.redirect(policy)
		.build()
}

/// Заголовок для запросов к этим хостам (ключ API CDN CurseForge). Значение помечено секретным.
#[derive(Clone)]
pub struct HostHeader {
	pub hosts: &'static [&'static str],
	pub name: reqwest::header::HeaderName,
	pub value: reqwest::header::HeaderValue,
}

#[derive(Debug, Clone)]
pub struct Job {
	pub url: String,
	pub path: PathBuf,
	/// sha1 в нижнем регистре. Нет — файл не проверяется (так у части библиотек загрузчиков).
	pub sha1: Option<String>,
	pub size: Option<u64>,
}

/// Как проверять уже скачанный файл.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verify {
	/// Есть и совпал размер (если известен). Для объектов ассетов: хеш уже в имени файла.
	Size,
	/// Размер и sha1 (если известен).
	Hash,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
	#[error("отменено")]
	Cancelled,
	#[error("{url}: HTTP {status}")]
	Http { url: String, status: u16 },
	#[error("{url}: {message}")]
	Network { url: String, message: String },
	#[error("{url}: sha1 не совпал (ожидался {expected}, получен {actual})")]
	Hash {
		url: String,
		expected: String,
		actual: String,
	},
	#[error("{url}: слишком медленно")]
	Slow { url: String },
	#[error("{path}: {message}")]
	Io { path: String, message: String },
}

impl DownloadError {
	fn io(path: &Path, err: impl std::fmt::Display) -> Self {
		Self::Io {
			path: path.display().to_string(),
			message: err.to_string(),
		}
	}

	/// Нет соединения (а не ошибка сервера или файла).
	pub fn is_network(&self) -> bool {
		matches!(self, Self::Network { .. })
	}
}

impl From<DownloadError> for AppError {
	fn from(err: DownloadError) -> Self {
		match err {
			DownloadError::Cancelled => AppError::new(ErrorCode::Cancelled),
			DownloadError::Io { .. } => AppError::storage(err.to_string()),
			_ => AppError::coded(ErrorCode::DownloadFailed, err.to_string()),
		}
	}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Snapshot {
	pub files_done: u64,
	pub files_total: u64,
	pub bytes_done: u64,
	pub bytes_total: u64,
	/// Байты из сети, без уже лежавших файлов: по ним считается скорость.
	pub bytes_received: u64,
	/// Байты уже скачанных файлов, прошедших проверку (входят в `bytes_done` и `bytes_total`).
	pub bytes_skipped: u64,
}

type ProgressFn = dyn Fn(Snapshot) + Send + Sync;

/// Счётчики прогресса. Колбэк вызывается не чаще раза в 100 мс (и всегда при [`Progress::flush`]).
pub struct Progress {
	files_done: AtomicU64,
	files_total: AtomicU64,
	bytes_done: AtomicU64,
	bytes_total: AtomicU64,
	bytes_received: AtomicU64,
	bytes_skipped: AtomicU64,
	last: Mutex<Option<Instant>>,
	on_tick: Box<ProgressFn>,
}

impl Progress {
	pub fn new(on_tick: impl Fn(Snapshot) + Send + Sync + 'static) -> Arc<Self> {
		Arc::new(Self {
			files_done: AtomicU64::new(0),
			files_total: AtomicU64::new(0),
			bytes_done: AtomicU64::new(0),
			bytes_total: AtomicU64::new(0),
			bytes_received: AtomicU64::new(0),
			bytes_skipped: AtomicU64::new(0),
			last: Mutex::new(None),
			on_tick: Box::new(on_tick),
		})
	}

	pub fn silent() -> Arc<Self> {
		Self::new(|_| {})
	}

	pub fn snapshot(&self) -> Snapshot {
		Snapshot {
			files_done: self.files_done.load(Ordering::Relaxed),
			files_total: self.files_total.load(Ordering::Relaxed),
			bytes_done: self.bytes_done.load(Ordering::Relaxed),
			bytes_total: self.bytes_total.load(Ordering::Relaxed),
			bytes_received: self.bytes_received.load(Ordering::Relaxed),
			bytes_skipped: self.bytes_skipped.load(Ordering::Relaxed),
		}
	}

	pub fn add_total(&self, files: u64, bytes: u64) {
		self.files_total.fetch_add(files, Ordering::Relaxed);
		self.bytes_total.fetch_add(bytes, Ordering::Relaxed);
		self.tick();
	}

	fn add_bytes(&self, bytes: u64) {
		self.bytes_done.fetch_add(bytes, Ordering::Relaxed);
		self.tick();
	}

	fn add_received(&self, bytes: u64) {
		self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
		self.bytes_done.fetch_add(bytes, Ordering::Relaxed);
		self.tick();
	}

	fn sub_bytes(&self, bytes: u64) {
		let _ = self
			.bytes_done
			.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
				Some(v.saturating_sub(bytes))
			});
	}

	pub fn file_done(&self) {
		self.files_done.fetch_add(1, Ordering::Relaxed);
		self.tick();
	}

	fn tick(&self) {
		let now = Instant::now();
		{
			let Ok(mut last) = self.last.lock() else {
				return;
			};
			if last.is_some_and(|t| now.duration_since(t) < PROGRESS_INTERVAL) {
				return;
			}
			*last = Some(now);
		}
		(self.on_tick)(self.snapshot());
	}

	/// Отправляет текущее состояние сразу, без ограничения частоты.
	pub fn flush(&self) {
		if let Ok(mut last) = self.last.lock() {
			*last = Some(Instant::now());
		}
		(self.on_tick)(self.snapshot());
	}
}

#[derive(Clone)]
pub struct Downloader {
	client: reqwest::Client,
	limit: Arc<Semaphore>,
	source: Arc<RwLock<DownloadSource>>,
	header: Option<Arc<HostHeader>>,
}

impl Downloader {
	pub fn new(client: reqwest::Client, concurrency: usize) -> Self {
		Self {
			client,
			limit: Arc::new(Semaphore::new(concurrency)),
			source: Arc::new(RwLock::new(DownloadSource::default())),
			header: None,
		}
	}

	/// Запросы к хостам `header.hosts` получают этот заголовок.
	pub fn with_host_header(mut self, header: HostHeader) -> Self {
		self.header = Some(Arc::new(header));
		self
	}

	fn get(&self, url: &str) -> reqwest::RequestBuilder {
		let builder = self.client.get(url);
		match &self.header {
			Some(h) if url::Url::parse(url).is_ok_and(|u| secret_allowed(&u, h.hosts)) => {
				builder.header(h.name.clone(), h.value.clone())
			}
			_ => builder,
		}
	}

	pub fn set_source(&self, source: DownloadSource) {
		if let Ok(mut s) = self.source.write() {
			*s = source;
		}
	}

	fn candidates(&self, url: &str) -> Vec<String> {
		let source = self.source.read().map(|s| *s).unwrap_or_default();
		mirrors::candidates(url, source)
	}

	/// Небольшой файл целиком в память (метаданные): повторы и зеркала, проверка sha1 при наличии.
	pub async fn bytes(
		&self,
		url: &str,
		sha1: Option<&str>,
		cancel: &CancellationToken,
	) -> Result<Vec<u8>, DownloadError> {
		let mut last = None;
		for candidate in self.candidates(url) {
			for attempt in 0..ATTEMPTS_PER_URL {
				if cancel.is_cancelled() {
					return Err(DownloadError::Cancelled);
				}
				let result = tokio::select! {
					_ = cancel.cancelled() => return Err(DownloadError::Cancelled),
					r = self.get_bytes(&candidate) => r,
				};
				let err = match result {
					Ok(bytes) => match sha1 {
						Some(expected) if hex_sha1(&bytes) != expected.to_ascii_lowercase() => {
							DownloadError::Hash {
								url: candidate.clone(),
								expected: expected.to_owned(),
								actual: hex_sha1(&bytes),
							}
						}
						_ => return Ok(bytes),
					},
					Err(err) => err,
				};
				let next_url = is_final_for_url(&err);
				last = Some(err);
				if next_url {
					break;
				}
				backoff(attempt, cancel).await?;
			}
		}
		Err(last.unwrap_or(DownloadError::Network {
			url: url.to_owned(),
			message: "нет адресов".into(),
		}))
	}

	async fn get_bytes(&self, url: &str) -> Result<Vec<u8>, DownloadError> {
		let resp = self
			.get(url)
			.timeout(Duration::from_secs(60))
			.send()
			.await
			.map_err(|e| net_error(url, e))?;
		let status = resp.status();
		if !status.is_success() {
			return Err(DownloadError::Http {
				url: url.to_owned(),
				status: status.as_u16(),
			});
		}
		Ok(resp.bytes().await.map_err(|e| net_error(url, e))?.to_vec())
	}

	/// Качает набор файлов параллельно. Уже скачанные и прошедшие проверку пропускаются.
	/// При первой ошибке остальные загрузки отменяются.
	pub async fn files(
		&self,
		jobs: Vec<Job>,
		verify: Verify,
		progress: Arc<Progress>,
		cancel: &CancellationToken,
	) -> Result<(), DownloadError> {
		let mut seen = HashSet::new();
		let jobs: Vec<Job> = jobs
			.into_iter()
			.filter(|j| seen.insert(j.path.clone()))
			.collect();
		progress.add_total(jobs.len() as u64, jobs.iter().filter_map(|j| j.size).sum());

		let child = cancel.child_token();
		let mut set = JoinSet::new();
		for job in jobs {
			let this = self.clone();
			let progress = progress.clone();
			let child = child.clone();
			set.spawn(async move {
				let _permit = tokio::select! {
					_ = child.cancelled() => return Err(DownloadError::Cancelled),
					p = this.limit.clone().acquire_owned() => p.map_err(|_| DownloadError::Cancelled)?,
				};
				this.file(&job, verify, &progress, &child).await
			});
		}

		let mut first_err: Option<DownloadError> = None;
		while let Some(joined) = set.join_next().await {
			let result = joined.unwrap_or_else(|e| {
				Err(DownloadError::Io {
					path: String::new(),
					message: format!("задача загрузки упала: {e}"),
				})
			});
			if let Err(err) = result {
				child.cancel();
				// Настоящая причина важнее, чем «отменено» у соседних задач.
				let replace = match &first_err {
					None => true,
					Some(DownloadError::Cancelled) => !matches!(err, DownloadError::Cancelled),
					Some(_) => false,
				};
				if replace {
					first_err = Some(err);
				}
			}
		}
		if cancel.is_cancelled() {
			return Err(DownloadError::Cancelled);
		}
		match first_err {
			Some(err) => Err(err),
			None => Ok(()),
		}
	}

	/// Один файл. `Ok(true)` — скачан, `Ok(false)` — уже был и прошёл проверку.
	pub async fn file(
		&self,
		job: &Job,
		verify: Verify,
		progress: &Progress,
		cancel: &CancellationToken,
	) -> Result<bool, DownloadError> {
		if is_valid(job, verify).await? {
			if let Some(size) = job.size {
				progress.bytes_skipped.fetch_add(size, Ordering::Relaxed);
				progress.add_bytes(size);
			}
			progress.file_done();
			return Ok(false);
		}
		let candidates = self.candidates(&job.url);
		let mut last = None;
		let mut counted_total = job.size.is_some();
		for (index, url) in candidates.iter().enumerate() {
			let has_next = index + 1 < candidates.len();
			for attempt in 0..ATTEMPTS_PER_URL {
				if cancel.is_cancelled() {
					return Err(DownloadError::Cancelled);
				}
				let mut written = 0u64;
				let result = self
					.try_file(
						url,
						job,
						progress,
						cancel,
						has_next,
						&mut written,
						&mut counted_total,
					)
					.await;
				match result {
					Ok(()) => {
						// Размер в метаданных мог отличаться от фактического.
						if let Some(size) = job.size {
							if written < size {
								progress.add_bytes(size - written);
							} else {
								progress.sub_bytes(written - size);
							}
						}
						progress.file_done();
						return Ok(true);
					}
					Err(err) => {
						progress.sub_bytes(written);
						if matches!(err, DownloadError::Cancelled | DownloadError::Io { .. }) {
							return Err(err);
						}
						log::warn!("загрузка не удалась (попытка {}): {err}", attempt + 1);
						let next_url = is_final_for_url(&err);
						last = Some(err);
						if next_url {
							break;
						}
						backoff(attempt, cancel).await?;
					}
				}
			}
		}
		Err(last.unwrap_or(DownloadError::Network {
			url: job.url.clone(),
			message: "нет адресов".into(),
		}))
	}

	#[allow(clippy::too_many_arguments)]
	async fn try_file(
		&self,
		url: &str,
		job: &Job,
		progress: &Progress,
		cancel: &CancellationToken,
		abort_if_slow: bool,
		written: &mut u64,
		counted_total: &mut bool,
	) -> Result<(), DownloadError> {
		let mut resp = tokio::select! {
			_ = cancel.cancelled() => return Err(DownloadError::Cancelled),
			r = self.get(url).send() => r.map_err(|e| net_error(url, e))?,
		};
		let status = resp.status();
		if !status.is_success() {
			return Err(DownloadError::Http {
				url: url.to_owned(),
				status: status.as_u16(),
			});
		}
		if !*counted_total {
			if let Some(len) = resp.content_length() {
				progress.add_total(0, len);
			}
			*counted_total = true;
		}

		let dir = job
			.path
			.parent()
			.ok_or_else(|| DownloadError::io(&job.path, "нет родительской папки"))?;
		tokio::fs::create_dir_all(dir)
			.await
			.map_err(|e| DownloadError::io(dir, e))?;
		let tmp = temp_path(&job.path);
		let mut file = tokio::fs::File::create(&tmp)
			.await
			.map_err(|e| DownloadError::io(&tmp, e))?;

		let result = async {
			let mut hasher = Sha1::new();
			let mut window_start = Instant::now();
			let mut window_bytes = 0u64;
			loop {
				let chunk = tokio::select! {
					_ = cancel.cancelled() => return Err(DownloadError::Cancelled),
					c = resp.chunk() => c.map_err(|e| net_error(url, e))?,
				};
				let Some(chunk) = chunk else { break };
				hasher.update(&chunk);
				file.write_all(&chunk)
					.await
					.map_err(|e| DownloadError::io(&tmp, e))?;
				let len = chunk.len() as u64;
				*written += len;
				window_bytes += len;
				progress.add_received(len);
				let elapsed = window_start.elapsed();
				if elapsed >= SLOW_WINDOW {
					if abort_if_slow && window_bytes < SLOW_BYTES_PER_SEC * elapsed.as_secs() {
						return Err(DownloadError::Slow {
							url: url.to_owned(),
						});
					}
					window_start = Instant::now();
					window_bytes = 0;
				}
			}
			file.flush().await.map_err(|e| DownloadError::io(&tmp, e))?;
			drop(file);
			if let Some(expected) = &job.sha1 {
				let actual = hex(&hasher.finalize());
				if !actual.eq_ignore_ascii_case(expected) {
					return Err(DownloadError::Hash {
						url: url.to_owned(),
						expected: expected.clone(),
						actual,
					});
				}
			}
			tokio::fs::rename(&tmp, &job.path)
				.await
				.map_err(|e| DownloadError::io(&job.path, e))
		}
		.await;
		if result.is_err() {
			let _ = tokio::fs::remove_file(&tmp).await;
		}
		result
	}
}

/// После этой ошибки на тот же адрес повторять бессмысленно — сразу к следующему.
fn is_final_for_url(err: &DownloadError) -> bool {
	match err {
		DownloadError::Http { status, .. } => matches!(status, 400..=428 | 430..=499),
		DownloadError::Slow { .. } => true,
		_ => false,
	}
}

async fn backoff(attempt: u32, cancel: &CancellationToken) -> Result<(), DownloadError> {
	let delay = Duration::from_millis(500 * u64::from(attempt + 1));
	tokio::select! {
		_ = cancel.cancelled() => Err(DownloadError::Cancelled),
		_ = tokio::time::sleep(delay) => Ok(()),
	}
}

fn net_error(url: &str, err: reqwest::Error) -> DownloadError {
	DownloadError::Network {
		url: url.to_owned(),
		message: err.without_url().to_string(),
	}
}

/// Временный файл загрузки: `<файл>.part-<8 hex>` рядом с целевым.
const PART_MARKER: &str = ".part-";

fn temp_path(path: &Path) -> PathBuf {
	let mut name = path.as_os_str().to_owned();
	name.push(format!("{PART_MARKER}{:08x}", rand::random::<u32>()));
	PathBuf::from(name)
}

fn is_part_name(name: &str) -> bool {
	name.rsplit_once(PART_MARKER).is_some_and(|(base, tail)| {
		!base.is_empty() && tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_hexdigit())
	})
}

/// Удаляет старые `*.part-*`, оставшиеся от прерванных загрузок. Долго — вызывать в фоне.
pub fn remove_stale_parts(roots: &[PathBuf], older_than: Duration) -> usize {
	let now = std::time::SystemTime::now();
	let mut removed = 0;
	let mut stack: Vec<PathBuf> = roots.to_vec();
	while let Some(dir) = stack.pop() {
		let Ok(entries) = std::fs::read_dir(&dir) else {
			continue;
		};
		for entry in entries.flatten() {
			let Ok(kind) = entry.file_type() else {
				continue;
			};
			if kind.is_dir() {
				stack.push(entry.path());
				continue;
			}
			if !kind.is_file() || !is_part_name(&entry.file_name().to_string_lossy()) {
				continue;
			}
			let old = entry
				.metadata()
				.and_then(|m| m.modified())
				.ok()
				.and_then(|t| now.duration_since(t).ok())
				.is_some_and(|age| age >= older_than);
			if old && std::fs::remove_file(entry.path()).is_ok() {
				removed += 1;
			}
		}
	}
	removed
}

/// Файл уже есть и проходит проверку.
pub async fn is_valid(job: &Job, verify: Verify) -> Result<bool, DownloadError> {
	let meta = match tokio::fs::metadata(&job.path).await {
		Ok(meta) => meta,
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
		Err(err) => return Err(DownloadError::io(&job.path, err)),
	};
	if !meta.is_file() {
		return Ok(false);
	}
	if job.size.is_some_and(|size| size != meta.len()) {
		return Ok(false);
	}
	match (&job.sha1, verify) {
		(Some(expected), Verify::Hash) => {
			let actual = file_sha1(job.path.clone()).await?;
			Ok(actual.eq_ignore_ascii_case(expected))
		}
		_ => Ok(true),
	}
}

pub async fn file_sha1(path: PathBuf) -> Result<String, DownloadError> {
	tokio::task::spawn_blocking(move || {
		let mut file = std::fs::File::open(&path).map_err(|e| DownloadError::io(&path, e))?;
		let mut hasher = Sha1::new();
		std::io::copy(&mut file, &mut hasher).map_err(|e| DownloadError::io(&path, e))?;
		Ok(hex(&hasher.finalize()))
	})
	.await
	.map_err(|e| DownloadError::Io {
		path: String::new(),
		message: e.to_string(),
	})?
}

pub fn hex_sha1(bytes: &[u8]) -> String {
	hex(&Sha1::digest(bytes))
}

/// Байты в hex нижним регистром.
pub fn to_hex(bytes: &[u8]) -> String {
	hex(bytes)
}

fn hex(bytes: &[u8]) -> String {
	use std::fmt::Write;
	bytes.iter().fold(String::with_capacity(40), |mut s, b| {
		let _ = write!(s, "{b:02x}");
		s
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sha1_hex() {
		assert_eq!(hex_sha1(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
	}

	#[test]
	fn secret_header_only_over_https() {
		let hosts: &[&str] = &["edge.forgecdn.net"];
		let u = |s: &str| url::Url::parse(s).unwrap();
		assert!(secret_allowed(&u("https://edge.forgecdn.net/a.jar"), hosts));
		assert!(!secret_allowed(&u("http://edge.forgecdn.net/a.jar"), hosts));
		assert!(!secret_allowed(&u("https://evil.example/a.jar"), hosts));
		let from = [u("https://edge.forgecdn.net/a.jar")];
		assert!(redirect_allowed(
			&from,
			&u("https://edge.forgecdn.net/b.jar"),
			hosts
		));
		assert!(!redirect_allowed(
			&from,
			&u("http://edge.forgecdn.net/b.jar"),
			hosts
		));
		assert!(!redirect_allowed(
			&from,
			&u("https://evil.example/b.jar"),
			hosts
		));
		let other = [u("https://piston-data.mojang.com/a")];
		assert!(redirect_allowed(
			&other,
			&u("http://mirror.example/a"),
			hosts
		));
	}

	#[test]
	fn removes_only_stale_parts() {
		let dir = tempfile::tempdir().unwrap();
		let deep = dir.path().join("libraries/a/b");
		std::fs::create_dir_all(&deep).unwrap();
		let day = Duration::from_secs(24 * 60 * 60);
		let old = std::time::SystemTime::now() - 2 * day;
		let make = |name: &str, aged: bool| {
			let path = deep.join(name);
			std::fs::write(&path, b"x").unwrap();
			if aged {
				std::fs::File::options()
					.write(true)
					.open(&path)
					.unwrap()
					.set_modified(old)
					.unwrap();
			}
			path
		};
		let stale = make("b-1.jar.part-0a1b2c3d", true);
		let fresh = make("b-1.jar.part-deadbeef", false);
		let not_part = make("b-1.jar.part-xyz", true);
		let jar = make("b-1.jar", true);
		assert_eq!(remove_stale_parts(&[dir.path().to_path_buf()], day), 1);
		assert!(!stale.exists());
		assert!(fresh.exists() && not_part.exists() && jar.exists());
		assert!(is_part_name(
			&temp_path(Path::new("x.jar")).to_string_lossy()
		));
	}

	#[tokio::test]
	async fn validates_existing_files() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("a.txt");
		std::fs::write(&path, b"abc").unwrap();
		let job = |sha1: &str, size| Job {
			url: String::new(),
			path: path.clone(),
			sha1: Some(sha1.into()),
			size: Some(size),
		};
		let good = job("a9993e364706816aba3e25717850c26c9cd0d89d", 3);
		assert!(is_valid(&good, Verify::Hash).await.unwrap());
		let bad_hash = job("0000000000000000000000000000000000000000", 3);
		assert!(!is_valid(&bad_hash, Verify::Hash).await.unwrap());
		assert!(is_valid(&bad_hash, Verify::Size).await.unwrap());
		assert!(!is_valid(&job("x", 4), Verify::Size).await.unwrap());
		let missing = Job {
			path: dir.path().join("none"),
			..good
		};
		assert!(!is_valid(&missing, Verify::Size).await.unwrap());
	}

	#[test]
	fn errors_map_to_codes() {
		let err: AppError = DownloadError::Cancelled.into();
		assert_eq!(err.code, ErrorCode::Cancelled);
		let err: AppError = DownloadError::Http {
			url: "u".into(),
			status: 404,
		}
		.into();
		assert_eq!(err.code, ErrorCode::DownloadFailed);
		assert_eq!(err.details.as_deref(), Some("u: HTTP 404"));
		assert!(is_final_for_url(&DownloadError::Http {
			url: String::new(),
			status: 404
		}));
		assert!(!is_final_for_url(&DownloadError::Http {
			url: String::new(),
			status: 503
		}));
	}
}
