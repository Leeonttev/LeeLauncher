//! Статус в Discord через его локальный IPC: что за сборка идёт и сколько. Без сети и SDK.
//! Discord не запущен или закрылся — тихо пробуем снова, пользователю ничего не показываем.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use discord_rich_presence::activity::{Activity, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tokio::sync::Notify;

use crate::game::ProcessState;
use crate::instances::Instance;
use crate::lang::{self, Lang};
use crate::launcher::Launcher;
use crate::meta::LoaderKind;

/// Application ID приложения «LeeLauncher» в Discord Developer Portal (не секрет).
/// Переменная `LEELAUNCHER_DISCORD_APP_ID` при сборке заменяет его; пусто — статус выключен.
const APP_ID: &str = match option_env!("LEELAUNCHER_DISCORD_APP_ID") {
	Some(id) => id,
	None => "1571634948295753788",
};

const SITE_URL: &str = "https://launcher.leeonttev.pro/";
const GITHUB_URL: &str = "https://github.com/leeonttev/LeeLauncher";
/// Пауза между попытками подключиться к Discord.
const RETRY: Duration = Duration::from_secs(20);
/// Как часто повторять статус, чтобы заметить закрытый или перезапущенный Discord.
const KEEPALIVE: Duration = Duration::from_secs(20);
/// Сколько ждать ответа Discord: завис — считаем связь потерянной.
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Discord не принимает строки короче 2 и длиннее 128 символов.
const TEXT_MAX: usize = 128;

fn app_id() -> Option<&'static str> {
	parse_app_id(APP_ID)
}

fn parse_app_id(raw: &str) -> Option<&str> {
	let id = raw.trim();
	(!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// Что показать в Discord. `None` у [`Presence`] — ничего (выключено).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desired {
	pub details: String,
	pub state: String,
	/// Начало игры, секунды Unix. Нет — без таймера.
	pub start: Option<i64>,
	/// Маленькая картинка: (ассет, подпись).
	pub small: Option<(&'static str, &'static str)>,
	pub buttons: [(&'static str, &'static str); 2],
}

/// Сведения об идущей игре для статуса.
pub struct Playing<'a> {
	pub instance: &'a Instance,
	pub start: Option<i64>,
}

fn loader_label(kind: LoaderKind) -> &'static str {
	match kind {
		LoaderKind::Vanilla => "Vanilla",
		LoaderKind::Fabric => "Fabric",
		LoaderKind::Forge => "Forge",
		LoaderKind::Neoforge => "NeoForge",
		LoaderKind::Quilt => "Quilt",
	}
}

fn clip(text: &str) -> String {
	text.chars().take(TEXT_MAX).collect()
}

/// Статус по идущей игре (или её отсутствию) и настройкам.
pub fn describe(playing: Option<Playing<'_>>, show_instance: bool, lang: Lang) -> Desired {
	let buttons = match lang {
		Lang::Ru => [("Скачать LeeLauncher", SITE_URL), ("GitHub", GITHUB_URL)],
		Lang::En => [("Download LeeLauncher", SITE_URL), ("GitHub", GITHUB_URL)],
	};
	let Some(Playing { instance, start }) = playing else {
		let (details, state) = match lang {
			Lang::Ru => ("В LeeLauncher", "Выбирает сборку"),
			Lang::En => ("In LeeLauncher", "Choosing an instance"),
		};
		return Desired {
			details: details.to_owned(),
			state: state.to_owned(),
			start: None,
			small: None,
			buttons,
		};
	};
	let name = instance.name.trim();
	let details = if show_instance && name.chars().count() >= 2 {
		clip(name)
	} else {
		"Minecraft".to_owned()
	};
	let kind = instance.loader.kind;
	let state = match kind {
		LoaderKind::Vanilla => format!("Minecraft {}", instance.game_version),
		_ => format!(
			"Minecraft {} · {}",
			instance.game_version,
			loader_label(kind)
		),
	};
	Desired {
		details,
		state: clip(&state),
		start,
		small: Some((kind.as_str(), loader_label(kind))),
		buttons,
	}
}

/// Статус Discord в `app.manage`. Изменения просят через [`refresh`].
pub struct Presence {
	notify: Arc<Notify>,
	connected: Arc<AtomicBool>,
	configured: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceStatus {
	/// В сборке задан Application ID: без него статус не работает.
	pub configured: bool,
	/// Сейчас есть связь с Discord.
	pub connected: bool,
}

impl Presence {
	/// Запускает свою задачу и поток связи с Discord (если задан Application ID).
	pub fn start(l: Arc<Launcher>) -> Self {
		let notify = Arc::new(Notify::new());
		let connected = Arc::new(AtomicBool::new(false));
		let Some(id) = app_id() else {
			log::debug!("Discord: Application ID не задан, статус выключен");
			return Self {
				notify,
				connected,
				configured: false,
			};
		};
		let (tx, rx) = mpsc::channel::<Option<Desired>>();
		let flag = connected.clone();
		let spawned = std::thread::Builder::new()
			.name("discord-presence".into())
			.spawn(move || ipc_loop(id, rx, flag));
		if let Err(err) = spawned {
			log::warn!("Discord: не удалось запустить поток: {err}");
			return Self {
				notify,
				connected,
				configured: true,
			};
		}
		let wake = notify.clone();
		tauri::async_runtime::spawn(async move {
			let mut last: Option<Option<Desired>> = None;
			loop {
				let next = current(&l).await;
				if last.as_ref() != Some(&next) {
					if tx.send(next.clone()).is_err() {
						break;
					}
					last = Some(next);
				}
				wake.notified().await;
			}
		});
		Self {
			notify,
			connected,
			configured: true,
		}
	}

