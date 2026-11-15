//! Запросы входа Microsoft → Xbox Live → Minecraft (схема официального лаунчера: device token + SISU).
//! Перенесено из Modrinth App (`theseus::state::minecraft_auth`, GPL-3.0).

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use base64::Engine;
use base64::prelude::{BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use rand::RngCore;
use reqwest::header::HeaderMap;
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::error::{AuthError, Step};

pub const MICROSOFT_CLIENT_ID: &str = "00000000402b5328";
pub const AUTH_REPLY_URL: &str = "https://login.live.com/oauth20_desktop.srf";
const REQUESTED_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
const TITLE_ID: &str = "1794566092";

/// Ключ устройства Xbox (P-256) — им подписываются запросы к Xbox Live.
#[derive(Clone)]
pub struct DeviceKey {
	pub id: Uuid,
	pub key: SigningKey,
	pub x: String,
	pub y: String,
}

impl DeviceKey {
	pub fn generate() -> Result<Self, AuthError> {
		Self::from_signing_key(Uuid::new_v4(), SigningKey::random(&mut rand::rngs::OsRng))
	}

	pub fn from_signing_key(id: Uuid, key: SigningKey) -> Result<Self, AuthError> {
		let point = VerifyingKey::from(&key).to_encoded_point(false);
		let coord = |c: Option<&p256::FieldBytes>| {
			c.map(|c| BASE64_URL_SAFE_NO_PAD.encode(c))
				.ok_or_else(|| AuthError::unexpected(Step::DeviceToken, "нет координаты ключа"))
		};
		Ok(Self {
			id,
			x: coord(point.x())?,
			y: coord(point.y())?,
			key,
		})
	}

	fn proof_key(&self) -> serde_json::Value {
		json!({
			"kty": "EC",
			"x": self.x,
			"y": self.y,
			"crv": "P-256",
			"alg": "ES256",
			"use": "sig"
		})
	}
}

impl std::fmt::Debug for DeviceKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DeviceKey")
			.field("id", &self.id)
			.finish_non_exhaustive()
	}
}

/// Токен Xbox (device, user, title, XSTS).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct XboxToken {
	pub issue_instant: DateTime<Utc>,
	pub not_after: DateTime<Utc>,
	pub token: String,
	#[serde(default)]
	pub display_claims: HashMap<String, serde_json::Value>,
}

impl std::fmt::Debug for XboxToken {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("XboxToken")
			.field("not_after", &self.not_after)
			.finish_non_exhaustive()
	}
}

/// Ответ с временем сервера: по нему подписывается следующий запрос, если часы сбиты.
pub struct Dated<T> {
	pub date: DateTime<Utc>,
	pub value: T,
}

