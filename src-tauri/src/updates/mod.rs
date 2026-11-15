//! Обновление с GitHub Releases. Плагин проверяет подпись и запускает установщик,
//! а пакет качаем сами (`fetch.rs`) — с докачкой и замером скорости.

mod fetch;
mod state;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, ErrorCode};
use crate::launcher::Launcher;

pub use state::LastUpdate;

pub const EVENT_PROGRESS: &str = "app-update-progress";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Вся проверка (`latest.json` — пара килобайт вместе с переходом на CDN GitHub).
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// Найденное обновление для интерфейса.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
	pub version: String,
	pub current_version: String,
	/// RFC 3339, как в `latest.json` (`pub_date`).
	pub date: Option<String>,
	/// Заметки к выпуску (markdown, поле `notes`).
	pub notes: Option<String>,
	/// Размер пакета из `latest.json`, если есть.
	pub size: Option<u64>,
	/// Пакет этой версии уже скачан и проверен: «Обновить» сразу ставит.
	pub downloaded: bool,
}

/// Состояние обновления в `app.manage`.
pub struct Updates {
	/// Папка данных (`update-state.json`).
	root: PathBuf,
	/// Скачанные пакеты: `update-<версия>.exe` и недокачанные `.part`.
	dir: PathBuf,
	/// Последнее найденное обновление.
	found: Mutex<Option<Update>>,
	/// Идущее скачивание или установка.
	task: Mutex<Option<CancellationToken>>,
	/// Этот запуск — первый после обновления.
	last: Option<LastUpdate>,
}

impl Updates {
	/// Идёт скачивание или установка обновления.
	pub fn busy(&self) -> bool {
		self.task.lock().map(|t| t.is_some()).unwrap_or(false)
	}
}

/// Проверку и установку можно включить в отладочной сборке переменной окружения:
/// иначе `pnpm tauri dev` увидит релиз и поставит его поверх себя.
fn enabled() -> bool {
	!cfg!(debug_assertions) || std::env::var_os("LEELAUNCHER_UPDATER_IN_DEV").is_some()
}

fn updater<R: Runtime>(app: &AppHandle<R>) -> Result<tauri_plugin_updater::Updater, AppError> {
	let handle = app.clone();
	app.updater_builder()
		.configure_client(|client| {
			client
				.connect_timeout(CONNECT_TIMEOUT)
				.read_timeout(READ_TIMEOUT)
		})
		// `install` сразу завершает процесс, событий выхода не будет: сохраняем окно сами.
		.on_before_exit(move || save_window_state(&handle))
		.build()
		.map_err(|err| AppError::coded(ErrorCode::UpdateFailed, err.to_string()))
}

#[cfg(desktop)]
fn save_window_state<R: Runtime>(app: &AppHandle<R>) {
	use tauri_plugin_window_state::{AppHandleExt, StateFlags};
	if let Err(err) = app.save_window_state(StateFlags::all() & !StateFlags::VISIBLE) {
		log::warn!("не удалось сохранить размер окна перед обновлением: {err}");
	}
}

#[cfg(not(desktop))]
fn save_window_state<R: Runtime>(_app: &AppHandle<R>) {}

/// Ошибка проверки: интерфейс её пропускает (лаунчер работает без обновления).
fn check_error(err: tauri_plugin_updater::Error) -> AppError {
	AppError::coded(ErrorCode::UpdateUnavailable, err.to_string())
}

/// Ошибка установки.
fn install_error(err: tauri_plugin_updater::Error) -> AppError {
	use tauri_plugin_updater::Error as E;
	let code = match &err {
		E::Minisign(_)
		| E::Base64(_)
		| E::SignatureUtf8(_)
		| E::SignedVersionMismatch { .. }
		| E::MissingSignedVersion => ErrorCode::UpdateBadSignature,
		E::Reqwest(_) | E::Network(_) => ErrorCode::UpdateUnavailable,
		_ => ErrorCode::UpdateFailed,
	};
	AppError::coded(code, err.to_string())
}

