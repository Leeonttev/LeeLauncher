use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{
	AppHandle, LogicalPosition, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
	WindowEvent,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::api::AUTH_REPLY_URL;
use super::error::{AuthError, Step};
use crate::MAIN_WINDOW;
use crate::lang::Lang;
#[cfg(not(target_os = "macos"))]
use crate::paths;

pub const LOGIN_WINDOW: &str = "microsoft-login";
const WIDTH: f64 = 520.0;
const HEIGHT: f64 = 680.0;
/// Сколько ждать пользователя в окне входа.
const TIMEOUT: Duration = Duration::from_secs(15 * 60);

type Outcome = Result<String, AuthError>;
type Sender = Arc<Mutex<Option<oneshot::Sender<Outcome>>>>;

fn send(tx: &Sender, outcome: Outcome) {
	if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
		let _ = tx.send(outcome);
	}
}

/// Разбирает адрес возврата Microsoft: `code` — успех, `error` — отказ.
/// Возвращает `None`, если это не адрес возврата.
fn parse_redirect(url: &Url) -> Option<Outcome> {
	let reply = Url::parse(AUTH_REPLY_URL).ok()?;
	if url.scheme() != reply.scheme() || url.host() != reply.host() || url.path() != reply.path() {
		return None;
	}
	let param = |name: &str| {
		url.query_pairs()
			.find(|(k, _)| k == name)
			.map(|(_, v)| v.into_owned())
	};
	if let Some(code) = param("code") {
		return Some(Ok(code));
	}
	Some(match param("error").as_deref() {
		Some("access_denied") | None => Err(AuthError::Cancelled),
		Some(error) => Err(AuthError::unexpected(
			Step::LoginWindow,
			format!(
				"{error}: {}",
				param("error_description").unwrap_or_default()
			),
		)),
	})
}

/// Заголовок окна входа — единственный текст интерфейса, который показывает само ядро.
fn title(lang: Lang) -> &'static str {
	match lang {
		Lang::Ru => "Вход в Microsoft",
		Lang::En => "Sign in to Microsoft",
	}
}

/// Язык страницы входа: login.live.com понимает `mkt` и ставит его выше Accept-Language.
fn set_market(url: &mut Url, lang: Lang) {
	let market = lang.microsoft_market();
	// Без `mkt` в адресе остальную строку запроса не перекодируем.
	if !url.query_pairs().any(|(k, _)| k == "mkt") {
		url.query_pairs_mut().append_pair("mkt", market);
		return;
	}
	let pairs: Vec<(String, String)> = url
		.query_pairs()
		.filter(|(k, _)| k != "mkt")
		.map(|(k, v)| (k.into_owned(), v.into_owned()))
		.collect();
	url.query_pairs_mut()
		.clear()
		.extend_pairs(pairs)
		.append_pair("mkt", market);
}

/// Закрывает окно входа, если оно открыто. Ожидающий вход завершится с `cancelled`.
pub fn close(app: &AppHandle) {
	if let Some(window) = app.get_webview_window(LOGIN_WINDOW) {
		let _ = window.destroy();
	}
}

/// Закрывает прошлое окно входа и ждёт, пока оно исчезнет: окно с тем же label нельзя создать раньше.
async fn close_and_wait(app: &AppHandle) {
	close(app);
	for _ in 0..50 {
		if app.get_webview_window(LOGIN_WINDOW).is_none() {
			return;
		}
		tokio::time::sleep(Duration::from_millis(20)).await;
	}
}

/// Позиция окна входа по центру над главным окном (логические координаты).
fn centered_over(main: &WebviewWindow) -> Option<LogicalPosition<f64>> {
	if main.is_minimized().unwrap_or(true) || !main.is_visible().unwrap_or(false) {
		return None;
	}
	let scale = main.scale_factor().ok()?;
	let pos = main.outer_position().ok()?.to_logical::<f64>(scale);
	let size = main.outer_size().ok()?.to_logical::<f64>(scale);
	Some(LogicalPosition::new(
		pos.x + (size.width - WIDTH) / 2.0,
		(pos.y + (size.height - HEIGHT) / 2.0).max(pos.y),
	))
}

/// Закрывает именно своё окно (не по label), даже если future бросили.
struct CloseOnDrop {
	window: WebviewWindow,
	main: Option<WebviewWindow>,
}

impl Drop for CloseOnDrop {
	fn drop(&mut self) {
		let _ = self.window.destroy();
		if let Some(main) = &self.main {
			let _ = main.set_focus();
		}
	}
}

