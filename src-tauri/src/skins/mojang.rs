//! Запросы к Mojang: профиль, смена скина и плаща. Тело успешного ответа — новый профиль.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, HeaderMap, RETRY_AFTER};
use reqwest::multipart::{Form, Part};
use reqwest::{Client, Response, StatusCode};

use super::SkinModel;
use super::profile::{ParsedProfile, parse_profile};
use crate::error::{AppError, ErrorCode};

pub const API_BASE: &str = "https://api.minecraftservices.com";
const PROFILE_PATH: &str = "/minecraft/profile";
const SKINS_PATH: &str = "/minecraft/profile/skins";
const ACTIVE_SKIN_PATH: &str = "/minecraft/profile/skins/active";
const ACTIVE_CAPE_PATH: &str = "/minecraft/profile/capes/active";

/// Запас ядра: точного лимита на смену скина в источниках нет, а частые 429 грозят блокировкой аккаунта.
pub const MIN_INTERVAL: Duration = Duration::from_secs(10);
pub const PAUSE_AFTER_429: Duration = Duration::from_secs(120);
const MAX_PAUSE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_ERROR_BODY: usize = 600;
const MAX_PROFILE_BODY: usize = 256 * 1024;
const PROFILE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn build_client() -> reqwest::Result<Client> {
	Client::builder()
		.user_agent(crate::http::USER_AGENT)
		.connect_timeout(Duration::from_secs(15))
		.timeout(Duration::from_secs(60))
		.redirect(reqwest::redirect::Policy::none())
		.build()
}

#[derive(Debug, Clone, Copy)]
pub enum Call<'a> {
	GetProfile,
	Upload { png: &'a [u8], model: SkinModel },
	ResetSkin,
	ShowCape(&'a str),
	HideCape,
}

/// Сбой запроса. `delivered` — сервер его видел (или мог видеть), и он считается отправленным.
#[derive(Debug)]
pub struct Failed {
	pub error: AppError,
	pub delivered: bool,
}

pub struct Mojang {
	http: Client,
	base: String,
	limiter: Limiter,
}

impl Mojang {
	pub fn new(http: Client, base: impl Into<String>, limiter: Limiter) -> Self {
		Self {
			http,
			base: base.into(),
			limiter,
		}
	}

	/// Одно действие изменения на аккаунт за раз и не чаще `MIN_INTERVAL`.
	pub fn begin(&self, account_id: &str) -> Result<Permit<'_>, AppError> {
		self.limiter.begin(account_id, Instant::now())
	}

	pub fn pause_left(&self) -> Option<Duration> {
		self.limiter.lock().pause_left(Instant::now())
	}

	async fn send(&self, call: Call<'_>, token: &str) -> reqwest::Result<Response> {
		let url = |path: &str| format!("{}{path}", self.base);
		let request = match call {
			Call::GetProfile => self.http.get(url(PROFILE_PATH)).timeout(PROFILE_TIMEOUT),
			Call::Upload { png, model } => {
				let file = Part::bytes(png.to_vec())
					.file_name("skin.png")
					.mime_str("image/png")?;
				let form = Form::new()
					.text("variant", model.as_str())
					.part("file", file);
				self.http.post(url(SKINS_PATH)).multipart(form)
			}
			Call::ResetSkin => self.http.delete(url(ACTIVE_SKIN_PATH)),
			Call::ShowCape(id) => self
				.http
				.put(url(ACTIVE_CAPE_PATH))
				.json(&serde_json::json!({ "capeId": id })),
			Call::HideCape => self.http.delete(url(ACTIVE_CAPE_PATH)),
		};
		request
			.header(ACCEPT, "application/json")
			.bearer_auth(token)
			.send()
			.await
	}

	/// Один запрос, на 401 — один повтор с новым токеном. `None` — тело ответа не разобралось.
	pub async fn call<F, Fut>(
		&self,
		permit: Option<&Permit<'_>>,
		call: Call<'_>,
		token: &F,
	) -> Result<Option<ParsedProfile>, Failed>
	where
		F: Fn(bool) -> Fut,
		Fut: Future<Output = Result<String, AppError>>,
	{
		let mut delivered = false;
		let mut current = self
			.limiter
			.check_token(token(false).await)
			.map_err(|error| Failed { error, delivered })?;
		let mut refreshed = false;
		loop {
			// Пока ждали токен, другой запрос мог получить 429.
			let now = Instant::now();
			let ready = match permit {
				Some(permit) => permit.before_send(now),
				None => match self.limiter.lock().pause_left(now) {
					Some(wait) => Err(too_often(wait)),
					None => Ok(()),
				},
			};
			ready.map_err(|error| Failed { error, delivered })?;
			let response = match self.send(call, &current).await {
				Ok(response) => response,
				Err(err) => {
					// До сервера запрос не дошёл: «Повторить» не должно упираться в паузу.
					let connect = err.is_connect();
					if let (true, Some(permit)) = (connect, permit) {
						permit.not_delivered();
					}
					return Err(Failed {
						error: network_error(err),
						delivered: delivered || !connect,
					});
				}
			};
			delivered = true;
			let status = response.status();
			if status.is_success() {
				return Ok(read_profile(response).await);
			}
			if status == StatusCode::UNAUTHORIZED {
				if refreshed {
					return Err(Failed {
						error: AppError::coded(ErrorCode::NeedsRelogin, "HTTP 401"),
						delivered,
					});
				}
				refreshed = true;
				log::info!("скины: токен отвергнут, обновляем");
				current = self
					.limiter
					.check_token(token(true).await)
					.map_err(|error| Failed { error, delivered })?;
				continue;
			}
			if status == StatusCode::TOO_MANY_REQUESTS {
				let retry_after = retry_after(response.headers(), Utc::now());
				let pause = pause_after_429(retry_after, self.limiter.pause);
				return Err(Failed {
					error: self.limiter.rate_limited(pause),
					delivered,
				});
			}
			log::warn!("скины: HTTP {}", status.as_u16());
			return Err(Failed {
				error: error_for_status(response, &current).await,
				delivered,
			});
		}
	}
}

