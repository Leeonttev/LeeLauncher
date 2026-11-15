//! Скачивание пакета обновления в файл с докачкой (`Range`) и замером скорости.

use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

/// Паузы перед повторами после обрыва связи.
pub const RETRY_PAUSES: [Duration; 3] = [
	Duration::from_secs(2),
	Duration::from_secs(5),
	Duration::from_secs(10),
];
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// Даже без новых байтов событие приходит раз в секунду: интерфейс увидит «ждём ответа».
const TICK: Duration = Duration::from_secs(1);
const SPEED_WINDOW: Duration = Duration::from_secs(3);
const SLOW_WINDOW: Duration = Duration::from_secs(10);
/// Нет новых байтов дольше — скорость неизвестна.
const STALL: Duration = Duration::from_secs(5);
const SLOW_BELOW: f64 = 200.0 * 1024.0;
const FAST_ABOVE: f64 = 400.0 * 1024.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reconnect {
	pub attempt: u32,
	pub of: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
	/// Байты в файле, вместе с докачанными раньше.
	pub downloaded: u64,
	pub total: Option<u64>,
	/// Скорость за последние 3 с; `None`, если байтов не было 5 с или замер ещё не набрался.
	pub speed_bps: Option<u64>,
	/// Осталось секунд; не раньше 3 с от первых байтов.
	pub eta_secs: Option<u64>,
	/// Средняя скорость за 10 с ниже 200 КБ/с (снимается выше 400 КБ/с).
	pub slow: bool,
	/// Идёт повтор после обрыва.
	pub reconnect: Option<Reconnect>,
}

/// Замер скорости по байтам из сети.
pub struct Meter {
	samples: VecDeque<(Instant, u64)>,
	first_byte: Option<Instant>,
	last_change: Option<Instant>,
	received: u64,
	slow: bool,
}

impl Meter {
	pub fn new() -> Self {
		Self {
			samples: VecDeque::new(),
			first_byte: None,
			last_change: None,
			received: 0,
			slow: false,
		}
	}

	pub fn add(&mut self, now: Instant, bytes: u64) {
		if bytes == 0 {
			return;
		}
		self.received += bytes;
		self.first_byte.get_or_insert(now);
		self.last_change = Some(now);
		self.samples.push_back((now, self.received));
		while self
			.samples
			.front()
			.is_some_and(|(t, _)| now.duration_since(*t) > SLOW_WINDOW)
		{
			self.samples.pop_front();
		}
	}

	/// Байтов в секунду за `window` до `now` (от последнего замера раньше окна).
	fn rate(&self, now: Instant, window: Duration) -> Option<f64> {
		let start = self.first_byte?;
		let from = self
			.samples
			.iter()
			.rev()
			.find(|(t, _)| now.duration_since(*t) >= window)
			.copied()
			.or_else(|| (now.duration_since(start) >= window).then_some((start, 0)))
			// Окно ещё не набралось — с первого байта.
			.unwrap_or((start, 0));
		let dt = now.duration_since(from.0).as_secs_f64();
		(dt >= 0.5).then(|| (self.received - from.1) as f64 / dt)
	}

	pub fn speed(&self, now: Instant) -> Option<u64> {
		if self
			.last_change
			.is_none_or(|t| now.duration_since(t) > STALL)
		{
			return None;
		}
		self.rate(now, SPEED_WINDOW).map(|r| r as u64)
	}

	pub fn eta(&self, now: Instant, downloaded: u64, total: Option<u64>) -> Option<u64> {
		let total = total?;
		let started = self.first_byte?;
		if now.duration_since(started) < SPEED_WINDOW {
			return None;
		}
		let speed = self.speed(now)?;
		(speed > 0).then(|| total.saturating_sub(downloaded).div_ceil(speed))
	}

	pub fn slow(&mut self, now: Instant) -> bool {
		let Some(start) = self.first_byte else {
			return self.slow;
		};
		if now.duration_since(start) >= SLOW_WINDOW {
			let rate = if self
				.last_change
				.is_some_and(|t| now.duration_since(t) <= SLOW_WINDOW)
			{
				self.rate(now, SLOW_WINDOW).unwrap_or(0.0)
			} else {
				0.0
			};
			if rate < SLOW_BELOW {
				self.slow = true;
			} else if rate > FAST_ABOVE {
				self.slow = false;
			}
		}
		self.slow
	}
}

