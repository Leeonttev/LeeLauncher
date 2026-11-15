use std::collections::BTreeMap;

use tauri::{AppHandle, Emitter, State};

use super::library::Library;
use super::{
	ApplyRequest, EVENT_CHANGED, SkinApplied, SkinEntry, SkinModel, SkinPatch, SkinProfile, Skins,
	SkinsAdded, WornSkin, with_library,
};
use crate::accounts::{self, Account, Accounts};
use crate::error::{AppError, ErrorCode};
use crate::lang::Lang;

pub fn emit_changed(app: &AppHandle) {
	if let Err(err) = app.emit(EVENT_CHANGED, ()) {
		log::warn!("не удалось отправить {EVENT_CHANGED}: {err}");
	}
}

async fn find_account(accounts: &Accounts, id: &str) -> Result<Account, AppError> {
	accounts
		.state()
		.await
		.accounts
		.into_iter()
		.find(|account| account.id == id)
		.ok_or_else(|| AppError::new(ErrorCode::NotFound))
}

#[tauri::command]
pub async fn skins_library(skins: State<'_, Skins>) -> Result<Vec<SkinEntry>, AppError> {
	with_library(&skins.library, Library::skins).await
}

#[tauri::command]
pub async fn skins_worn(skins: State<'_, Skins>) -> Result<BTreeMap<String, WornSkin>, AppError> {
	skins.worn().await
}

async fn add_paths(
	app: &AppHandle,
	skins: &Skins,
	paths: Vec<String>,
) -> Result<SkinsAdded, AppError> {
	let added = with_library(&skins.library, move |library| library.add_files(&paths)).await?;
	if !added.added.is_empty() {
		emit_changed(app);
	}
	Ok(added)
}

#[tauri::command]
pub async fn skin_add_files(
	app: AppHandle,
	skins: State<'_, Skins>,
	paths: Vec<String>,
) -> Result<SkinsAdded, AppError> {
	add_paths(&app, &skins, paths).await
}

/// Системное окно выбора PNG над главным окном. `null` — окно закрыли без выбора.
#[tauri::command]
pub async fn skin_pick_files(
	app: AppHandle,
	skins: State<'_, Skins>,
	lang: Option<Lang>,
) -> Result<Option<SkinsAdded>, AppError> {
	use crate::modpacks::commands::{dialog_title, with_parent};
	use tauri_plugin_dialog::DialogExt;
	let lang = lang.unwrap_or_default();
	let mut dialog = app
		.dialog()
		.file()
		.set_title(dialog_title(
			lang,
			"Выберите файлы скинов",
			"Choose skin files",
		))
		.add_filter(
			dialog_title(lang, "Скины (*.png)", "Skins (*.png)"),
			&["png"],
		);
	if let Some(dir) = dirs::download_dir().filter(|d| d.is_dir()) {
		dialog = dialog.set_directory(dir);
	}
	let (tx, rx) = tokio::sync::oneshot::channel();
	with_parent(&app, dialog).pick_files(move |files| {
		let _ = tx.send(files);
	});
	let Some(files) = rx.await.ok().flatten() else {
		return Ok(None);
	};
	let paths = files
		.into_iter()
		.filter_map(|file| file.into_path().ok())
		.map(|path| path.to_string_lossy().into_owned())
		.collect();
	add_paths(&app, &skins, paths).await.map(Some)
}

#[tauri::command]
pub async fn skin_update(
	app: AppHandle,
	skins: State<'_, Skins>,
	id: String,
	patch: SkinPatch,
) -> Result<SkinEntry, AppError> {
	let result = with_library(&skins.library, move |library| {
		library.update_skin(&id, patch)
	})
	.await;
	emit_on_change(&app, &result);
	result
}

/// Скин изменился или, при `skin_not_found`, мог пропасть его файл.
fn emit_on_change<T>(app: &AppHandle, result: &Result<T, AppError>) {
	match result {
		Ok(_) => emit_changed(app),
		Err(err) if err.code == ErrorCode::SkinNotFound => emit_changed(app),
		Err(_) => {}
	}
}

#[tauri::command]
pub async fn skin_remove(
	app: AppHandle,
	skins: State<'_, Skins>,
	id: String,
) -> Result<(), AppError> {
	with_library(&skins.library, move |library| {
		library.remove(&id, crate::trash::delete)
	})
	.await?;
	emit_changed(&app);
	Ok(())
}

#[tauri::command]
pub async fn skin_reveal(
	app: AppHandle,
	skins: State<'_, Skins>,
	id: String,
) -> Result<(), AppError> {
	use tauri_plugin_opener::OpenerExt;
	let found = with_library(&skins.library, move |library| library.reveal_path(&id)).await;
	if matches!(&found, Err(err) if err.code == ErrorCode::SkinNotFound) {
		emit_changed(&app);
	}
	app.opener()
		.reveal_item_in_dir(found?)
		.map_err(|err| AppError::unknown(err.to_string()))
}

/// `capeId`: поля нет — не меняется, `null` — скрыть плащ. Tauri не различает их у `Option`
/// (оба дают `None`), поэтому значение читается из тела запроса.
fn cape_change(body: &serde_json::Value) -> Result<Option<Option<String>>, AppError> {
	match body.get("capeId") {
		None => Ok(None),
		Some(serde_json::Value::Null) => Ok(Some(None)),
		Some(serde_json::Value::String(id)) => Ok(Some(Some(id.clone()))),
		Some(_) => Err(AppError::coded(ErrorCode::InvalidValue, "capeId")),
	}
}