/// Не больше 256 КБ. В лог тело не пишется.
async fn read_profile(mut response: Response) -> Option<ParsedProfile> {
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await.ok()? {
		bytes.extend_from_slice(&chunk);
		if bytes.len() > MAX_PROFILE_BODY {
			return None;
		}
	}
	let parsed = parse_profile(&bytes);
	if parsed.is_none() {
		log::warn!("скины: ответ сервера не разобрался как профиль");
	}
	parsed
}

fn network_error(err: reqwest::Error) -> AppError {
	AppError::coded(ErrorCode::Network, err.without_url().to_string())
}

async fn error_for_status(response: Response, token: &str) -> AppError {
	let status = response.status();
	match status.as_u16() {
		400 => AppError::coded(ErrorCode::SkinRejected, details(response, token).await),
		404 => AppError::new(ErrorCode::NoMinecraft),
		500..=599 => AppError::coded(
			ErrorCode::ServiceUnavailable,
			format!("HTTP {}", status.as_u16()),
		),
		_ => AppError::coded(ErrorCode::Unknown, details(response, token).await),
	}
}

/// HTTP-код и начало тела ответа с ошибкой, без токена.
async fn details(mut response: Response, token: &str) -> String {
	let status = response.status().as_u16();
	let mut bytes = Vec::new();
	while bytes.len() < MAX_ERROR_BODY {
		match response.chunk().await {
			Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
			_ => break,
		}
	}
	let text = String::from_utf8_lossy(&bytes).replace(token, "***");
	let mut body = text.trim().to_owned();
	if body.len() > MAX_ERROR_BODY {
		body.truncate(body.floor_char_boundary(MAX_ERROR_BODY));
		body.push('…');
	}
	if body.is_empty() {
		format!("HTTP {status}")
	} else {
		format!("HTTP {status}: {body}")
	}
}

fn retry_after(headers: &HeaderMap, now: DateTime<Utc>) -> Option<Duration> {
	let text = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
	if let Ok(seconds) = text.parse::<u64>() {
		return Some(Duration::from_secs(seconds));
	}
	let at = DateTime::parse_from_rfc2822(text).ok()?;
	Some((at.with_timezone(&Utc) - now).to_std().unwrap_or_default())
}

fn pause_after_429(retry_after: Option<Duration>, minimum: Duration) -> Duration {
	retry_after.unwrap_or_default().max(minimum).min(MAX_PAUSE)
}

/// На аккаунт один запрос за раз и не чаще `interval`; после 429 пауза для всех аккаунтов:
/// общий лимит Mojang считается по адресу компьютера. Только в памяти.
pub struct Limiter {
	interval: Duration,
	pause: Duration,
	state: Mutex<LimitState>,
}

#[derive(Default)]
struct LimitState {
	accounts: HashMap<String, Slot>,
	paused_until: Option<Instant>,
}