#[derive(Deserialize)]
pub struct OAuthToken {
	pub access_token: String,
	pub refresh_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SisuAuthorize {
	pub title_token: XboxToken,
	pub user_token: XboxToken,
}

#[derive(Deserialize)]
pub struct MinecraftToken {
	pub access_token: String,
	pub expires_in: i64,
}

#[derive(Deserialize)]
pub struct MinecraftProfile {
	pub id: Uuid,
	pub name: String,
}

pub struct LoginStart {
	pub verifier: String,
	pub session_id: String,
	pub auth_url: String,
}

pub fn random_hex(bytes: usize) -> String {
	let mut buf = vec![0u8; bytes];
	rand::rngs::OsRng.fill_bytes(&mut buf);
	buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn device_token(
	http: &Client,
	key: &DeviceKey,
	date: DateTime<Utc>,
) -> Result<Dated<XboxToken>, AuthError> {
	let res = signed_request(
		http,
		"https://device.auth.xboxlive.com/device/authenticate",
		"/device/authenticate",
		json!({
			"Properties": {
				"AuthMethod": "ProofOfPossession",
				"Id": format!("{{{}}}", key.id.to_string().to_uppercase()),
				"DeviceType": "Win32",
				"Version": "10.16.0",
				"ProofKey": key.proof_key(),
			},
			"RelyingParty": "http://auth.xboxlive.com",
			"TokenType": "JWT"
		}),
		key,
		Step::DeviceToken,
		date,
	)
	.await?;
	Ok(Dated {
		date: res.date,
		value: res.body,
	})
}

/// Начало входа: PKCE и SISU-сессия. Возвращает адрес страницы входа Microsoft.
pub async fn sisu_authenticate(
	http: &Client,
	device_token: &str,
	key: &DeviceKey,
	date: DateTime<Utc>,
) -> Result<LoginStart, AuthError> {
	#[derive(Deserialize)]
	#[serde(rename_all = "PascalCase")]
	struct RedirectUri {
		msa_oauth_redirect: String,
	}

	// 64 случайных байта в hex = 128 символов, максимум для PKCE verifier.
	let verifier = random_hex(64);
	let challenge = BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(&verifier));

	let res = signed_request::<RedirectUri>(
		http,
		"https://sisu.xboxlive.com/authenticate",
		"/authenticate",
		json!({
			"AppId": MICROSOFT_CLIENT_ID,
			"DeviceToken": device_token,
			"Offers": [REQUESTED_SCOPE],
			"Query": {
				"code_challenge": challenge,
				"code_challenge_method": "S256",
				"state": random_hex(32),
				"prompt": "select_account"
			},
			"RedirectUri": AUTH_REPLY_URL,
			"Sandbox": "RETAIL",
			"TokenType": "code",
			"TitleId": TITLE_ID,
		}),
		key,
		Step::SisuAuthenticate,
		date,
	)
	.await?;

	let session_id = res
		.headers
		.get("X-SessionId")
		.and_then(|v| v.to_str().ok())
		.ok_or_else(|| AuthError::unexpected(Step::SisuAuthenticate, "нет заголовка X-SessionId"))?
		.to_owned();

	Ok(LoginStart {
		verifier,
		session_id,
		auth_url: res.body.msa_oauth_redirect,
	})
}

pub async fn oauth_token(
	http: &Client,
	code: &str,
	verifier: &str,
) -> Result<Dated<OAuthToken>, AuthError> {
	oauth_request(
		http,
		&[
			("client_id", MICROSOFT_CLIENT_ID),
			("code", code),
			("code_verifier", verifier),
			("grant_type", "authorization_code"),
			("redirect_uri", AUTH_REPLY_URL),
			("scope", REQUESTED_SCOPE),
		],
		Step::OAuthToken,
	)
	.await
}

pub async fn oauth_refresh(
	http: &Client,
	refresh_token: &str,
) -> Result<Dated<OAuthToken>, AuthError> {
	oauth_request(
		http,
		&[
			("client_id", MICROSOFT_CLIENT_ID),
			("refresh_token", refresh_token),
			("grant_type", "refresh_token"),
			("redirect_uri", AUTH_REPLY_URL),
			("scope", REQUESTED_SCOPE),
		],
		Step::OAuthRefresh,
	)
	.await
}

async fn oauth_request(
	http: &Client,
	form: &[(&str, &str)],
	step: Step,
) -> Result<Dated<OAuthToken>, AuthError> {
	let res = with_retry(step, || {
		http.post("https://login.live.com/oauth20_token.srf")
			.header("Accept", "application/json")
			.form(form)
			.send()
	})
	.await?;
	let date = server_date(res.headers());
	let body = read_json(res, step).await?;
	Ok(Dated { date, value: body })
}

pub async fn sisu_authorize(
	http: &Client,
	session_id: Option<&str>,
	access_token: &str,
	device_token: &str,
	key: &DeviceKey,
	date: DateTime<Utc>,
) -> Result<Dated<SisuAuthorize>, AuthError> {
	let res = signed_request(
		http,
		"https://sisu.xboxlive.com/authorize",
		"/authorize",
		json!({
			"AccessToken": format!("t={access_token}"),
			"AppId": MICROSOFT_CLIENT_ID,
			"DeviceToken": device_token,
			"ProofKey": key.proof_key(),
			"Sandbox": "RETAIL",
			"SessionId": session_id,
			"SiteName": "user.auth.xboxlive.com",
			"RelyingParty": "http://xboxlive.com",
			"UseModernGamertag": true
		}),
		key,
		Step::SisuAuthorize,
		date,
	)
	.await?;
	Ok(Dated {
		date: res.date,
		value: res.body,
	})
}

pub async fn xsts_authorize(
	http: &Client,
	authorize: &SisuAuthorize,
	device_token: &str,
	key: &DeviceKey,
	date: DateTime<Utc>,
) -> Result<XboxToken, AuthError> {
	let res = signed_request(
		http,
		"https://xsts.auth.xboxlive.com/xsts/authorize",
		"/xsts/authorize",
		json!({
			"RelyingParty": "rp://api.minecraftservices.com/",
			"TokenType": "JWT",
			"Properties": {
				"SandboxId": "RETAIL",
				"UserTokens": [authorize.user_token.token],
				"DeviceToken": device_token,
				"TitleToken": authorize.title_token.token,
			},
		}),
		key,
		Step::Xsts,
		date,
	)
	.await?;
	Ok(res.body)
}

pub async fn minecraft_token(http: &Client, xsts: &XboxToken) -> Result<MinecraftToken, AuthError> {
	let uhs = xsts
		.display_claims
		.get("xui")
		.and_then(|x| x.get(0))
		.and_then(|x| x.get("uhs"))
		.and_then(|x| x.as_str())
		.ok_or_else(|| {
			AuthError::unexpected(Step::MinecraftToken, "нет user hash в ответе XSTS")
		})?;
	let body = json!({
		"platform": "PC_LAUNCHER",
		"xtoken": format!("XBL3.0 x={uhs};{}", xsts.token),
	});
	let res = with_retry(Step::MinecraftToken, || {
		http.post("https://api.minecraftservices.com/launcher/login")
			.header("Accept", "application/json")
			.json(&body)
			.send()
	})
	.await?;
	read_json(res, Step::MinecraftToken).await
}

/// Есть ли у аккаунта лицензия Minecraft: Java Edition (покупка или Game Pass).
pub async fn has_java_license(http: &Client, mc_token: &str) -> Result<bool, AuthError> {
	#[derive(Deserialize)]
	struct Entitlements {
		#[serde(default)]
		items: Vec<Item>,
	}
	#[derive(Deserialize)]
	struct Item {
		name: String,
	}

	let url = format!(
		"https://api.minecraftservices.com/entitlements/license?requestId={}",
		Uuid::new_v4()
	);
	let res = with_retry(Step::Entitlements, || {
		http.get(&url)
			.header("Accept", "application/json")
			.bearer_auth(mc_token)
			.send()
	})
	.await?;
	let body: Entitlements = read_json(res, Step::Entitlements).await?;
	Ok(body
		.items
		.iter()
		.any(|i| i.name == "game_minecraft" || i.name == "product_minecraft"))
}

pub async fn minecraft_profile(
	http: &Client,
	mc_token: &str,
) -> Result<MinecraftProfile, AuthError> {
	let res = with_retry(Step::Profile, || {
		http.get("https://api.minecraftservices.com/minecraft/profile")
			.header("Accept", "application/json")
			.bearer_auth(mc_token)
			.send()
	})
	.await?;
	read_json(res, Step::Profile).await
}

/// Повторяет запрос при ошибке соединения или тайм-ауте (до 5 попыток).
async fn with_retry<F, Fut>(step: Step, request: F) -> Result<Response, AuthError>
where
	F: Fn() -> Fut,
	Fut: Future<Output = reqwest::Result<Response>>,
{
	const ATTEMPTS: u32 = 5;
	let mut attempt = 1;
	loop {
		match request().await {
			Ok(res) => return Ok(res),
			Err(err) if (err.is_connect() || err.is_timeout()) && attempt < ATTEMPTS => {
				log::debug!("{step}: ошибка соединения, попытка {attempt}");
				tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
				attempt += 1;
			}
			Err(err) => return Err(AuthError::from_reqwest(step, err)),
		}
	}
}

/// Тело успешного ответа никуда не сохраняется: в нём токены.
async fn read_json<T: DeserializeOwned>(res: Response, step: Step) -> Result<T, AuthError> {
	let status = res.status();
	let skew = clock_skew(res.headers());
	let text = res
		.text()
		.await
		.map_err(|err| AuthError::from_reqwest(step, err))?;
	if !status.is_success() {
		return Err(AuthError::Response {
			step,
			status: status.as_u16(),
			body: text,
			clock_skew: skew,
		});
	}
	serde_json::from_str(&text)
		.map_err(|err| AuthError::unexpected(step, format!("неожиданный ответ: {err}")))
}

struct SignedResponse<T> {
	headers: HeaderMap,
	date: DateTime<Utc>,
	body: T,
}

/// Время в формате Windows FILETIME (сотни наносекунд с 1601-01-01).
fn filetime(date: DateTime<Utc>) -> u64 {
	const UNIX_TO_WINDOWS_SECS: u64 = 11_644_473_600;
	(date.timestamp().max(0) as u64 + UNIX_TO_WINDOWS_SECS) * 10_000_000
}

/// Подпись запроса Xbox: ECDSA P-256 над `версия | время | метод | путь | авторизация | тело`.
fn sign(key: &DeviceKey, url_path: &str, body: &[u8], date: DateTime<Utc>) -> String {
	let time = filetime(date);

	let mut buffer = Vec::with_capacity(body.len() + url_path.len() + 32);
	buffer.extend_from_slice(&1_u32.to_be_bytes());
	buffer.push(0);
	buffer.extend_from_slice(&time.to_be_bytes());
	buffer.push(0);
	buffer.extend_from_slice(b"POST");
	buffer.push(0);
	buffer.extend_from_slice(url_path.as_bytes());
	buffer.push(0);
	// Заголовок Authorization не отправляем — пустое поле.
	buffer.push(0);
	buffer.extend_from_slice(body);
	buffer.push(0);

	let signature: Signature = key.key.sign(&buffer);

	let mut out = Vec::with_capacity(76);
	out.extend_from_slice(&1_u32.to_be_bytes());
	out.extend_from_slice(&time.to_be_bytes());
	out.extend_from_slice(&signature.to_bytes());
	BASE64_STANDARD.encode(out)
}

async fn signed_request<T: DeserializeOwned>(
	http: &Client,
	url: &str,
	url_path: &str,
	body: serde_json::Value,
	key: &DeviceKey,
	step: Step,
	date: DateTime<Utc>,
) -> Result<SignedResponse<T>, AuthError> {
	let body = serde_json::to_vec(&body)
		.map_err(|err| AuthError::unexpected(step, format!("сериализация запроса: {err}")))?;
	let signature = sign(key, url_path, &body, date);
	let is_sisu_authorize = url_path == "/authorize";

	let res = with_retry(step, || {
		let mut req = http
			.post(url)
			.header("Content-Type", "application/json; charset=utf-8")
			.header("Accept", "application/json")
			.header("Signature", &signature);
		if !is_sisu_authorize {
			req = req.header("x-xbl-contract-version", "1");
		}
		req.body(body.clone()).send()
	})
	.await?;

	let headers = res.headers().clone();
	let date = server_date(&headers);
	let body = read_json(res, step).await?;
	Ok(SignedResponse {
		headers,
		date,
		body,
	})
}

fn parse_date(headers: &HeaderMap) -> Option<DateTime<Utc>> {
	headers
		.get(reqwest::header::DATE)
		.and_then(|v| v.to_str().ok())
		.and_then(|v| DateTime::parse_from_rfc2822(v).ok())
		.map(|d| d.with_timezone(&Utc))
}

fn server_date(headers: &HeaderMap) -> DateTime<Utc> {
	parse_date(headers).unwrap_or_else(Utc::now)
}

fn clock_skew(headers: &HeaderMap) -> Option<i64> {
	parse_date(headers).map(|server| (Utc::now() - server).num_seconds())
}

#[cfg(test)]
mod tests {
	use p256::ecdsa::signature::Verifier;

	use super::*;

	#[test]
	fn signature_layout() {
		let key = DeviceKey::generate().unwrap();
		let date = DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z")
			.unwrap()
			.with_timezone(&Utc);
		let raw = BASE64_STANDARD
			.decode(sign(&key, "/authorize", b"{}", date))
			.unwrap();
		assert_eq!(raw.len(), 4 + 8 + 64);
		assert_eq!(&raw[..4], &[0, 0, 0, 1]);
		assert_eq!(
			u64::from_be_bytes(raw[4..12].try_into().unwrap()),
			filetime(date)
		);

		let mut signed = Vec::new();
		signed.extend_from_slice(&[0, 0, 0, 1, 0]);
		signed.extend_from_slice(&filetime(date).to_be_bytes());
		signed.extend_from_slice(b"\0POST\0/authorize\0\0{}\0");
		let sig = Signature::from_slice(&raw[12..]).unwrap();
		VerifyingKey::from(&key.key).verify(&signed, &sig).unwrap();
	}

	#[test]
	fn filetime_epoch() {
		let unix_epoch = DateTime::from_timestamp(0, 0).unwrap();
		assert_eq!(filetime(unix_epoch), 116_444_736_000_000_000);
	}
}