/// Текущий скин и плащи аккаунта. При запуске приложения не вызывается: только по команде окна.
#[tauri::command]
pub async fn skin_profile(
	app: AppHandle,
	skins: State<'_, Skins>,
	accounts: State<'_, Accounts>,
	account_id: String,
	refresh: Option<bool>,
) -> Result<SkinProfile, AppError> {
	let account = find_account(&accounts, &account_id).await?;
	let accounts = accounts.inner();
	let outcome = {
		let (app, id) = (&app, account_id.as_str());
		let token = move |force| async move { token_of(app, accounts, id, force).await };
		skins
			.profile(&account_id, account.kind, refresh.unwrap_or(false), token)
			.await?
	};
	if account_gone(&app, &skins, accounts, &account_id).await {
		return Err(AppError::new(ErrorCode::NotFound));
	}
	if outcome.changed {
		emit_changed(&app);
	}
	Ok(outcome.value)
}

#[tauri::command]
pub async fn skin_save_current(
	app: AppHandle,
	skins: State<'_, Skins>,
	accounts: State<'_, Accounts>,
	account_id: String,
) -> Result<SkinEntry, AppError> {
	let account = find_account(&accounts, &account_id).await?;
	let entry = skins
		.save_current(&account_id, account.kind, &account.username)
		.await?;
	emit_changed(&app);
	Ok(entry)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn skin_apply(
	app: AppHandle,
	request: tauri::ipc::Request<'_>,
	skins: State<'_, Skins>,
	accounts: State<'_, Accounts>,
	account_id: String,
	skin_id: Option<String>,
	model: Option<SkinModel>,
	allow_lose_previous: Option<bool>,
) -> Result<SkinApplied, AppError> {
	let cape = match request.body() {
		tauri::ipc::InvokeBody::Json(body) => cape_change(body)?,
		tauri::ipc::InvokeBody::Raw(_) => None,
	};
	let account = find_account(&accounts, &account_id).await?;
	let accounts = accounts.inner();
	let request = ApplyRequest {
		skin_id,
		model,
		cape,
		allow_lose_previous: allow_lose_previous.unwrap_or(false),
	};
	let result = {
		let (app, id) = (&app, account_id.as_str());
		let token = move |force| async move { token_of(app, accounts, id, force).await };
		skins
			.apply(&account_id, account.kind, &account.username, request, token)
			.await
	};
	if matches!(&result, Err(err) if err.code == ErrorCode::SkinNotFound) {
		emit_changed(&app);
	}
	let outcome = result?;
	log::info!("аккаунт {account_id}: изменения отправлены");
	if account_gone(&app, &skins, accounts, &account_id).await {
		return Err(AppError::new(ErrorCode::NotFound));
	}
	if outcome.changed {
		emit_changed(&app);
	}
	Ok(outcome.value)
}

/// Аккаунт удалили, пока шёл запрос: записанный профиль ему больше не нужен.
async fn account_gone(
	app: &AppHandle,
	skins: &Skins,
	accounts: &Accounts,
	account_id: &str,
) -> bool {
	if find_account(accounts, account_id).await.is_ok() {
		return false;
	}
	forget_account(app, skins, account_id).await;
	true
}

async fn token_of(
	app: &AppHandle,
	accounts: &Accounts,
	id: &str,
	force: bool,
) -> Result<String, AppError> {
	accounts
		.skin_token(id, force, &|state| accounts::emit_changed(app, state))
		.await
}

#[tauri::command]
pub async fn skin_reset(
	app: AppHandle,
	skins: State<'_, Skins>,
	accounts: State<'_, Accounts>,
	account_id: String,
) -> Result<SkinProfile, AppError> {
	let account = find_account(&accounts, &account_id).await?;
	let accounts = accounts.inner();
	let outcome = {
		let (app, id) = (&app, account_id.as_str());
		let token = move |force| async move { token_of(app, accounts, id, force).await };
		skins.reset(&account_id, account.kind, token).await?
	};
	log::info!("аккаунт {account_id}: скин снят");
	if account_gone(&app, &skins, accounts, &account_id).await {
		return Err(AppError::new(ErrorCode::NotFound));
	}
	if outcome.changed {
		emit_changed(&app);
	}
	Ok(outcome.value)
}

/// Вызывается при удалении аккаунта. Аккаунт уже удалён, поэтому ошибка только в лог.
pub async fn forget_account(app: &AppHandle, skins: &Skins, account_id: &str) {
	match skins.forget(account_id).await {
		Ok(true) => emit_changed(app),
		Ok(false) => {}
		Err(err) => log::warn!("аккаунт {account_id}: профиль скина не удалён: {err}"),
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn cape_argument_keeps_null_apart_from_missing() {
		assert_eq!(cape_change(&json!({ "accountId": "a" })).unwrap(), None);
		assert_eq!(cape_change(&json!({ "capeId": null })).unwrap(), Some(None));
		assert_eq!(
			cape_change(&json!({ "capeId": "x" })).unwrap(),
			Some(Some("x".to_owned()))
		);
		let err = cape_change(&json!({ "capeId": 5 })).unwrap_err();
		assert_eq!(err.code, ErrorCode::InvalidValue);
	}
}