#[derive(Default, Clone, Copy)]
struct Slot {
	busy: bool,
	last_sent: Option<Instant>,
	/// Значение до последней отправки: вернуть, если она не дошла до сервера.
	previous: Option<Instant>,
}

fn whole_seconds(wait: Duration) -> u64 {
	wait.as_secs() + u64::from(wait.subsec_nanos() > 0)
}

/// `skin_too_often`: в `details` — сколько секунд ждать.
pub fn too_often(wait: Duration) -> AppError {
	AppError::coded(
		ErrorCode::SkinTooOften,
		whole_seconds(wait).max(1).to_string(),
	)
}

impl Default for Limiter {
	fn default() -> Self {
		Self::new(MIN_INTERVAL, PAUSE_AFTER_429)
	}
}

impl Limiter {
	pub fn new(interval: Duration, pause: Duration) -> Self {
		Self {
			interval,
			pause,
			state: Mutex::default(),
		}
	}

	fn lock(&self) -> std::sync::MutexGuard<'_, LimitState> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn begin<'a>(&'a self, account_id: &str, now: Instant) -> Result<Permit<'a>, AppError> {
		let mut state = self.lock();
		if let Some(wait) = state.pause_left(now) {
			return Err(too_often(wait));
		}
		let slot = state.accounts.entry(account_id.to_owned()).or_default();
		if slot.busy {
			return Err(too_often(self.interval));
		}
		if let Some(last) = slot.last_sent {
			let wait = self
				.interval
				.saturating_sub(now.saturating_duration_since(last));
			if !wait.is_zero() {
				return Err(too_often(wait));
			}
		}
		slot.busy = true;
		Ok(Permit {
			limiter: self,
			account_id: account_id.to_owned(),
		})
	}

	/// Новая пауза не сокращает идущую. Возвращает, сколько теперь осталось ждать.
	fn pause_all(&self, now: Instant, pause: Duration) -> Duration {
		let mut state = self.lock();
		let until = state
			.paused_until
			.map_or(now + pause, |until| until.max(now + pause));
		state.paused_until = Some(until);
		until.saturating_duration_since(now)
	}

	/// 429: пауза и `rate_limited`, в `details` — сколько на самом деле ждать, в секундах.
	fn rate_limited(&self, pause: Duration) -> AppError {
		let wait = self.pause_all(Instant::now(), pause);
		log::warn!("скины: 429, пауза {} с", wait.as_secs());
		AppError::coded(ErrorCode::RateLimited, whole_seconds(wait).to_string())
	}

	/// 429 при обновлении токена — такой же сигнал сбавить темп, как 429 на сам запрос.
	fn check_token(&self, token: Result<String, AppError>) -> Result<String, AppError> {
		token.map_err(|err| {
			if err.code == ErrorCode::RateLimited {
				self.rate_limited(self.pause)
			} else {
				err
			}
		})
	}
}

impl LimitState {
	fn pause_left(&self, now: Instant) -> Option<Duration> {
		self.paused_until
			.map(|until| until.saturating_duration_since(now))
			.filter(|wait| !wait.is_zero())
	}
}

pub struct Permit<'a> {
	limiter: &'a Limiter,
	account_id: String,
}

impl Permit<'_> {
	fn with_slot(&self, change: impl FnOnce(&mut Slot)) {
		if let Some(slot) = self.limiter.lock().accounts.get_mut(&self.account_id) {
			change(slot);
		}
	}

	/// Проверка паузы и отметка времени одним шагом, перед каждой отправкой.
	fn before_send(&self, now: Instant) -> Result<(), AppError> {
		let mut state = self.limiter.lock();
		if let Some(wait) = state.pause_left(now) {
			return Err(too_often(wait));
		}
		if let Some(slot) = state.accounts.get_mut(&self.account_id) {
			slot.previous = slot.last_sent;
			slot.last_sent = Some(now);
		}
		Ok(())
	}

	/// Отменяет только последнюю отправку: если прошлая в этой попытке дошла, пауза от неё остаётся.
	fn not_delivered(&self) {
		self.with_slot(|slot| slot.last_sent = slot.previous);
	}
}