fn io_error(err: std::io::Error) -> AppError {
	// ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL.
	if matches!(err.raw_os_error(), Some(39 | 112)) {
		return AppError::coded(ErrorCode::NotEnoughSpace, err.to_string());
	}
	AppError::storage(err.to_string())
}

pub fn info(update: &Update, downloaded: bool) -> UpdateInfo {
	let mut info = info_from(
		&update.version,
		&update.current_version,
		&update.target,
		update.body.as_deref(),
		&update.raw_json,
	);
	info.downloaded = downloaded;
	info
}

fn info_from(
	version: &str,
	current: &str,
	target: &str,
	body: Option<&str>,
	raw: &serde_json::Value,
) -> UpdateInfo {
	let text = |value: Option<&str>| {
		value
			.map(str::trim)
			.filter(|s| !s.is_empty())
			.map(str::to_owned)
	};
	UpdateInfo {
		version: version.to_owned(),
		current_version: current.to_owned(),
		date: text(raw.get("pub_date").and_then(|v| v.as_str())),
		notes: text(raw.get("notes").and_then(|v| v.as_str())).or_else(|| text(body)),
		size: raw
			.get("platforms")
			.and_then(|p| p.get(target).or_else(|| p.get("windows-x86_64")))
			.and_then(|p| p.get("size"))
			.and_then(|s| s.as_u64()),
		downloaded: false,
	}
}

/// Имя файла пакета. Версия из `latest.json` — чужие данные: только безопасные символы.
fn package_name(version: &str) -> Option<String> {
	let safe = !version.is_empty()
		&& version.len() <= 64
		&& version
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
		&& !version.contains("..");
	safe.then(|| format!("update-{version}.exe"))
}

/// Версия по имени файла пакета (`update-<версия>.exe` или `.exe.part`).
fn package_version(name: &str) -> Option<&str> {
	let rest = name.strip_prefix("update-")?;
	rest.strip_suffix(".exe.part")
		.or_else(|| rest.strip_suffix(".exe"))
}

/// Убирает пакеты версий не новее текущей: обновление поставлено или устарело.
fn cleanup(dir: &Path, current: &str) {
	let Ok(current) = semver::Version::parse(current) else {
		return;
	};
	let Ok(entries) = std::fs::read_dir(dir) else {
		return;
	};
	for entry in entries.flatten() {
		let name = entry.file_name();
		let name = name.to_string_lossy();
		let newer = package_version(&name)
			.and_then(|v| semver::Version::parse(v).ok())
			.is_some_and(|v| v > current);
		if !newer && let Err(err) = std::fs::remove_file(entry.path()) {
			log::warn!("не удалось убрать старый пакет обновления {name}: {err}");
		}
	}
}

/// Пакет этой версии уже лежит и проходит проверку подписи.
async fn verified_package(update: &Update, path: &Path) -> Option<Vec<u8>> {
	let bytes = tokio::fs::read(path).await.ok()?;
	match update.verify(&bytes) {
		Ok(()) => Some(bytes),
		Err(err) => {
			log::warn!(
				"скачанный пакет {} не прошёл проверку: {err}",
				update.version
			);
			let _ = tokio::fs::remove_file(path).await;
			None
		}
	}
}

/// Игра запускается или идёт: установщик закроет лаунчер, и лог с временем сессии пропадут.
fn game_running(l: &Launcher) -> bool {
	l.game_running()
}

fn found(state: &Updates) -> Option<Update> {
	state.found.lock().ok().and_then(|f| f.clone())
}

async fn find<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Update>, AppError> {
	let updater = updater(app)?;
	match tokio::time::timeout(CHECK_TIMEOUT, updater.check()).await {
		Ok(result) => result.map_err(|err| {
			log::warn!("проверка обновления не удалась: {err}");
			check_error(err)
		}),
		Err(_) => {
			log::warn!("проверка обновления: GitHub не ответил за {CHECK_TIMEOUT:?}");
			Err(AppError::coded(ErrorCode::UpdateUnavailable, "timeout"))
		}
	}
}

