//! Работа в трее во время игры: крестик прячет окно, пока идёт игра, а когда все игры
//! закрылись (и загрузки закончились), лаунчер завершается сам.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

use crate::MAIN_WINDOW;
use crate::lang::{self, Lang};
use crate::launcher::Launcher;

const TRAY_ID: &str = "main";
const MENU_OPEN: &str = "open";
const MENU_QUIT: &str = "quit";
/// Как часто проверять, закончились ли загрузки, перед выходом.
const WAIT_POLL: Duration = Duration::from_secs(2);
/// Окно спрятано в трей.
pub const EVENT_HIDDEN: &str = "app-hidden-to-tray";
/// «Выйти» из трея, пока идут игры: интерфейс спрашивает, закрыть ли их (`app_quit`).
pub const EVENT_QUIT_REQUESTED: &str = "app-quit-requested";
/// Сколько ждать, пока остановленные игры закроются, перед выходом.
const QUIT_WAIT: Duration = Duration::from_secs(8);
const QUIT_POLL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct Tray {
	/// Окно спрятано крестиком во время игры.
	hidden: AtomicBool,
	/// Уже прятали в этом запуске.
	hidden_before: AtomicBool,
	/// Ждём конца загрузок, чтобы выйти.
	waiting: AtomicBool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Hidden {
	/// Окно прячется в трей впервые за этот запуск лаунчера.
	first: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuitRequested {
	/// Сколько игр запускается или идёт.
	games: usize,
}

struct Texts {
	tooltip: &'static str,
	downloads: &'static str,
	open: &'static str,
	quit: &'static str,
}

fn texts(lang: Lang) -> Texts {
	match lang {
		Lang::Ru => Texts {
			tooltip: "LeeLauncher — идёт игра",
			downloads: "LeeLauncher — идут загрузки",
			open: "Открыть LeeLauncher",
			quit: "Выйти",
		},
		Lang::En => Texts {
			tooltip: "LeeLauncher — game running",
			downloads: "LeeLauncher — downloads in progress",
			open: "Open LeeLauncher",
			quit: "Quit",
		},
	}
}

fn game_running<R: Runtime>(app: &AppHandle<R>) -> bool {
	app.try_state::<Arc<Launcher>>()
		.is_some_and(|l| l.game_running())
}

/// Идут загрузки или скачивание обновления: выходить рано.
fn work_pending<R: Runtime>(app: &AppHandle<R>) -> bool {
	let downloads = app
		.try_state::<Arc<Launcher>>()
		.is_some_and(|l| l.downloads.has_active());
	let update = app
		.try_state::<crate::updates::Updates>()
		.is_some_and(|u| u.busy());
	downloads || update
}

fn is_hidden<R: Runtime>(app: &AppHandle<R>) -> bool {
	app.try_state::<Tray>()
		.is_some_and(|s| s.hidden.load(Ordering::Relaxed))
}

/// Подсказка значка: игра, если идёт, иначе загрузки.
fn tooltip<R: Runtime>(app: &AppHandle<R>, t: &Texts) -> &'static str {
	if !game_running(app) && work_pending(app) {
		t.downloads
	} else {
		t.tooltip
	}
}

fn main_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
	app.get_webview_window(MAIN_WINDOW)
}

fn menu<R: Runtime>(app: &AppHandle<R>, t: &Texts) -> tauri::Result<Menu<R>> {
	let open = MenuItem::with_id(app, MENU_OPEN, t.open, true, None::<&str>)?;
	let quit = MenuItem::with_id(app, MENU_QUIT, t.quit, true, None::<&str>)?;
	Menu::with_items(app, &[&open, &quit])
}

/// Крестик, Alt+F4 и прочие закрытия главного окна. `true` — окно спрятано, закрывать не надо.
pub fn on_close_requested<R: Runtime>(window: &tauri::Window<R>) -> bool {
	if window.label() != MAIN_WINDOW {
		return false;
	}
	let app = window.app_handle();
	if !game_running(app) {
		return false;
	}
	hide(app);
	true
}

fn hide<R: Runtime>(app: &AppHandle<R>) {
	let Some(state) = app.try_state::<Tray>() else {
		return;
	};
	if let Err(err) = show_icon(app) {
		// Без значка спрятанное окно не вернуть ничем, кроме второго запуска: лучше не прятать.
		log::warn!("трей: не удалось показать значок: {err}");
		return;
	}
	if let Some(window) = main_window(app)
		&& let Err(err) = window.hide()
	{
		log::warn!("трей: не удалось спрятать окно: {err}");
		remove_icon(app);
		return;
	}
	state.hidden.store(true, Ordering::Relaxed);
	let first = !state.hidden_before.swap(true, Ordering::Relaxed);
	log::info!("окно спрятано в трей: идёт игра");
	if let Err(err) = app.emit(EVENT_HIDDEN, Hidden { first }) {
		log::warn!("не удалось отправить {EVENT_HIDDEN}: {err}");
	}
	// Игра могла закрыться, пока окно пряталось: её событие видело окно ещё открытым.
	try_exit(app);
}

