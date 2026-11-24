//! Команды Tauri запуска игры: запуск, остановка, лог, запущенные игры.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use super::logs::LogLine;
use super::{EVENT_LOG, EVENT_PROCESS, GameEvents, LogBatch, ProcessEvent, RunningGame};
use crate::accounts::Accounts;
use crate::error::{AppError, ErrorCode};
use crate::instances::Instance;
use crate::instances::commands::EVENT_CHANGED;
use crate::launcher::Launcher;

struct TauriGameEvents(AppHandle);

fn emit<T: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
	if let Err(err) = app.emit(event, payload) {
		log::warn!("не удалось отправить {event}: {err}");
	}
}

impl GameEvents for TauriGameEvents {
	fn process(&self, event: ProcessEvent) {
		let exited = event.state == super::ProcessState::Exited;
		// Вылет или ошибка запуска: окно из трея нужно показать.
		let failed = event.crashed == Some(true) || event.error.is_some();
		emit(&self.0, EVENT_PROCESS, event);
		crate::presence::refresh(&self.0);
		if exited {
			crate::tray::on_game_exited(&self.0, failed);
		}
	}

	fn log(&self, batch: LogBatch) {
		emit(&self.0, EVENT_LOG, batch);
	}

	fn changed(&self, instances: Vec<Instance>) {
		emit(&self.0, EVENT_CHANGED, instances);
	}

	fn server_packs_guarded(&self, guarded: super::server_packs::Guarded) {
		emit(&self.0, super::server_packs::EVENT, guarded);
	}
}

#[tauri::command]
pub async fn instance_launch(
	app: AppHandle,
	l: State<'_, Arc<Launcher>>,
	accounts: State<'_, Accounts>,
	id: String,
	account_id: Option<String>,
	world: Option<String>,
) -> Result<(), AppError> {
	if let Some(world) = &world {
		check_world(&l, &id, world).await?;
	}
	let accounts = accounts.inner();
	let events: Arc<dyn GameEvents> = Arc::new(TauriGameEvents(app.clone()));
	let credentials = || async {
		let account_id = match account_id {
			Some(id) => id,
			None => accounts
				.state()
				.await
				.active_id
				.ok_or_else(|| AppError::new(ErrorCode::NoAccount))?,
		};
		accounts
			.ensure_valid(&app, &account_id)
			.await
			.map_err(|err| match err.code {
				ErrorCode::NotFound => AppError::coded(ErrorCode::NoAccount, account_id.clone()),
				_ => err,
			})
	};
	super::launch(l.inner().clone(), id, world, events, credentials).await?;
	crate::java::runtime::refresh_index_soon(l.inner().clone());
	Ok(())
}

/// Мир для входа сразу: есть в `saves/` и версия сборки умеет `--quickPlaySingleplayer`.
async fn check_world(l: &Launcher, id: &str, world: &str) -> Result<(), AppError> {
	let instance = l.instances.get(id).await?;
	let dir = crate::gamedata::world_dir(&l.layout.game_dir(id), world)?;
	if !dir.join("level.dat").is_file() {
		return Err(AppError::coded(
			ErrorCode::FileNotFound,
			format!("saves/{world}"),
		));
	}
	if !crate::gamedata::commands::quick_play(l, &instance) {
		return Err(AppError::coded(
			ErrorCode::InvalidValue,
			"quickPlaySingleplayer: версия игры не умеет входить сразу в мир",
		));
	}
	Ok(())
}

/// Находит игры, пережившие прошлый экземпляр лаунчера (вызывается при старте).
pub async fn recover(app: AppHandle, l: Arc<Launcher>) {
	let events: Arc<dyn GameEvents> = Arc::new(TauriGameEvents(app));
	let count = super::recover(&l, events).await;
	if count > 0 {
		log::info!("найдено игр с прошлого запуска: {count}");
	}
}

#[tauri::command]
pub async fn instance_kill(l: State<'_, Arc<Launcher>>, id: String) -> Result<(), AppError> {
	if l.instances.kill(&id) {
		log::info!("игра {id}: остановка по просьбе пользователя");
	}
	Ok(())
}

#[tauri::command]
pub async fn instance_log(
	l: State<'_, Arc<Launcher>>,
	id: String,
) -> Result<Vec<LogLine>, AppError> {
	l.instances.get(&id).await?;
	Ok(l.games.log(&id))
}

#[tauri::command]
pub async fn instances_running(l: State<'_, Arc<Launcher>>) -> Result<Vec<RunningGame>, AppError> {
	Ok(l.games.running())
}
