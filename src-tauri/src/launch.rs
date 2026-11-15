//! Ключи запуска: `--lang ru|en` (язык от установщика) и `--self-test` (пробный запуск).

use std::io::Write;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tauri::{WebviewUrl, WebviewWindowBuilder};

static LANG: OnceLock<Option<&'static str>> = OnceLock::new();

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
	pub self_test: bool,
	pub lang: Option<&'static str>,
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Args {
	let mut out = Args::default();
	let mut args = args.into_iter().skip(1);
	while let Some(arg) = args.next() {
		match arg.as_str() {
			"--self-test" => out.self_test = true,
			"--lang" => out.lang = args.next().as_deref().and_then(lang_code),
			_ => {
				if let Some(value) = arg.strip_prefix("--lang=") {
					out.lang = lang_code(value);
				}
			}
		}
	}
	out
}

fn lang_code(value: &str) -> Option<&'static str> {
	match value {
		"ru" => Some("ru"),
		"en" => Some("en"),
		_ => None,
	}
}

pub fn set_lang(lang: Option<&'static str>) {
	let _ = LANG.set(lang);
}

/// Язык, выбранный в установщике. Интерфейс берёт его, только если язык ещё не выбран.
#[tauri::command]
pub fn app_launch_lang() -> Option<String> {
	LANG.get().copied().flatten().map(str::to_owned)
}

/// Коды выхода пробного запуска.
pub const SELF_TEST_OK: i32 = 0;
pub const SELF_TEST_TIMEOUT: i32 = 2;
pub const SELF_TEST_NO_WEBVIEW: i32 = 3;
pub const SELF_TEST_EMPTY: i32 = 4;

const SELF_TEST_TIMEOUT_AFTER: Duration = Duration::from_secs(30);
/// Адрес, на который страница «уходит», когда интерфейс отрисовался. Перехода не будет.
const SELF_TEST_DONE: &str = "https://self-test.invalid/";

const SELF_TEST_SCRIPT: &str = r#"
(function () {
	var started = Date.now();
	function check() {
		var root = document.getElementById("root");
		var n = root ? root.childElementCount : 0;
		if (n > 0 || Date.now() - started > 20000) {
			location.href = "https://self-test.invalid/?children=" + n;
			return;
		}
		setTimeout(check, 50);
	}
	if (document.readyState === "loading") {
		document.addEventListener("DOMContentLoaded", function () { setTimeout(check, 0); });
	} else {
		setTimeout(check, 0);
	}
})();
"#;

fn finish(code: i32, lines: &[String]) -> ! {
	let mut out = std::io::stdout();
	for line in lines {
		let _ = writeln!(out, "{line}");
	}
	let _ = out.flush();
	std::process::exit(code);
}

/// Пробный запуск для установщика: невидимое окно с интерфейсом во временной папке WebView2.
/// Данные пользователя, перенос, плагины и одиночный экземпляр не участвуют.
pub fn self_test(context: tauri::Context<tauri::Wry>) {
	let started = Instant::now();
	let version = match tauri::webview_version() {
		Ok(v) => v,
		Err(err) => finish(SELF_TEST_NO_WEBVIEW, &[format!("error=webview2: {err}")]),
	};
	std::thread::spawn(|| {
		std::thread::sleep(SELF_TEST_TIMEOUT_AFTER);
		finish(SELF_TEST_TIMEOUT, &["error=timeout".to_owned()]);
	});
	let data = std::env::temp_dir().join(format!("LeeLauncher-selftest-{}", std::process::id()));
	let result = tauri::Builder::default()
		.setup(move |app| {
			let built =
				WebviewWindowBuilder::new(app, "self-test", WebviewUrl::App("index.html".into()))
					.visible(false)
					.skip_taskbar(true)
					.focused(false)
					.inner_size(960.0, 600.0)
					.data_directory(data)
					.initialization_script(SELF_TEST_SCRIPT)
					.on_navigation(move |url| {
						if !url.as_str().starts_with(SELF_TEST_DONE) {
							return true;
						}
						let children = url
							.query_pairs()
							.find(|(k, _)| k == "children")
							.and_then(|(_, v)| v.parse::<u32>().ok())
							.unwrap_or(0);
						let lines = [
							format!("webview={version}"),
							format!("ms={}", started.elapsed().as_millis()),
						];
						let code = if children > 0 {
							SELF_TEST_OK
						} else {
							SELF_TEST_EMPTY
						};
						finish(code, &lines);
					})
					.build();
			if let Err(err) = built {
				finish(SELF_TEST_NO_WEBVIEW, &[format!("error=window: {err}")]);
			}
			Ok(())
		})
		.run(context);
	if let Err(err) = result {
		finish(SELF_TEST_NO_WEBVIEW, &[format!("error=tauri: {err}")]);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn args(list: &[&str]) -> Args {
		parse(list.iter().map(|s| s.to_string()))
	}

	#[test]
	fn parses_args() {
		assert_eq!(args(&["app"]), Args::default());
		assert_eq!(
			args(&["app", "--self-test", "--lang", "ru"]),
			Args {
				self_test: true,
				lang: Some("ru")
			}
		);
		assert_eq!(args(&["app", "--lang=en"]).lang, Some("en"));
		assert_eq!(args(&["app", "--lang", "de"]).lang, None);
		assert_eq!(args(&["app", "--lang"]).lang, None);
	}
}
