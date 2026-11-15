use tauri::{AppHandle, Emitter, State};

use super::microsoft::{self, window};
use super::{Account, Accounts, AccountsState, EVENT_LOGIN_PROGRESS, LoginProgress, emit_changed};
use crate::error::{AppError, ErrorCode};
use crate::lang::Lang;

#[tauri::command]
pub async fn accounts_list(accounts: State<'_, Accounts>) -> Result<AccountsState, AppError> {
	Ok(accounts.state().await)
}

#[tauri::command]
pub async fn account_add_offline(
	app: AppHandle,
	accounts: State<'_, Accounts>,
	username: String,
) -> Result<Account, AppError> {
	let (account, state) = accounts.add_offline(&username).await?;
	log::info!("добавлен офлайн-аккаунт {}", account.username);
	emit_changed(&app, &state);
	Ok(account)
}

#[tauri::command]
pub async fn account_login_microsoft(
	app: AppHandle,
	accounts: State<'_, Accounts>,
	lang: Option<Lang>,
) -> Result<Account, AppError> {
	let lang = lang.unwrap_or_default();
	let accounts = accounts.inner();
	let cancel = accounts.begin_login();
	let on_step = |step| {
		if let Err(err) = app.emit(EVENT_LOGIN_PROGRESS, LoginProgress { step }) {
			log::warn!("не удалось отправить {EVENT_LOGIN_PROGRESS}: {err}");
		}
	};

	log::info!("вход Microsoft: начало");
	let session = match microsoft::login(&app, accounts, lang, on_step, &cancel).await {
		Ok(session) => session,
		Err(err) => {
			let err = AppError::from(err);
			if err.code == ErrorCode::Cancelled {
				log::info!("вход Microsoft: отменён");
			} else {
				log::warn!(
					"вход Microsoft: ошибка {:?}: {}",
					err.code,
					err.details.as_deref().unwrap_or("")
				);
			}
			return Err(err);
		}
	};
	// Отмена после последнего шага: результат не сохраняем.
	if cancel.is_cancelled() {
		log::info!("вход Microsoft: отменён");
		return Err(AppError::new(ErrorCode::Cancelled));
	}
	let (account, state) = accounts.upsert_microsoft(session).await?;
	log::info!("вход Microsoft: успешно, {}", account.username);
	emit_changed(&app, &state);
	Ok(account)
}

#[tauri::command]
pub async fn account_login_microsoft_cancel(
	app: AppHandle,
	accounts: State<'_, Accounts>,
) -> Result<(), AppError> {
	accounts.cancel_login();
	window::close(&app);
	Ok(())
}

#[tauri::command]
pub async fn account_set_active(
	app: AppHandle,
	accounts: State<'_, Accounts>,
	id: String,
) -> Result<AccountsState, AppError> {
	let state = accounts.set_active(&id).await?;
	emit_changed(&app, &state);
	Ok(state)
}

#[tauri::command]
pub async fn account_remove(
	app: AppHandle,
	accounts: State<'_, Accounts>,
	skins: State<'_, crate::skins::Skins>,
	id: String,
) -> Result<AccountsState, AppError> {
	let state = accounts.remove(&id).await?;
	log::info!("аккаунт {id} удалён");
	emit_changed(&app, &state);
	crate::skins::commands::forget_account(&app, &skins, &id).await;
	Ok(state)
}