#[derive(Debug)]
pub enum FetchError {
	Cancelled,
	/// Сеть или сервер (после всех повторов).
	Network(String),
	Io(io::Error),
}

/// Ограничитель частоты событий: первое и последнее отправляются всегда.
pub struct Throttle {
	last: Option<Instant>,
}

impl Throttle {
	pub fn new() -> Self {
		Self { last: None }
	}

	pub fn ready(&mut self, now: Instant, force: bool) -> bool {
		let due = force
			|| self
				.last
				.is_none_or(|t| now.duration_since(t) >= PROGRESS_EVERY);
		if due {
			self.last = Some(now);
		}
		due
	}
}

/// Повторять ли после этой ошибки: обрыв, тайм-аут, 5xx, 408, 429.
fn retryable(status: Option<StatusCode>) -> bool {
	match status {
		None => true,
		Some(s) => {
			s.is_server_error()
				|| s == StatusCode::REQUEST_TIMEOUT
				|| s == StatusCode::TOO_MANY_REQUESTS
		}
	}
}

/// Чем закончилось скачивание.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fetched {
	/// Часть файла взята с прошлых попыток (докачка или ответ 416). Если подпись такого файла
	/// не сошлась, его стоит скачать заново целиком.
	pub resumed: bool,
}

/// Общее для всех попыток одного скачивания.
struct Run {
	meter: Meter,
	throttle: Throttle,
	reconnect: Option<Reconnect>,
	total: Option<u64>,
	resumed: bool,
	/// В этой попытке пришли байты.
	progressed: bool,
}

/// Качает `url` в `part`, дописывая то, что уже есть. Возвращается, когда файл докачан.
pub async fn fetch(
	client: &reqwest::Client,
	url: &str,
	part: &Path,
	token: &CancellationToken,
	pauses: &[Duration],
	mut emit: impl FnMut(UpdateProgress),
) -> Result<Fetched, FetchError> {
	let mut run = Run {
		meter: Meter::new(),
		throttle: Throttle::new(),
		reconnect: None,
		total: None,
		resumed: false,
		progressed: false,
	};
	let mut attempt = 0u32;
	loop {
		run.progressed = false;
		match fetch_once(client, url, part, token, &mut run, &mut emit).await {
			Ok(()) => {
				return Ok(Fetched {
					resumed: run.resumed,
				});
			}
			Err(Attempt::Fatal(err)) => return Err(err),
			// Сервер прислал не тот кусок: начинаем файл заново, это не обрыв.
			Err(Attempt::Restart) => {
				remove(part).await?;
				run.resumed = false;
				continue;
			}
			Err(Attempt::Retry(status, text)) => {
				// Байты шли — считаем повторы заново.
				if run.progressed {
					attempt = 0;
				}
				if !retryable(status) || attempt as usize >= pauses.len() {
					return Err(FetchError::Network(text));
				}
				log::warn!(
					"обновление: связь прервалась ({text}), повтор {}",
					attempt + 1
				);
			}
		}
		let pause = pauses[attempt as usize];
		attempt += 1;
		run.reconnect = Some(Reconnect {
			attempt,
			of: pauses.len() as u32,
		});
		emit(UpdateProgress {
			downloaded: file_len(part).await,
			total: run.total,
			speed_bps: None,
			eta_secs: None,
			slow: run.meter.slow,
			reconnect: run.reconnect,
		});
		tokio::select! {
			_ = tokio::time::sleep(pause) => {}
			_ = token.cancelled() => return Err(FetchError::Cancelled),
		}
	}
}

enum Attempt {
	Retry(Option<StatusCode>, String),
	Restart,
	Fatal(FetchError),
}

async fn file_len(path: &Path) -> u64 {
	tokio::fs::metadata(path)
		.await
		.map(|m| m.len())
		.unwrap_or(0)
}

async fn remove(path: &Path) -> Result<(), FetchError> {
	match tokio::fs::remove_file(path).await {
		Err(e) if e.kind() != io::ErrorKind::NotFound => Err(FetchError::Io(e)),
		_ => Ok(()),
	}
}

/// Начало и полный размер из `Content-Range: bytes START-END/TOTAL`.
fn content_range(value: &str) -> Option<(u64, Option<u64>)> {
	let rest = value.trim().strip_prefix("bytes ")?;
	let (range, total) = rest.split_once('/')?;
	let (start, _) = range.split_once('-')?;
	Some((start.trim().parse().ok()?, total.trim().parse().ok()))
}