/// Найденное раньше обновление или новая проверка.
async fn current_update<R: Runtime>(
	app: &AppHandle<R>,
	state: &Updates,
) -> Result<Update, AppError> {
	if let Some(update) = found(state) {
		return Ok(update);
	}
	let update = find(app)
		.await?
		.ok_or_else(|| AppError::coded(ErrorCode::UpdateUnavailable, "no update"))?;
	if let Ok(mut slot) = state.found.lock() {
		*slot = Some(update.clone());
	}
	Ok(update)
}

/// Освобождает слот скачивания, даже если задача упала с паникой.
struct Slot<'a>(&'a Mutex<Option<CancellationToken>>);

impl Drop for Slot<'_> {
	fn drop(&mut self) {
		if let Ok(mut task) = self.0.lock() {
			*task = None;
		}
	}
}

/// Занимает слот скачивания/установки на время `run`.
async fn exclusive<F, T>(
	state: &Updates,
	run: impl FnOnce(CancellationToken) -> F,
) -> Result<T, AppError>
where
	F: std::future::Future<Output = Result<T, AppError>>,
{
	let (token, _slot) = {
		let mut task = state
			.task
			.lock()
			.map_err(|_| AppError::unknown("update task lock"))?;
		if task.is_some() {
			return Err(AppError::new(ErrorCode::UpdateInProgress));
		}
		let token = CancellationToken::new();
		*task = Some(token.clone());
		(token, Slot(&state.task))
	};
	run(token).await
}

fn fetch_error(err: fetch::FetchError) -> AppError {
	match err {
		fetch::FetchError::Cancelled => {
			log::info!("скачивание обновления отменено");
			AppError::new(ErrorCode::Cancelled)
		}
		fetch::FetchError::Network(text) => {
			log::warn!("обновление не скачалось: {text}");
			AppError::coded(ErrorCode::UpdateUnavailable, text)
		}
		fetch::FetchError::Io(err) => {
			log::warn!("обновление не записалось: {err}");
			io_error(err)
		}
	}
}

/// Скачивает пакет (или берёт уже скачанный) и возвращает его проверенные байты.
async fn download<R: Runtime>(
	app: &AppHandle<R>,
	state: &Updates,
	update: &Update,
	token: &CancellationToken,
) -> Result<Vec<u8>, AppError> {
	let name = package_name(&update.version)
		.ok_or_else(|| AppError::coded(ErrorCode::UpdateFailed, "bad version"))?;
	tokio::fs::create_dir_all(&state.dir)
		.await
		.map_err(io_error)?;
	let path = state.dir.join(&name);
	if let Some(bytes) = verified_package(update, &path).await {
		return Ok(bytes);
	}
	let part = state.dir.join(format!("{name}.part"));
	let client = crate::download::build_client()
		.map_err(|err| AppError::coded(ErrorCode::UpdateFailed, err.to_string()))?;
	log::info!("скачиваем обновление {}", update.version);
	let mut fresh = false;
	let bytes = loop {
		let fetched = fetch::fetch(
			&client,
			update.download_url.as_str(),
			&part,
			token,
			&fetch::RETRY_PAUSES,
			|progress| {
				let _ = app.emit(EVENT_PROGRESS, progress);
			},
		)
		.await
		.map_err(fetch_error)?;
		let bytes = tokio::fs::read(&part).await.map_err(io_error)?;
		match update.verify(&bytes) {
			Ok(()) => break bytes,
			Err(err) => {
				let _ = tokio::fs::remove_file(&part).await;
				// Докачанный файл мог склеиться неправильно: один раз пробуем целиком.
				if fetched.resumed && !fresh {
					log::warn!("докачанный пакет не прошёл проверку подписи, качаем заново: {err}");
					fresh = true;
					continue;
				}
				log::error!(
					"пакет обновления {} не прошёл проверку подписи: {err}",
					update.version
				);
				return Err(install_error(err));
			}
		}
	};
	tokio::fs::rename(&part, &path).await.map_err(io_error)?;
	Ok(bytes)
}