impl Drop for Permit<'_> {
	fn drop(&mut self) {
		self.with_slot(|slot| slot.busy = false);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn seconds_of(err: &AppError) -> u64 {
		assert_eq!(err.code, ErrorCode::SkinTooOften);
		err.details.as_deref().unwrap().parse().unwrap()
	}

	#[test]
	fn limiter_interval_busy_and_accounts() {
		let limiter = Limiter::default();
		let start = Instant::now();

		let first = limiter.begin("a", start).unwrap();
		// Пока запрос идёт, второй на этот аккаунт не уходит, на другой — уходит.
		assert_eq!(seconds_of(&limiter.begin("a", start).err().unwrap()), 10);
		let other = limiter.begin("b", start).unwrap();
		drop(other);
		first.before_send(start).unwrap();
		drop(first);

		let at = start + Duration::from_millis(3500);
		assert_eq!(seconds_of(&limiter.begin("a", at).err().unwrap()), 7);
		let later = start + MIN_INTERVAL;
		assert!(limiter.begin("a", later).is_ok());
	}

	#[test]
	fn limiter_without_sending_or_delivery() {
		let limiter = Limiter::default();
		let start = Instant::now();
		// Токен не получили, запрос не ушёл: пауза не начинается.
		drop(limiter.begin("a", start).unwrap());
		let permit = limiter.begin("a", start).unwrap();
		permit.before_send(start).unwrap();
		// Соединения не было: прежнее время возвращается.
		permit.not_delivered();
		drop(permit);
		assert!(limiter.begin("a", start).is_ok());
	}

	#[test]
	fn lost_retry_keeps_pause_of_delivered_request() {
		let limiter = Limiter::default();
		let start = Instant::now();
		let permit = limiter.begin("a", start).unwrap();
		permit.before_send(start).unwrap();
		let retry = start + Duration::from_secs(4);
		permit.before_send(retry).unwrap();
		permit.not_delivered();
		drop(permit);
		// Первый запрос дошёл: 10 с считаются от него.
		assert_eq!(seconds_of(&limiter.begin("a", retry).err().unwrap()), 6);
	}

	#[test]
	fn pause_blocks_sending_after_begin() {
		let limiter = Limiter::default();
		let start = Instant::now();
		let permit = limiter.begin("a", start).unwrap();
		limiter.pause_all(start, PAUSE_AFTER_429);
		assert_eq!(seconds_of(&permit.before_send(start).unwrap_err()), 120);
	}

	#[test]
	fn shorter_pause_does_not_cut_longer() {
		let limiter = Limiter::default();
		let start = Instant::now();
		let long = Duration::from_secs(3600);
		assert_eq!(limiter.pause_all(start, long), long);
		assert_eq!(limiter.pause_all(start, PAUSE_AFTER_429), long);
		assert_eq!(seconds_of(&limiter.begin("b", start).err().unwrap()), 3600);
		// Длиннее — продлевает.
		let longer = Duration::from_secs(7200);
		assert_eq!(limiter.pause_all(start, longer), longer);
	}

	#[test]
	fn pause_is_for_all_accounts() {
		let limiter = Limiter::default();
		let start = Instant::now();
		limiter.pause_all(start, PAUSE_AFTER_429);
		assert_eq!(seconds_of(&limiter.begin("a", start).err().unwrap()), 120);
		let at = start + Duration::from_secs(100);
		assert_eq!(seconds_of(&limiter.begin("b", at).err().unwrap()), 20);
		assert!(limiter.begin("b", start + PAUSE_AFTER_429).is_ok());
	}

	#[test]
	fn pause_length() {
		let min = PAUSE_AFTER_429;
		assert_eq!(pause_after_429(None, min), min);
		assert_eq!(pause_after_429(Some(Duration::from_secs(30)), min), min);
		assert_eq!(
			pause_after_429(Some(Duration::from_secs(300)), min),
			Duration::from_secs(300)
		);
		assert_eq!(pause_after_429(Some(Duration::MAX), min), MAX_PAUSE);
	}

	#[test]
	fn parses_retry_after() {
		let now = DateTime::parse_from_rfc3339("2026-11-04T10:00:00Z")
			.unwrap()
			.with_timezone(&Utc);
		let parse = |value: &str| {
			let mut headers = HeaderMap::new();
			headers.insert(RETRY_AFTER, value.parse().unwrap());
			retry_after(&headers, now)
		};
		assert_eq!(parse("45"), Some(Duration::from_secs(45)));
		assert_eq!(
			parse("Wed, 04 Nov 2026 10:01:30 GMT"),
			Some(Duration::from_secs(90))
		);
		assert_eq!(parse("Wed, 04 Nov 2026 09:00:00 GMT"), Some(Duration::ZERO));
		assert_eq!(parse("soon"), None);
		assert_eq!(retry_after(&HeaderMap::new(), now), None);
	}
}