	pub fn status(&self) -> PresenceStatus {
		PresenceStatus {
			configured: self.configured,
			connected: self.connected.load(Ordering::Relaxed),
		}
	}
}

/// Пересчитать статус: игра запустилась или закрылась, сменились настройки, сборка или язык.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
	if let Some(presence) = app.try_state::<Presence>() {
		presence.notify.notify_one();
	}
}

/// Что показать сейчас: последняя запущенная игра или «в лаунчере».
async fn current(l: &Launcher) -> Option<Desired> {
	let settings = l.settings.get();
	if !settings.discord_presence {
		return None;
	}
	let game = l
		.games
		.running()
		.into_iter()
		.filter(|g| g.state == ProcessState::Running)
		.max_by(|a, b| a.started_at.cmp(&b.started_at));
	let instance = match &game {
		Some(game) => l.instances.get(&game.instance_id).await.ok(),
		None => None,
	};
	let playing = instance.as_ref().map(|instance| Playing {
		instance,
		start: game.as_ref().and_then(|g| {
			chrono::DateTime::parse_from_rfc3339(&g.started_at)
				.ok()
				.map(|t| t.timestamp())
		}),
	});
	Some(describe(
		playing,
		settings.discord_show_instance,
		lang::ui(),
	))
}

fn activity(d: &Desired) -> Activity<'_> {
	let mut assets = Assets::new().large_image("logo").large_text("LeeLauncher");
	if let Some((key, text)) = d.small {
		assets = assets.small_image(key).small_text(text);
	}
	let mut activity = Activity::new()
		.details(d.details.as_str())
		.state(d.state.as_str())
		.assets(assets)
		.buttons(
			d.buttons
				.iter()
				.map(|(label, url)| Button::new(*label, *url))
				.collect(),
		);
	if let Some(start) = d.start {
		activity = activity.timestamps(Timestamps::new().start(start));
	}
	activity
}

/// Отправляет статус и читает ответ Discord, чтобы его не копить в канале.
fn send(client: &mut DiscordIpcClient, d: &Desired) -> Result<(), String> {
	client
		.set_activity(activity(d))
		.map_err(|e| e.to_string())?;
	let (op, reply) = client.recv().map_err(|e| e.to_string())?;
	// 2 — Discord закрывает соединение (например, неверный Application ID).
	if op == 2 {
		return Err(format!("Discord закрыл соединение: {reply}"));
	}
	if reply.get("evt").and_then(|e| e.as_str()) == Some("ERROR") {
		log::debug!("Discord отклонил статус: {reply}");
	}
	Ok(())
}

fn clear(client: &mut DiscordIpcClient) {
	if client.clear_activity().is_ok() {
		let _ = client.recv();
	}
	let _ = client.close();
}

/// Чтение из канала Discord не умеет тайм-аут, поэтому обмен идёт в отдельном потоке.
/// Не дождались — поток бросаем: он сам закончится, когда Discord закроет канал.
fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
	let (tx, rx) = mpsc::channel();
	std::thread::Builder::new()
		.name("discord-io".into())
		.spawn(move || {
			let _ = tx.send(f());
		})
		.ok()?;
	rx.recv_timeout(IO_TIMEOUT).ok()
}