#[tauri::command]
pub async fn app_update_check<R: Runtime>(
	app: AppHandle<R>,
	state: State<'_, Updates>,
) -> Result<Option<UpdateInfo>, AppError> {
	if !enabled() {
		return Ok(None);
	}
	let update = find(&app).await?;
	let result = match &update {
		Some(u) => {
			let ready = match package_name(&u.version) {
				Some(name) => verified_package(u, &state.dir.join(name)).await.is_some(),
				None => false,
			};
			log::info!("найдено обновление {} → {}", u.current_version, u.version);
			Some(info(u, ready))
		}
		None => {
			log::debug!("обновлений нет");
			None
		}
	};
	if let Ok(mut slot) = state.found.lock() {
		*slot = update;
	}
	Ok(result)
}

#[tauri::command]
pub async fn app_update_download<R: Runtime>(
	app: AppHandle<R>,
	state: State<'_, Updates>,
) -> Result<(), AppError> {
	if !enabled() {
		return Err(AppError::coded(ErrorCode::UpdateFailed, "debug build"));
	}
	let update = current_update(&app, &state).await?;
	let state = &*state;
	exclusive(state, |token| async move {
		download(&app, state, &update, &token).await.map(|_| ())
	})
	.await
}

#[tauri::command]
pub async fn app_update_install<R: Runtime>(
	app: AppHandle<R>,
	state: State<'_, Updates>,
	l: State<'_, Arc<Launcher>>,
) -> Result<(), AppError> {
	if !enabled() {
		return Err(AppError::coded(ErrorCode::UpdateFailed, "debug build"));
	}
	if game_running(&l) {
		return Err(AppError::new(ErrorCode::GameRunning));
	}
	let update = current_update(&app, &state).await?;
	let state = &*state;
	exclusive(state, |token| async move {
		let bytes = download(&app, state, &update, &token).await?;
		// За время скачивания игру могли запустить.
		if game_running(&l) {
			return Err(AppError::new(ErrorCode::GameRunning));
		}
		if token.is_cancelled() {
			return Err(AppError::new(ErrorCode::Cancelled));
		}
		state::set_pending(
			&state.root,
			state::Pending {
				version: update.version.clone(),
				notes: info(&update, true).notes,
			},
		);
		log::info!("устанавливаем обновление {}", update.version);
		// На Windows `install` запускает установщик NSIS (`/P /UPDATE /R`) и завершает процесс.
		update.install(&bytes).map_err(|err| {
			log::error!("обновление не установилось: {err}");
			install_error(err)
		})
	})
	.await
}

#[tauri::command]
pub fn app_update_cancel(state: State<'_, Updates>) {
	if let Ok(task) = state.task.lock()
		&& let Some(token) = task.as_ref()
	{
		token.cancel();
	}
}

/// Этот запуск — первый после обновления: тост «Обновлено до …».
#[tauri::command]
pub fn app_last_update(state: State<'_, Updates>) -> Option<LastUpdate> {
	state.last.clone()
}

/// Перезапуск. На Windows новую версию запускает сам установщик.
#[tauri::command]
pub fn app_restart<R: Runtime>(app: AppHandle<R>) {
	save_window_state(&app);
	app.restart();
}