async fn fetch_once(
	client: &reqwest::Client,
	url: &str,
	part: &Path,
	token: &CancellationToken,
	run: &mut Run,
	emit: &mut impl FnMut(UpdateProgress),
) -> Result<(), Attempt> {
	let offset = file_len(part).await;
	let mut request = client.get(url);
	if offset > 0 {
		request = request.header(RANGE, format!("bytes={offset}-"));
	}
	let send = tokio::select! {
		r = request.send() => r,
		_ = token.cancelled() => return Err(Attempt::Fatal(FetchError::Cancelled)),
	};
	let mut response = send.map_err(|e| Attempt::Retry(e.status(), e.without_url().to_string()))?;
	let status = response.status();
	if status == StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
		// Похоже, файл уже целиком у нас. Если нет, это покажет проверка подписи.
		run.resumed = true;
		return Ok(());
	}
	if !status.is_success() {
		return Err(Attempt::Retry(Some(status), format!("HTTP {status}")));
	}
	let header = |name| {
		response
			.headers()
			.get(name)
			.and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
			.map(str::to_owned)
	};
	let length = header(CONTENT_LENGTH).and_then(|v| v.parse::<u64>().ok());
	let append = offset > 0 && status == StatusCode::PARTIAL_CONTENT;
	let (start, total) = if append {
		match header(CONTENT_RANGE).as_deref().and_then(content_range) {
			Some((start, total)) if start == offset => {
				(offset, total.or(length.map(|l| l + offset)))
			}
			_ => {
				log::warn!("обновление: сервер прислал не тот кусок файла, качаем заново");
				return Err(Attempt::Restart);
			}
		}
	} else {
		(0, length)
	};
	if append {
		run.resumed = true;
	}
	if total.is_some() {
		run.total = total;
	}
	let mut file = tokio::fs::OpenOptions::new()
		.create(true)
		.write(true)
		.append(append)
		.truncate(!append)
		.open(part)
		.await
		.map_err(|e| Attempt::Fatal(FetchError::Io(e)))?;
	let mut downloaded = start;
	let mut tick = tokio::time::interval(TICK);
	tick.tick().await;
	loop {
		let chunk = tokio::select! {
			c = response.chunk() => c,
			_ = tick.tick() => {
				let now = Instant::now();
				emit(progress(&mut run.meter, now, downloaded, total, run.reconnect));
				continue;
			}
			_ = token.cancelled() => {
				let _ = file.flush().await;
				return Err(Attempt::Fatal(FetchError::Cancelled));
			}
		};
		match chunk {
			Ok(Some(bytes)) => {
				file.write_all(&bytes)
					.await
					.map_err(|e| Attempt::Fatal(FetchError::Io(e)))?;
				downloaded += bytes.len() as u64;
				let now = Instant::now();
				run.meter.add(now, bytes.len() as u64);
				run.progressed = true;
				run.reconnect = None;
				let last = total.is_some_and(|t| downloaded >= t);
				if run.throttle.ready(now, last) {
					emit(progress(&mut run.meter, now, downloaded, total, None));
				}
			}
			Ok(None) => break,
			Err(e) => {
				let _ = file.flush().await;
				return Err(Attempt::Retry(None, e.without_url().to_string()));
			}
		}
	}
	file.flush()
		.await
		.map_err(|e| Attempt::Fatal(FetchError::Io(e)))?;
	if let Some(t) = total
		&& downloaded < t
	{
		return Err(Attempt::Retry(
			None,
			format!("оборвалось на {downloaded} из {t}"),
		));
	}
	emit(progress(
		&mut run.meter,
		Instant::now(),
		downloaded,
		total.or(Some(downloaded)),
		None,
	));
	Ok(())
}