fn show_icon<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
	let t = texts(lang::ui());
	if let Some(tray) = app.tray_by_id(TRAY_ID) {
		tray.set_menu(Some(menu(app, &t)?))?;
		tray.set_tooltip(Some(tooltip(app, &t)))?;
		return tray.set_visible(true);
	}
	let mut builder = TrayIconBuilder::with_id(TRAY_ID)
		.tooltip(t.tooltip)
		.menu(&menu(app, &t)?)
		.show_menu_on_left_click(false)
		.on_menu_event(|app, event| match event.id().as_ref() {
			MENU_OPEN => show(app),
			MENU_QUIT => quit(app),
			_ => {}
		})
		.on_tray_icon_event(|tray, event| {
			if let TrayIconEvent::Click {
				button: MouseButton::Left,
				button_state: MouseButtonState::Up,
				..
			} = event
			{
				show(tray.app_handle());
			}
		});
	if let Some(icon) = app.default_window_icon() {
		builder = builder.icon(icon.clone());
	}
	builder.build(app)?;
	Ok(())
}

fn remove_icon<R: Runtime>(app: &AppHandle<R>) {
	if let Some(tray) = app.tray_by_id(TRAY_ID) {
		let _ = tray.set_visible(false);
	}
}

/// Показать главное окно: значок трея, второй запуск.
pub fn show<R: Runtime>(app: &AppHandle<R>) {
	if let Some(state) = app.try_state::<Tray>() {
		state.hidden.store(false, Ordering::Relaxed);
	}
	remove_icon(app);
	if let Some(window) = main_window(app) {
		let _ = window.show();
		let _ = window.unminimize();
		let _ = window.set_focus();
	}
}

/// «Выйти» из трея. Идут игры — показываем окно: интерфейс спросит, закрыть ли их.
fn quit<R: Runtime>(app: &AppHandle<R>) {
	let games = app
		.try_state::<Arc<Launcher>>()
		.map(|l| {
			let running = l.games.running().len();
			if running == 0 && l.instances.any_running() {
				1
			} else {
				running
			}
		})
		.unwrap_or(0);
	if games == 0 {
		log::info!("выход из трея");
		remove_icon(app);
		app.exit(0);
		return;
	}
	log::info!("выход из трея: идут игры ({games}), спрашиваем");
	show(app);
	if let Err(err) = app.emit(EVENT_QUIT_REQUESTED, QuitRequested { games }) {
		log::warn!("не удалось отправить {EVENT_QUIT_REQUESTED}: {err}");
	}
}

/// Выход из лаунчера: сначала останавливает все игры (с потомками) и ждёт их конца,
/// чтобы записалось время игры.
#[tauri::command]
pub async fn app_quit(app: AppHandle) -> Result<(), crate::error::AppError> {
	if let Some(l) = app.try_state::<Arc<Launcher>>() {
		let l = l.inner().clone();
		let stopped = l.instances.kill_all();
		if stopped > 0 {
			log::info!("выход: останавливаем игры ({stopped})");
			let deadline = tokio::time::Instant::now() + QUIT_WAIT;
			while l.game_running() && tokio::time::Instant::now() < deadline {
				tokio::time::sleep(QUIT_POLL).await;
			}
			if l.game_running() {
				log::warn!("выход: игры не закрылись за {QUIT_WAIT:?}");
			}
		}
	}
	log::info!("выход по просьбе пользователя");
	remove_icon(&app);
	app.exit(0);
	Ok(())
}

/// Игра закрылась. Вылет или ошибка запуска — показываем окно, чтобы было видно, что случилось;
/// обычный выход — пробуем завершить лаунчер.
pub fn on_game_exited<R: Runtime>(app: &AppHandle<R>, failed: bool) {
	if !is_hidden(app) {
		return;
	}
	if failed {
		log::info!("игра вылетела, показываем окно из трея");
		show(app);
		return;
	}
	try_exit(app);
}

/// Окно в трее, игр нет: выходим, а если идут загрузки — ждём их конца.
fn try_exit<R: Runtime>(app: &AppHandle<R>) {
	if !is_hidden(app) || game_running(app) {
		return;
	}
	if !work_pending(app) {
		log::info!("игры закрыты, окно в трее — выходим");
		remove_icon(app);
		app.exit(0);
		return;
	}
	let Some(state) = app.try_state::<Tray>() else {
		return;
	};
	if let Some(tray) = app.tray_by_id(TRAY_ID) {
		let _ = tray.set_tooltip(Some(texts(lang::ui()).downloads));
	}
	if state.waiting.swap(true, Ordering::Relaxed) {
		return;
	}
	log::info!("игры закрыты, ждём конца загрузок, чтобы выйти");
	let app = app.clone();
	tauri::async_runtime::spawn(async move {
		loop {
			tokio::time::sleep(WAIT_POLL).await;
			// Окно открыли или снова запустили игру — ждать нечего, решит следующее закрытие.
			if !is_hidden(&app) || game_running(&app) || !work_pending(&app) {
				break;
			}
		}
		if let Some(state) = app.try_state::<Tray>() {
			state.waiting.store(false, Ordering::Relaxed);
		}
		relabel(&app);
		try_exit(&app);
	});
}

/// Сменился язык интерфейса: переписать меню и подсказку, если значок уже есть.
pub fn relabel<R: Runtime>(app: &AppHandle<R>) {
	let Some(tray) = app.tray_by_id(TRAY_ID) else {
		return;
	};
	let t = texts(lang::ui());
	let result = menu(app, &t).and_then(|m| {
		tray.set_menu(Some(m))?;
		tray.set_tooltip(Some(tooltip(app, &t)))
	});
	if let Err(err) = result {
		log::warn!("трей: не удалось сменить язык: {err}");
	}
}