/// Поток связи с Discord. Чтение и запись канала блокирующие, поэтому не в tokio.
fn ipc_loop(id: &'static str, rx: mpsc::Receiver<Option<Desired>>, connected: Arc<AtomicBool>) {
	let mut client: Option<DiscordIpcClient> = None;
	let mut want: Option<Desired> = None;
	let mut next_try = Instant::now();
	loop {
		let wait = match (&want, &client) {
			(None, _) => None,
			(Some(_), Some(_)) => Some(KEEPALIVE),
			(Some(_), None) => Some(next_try.saturating_duration_since(Instant::now())),
		};
		let message = match wait {
			None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
			Some(wait) => rx.recv_timeout(wait),
		};
		match message {
			Ok(next) => want = next,
			// Пора повторить попытку или проверить, жив ли Discord (статус отправится снова).
			Err(mpsc::RecvTimeoutError::Timeout) => {}
			Err(mpsc::RecvTimeoutError::Disconnected) => break,
		}

		let Some(desired) = &want else {
			if let Some(mut old) = client.take() {
				with_timeout(move || clear(&mut old));
				connected.store(false, Ordering::Relaxed);
				log::debug!("Discord: статус выключен");
			}
			continue;
		};
		if client.is_none() {
			if Instant::now() < next_try {
				continue;
			}
			let connect = with_timeout(move || {
				let mut fresh = DiscordIpcClient::new(id);
				fresh.connect().map(|()| fresh).map_err(|e| e.to_string())
			})
			.unwrap_or_else(|| Err("Discord не ответил".to_owned()));
			match connect {
				Ok(fresh) => {
					log::debug!("Discord: подключились");
					client = Some(fresh);
				}
				Err(err) => {
					log::debug!("Discord: не подключились ({err}), повтор через {RETRY:?}");
					next_try = Instant::now() + RETRY;
					continue;
				}
			}
		}
		if let Some(mut active) = client.take() {
			let desired = desired.clone();
			let sent = with_timeout(move || {
				let result = send(&mut active, &desired);
				(active, result)
			});
			let result = match sent {
				Some((active, result)) => {
					client = Some(active);
					result
				}
				None => Err("Discord не ответил".to_owned()),
			};
			match result {
				Ok(()) => connected.store(true, Ordering::Relaxed),
				Err(err) => {
					log::debug!("Discord: связь потеряна ({err}), повтор через {RETRY:?}");
					client = None;
					connected.store(false, Ordering::Relaxed);
					next_try = Instant::now() + RETRY;
				}
			}
		}
	}
}

#[tauri::command]
pub fn discord_status(presence: State<'_, Presence>) -> PresenceStatus {
	presence.status()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::instances::Loader;

	fn instance(name: &str, kind: LoaderKind) -> Instance {
		let mut value: Instance = serde_json::from_value(serde_json::json!({
			"id": "a",
			"name": name,
			"gameVersion": "1.21.1",
			"loader": { "kind": "vanilla" },
			"createdAt": "2026-11-15T10:00:00Z",
			"installState": "installed"
		}))
		.unwrap();
		value.loader = Loader {
			kind,
			version: None,
		};
		value
	}

	#[test]
	fn idle_text_by_language() {
		let ru = describe(None, true, Lang::Ru);
		assert_eq!(ru.details, "В LeeLauncher");
		assert_eq!(ru.state, "Выбирает сборку");
		assert_eq!(ru.start, None);
		assert_eq!(ru.buttons[0], ("Скачать LeeLauncher", SITE_URL));
		let en = describe(None, true, Lang::En);
		assert_eq!(en.buttons[0].0, "Download LeeLauncher");
		assert_eq!(en.buttons[1], ("GitHub", GITHUB_URL));
	}

	#[test]
	fn playing_text() {
		let i = instance("Мои моды", LoaderKind::Fabric);
		let d = describe(
			Some(Playing {
				instance: &i,
				start: Some(1_700_000_000),
			}),
			true,
			Lang::Ru,
		);
		assert_eq!(d.details, "Мои моды");
		assert_eq!(d.state, "Minecraft 1.21.1 · Fabric");
		assert_eq!(d.start, Some(1_700_000_000));
		assert_eq!(d.small, Some(("fabric", "Fabric")));

		let hidden = describe(
			Some(Playing {
				instance: &i,
				start: None,
			}),
			false,
			Lang::En,
		);
		assert_eq!(hidden.details, "Minecraft");

		let vanilla = instance("x", LoaderKind::Vanilla);
		let d = describe(
			Some(Playing {
				instance: &vanilla,
				start: None,
			}),
			true,
			Lang::Ru,
		);
		// Discord не примет строку из одного символа.
		assert_eq!(d.details, "Minecraft");
		assert_eq!(d.state, "Minecraft 1.21.1");
		assert_eq!(d.small, Some(("vanilla", "Vanilla")));
	}

	#[test]
	fn activity_shape() {
		let i = instance("Сборка", LoaderKind::Neoforge);
		let d = describe(
			Some(Playing {
				instance: &i,
				start: Some(5),
			}),
			true,
			Lang::Ru,
		);
		let json = serde_json::to_value(activity(&d)).unwrap();
		assert_eq!(json["details"], "Сборка");
		assert_eq!(json["assets"]["large_image"], "logo");
		assert_eq!(json["assets"]["small_image"], "neoforge");
		assert_eq!(json["timestamps"]["start"], 5);
		assert_eq!(json["buttons"][1]["url"], GITHUB_URL);
	}

	#[test]
	fn app_id_must_be_digits() {
		assert_eq!(parse_app_id(""), None);
		assert_eq!(parse_app_id("  "), None);
		assert_eq!(parse_app_id("YOUR_ID"), None);
		assert_eq!(parse_app_id(" 1234567890 "), Some("1234567890"));
	}
}
