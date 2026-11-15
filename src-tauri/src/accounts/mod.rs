//! Аккаунты: офлайн (по нику) и Microsoft.

pub mod commands;
mod microsoft;
mod offline;
mod store;

use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use self::microsoft::error::AuthError;
use self::microsoft::{DeviceSecret, MsSecret};
use self::store::{AccountsFile, StoredAccount};
use crate::error::{AppError, ErrorCode};
use crate::secure;

pub const EVENT_CHANGED: &str = "accounts-changed";
pub const EVENT_LOGIN_PROGRESS: &str = "account-login-progress";

/// Что делать, когда список аккаунтов изменился (в работе — разослать `accounts-changed`).
pub type OnChanged<'a> = &'a (dyn Fn(&AccountsState) + Sync);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
	Offline,
	Microsoft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
	/// Внутренний id записи (uuid v4), не uuid игрока.
	pub id: String,
	pub kind: AccountKind,
	pub username: String,
	/// UUID игрока с дефисами.
	pub uuid: String,
	/// ISO 8601.
	pub added_at: String,
	#[serde(default)]
	pub needs_relogin: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsState {
	pub accounts: Vec<Account>,
	pub active_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginStep {
	WaitingBrowser,
	Xbox,
	Minecraft,
	Profile,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginProgress {
	pub step: LoginStep,
}

/// Данные для запуска игры. Токен в `Debug` не печатается.
#[derive(Clone)]
pub struct LaunchCredentials {
	pub username: String,
	pub uuid: String,
	pub access_token: String,
	/// `msa` или `legacy` — значение `${user_type}` в аргументах игры.
	pub user_type: &'static str,
}

impl std::fmt::Debug for LaunchCredentials {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("LaunchCredentials")
			.field("username", &self.username)
			.field("uuid", &self.uuid)
			.field("user_type", &self.user_type)
			.finish_non_exhaustive()
	}
}

/// Состояние аккаунтов в памяти; кладётся в состояние Tauri.
pub struct Accounts {
	path: PathBuf,
	file: Mutex<AccountsFile>,
	/// Файл есть, но прочитать его не удалось: любая запись запрещена, чтобы не затереть его пустым списком.
	read_only: bool,
	/// Один запрос device token за раз.
	device_lock: Mutex<()>,
	/// Одно обновление токена за раз.
	refresh_lock: Mutex<()>,
	/// Отмена текущей попытки входа Microsoft. Новый вход отменяет прошлую попытку и ставит свой токен.
	login_cancel: std::sync::Mutex<CancellationToken>,
	http: reqwest::Client,
}

fn now_iso() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

impl AccountsFile {
	fn state(&self) -> AccountsState {
		AccountsState {
			accounts: self.accounts.iter().map(|a| a.account.clone()).collect(),
			active_id: self.active_id.clone(),
		}
	}

	fn find(&self, id: &str) -> Option<&StoredAccount> {
		self.accounts.iter().find(|a| a.account.id == id)
	}
}

fn encrypt<T: Serialize>(value: &T) -> Result<String, AppError> {
	let json = serde_json::to_vec(value).map_err(|err| AppError::storage(err.to_string()))?;
	secure::protect(&json).map_err(|err| AppError::storage(format!("шифрование: {err}")))
}

fn decrypt<T: for<'de> Deserialize<'de>>(text: &str) -> Option<T> {
	let bytes = secure::unprotect(text)
		.inspect_err(|err| log::warn!("не удалось расшифровать секрет: {err}"))
		.ok()?;
	serde_json::from_slice(&bytes)
		.inspect_err(|err| log::warn!("секрет повреждён: {err}"))
		.ok()
}

impl Accounts {
	pub fn load(data_dir: &Path, http: reqwest::Client) -> Self {
		let path = store::path_in(data_dir);
		let store::Loaded { file, writable } = store::load(&path);
		log::info!("аккаунтов загружено: {}", file.accounts.len());
		if !writable {
			log::error!(
				"{} не прочитан: изменения аккаунтов не сохранятся до перезапуска",
				path.display()
			);
		}
		Self {
			path,
			file: Mutex::new(file),
			read_only: !writable,
			device_lock: Mutex::new(()),
			refresh_lock: Mutex::new(()),
			login_cancel: std::sync::Mutex::new(CancellationToken::new()),
			http,
		}
	}

	pub async fn state(&self) -> AccountsState {
		self.file.lock().await.state()
	}

	/// Меняет копию файла, сохраняет её на диск и только после успешной записи заменяет состояние в памяти.
	async fn update<T>(
		&self,
		change: impl FnOnce(&mut AccountsFile) -> Result<T, AppError>,
	) -> Result<(T, AccountsState), AppError> {
		if self.read_only {
			return Err(AppError::storage(format!(
				"{} не удалось прочитать при запуске; запись отключена, чтобы не потерять его",
				self.path.display()
			)));
		}
		let mut file = self.file.lock().await;
		let mut next = file.clone();
		let result = change(&mut next)?;
		store::save(&self.path, &next).map_err(|err| {
			log::error!("не удалось записать {}: {err}", self.path.display());
			AppError::storage(err.to_string())
		})?;
		*file = next;
		Ok((result, file.state()))
	}

	pub async fn add_offline(&self, username: &str) -> Result<(Account, AccountsState), AppError> {
		let username = username.trim();
		if !offline::is_valid_username(username) {
			return Err(AppError::new(ErrorCode::InvalidUsername));
		}
		self.update(|file| {
			let duplicate = file.accounts.iter().any(|a| {
				a.account.kind == AccountKind::Offline
					&& a.account.username.eq_ignore_ascii_case(username)
			});
			if duplicate {
				return Err(AppError::new(ErrorCode::Duplicate));
			}
			let account = Account {
				id: Uuid::new_v4().to_string(),
				kind: AccountKind::Offline,
				username: username.to_owned(),
				uuid: offline::offline_uuid(username).to_string(),
				added_at: now_iso(),
				needs_relogin: false,
			};
			file.accounts.push(StoredAccount {
				account: account.clone(),
				secret: None,
			});
			file.active_id = Some(account.id.clone());
			Ok(account)
		})
		.await
	}

	/// Сохраняет результат входа Microsoft: обновляет аккаунт с тем же uuid игрока или добавляет новый.
	async fn upsert_microsoft(
		&self,
		session: microsoft::Session,
	) -> Result<(Account, AccountsState), AppError> {
		// Иначе запоздавшее обновление токена затрёт свежий секрет.
		let _guard = self.refresh_lock.lock().await;
		let secret = encrypt(&session.secret)?;
		let uuid = session.profile.id.hyphenated().to_string();
		let username = session.profile.name;
		self.update(move |file| {
			let existing = file.accounts.iter_mut().find(|a| {
				a.account.kind == AccountKind::Microsoft
					&& a.account.uuid.eq_ignore_ascii_case(&uuid)
			});
			let account = match existing {
				Some(stored) => {
					stored.account.username = username;
					stored.account.needs_relogin = false;
					stored.secret = Some(secret);
					stored.account.clone()
				}
				None => {
					let account = Account {
						id: Uuid::new_v4().to_string(),
						kind: AccountKind::Microsoft,
						username,
						uuid,
						added_at: now_iso(),
						needs_relogin: false,
					};
					file.accounts.push(StoredAccount {
						account: account.clone(),
						secret: Some(secret),
					});
					account
				}
			};
			file.active_id = Some(account.id.clone());
			Ok(account)
		})
		.await
	}

	pub async fn set_active(&self, id: &str) -> Result<AccountsState, AppError> {
		self.update(|file| {
			if file.find(id).is_none() {
				return Err(AppError::new(ErrorCode::NotFound));
			}
			file.active_id = Some(id.to_owned());
			Ok(())
		})
		.await
		.map(|(_, state)| state)
	}

	pub async fn remove(&self, id: &str) -> Result<AccountsState, AppError> {
		self.update(|file| {
			let index = file
				.accounts
				.iter()
				.position(|a| a.account.id == id)
				.ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
			file.accounts.remove(index);
			if file.active_id.as_deref() == Some(id) {
				file.active_id = file.accounts.first().map(|a| a.account.id.clone());
			}
			Ok(())
		})
		.await
		.map(|(_, state)| state)
	}

	async fn load_device(&self) -> Option<DeviceSecret> {
		let text = self.file.lock().await.device.clone()?;
		decrypt(&text)
	}

	/// Ошибка записи не мешает входу: ключ создастся заново в следующий раз.
	async fn save_device(&self, device: DeviceSecret) {
		let sealed = match encrypt(&device) {
			Ok(sealed) => sealed,
			Err(err) => {
				log::error!("не удалось зашифровать ключ устройства: {err}");
				return;
			}
		};
		if let Err(err) = self
			.update(|file| {
				file.device = Some(sealed);
				Ok(())
			})
			.await
		{
			log::error!("не удалось сохранить ключ устройства: {err}");
		}
	}

	/// Начинает новую попытку входа: прошлая (если шла) отменяется.
	fn begin_login(&self) -> CancellationToken {
		let token = CancellationToken::new();
		let mut current = self
			.login_cancel
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		current.cancel();
		*current = token.clone();
		token
	}

	fn cancel_login(&self) {
		self.login_cancel
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.cancel();
	}

	async fn mark_needs_relogin(&self, on_changed: OnChanged<'_>, id: &str) {
		match self
			.update(|file| {
				if let Some(stored) = file.accounts.iter_mut().find(|a| a.account.id == id) {
					stored.account.needs_relogin = true;
				}
				Ok(())
			})
			.await
		{
			Ok((_, state)) => on_changed(&state),
			Err(err) => log::error!("не удалось отметить аккаунт для повторного входа: {err}"),
		}
	}

	/// Данные для запуска, при необходимости с обновлённым токеном. Без сети — старый токен
	/// (одиночная игра всё равно запустится).
	pub async fn ensure_valid(
		&self,
		app: &AppHandle,
		id: &str,
	) -> Result<LaunchCredentials, AppError> {
		self.credentials(
			id,
			false,
			|token| async move { microsoft::refresh(self, &token).await },
			&|state| emit_changed(app, state),
		)
		.await
	}

	/// Токен для запросов скинов. `force` — Mojang отверг прошлый, обновить в любом случае.
	pub async fn skin_token(
		&self,
		id: &str,
		force: bool,
		on_changed: OnChanged<'_>,
	) -> Result<String, AppError> {
		let refresh = |token: String| async move {
			let secret = microsoft::refresh_secret(self, &token).await?;
			Ok((secret, None))
		};
		self.credentials(id, force, refresh, on_changed)
			.await
			.map(|credentials| credentials.access_token)
	}

	async fn credentials<F, Fut>(
		&self,
		id: &str,
		force: bool,
		refresh: F,
		on_changed: OnChanged<'_>,
	) -> Result<LaunchCredentials, AppError>
	where
		F: FnOnce(String) -> Fut,
		Fut: Future<Output = Result<(MsSecret, Option<microsoft::MinecraftProfile>), AuthError>>,
	{
		let find = || async {
			self.file
				.lock()
				.await
				.find(id)
				.cloned()
				.ok_or_else(|| AppError::new(ErrorCode::NotFound))
		};
		// Офлайн-аккаунту обновлять нечего: он не ждёт чужое обновление токена.
		let stored = find().await?;
		if stored.account.kind == AccountKind::Offline {
			return Ok(LaunchCredentials {
				username: stored.account.username,
				uuid: stored.account.uuid,
				access_token: "0".into(),
				user_type: "legacy",
			});
		}

		let _guard = self.refresh_lock.lock().await;
		// Читаем заново: пока ждали замок, токен могли обновить или аккаунт войти снова.
		let stored = find().await?;
		let account = stored.account;
		if account.needs_relogin {
			return Err(AppError::new(ErrorCode::NeedsRelogin));
		}

		let Some(secret) = stored.secret.as_deref().and_then(decrypt::<MsSecret>) else {
			log::warn!("аккаунт {id}: секрет не прочитан, нужен повторный вход");
			self.mark_needs_relogin(on_changed, id).await;
			return Err(AppError::new(ErrorCode::NeedsRelogin));
		};
		let credentials = |username: String, token: String| LaunchCredentials {
			username,
			uuid: account.uuid.clone(),
			access_token: token,
			user_type: "msa",
		};
		if !force && secret.is_fresh(Utc::now()) {
			return Ok(credentials(account.username, secret.access_token));
		}

		log::info!("аккаунт {id}: обновляем токен Minecraft");
		match refresh(secret.refresh_token.clone()).await {
			Ok((secret, profile)) => {
				let sealed = encrypt(&secret)?;
				let new_name = profile.map(|p| p.name);
				let (username, state) = self
					.update(|file| {
						let stored = file
							.accounts
							.iter_mut()
							.find(|a| a.account.id == id)
							.ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
						stored.secret = Some(sealed);
						if let Some(name) = new_name {
							stored.account.username = name;
						}
						Ok(stored.account.username.clone())
					})
					.await?;
				on_changed(&state);
				Ok(credentials(username, secret.access_token))
			}
			Err(err) if err.is_invalid_grant() => {
				log::warn!("аккаунт {id}: refresh token отозван: {err}");
				self.mark_needs_relogin(on_changed, id).await;
				Err(AppError::new(ErrorCode::NeedsRelogin))
			}
			Err(err) if err.is_network() && !force => {
				log::warn!("аккаунт {id}: нет сети, используем старый токен: {err}");
				Ok(credentials(account.username, secret.access_token))
			}
			Err(err) => {
				log::warn!("аккаунт {id}: не удалось обновить токен: {err}");
				Err(err.into())
			}
		}
	}
}

pub fn emit_changed(app: &AppHandle, state: &AccountsState) {
	if let Err(err) = app.emit(EVENT_CHANGED, state) {
		log::warn!("не удалось отправить {EVENT_CHANGED}: {err}");
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn accounts(dir: &Path) -> Accounts {
		Accounts::load(dir, crate::http::build_client().unwrap())
	}

	#[tokio::test]
	async fn offline_add_switch_remove() {
		let dir = tempfile::tempdir().unwrap();
		let accounts = accounts(dir.path());

		let (steve, state) = accounts.add_offline("Steve").await.unwrap();
		assert_eq!(state.active_id.as_deref(), Some(steve.id.as_str()));
		assert_eq!(steve.uuid, offline::offline_uuid("Steve").to_string());

		let err = accounts.add_offline("steve").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::Duplicate);
		let err = accounts.add_offline("x").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::InvalidUsername);

		let (alex, _) = accounts.add_offline("Alex").await.unwrap();
		let state = accounts.set_active(&steve.id).await.unwrap();
		assert_eq!(state.active_id.as_deref(), Some(steve.id.as_str()));
		assert_eq!(
			accounts.set_active("nope").await.unwrap_err().code,
			ErrorCode::NotFound
		);

		let state = accounts.remove(&steve.id).await.unwrap();
		assert_eq!(state.active_id.as_deref(), Some(alex.id.as_str()));
		let state = accounts.remove(&alex.id).await.unwrap();
		assert!(state.accounts.is_empty());
		assert_eq!(state.active_id, None);

		// Состояние пережило перезапуск.
		accounts.add_offline("Notch").await.unwrap();
		let reloaded = self::accounts(dir.path());
		let state = reloaded.state().await;
		assert_eq!(state.accounts.len(), 1);
		assert_eq!(state.accounts[0].username, "Notch");
	}

	#[tokio::test]
	async fn unreadable_file_is_read_only() {
		let dir = tempfile::tempdir().unwrap();
		// Папка вместо файла: чтение падает не с «файла нет».
		let path = store::path_in(dir.path());
		std::fs::create_dir(&path).unwrap();
		std::fs::write(path.join("keep"), b"x").unwrap();

		let accounts = accounts(dir.path());
		assert!(accounts.state().await.accounts.is_empty());
		let err = accounts.add_offline("Steve").await.unwrap_err();
		assert_eq!(err.code, ErrorCode::Storage);
		assert!(accounts.state().await.accounts.is_empty());
		assert!(path.is_dir());
		assert!(path.join("keep").exists());
	}

	#[tokio::test]
	async fn corrupt_file_backed_up_then_writable() {
		let dir = tempfile::tempdir().unwrap();
		let path = store::path_in(dir.path());
		std::fs::write(&path, b"{ broken").unwrap();

		let accounts = accounts(dir.path());
		accounts.add_offline("Steve").await.unwrap();
		let backups: Vec<_> = std::fs::read_dir(dir.path())
			.unwrap()
			.filter_map(|e| e.ok())
			.filter(|e| {
				e.file_name()
					.to_string_lossy()
					.starts_with("accounts.json.bak-")
			})
			.collect();
		assert_eq!(backups.len(), 1);
		assert_eq!(std::fs::read(backups[0].path()).unwrap(), b"{ broken");
	}

	async fn microsoft_account(accounts: &Accounts, expires_in_minutes: i64) -> String {
		let session = microsoft::Session {
			profile: microsoft::MinecraftProfile {
				id: Uuid::new_v4(),
				name: "Steve".into(),
			},
			secret: MsSecret {
				refresh_token: "old-refresh".into(),
				access_token: "old-access".into(),
				expires_at: Utc::now() + chrono::Duration::minutes(expires_in_minutes),
			},
		};
		accounts.upsert_microsoft(session).await.unwrap().0.id
	}

	fn new_secret() -> MsSecret {
		MsSecret {
			refresh_token: "new-refresh".into(),
			access_token: "new-access".into(),
			expires_at: Utc::now() + chrono::Duration::hours(24),
		}
	}

	fn no_refresh(
		_: String,
	) -> std::future::Ready<Result<(MsSecret, Option<microsoft::MinecraftProfile>), AuthError>> {
		panic!("обновление здесь не нужно")
	}

	#[tokio::test]
	async fn fresh_token_is_refreshed_only_when_forced() {
		let dir = tempfile::tempdir().unwrap();
		let accounts = accounts(dir.path());
		let id = microsoft_account(&accounts, 60).await;

		let same = accounts
			.credentials(&id, false, no_refresh, &|_| {})
			.await
			.unwrap();
		assert_eq!(same.access_token, "old-access");

		let changed = std::sync::atomic::AtomicUsize::new(0);
		let forced = accounts
			.credentials(
				&id,
				true,
				|token| {
					assert_eq!(token, "old-refresh");
					std::future::ready(Ok((new_secret(), None)))
				},
				&|_| {
					changed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
				},
			)
			.await
			.unwrap();
		assert_eq!(forced.access_token, "new-access");
		assert_eq!(forced.username, "Steve");
		assert_eq!(changed.load(std::sync::atomic::Ordering::SeqCst), 1);

		// Новый токен сохранён и пережил перезапуск.
		let reloaded = self::accounts(dir.path());
		let again = reloaded
			.credentials(&id, false, no_refresh, &|_| {})
			.await
			.unwrap();
		assert_eq!(again.access_token, "new-access");
	}

	#[tokio::test]
	async fn revoked_refresh_token_needs_relogin() {
		let dir = tempfile::tempdir().unwrap();
		let accounts = accounts(dir.path());
		let id = microsoft_account(&accounts, 60).await;

		let notified = std::sync::Mutex::new(Vec::new());
		let err = accounts
			.credentials(
				&id,
				true,
				|_| {
					std::future::ready(Err(AuthError::Response {
						step: microsoft::error::Step::OAuthRefresh,
						status: 400,
						body: r#"{"error":"invalid_grant"}"#.into(),
						clock_skew: None,
					}))
				},
				&|state| {
					notified
						.lock()
						.unwrap()
						.push(state.accounts[0].needs_relogin);
				},
			)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::NeedsRelogin);
		assert_eq!(*notified.lock().unwrap(), [true]);
		assert!(accounts.state().await.accounts[0].needs_relogin);

		// Дальше без запросов.
		let err = accounts.skin_token(&id, false, &|_| {}).await.unwrap_err();
		assert_eq!(err.code, ErrorCode::NeedsRelogin);
	}

	#[tokio::test]
	async fn network_error_old_token_only_without_force() {
		let dir = tempfile::tempdir().unwrap();
		let accounts = accounts(dir.path());
		let id = microsoft_account(&accounts, 1).await;
		let offline = |_: String| {
			std::future::ready(Err(AuthError::Network {
				step: microsoft::error::Step::OAuthRefresh,
				message: "нет сети".into(),
			}))
		};

		let old = accounts
			.credentials(&id, false, offline, &|_| {})
			.await
			.unwrap();
		assert_eq!(old.access_token, "old-access");
		// Mojang уже отверг этот токен: отдавать его снова бессмысленно.
		let err = accounts
			.credentials(&id, true, offline, &|_| {})
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::Network);
		assert!(!accounts.state().await.accounts[0].needs_relogin);
	}

	#[tokio::test]
	async fn other_refresh_error_keeps_account() {
		let dir = tempfile::tempdir().unwrap();
		let accounts = accounts(dir.path());
		let id = microsoft_account(&accounts, 60).await;
		let err = accounts
			.credentials(
				&id,
				true,
				|_| {
					std::future::ready(Err(AuthError::Response {
						step: microsoft::error::Step::MinecraftToken,
						status: 503,
						body: String::new(),
						clock_skew: None,
					}))
				},
				&|_| panic!("аккаунт не менялся"),
			)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::ServiceUnavailable);
		assert!(!accounts.state().await.accounts[0].needs_relogin);
	}

	#[tokio::test]
	async fn offline_does_not_wait_for_refresh() {
		let dir = tempfile::tempdir().unwrap();
		let accounts = accounts(dir.path());
		let (steve, _) = accounts.add_offline("Steve").await.unwrap();
		let _busy = accounts.refresh_lock.lock().await;
		let credentials = tokio::time::timeout(
			std::time::Duration::from_secs(1),
			accounts.credentials(&steve.id, false, no_refresh, &|_| {}),
		)
		.await
		.expect("офлайн-аккаунт ждал чужое обновление")
		.unwrap();
		assert_eq!(credentials.access_token, "0");
	}

	#[tokio::test]
	async fn late_refresh_does_not_spoil_new_login() {
		let dir = tempfile::tempdir().unwrap();
		let accounts = accounts(dir.path());
		let uuid = Uuid::new_v4();
		let session = |token: &str| microsoft::Session {
			profile: microsoft::MinecraftProfile {
				id: uuid,
				name: "Steve".into(),
			},
			secret: MsSecret {
				refresh_token: format!("{token}-refresh"),
				access_token: format!("{token}-access"),
				expires_at: Utc::now() + chrono::Duration::hours(1),
			},
		};
		let id = accounts
			.upsert_microsoft(session("old"))
			.await
			.unwrap()
			.0
			.id;

		let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
		let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
		// Обновление начато старым токеном и упрётся в «отозван» уже после нового входа.
		let refresh = accounts.credentials(
			&id,
			true,
			move |_| async move {
				let _ = entered_tx.send(());
				let _ = release_rx.await;
				Err(AuthError::Response {
					step: microsoft::error::Step::OAuthRefresh,
					status: 400,
					body: r#"{"error":"invalid_grant"}"#.into(),
					clock_skew: None,
				})
			},
			&|_| {},
		);
		let login = async {
			let _ = entered_rx.await;
			accounts.upsert_microsoft(session("new")).await.unwrap();
		};
		let release = async {
			tokio::time::sleep(std::time::Duration::from_millis(50)).await;
			let _ = release_tx.send(());
		};
		let (refreshed, (), ()) = tokio::join!(refresh, login, release);
		assert_eq!(refreshed.unwrap_err().code, ErrorCode::NeedsRelogin);

		assert!(!accounts.state().await.accounts[0].needs_relogin);
		let credentials = accounts
			.credentials(&id, false, no_refresh, &|_| {})
			.await
			.unwrap();
		assert_eq!(credentials.access_token, "new-access");
	}

	#[test]
	fn new_login_cancels_previous() {
		let dir = tempfile::tempdir().unwrap();
		let accounts = accounts(dir.path());
		let first = accounts.begin_login();
		let second = accounts.begin_login();
		assert!(first.is_cancelled());
		assert!(!second.is_cancelled());
		accounts.cancel_login();
		assert!(second.is_cancelled());
	}

	#[test]
	fn state_serializes_camel_case() {
		let state = AccountsState {
			accounts: vec![Account {
				id: "a".into(),
				kind: AccountKind::Microsoft,
				username: "Steve".into(),
				uuid: "u".into(),
				added_at: "t".into(),
				needs_relogin: true,
			}],
			active_id: Some("a".into()),
		};
		let json = serde_json::to_value(&state).unwrap();
		assert_eq!(json["activeId"], "a");
		assert_eq!(json["accounts"][0]["kind"], "microsoft");
		assert_eq!(json["accounts"][0]["needsRelogin"], true);
		assert_eq!(json["accounts"][0]["addedAt"], "t");
		assert!(json["accounts"][0].get("secret").is_none());

		let progress = serde_json::to_value(LoginProgress {
			step: LoginStep::WaitingBrowser,
		})
		.unwrap();
		assert_eq!(progress["step"], "waiting_browser");
	}

	#[test]
	fn secret_is_encrypted_on_disk() {
		let secret = MsSecret {
			refresh_token: "M.R3_secret_refresh".into(),
			access_token: "eyJsecret_access".into(),
			expires_at: Utc::now(),
		};
		let sealed = encrypt(&secret).unwrap();
		assert!(!sealed.contains("secret_refresh"));
		let back: MsSecret = decrypt(&sealed).unwrap();
		assert_eq!(back.refresh_token, secret.refresh_token);
		assert!(!format!("{back:?}").contains("secret"));
	}
}
