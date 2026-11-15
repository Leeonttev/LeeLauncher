mod api;
pub mod error;
pub mod window;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub use self::api::MinecraftProfile;
use self::api::{DeviceKey, XboxToken};
use self::error::AuthError;
use super::{Accounts, LoginStep};
use crate::lang::Lang;

/// Секреты аккаунта Microsoft. Хранятся только зашифрованными, в интерфейс не уходят.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsSecret {
	pub refresh_token: String,
	pub access_token: String,
	/// Когда истекает токен Minecraft.
	pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for MsSecret {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("MsSecret")
			.field("expires_at", &self.expires_at)
			.finish_non_exhaustive()
	}
}

impl MsSecret {
	/// Токен ещё годится с запасом 5 минут — на запуск игры и небольшой сдвиг часов.
	pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
		self.expires_at > now + Duration::minutes(5)
	}
}

/// Ключ устройства Xbox и его токен (живёт около двух недель), общие для всех аккаунтов.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSecret {
	id: Uuid,
	/// Закрытый ключ P-256, 32 байта в base64.
	key: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	token: Option<XboxToken>,
}

pub struct Session {
	pub profile: MinecraftProfile,
	pub secret: MsSecret,
}

/// Ключ устройства и действующий device token.
pub struct Device {
	key: DeviceKey,
	token: String,
	date: DateTime<Utc>,
}

/// Возвращает действующий токен устройства: из сохранённого или запрашивает новый (и сохраняет).
async fn device(accounts: &Accounts, date: DateTime<Utc>) -> Result<Device, AuthError> {
	use base64::Engine;
	use base64::prelude::BASE64_STANDARD;

	let _guard = accounts.device_lock.lock().await;
	let stored = accounts.load_device().await;

	let key = stored.as_ref().and_then(|s| {
		let bytes = BASE64_STANDARD.decode(&s.key).ok()?;
		let key = p256::ecdsa::SigningKey::from_slice(&bytes).ok()?;
		DeviceKey::from_signing_key(s.id, key).ok()
	});
	if let (Some(key), Some(token)) = (&key, stored.as_ref().and_then(|s| s.token.as_ref()))
		&& token.not_after > date + Duration::minutes(1)
	{
		return Ok(Device {
			key: key.clone(),
			token: token.token.clone(),
			date,
		});
	}

	let key = match key {
		Some(key) => key,
		None => DeviceKey::generate()?,
	};
	let res = api::device_token(&accounts.http, &key, date).await?;
	accounts
		.save_device(DeviceSecret {
			id: key.id,
			key: BASE64_STANDARD.encode(key.key.to_bytes()),
			token: Some(res.value.clone()),
		})
		.await;
	Ok(Device {
		key,
		token: res.value.token,
		date: res.date,
	})
}

/// Xbox → Minecraft: SISU authorize, XSTS и токен Minecraft.
async fn minecraft_access(
	accounts: &Accounts,
	device: &Device,
	session_id: Option<&str>,
	ms_access_token: &str,
	date: DateTime<Utc>,
	on_step: &impl Fn(LoginStep),
) -> Result<MsSecretParts, AuthError> {
	let http = &accounts.http;
	let sisu = api::sisu_authorize(
		http,
		session_id,
		ms_access_token,
		&device.token,
		&device.key,
		date,
	)
	.await?;
	let xsts =
		api::xsts_authorize(http, &sisu.value, &device.token, &device.key, sisu.date).await?;

	on_step(LoginStep::Minecraft);
	let issued = Utc::now();
	let mc = api::minecraft_token(http, &xsts).await?;
	Ok(MsSecretParts {
		access_token: mc.access_token,
		expires_at: issued + Duration::seconds(mc.expires_in),
	})
}

struct MsSecretParts {
	access_token: String,
	expires_at: DateTime<Utc>,
}

/// Полный вход: окно Microsoft → Xbox → Minecraft → лицензия и профиль. `cancel` прерывает сразу.
pub async fn login(
	app: &AppHandle,
	accounts: &Accounts,
	lang: Lang,
	on_step: impl Fn(LoginStep),
	cancel: &CancellationToken,
) -> Result<Session, AuthError> {
	tokio::select! {
		biased;
		_ = cancel.cancelled() => Err(AuthError::Cancelled),
		result = login_steps(app, accounts, lang, on_step, cancel) => result,
	}
}

async fn login_steps(
	app: &AppHandle,
	accounts: &Accounts,
	lang: Lang,
	on_step: impl Fn(LoginStep),
	cancel: &CancellationToken,
) -> Result<Session, AuthError> {
	let check = || {
		if cancel.is_cancelled() {
			Err(AuthError::Cancelled)
		} else {
			Ok(())
		}
	};
	let http = &accounts.http;

	on_step(LoginStep::WaitingBrowser);
	let device = device(accounts, Utc::now()).await?;
	let start = api::sisu_authenticate(http, &device.token, &device.key, device.date).await?;
	log::debug!("вход Microsoft: SISU-сессия создана");
	check()?;
	let code = window::run(app, &start.auth_url, lang, cancel).await?;
	check()?;

	on_step(LoginStep::Xbox);
	let oauth = api::oauth_token(http, &code, &start.verifier).await?;
	check()?;
	let mc = minecraft_access(
		accounts,
		&device,
		Some(&start.session_id),
		&oauth.value.access_token,
		oauth.date,
		&on_step,
	)
	.await?;
	check()?;

	on_step(LoginStep::Profile);
	let licensed = api::has_java_license(http, &mc.access_token).await?;
	// Нет профиля (404) — игра не куплена.
	let profile = api::minecraft_profile(http, &mc.access_token).await?;
	if !licensed {
		// Лицензии в entitlements нет, но профиль есть — бывает с Game Pass; не мешаем входу.
		log::warn!("вход Microsoft: в entitlements нет лицензии Java Edition, профиль найден");
	}
	check()?;

	Ok(Session {
		profile,
		secret: MsSecret {
			refresh_token: oauth.value.refresh_token,
			access_token: mc.access_token,
			expires_at: mc.expires_at,
		},
	})
}

/// Обновляет токен Minecraft по refresh token Microsoft. Возвращает новые секреты
/// (Microsoft меняет и refresh token) и профиль, если его удалось получить.
pub async fn refresh(
	accounts: &Accounts,
	refresh_token: &str,
) -> Result<(MsSecret, Option<MinecraftProfile>), AuthError> {
	let secret = refresh_secret(accounts, refresh_token).await?;
	let profile = match api::minecraft_profile(&accounts.http, &secret.access_token).await {
		Ok(profile) => Some(profile),
		Err(err) => {
			log::warn!("обновление токена: профиль не получен: {err}");
			None
		}
	};
	Ok((secret, profile))
}

/// То же без запроса профиля. Им пользуются скины: `GET /minecraft/profile` там запрещён.
pub async fn refresh_secret(
	accounts: &Accounts,
	refresh_token: &str,
) -> Result<MsSecret, AuthError> {
	let oauth = api::oauth_refresh(&accounts.http, refresh_token).await?;
	let device = device(accounts, oauth.date).await?;
	let mc = minecraft_access(
		accounts,
		&device,
		None,
		&oauth.value.access_token,
		device.date,
		&|_| {},
	)
	.await?;
	Ok(MsSecret {
		refresh_token: oauth.value.refresh_token,
		access_token: mc.access_token,
		expires_at: mc.expires_at,
	})
}