fn progress(
	meter: &mut Meter,
	now: Instant,
	downloaded: u64,
	total: Option<u64>,
	reconnect: Option<Reconnect>,
) -> UpdateProgress {
	UpdateProgress {
		downloaded,
		total,
		speed_bps: meter.speed(now),
		eta_secs: meter.eta(now, downloaded, total),
		slow: meter.slow(now),
		reconnect,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn throttle_limits_events() {
		let start = Instant::now();
		let mut t = Throttle::new();
		assert!(t.ready(start, false));
		assert!(!t.ready(start + Duration::from_millis(50), false));
		assert!(t.ready(start + Duration::from_millis(60), true));
		assert!(!t.ready(start + Duration::from_millis(100), false));
		assert!(t.ready(start + Duration::from_millis(160), false));
	}

	#[test]
	fn meter_speed_eta_slow() {
		let t0 = Instant::now();
		let mut m = Meter::new();
		assert_eq!(m.speed(t0), None);
		// 100 КБ/с ровно 12 секунд.
		for i in 1..=12 {
			m.add(t0 + Duration::from_secs(i), 100 * 1024);
		}
		let now = t0 + Duration::from_secs(12);
		let speed = m.speed(now).unwrap();
		assert!(
			(100 * 1024 - 10..=100 * 1024 + 10).contains(&speed),
			"{speed}"
		);
		assert_eq!(m.eta(now, 0, Some(1024 * 1024)), Some(11));
		assert!(m.slow(now));
		// Разогнались до 1 МБ/с — через 10 с «медленно» снимается.
		for i in 13..=24 {
			m.add(t0 + Duration::from_secs(i), 1024 * 1024);
		}
		assert!(!m.slow(t0 + Duration::from_secs(24)));
		// Тишина больше 5 с — скорость неизвестна, ETA тоже.
		let quiet = t0 + Duration::from_secs(31);
		assert_eq!(m.speed(quiet), None);
		assert_eq!(m.eta(quiet, 0, Some(10)), None);
	}

	#[test]
	fn eta_waits_three_seconds() {
		let t0 = Instant::now();
		let mut m = Meter::new();
		m.add(t0, 1024);
		m.add(t0 + Duration::from_secs(1), 1024);
		assert_eq!(m.eta(t0 + Duration::from_secs(1), 0, Some(10_000)), None);
	}

	#[test]
	fn retry_rules() {
		assert!(retryable(None));
		assert!(retryable(Some(StatusCode::BAD_GATEWAY)));
		assert!(retryable(Some(StatusCode::TOO_MANY_REQUESTS)));
		assert!(!retryable(Some(StatusCode::NOT_FOUND)));
		assert!(!retryable(Some(StatusCode::FORBIDDEN)));
	}

	#[test]
	fn parses_content_range() {
		assert_eq!(content_range("bytes 100-199/200"), Some((100, Some(200))));
		assert_eq!(content_range("bytes 5-9/*"), Some((5, None)));
		assert_eq!(content_range("items 1-2/3"), None);
	}

	type Respond = Box<dyn Fn(usize, &str, &[u8]) -> Vec<u8> + Send>;

	/// Локальный сервер: на каждый запрос `respond(номер, запрос, тело)` отдаёт сырые байты ответа.
	async fn serve(body: Vec<u8>, count: usize, respond: Respond) -> String {
		use tokio::io::{AsyncReadExt, AsyncWriteExt};
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		tokio::spawn(async move {
			for n in 0..count {
				let (mut sock, _) = listener.accept().await.unwrap();
				let mut buf = vec![0u8; 4096];
				let len = sock.read(&mut buf).await.unwrap();
				let req = String::from_utf8_lossy(&buf[..len]).to_lowercase();
				sock.write_all(&respond(n, &req, &body)).await.unwrap();
				let _ = sock.shutdown().await;
			}
		});
		format!("http://{addr}/u.exe")
	}

	fn ok(body: &[u8]) -> Vec<u8> {
		let mut out = format!(
			"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
			body.len()
		)
		.into_bytes();
		out.extend_from_slice(body);
		out
	}

	fn partial(body: &[u8], from: usize, claimed: usize) -> Vec<u8> {
		let tail = &body[from..];
		let mut out = format!(
			"HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {claimed}-{}/{}\r\nConnection: close\r\n\r\n",
			tail.len(),
			body.len() - 1,
			body.len()
		)
		.into_bytes();
		out.extend_from_slice(tail);
		out
	}

	fn sample() -> Vec<u8> {
		(0..200_000u32).map(|i| (i % 251) as u8).collect()
	}

	async fn run_fetch(
		url: &str,
		part: &Path,
	) -> (Result<Fetched, FetchError>, Vec<UpdateProgress>) {
		let client = reqwest::Client::new();
		let mut events = Vec::new();
		let token = CancellationToken::new();
		let pauses = [Duration::from_millis(10); 3];
		let fut = fetch(&client, url, part, &token, &pauses, |p| events.push(p));
		let result = tokio::time::timeout(Duration::from_secs(30), fut)
			.await
			.expect("скачивание зависло");
		(result, events)
	}

	/// Первый ответ обрывается на середине, второй отдаёт хвост по `Range`.
	#[tokio::test]
	async fn resumes_with_range() {
		let body = sample();
		let url = serve(
			body.clone(),
			2,
			Box::new(|n, req, body| {
				if n == 0 {
					assert!(!req.contains("range:"));
					let mut out = ok(body);
					out.truncate(out.len() - 120_000);
					out
				} else {
					assert!(req.contains("range: bytes=80000-"), "{req}");
					partial(body, 80_000, 80_000)
				}
			}),
		)
		.await;
		let dir = tempfile::tempdir().unwrap();
		let part = dir.path().join("update.part");
		let (result, events) = run_fetch(&url, &part).await;
		assert_eq!(result.unwrap(), Fetched { resumed: true });
		assert_eq!(std::fs::read(&part).unwrap(), body);
		// Полоса при повторе не обнуляется: известны и скачанное, и полный размер.
		let reconnect = events.iter().find(|e| e.reconnect.is_some()).unwrap();
		assert_eq!(reconnect.downloaded, 80_000);
		assert_eq!(reconnect.total, Some(body.len() as u64));
		assert_eq!(events.last().unwrap().downloaded, body.len() as u64);
	}

	/// Сервер не умеет `Range` и отдаёт файл целиком: начало файла переписывается.
	#[tokio::test]
	async fn full_answer_instead_of_partial() {
		let body = sample();
		let url = serve(
			body.clone(),
			1,
			Box::new(|_, req, body| {
				assert!(req.contains("range: bytes=1000-"), "{req}");
				ok(body)
			}),
		)
		.await;
		let dir = tempfile::tempdir().unwrap();
		let part = dir.path().join("update.part");
		std::fs::write(&part, vec![9u8; 1000]).unwrap();
		let (result, _) = run_fetch(&url, &part).await;
		assert_eq!(result.unwrap(), Fetched { resumed: false });
		assert_eq!(std::fs::read(&part).unwrap(), body);
	}

	/// 416 на докачку: файл считается готовым, но помечается докачанным.
	#[tokio::test]
	async fn range_not_satisfiable() {
		let url = serve(
			Vec::new(),
			1,
			Box::new(|_, _, _| {
				b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
					.to_vec()
			}),
		)
		.await;
		let dir = tempfile::tempdir().unwrap();
		let part = dir.path().join("update.part");
		std::fs::write(&part, b"old").unwrap();
		let (result, _) = run_fetch(&url, &part).await;
		assert_eq!(result.unwrap(), Fetched { resumed: true });
	}

	/// 206 с другим началом: кусок не дописывается, файл качается заново.
	#[tokio::test]
	async fn wrong_content_range_restarts() {
		let body = sample();
		let url = serve(
			body.clone(),
			2,
			Box::new(|n, req, body| {
				if n == 0 {
					assert!(req.contains("range: bytes=1000-"), "{req}");
					partial(body, 0, 0)
				} else {
					assert!(!req.contains("range:"), "{req}");
					ok(body)
				}
			}),
		)
		.await;
		let dir = tempfile::tempdir().unwrap();
		let part = dir.path().join("update.part");
		std::fs::write(&part, vec![9u8; 1000]).unwrap();
		let (result, _) = run_fetch(&url, &part).await;
		assert_eq!(result.unwrap(), Fetched { resumed: false });
		assert_eq!(std::fs::read(&part).unwrap(), body);
	}

	/// Байты снова пошли — счётчик повторов начинается заново: четыре обрыва подряд с
	/// продвижением не исчерпывают три повтора.
	#[tokio::test]
	async fn retries_reset_after_progress() {
		let body = sample();
		let url = serve(
			body.clone(),
			5,
			Box::new(|n, _, body| {
				let from = n * 40_000;
				let mut out = if n == 0 {
					ok(body)
				} else {
					partial(body, from, from)
				};
				if n < 4 {
					let keep = out.len() - (body.len() - from - 40_000);
					out.truncate(keep);
				}
				out
			}),
		)
		.await;
		let dir = tempfile::tempdir().unwrap();
		let part = dir.path().join("update.part");
		let (result, _) = run_fetch(&url, &part).await;
		assert!(result.is_ok(), "{result:?}");
		assert_eq!(std::fs::read(&part).unwrap(), body);
	}
}