/// Окно входа Microsoft, ждёт код авторизации. InPrivate — чтобы можно было войти в другую
/// учётную запись. В capabilities окна нет: страница внешняя.
pub async fn run(
	app: &AppHandle,
	auth_url: &str,
	lang: Lang,
	cancel: &CancellationToken,
) -> Result<String, AuthError> {
	let mut url = Url::parse(auth_url)
		.map_err(|err| AuthError::unexpected(Step::LoginWindow, format!("адрес входа: {err}")))?;
	set_market(&mut url, lang);

	close_and_wait(app).await;
	if cancel.is_cancelled() {
		return Err(AuthError::Cancelled);
	}

	let (tx, rx) = oneshot::channel();
	let tx: Sender = Arc::new(Mutex::new(Some(tx)));
	let nav_tx = tx.clone();

	let window_err = |err: tauri::Error| AuthError::unexpected(Step::LoginWindow, err.to_string());

	let mut builder = WebviewWindowBuilder::new(app, LOGIN_WINDOW, WebviewUrl::External(url))
		.title(title(lang))
		.inner_size(WIDTH, HEIGHT)
		.min_inner_size(420.0, 520.0)
		.resizable(true)
		.maximizable(false)
		.minimizable(false)
		.focused(true)
		.incognito(true)
		.on_navigation(move |url| {
			log::debug!(
				"окно входа: переход на {}{}",
				url.host_str().unwrap_or(""),
				url.path()
			);
			match parse_redirect(url) {
				Some(outcome) => {
					send(&nav_tx, outcome);
					false
				}
				None => true,
			}
		});
	// Та же папка WebView2, что у главного окна: другая папка создала бы второй профиль на диске.
	// WKWebView папку не выбирает, приватность ему даёт `incognito`.
	#[cfg(not(target_os = "macos"))]
	{
		builder = builder.data_directory(paths::webview_data_dir());
	}

	let main = app.get_webview_window(MAIN_WINDOW);
	match main.as_ref().and_then(centered_over) {
		Some(pos) => builder = builder.position(pos.x, pos.y),
		None => builder = builder.center(),
	}
	if let Some(main) = &main {
		builder = builder.parent(main).map_err(window_err)?;
	}

	let window = builder.build().map_err(window_err)?;
	let window = CloseOnDrop { window, main };
	if cancel.is_cancelled() {
		log::debug!("окно входа: отмена пришла во время открытия");
		return Err(AuthError::Cancelled);
	}
	log::debug!("окно входа открыто");
	let close_tx = tx.clone();
	window.window.on_window_event(move |event| {
		if matches!(event, WindowEvent::Destroyed) {
			log::debug!("окно входа закрыто");
			send(&close_tx, Err(AuthError::Cancelled));
		}
	});
	drop(tx);

	tokio::select! {
		biased;
		_ = cancel.cancelled() => Err(AuthError::Cancelled),
		outcome = rx => match outcome {
			Ok(outcome) => outcome,
			// Отправитель пропал вместе с окном.
			Err(_) => Err(AuthError::Cancelled),
		},
		_ = tokio::time::sleep(TIMEOUT) => {
			log::info!("окно входа: время ожидания истекло");
			Err(AuthError::Timeout)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn parse(s: &str) -> Option<Outcome> {
		parse_redirect(&Url::parse(s).unwrap())
	}

	#[test]
	fn market_param() {
		let mut url = Url::parse(
			"https://login.live.com/oauth20_authorize.srf?client_id=1&redirect_uri=https%3A%2F%2Flogin.live.com%2Foauth20_desktop.srf&mkt=de-DE",
		)
		.unwrap();
		set_market(&mut url, Lang::En);
		let pairs: Vec<_> = url.query_pairs().into_owned().collect();
		assert_eq!(pairs.iter().filter(|(k, _)| k == "mkt").count(), 1);
		assert!(pairs.contains(&("mkt".into(), "en-US".into())));
		assert!(pairs.contains(&(
			"redirect_uri".into(),
			"https://login.live.com/oauth20_desktop.srf".into()
		)));
		set_market(&mut url, Lang::Ru);
		assert!(url.query_pairs().any(|(k, v)| k == "mkt" && v == "ru-RU"));
	}

	#[test]
	fn redirect_parsing() {
		assert!(parse("https://login.live.com/oauth20_authorize.srf?code=x").is_none());
		assert!(parse("https://example.com/oauth20_desktop.srf?code=x").is_none());
		assert_eq!(
			parse("https://login.live.com/oauth20_desktop.srf?code=M.C1_abc&state=1")
				.unwrap()
				.unwrap(),
			"M.C1_abc"
		);
		assert!(matches!(
			parse("https://login.live.com/oauth20_desktop.srf?error=access_denied"),
			Some(Err(AuthError::Cancelled))
		));
		assert!(matches!(
			parse(
				"https://login.live.com/oauth20_desktop.srf?error=server_error&error_description=x"
			),
			Some(Err(AuthError::Unexpected { .. }))
		));
	}
}