/// Регистрация состояния (в `setup`): запоминает версию запуска и убирает поставленные пакеты.
pub fn manage<R: Runtime>(app: &tauri::App<R>, layout: &crate::paths::Layout) {
	let current = app.package_info().version.to_string();
	let root = layout.root().to_path_buf();
	let dir = layout.cache_dir().join("updates");
	let last = state::on_start(&root, &current);
	if let Some(last) = &last {
		log::info!("лаунчер обновлён: {} → {}", last.updated_from, last.version);
	}
	cleanup(&dir, &current);
	app.manage(Updates {
		root,
		dir,
		found: Mutex::new(None),
		task: Mutex::new(None),
		last,
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn classifies_errors() {
		use tauri_plugin_updater::Error as E;
		assert_eq!(
			check_error(E::ReleaseNotFound).code,
			ErrorCode::UpdateUnavailable
		);
		assert_eq!(
			install_error(E::Network("404".into())).code,
			ErrorCode::UpdateUnavailable
		);
		assert_eq!(
			install_error(E::MissingSignedVersion).code,
			ErrorCode::UpdateBadSignature
		);
		assert_eq!(
			install_error(E::SignedVersionMismatch {
				signed: "0.1.0".into(),
				announced: "0.2.0".into()
			})
			.code,
			ErrorCode::UpdateBadSignature
		);
		assert_eq!(
			install_error(E::BinaryNotFoundInArchive).code,
			ErrorCode::UpdateFailed
		);
		assert_eq!(
			io_error(std::io::Error::from_raw_os_error(112)).code,
			ErrorCode::NotEnoughSpace
		);
	}

	#[test]
	fn reads_latest_json() {
		let raw = serde_json::json!({
			"version": "0.2.0",
			"notes": "  Что нового  ",
			"pub_date": "2026-11-20T10:00:00Z",
			"platforms": { "windows-x86_64": { "url": "https://x", "signature": "s", "size": 123 } }
		});
		let i = info_from("0.2.0", "0.1.0", "windows-x86_64", None, &raw);
		assert_eq!(i.notes.as_deref(), Some("Что нового"));
		assert_eq!(i.date.as_deref(), Some("2026-11-20T10:00:00Z"));
		assert_eq!(i.size, Some(123));
		let raw = serde_json::json!({ "version": "0.2.0", "notes": "", "platforms": {} });
		let i = info_from("0.2.0", "0.1.0", "windows-x86_64", Some("тело"), &raw);
		assert_eq!(i.notes.as_deref(), Some("тело"));
		assert_eq!(i.size, None);
		assert_eq!(i.date, None);
	}

	#[test]
	fn package_names() {
		assert_eq!(package_name("0.2.0").as_deref(), Some("update-0.2.0.exe"));
		assert_eq!(
			package_name("1.0.0-beta.1+b2").as_deref(),
			Some("update-1.0.0-beta.1+b2.exe")
		);
		assert_eq!(package_name("../x"), None);
		assert_eq!(package_name("a\\b"), None);
		assert_eq!(package_name(""), None);
		assert_eq!(package_version("update-0.2.0.exe"), Some("0.2.0"));
		assert_eq!(package_version("update-0.2.0.exe.part"), Some("0.2.0"));
		assert_eq!(package_version("other.exe"), None);
	}

	#[test]
	fn cleanup_keeps_newer_packages() {
		let dir = tempfile::tempdir().unwrap();
		for name in [
			"update-0.1.0.exe",
			"update-0.2.0.exe.part",
			"update-0.3.0.exe",
			"junk.txt",
		] {
			std::fs::write(dir.path().join(name), b"x").unwrap();
		}
		cleanup(dir.path(), "0.2.0");
		let mut left: Vec<_> = std::fs::read_dir(dir.path())
			.unwrap()
			.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
			.collect();
		left.sort();
		assert_eq!(left, ["update-0.3.0.exe"]);
	}

	#[test]
	fn serializes_for_frontend() {
		let info = UpdateInfo {
			version: "0.2.0".into(),
			current_version: "0.1.0".into(),
			date: Some("2026-11-20T10:00:00Z".into()),
			notes: None,
			size: None,
			downloaded: false,
		};
		let json = serde_json::to_value(&info).unwrap();
		assert_eq!(json["currentVersion"], "0.1.0");
		assert!(json["notes"].is_null());
		let json = serde_json::to_value(fetch::UpdateProgress {
			downloaded: 5,
			total: Some(10),
			speed_bps: None,
			eta_secs: Some(3),
			slow: false,
			reconnect: Some(fetch::Reconnect { attempt: 1, of: 3 }),
		})
		.unwrap();
		assert_eq!(json["downloaded"], 5);
		assert_eq!(json["etaSecs"], 3);
		assert!(json["speedBps"].is_null());
		assert_eq!(json["reconnect"]["of"], 3);
	}
}
