#![windows_subsystem = "windows"]

mod app;
mod art_data;
mod gfx;
mod install;
mod preview;
mod svgpath;
mod sys;
mod text;
mod theme;
mod ui;
mod window;

use std::path::PathBuf;

use app::{App, Mode};
use text::Lang;

#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
	uninstall: bool,
	quiet: bool,
	from: Option<PathBuf>,
	update: bool,
	relaunch: bool,
	/// Аргументы лаунчера после `/ARGS` (их передаёт updater).
	launcher_args: Vec<String>,
	dir: Option<PathBuf>,
	preview: Option<PathBuf>,
	lang: Option<Lang>,
}

/// Ключи: свои (`--uninstall`, `--update`) и те, что updater Tauri передаёт установщику NSIS
/// (`/P`, `/S`, `/UPDATE`, `/R`, `/ARGS …`, `/D=папка`).
fn parse(args: impl IntoIterator<Item = String>) -> Args {
	let mut out = Args::default();
	let mut it = args.into_iter().skip(1);
	while let Some(a) = it.next() {
		match a.as_str() {
			"--uninstall" => out.uninstall = true,
			"--quiet" | "/S" => out.quiet = true,
			"--from" => out.from = it.next().map(PathBuf::from),
			"--update" | "/UPDATE" => out.update = true,
			"--relaunch" | "/R" => out.relaunch = true,
			"/P" => {}
			"--preview" => out.preview = it.next().map(PathBuf::from),
			"--lang" => {
				out.lang = match it.next().as_deref() {
					Some("ru") => Some(Lang::Ru),
					Some("en") => Some(Lang::En),
					_ => None,
				}
			}
			"/ARGS" => {
				out.launcher_args = it.by_ref().collect();
			}
			_ => {
				if let Some(d) = a.strip_prefix("/D=") {
					out.dir = Some(PathBuf::from(d));
				}
			}
		}
	}
	out
}

fn main() {
	let args = parse(std::env::args_os().map(|a| a.to_string_lossy().into_owned()));
	sys::com_init();
	if let Some(dir) = &args.preview {
		match preview::run(dir) {
			Ok(files) => {
				for f in files {
					println!("{}", f.display());
				}
			}
			Err(e) => {
				eprintln!("{e}");
				std::process::exit(1);
			}
		}
		return;
	}
	let lang = args.lang.unwrap_or_else(Lang::system);
	std::thread::spawn(sys::clean_self_test_dirs);
	let existing = sys::existing_install();

	if args.uninstall {
		let dir = args
			.from
			.clone()
			.or_else(|| existing.as_ref().map(|e| e.dir.clone()))
			.or_else(|| {
				std::env::current_exe()
					.ok()
					.and_then(|p| p.parent().map(PathBuf::from))
			});
		let Some(dir) = dir else {
			return;
		};
		// Из папки программы себя не удалить: копия во временной папке продолжит.
		if args.from.is_none()
			&& let Ok(me) = std::env::current_exe()
			&& me.parent().is_some_and(|p| p == dir)
		{
			let copy = std::env::temp_dir()
				.join(format!("LeeLauncher-uninstall-{}.exe", std::process::id()));
			if std::fs::copy(&me, &copy).is_ok() {
				let mut cmd = std::process::Command::new(&copy);
				cmd.args(["--uninstall", "--from"]).arg(&dir);
				if args.quiet {
					cmd.arg("--quiet");
				}
				cmd.args(["--lang", lang.code()]);
				if cmd.spawn().is_ok() {
					return;
				}
			}
		}
		let temp_copy = args.from.is_some();
		if args.quiet {
			let ours = sys::install_dir_created();
			install::remove_system(&dir);
			let me = std::env::current_exe().ok();
			let _ = install::remove_files(&dir, me.as_deref(), ours);
		} else {
			let _ = window::run(|spawn| {
				let mut a = App::new(Mode::Uninstall, lang, Some(spawn));
				a.dir = dir.clone();
				a.start_data_size();
				a
			});
		}
		if temp_copy && let Ok(me) = std::env::current_exe() {
			sys::delete_after_exit(&me);
		}
		return;
	}

	if args.update {
		let dir = args
			.dir
			.clone()
			.or_else(|| existing.as_ref().map(|e| e.dir.clone()))
			.unwrap_or_else(sys::default_dir);
		let _ = window::run(|spawn| {
			let mut a = App::new(Mode::Silent, lang, Some(spawn));
			a.dir = dir;
			a.relaunch = args.relaunch;
			a.relaunch_args = args.launcher_args.clone();
			a.start_silent();
			a
		});
		return;
	}

	let _ = window::run(|spawn| {
		let mut a = App::new(Mode::Wizard, lang, Some(spawn));
		a.dir = args.dir.clone().unwrap_or_else(sys::default_dir);
		a.existing = existing;
		a
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	fn args(list: &[&str]) -> Args {
		parse(list.iter().map(|s| s.to_string()))
	}

	#[test]
	fn parses_updater_arguments() {
		// Так их передаёт tauri-plugin-updater 2.14 в режиме passive с перезапуском.
		let a = args(&["setup.exe", "/P", "/UPDATE", "/R", "/ARGS", "--lang", "ru"]);
		assert!(a.update && a.relaunch);
		assert_eq!(a.launcher_args, ["--lang", "ru"]);
		let a = args(&["setup.exe", "/P", "/UPDATE"]);
		assert!(a.update && !a.relaunch && a.launcher_args.is_empty());
	}

	#[test]
	fn parses_own_arguments() {
		let a = args(&["u.exe", "--uninstall", "--from", r"C:\x", "--quiet"]);
		assert!(a.uninstall && a.quiet);
		assert_eq!(a.from, Some(PathBuf::from(r"C:\x")));
		let a = args(&["setup.exe", r"/D=C:\Games\LeeLauncher", "--lang", "en"]);
		assert_eq!(a.dir, Some(PathBuf::from(r"C:\Games\LeeLauncher")));
		assert_eq!(a.lang, Some(Lang::En));
	}
}
