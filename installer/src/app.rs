//! Состояние установщика, отрисовка экранов и действия.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::gfx::{Align, Rect, Weight};
use crate::install::{self, Placement, Shortcuts};
use crate::sys::{self, Existing};
use crate::text::{Lang, size, t, tf};
use crate::ui::{BUTTON_H, Status, Ui, Variant, middle_ellipsis};

pub const W: f32 = 760.0;
pub const H: f32 = 480.0;
pub const SILENT_W: f32 = 400.0;
pub const SILENT_H: f32 = 168.0;
pub const SILENT_ERR_H: f32 = 192.0;
const SIDE: f32 = 224.0;
const X0: f32 = SIDE + 40.0;
const CW: f32 = W - SIDE - 80.0;
const TOP: f32 = 48.0;
const FOOT_Y: f32 = H - 80.0 + 22.0;
const RIGHT: f32 = W - 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
	Welcome,
	Folder,
	Checks,
	WebView,
	Install,
	Test,
	Done,
}

const STEPS: [(Step, &str); 7] = [
	(Step::Welcome, "step.welcome"),
	(Step::Folder, "step.folder"),
	(Step::Checks, "step.checks"),
	(Step::WebView, "step.webview"),
	(Step::Install, "step.install"),
	(Step::Test, "step.test"),
	(Step::Done, "step.done"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
	Wizard,
	Uninstall,
	Silent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wv {
	Explain,
	Running(f64),
	Error(String),
	Done(String, f64),
}

#[derive(Clone, Debug)]
pub struct Failure {
	pub title: &'static str,
	pub text: String,
	pub details: String,
	/// Второе действие в подвале: id и текст.
	pub second: Option<(&'static str, &'static str)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Un {
	Confirm,
	Running(f32),
	Done,
	Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modal {
	Cancel,
	WaitWebView,
}

/// Сообщения от фоновых задач.
pub enum Event {
	Check(usize, Status, String),
	ChecksDone,
	Recheck(usize, Status, String),
	WebView(Result<String, String>),
	Progress(f32),
	Phase(usize, Status),
	Installed(Result<(), (bool, String)>),
	Tested(Result<(u64, String), (i32, String)>),
	DataSize(u64),
	Uninstalled(Result<(), String>),
	SilentDone(Result<(), String>),
}

/// Кнопка подвала: id, подпись, вид, активна, иконка, крутится.
type FooterButton = (
	&'static str,
	String,
	Variant,
	bool,
	Option<&'static str>,
	bool,
);

pub type Send = Arc<dyn Fn(Event) + std::marker::Send + Sync>;

/// Запускает фоновую задачу; в режиме снимков задач нет.
pub type Spawner =
	Arc<dyn Fn(Box<dyn FnOnce(Send) + std::marker::Send>) + std::marker::Send + Sync>;

pub struct App {
	pub lang: Lang,
	pub mode: Mode,
	pub step: Step,
	pub step_at: f64,
	pub now: f64,
	pub existing: Option<Existing>,
	/// 0 — обновить/переустановить, 1 — удалить.
	pub choice: usize,
	pub dir: PathBuf,
	pub dir_error: Option<&'static str>,
	pub desktop: bool,
	pub start_menu: bool,
	pub free: Option<u64>,
	pub checks: Vec<(Status, String)>,
	pub checks_running: bool,
	pub need_webview: bool,
	pub webview: Option<String>,
	pub wv: Wv,
	pub progress: f32,
	pub phases: [Status; 3],
	pub test: [Status; 3],
	pub test_hint: [String; 3],
	pub failure: Option<Failure>,
	pub details_open: bool,
	pub copied_at: Option<f64>,
	pub modal: Option<Modal>,
	pub un: Un,
	pub delete_data: bool,
	pub data_size: Option<u64>,
	pub silent_error: Option<String>,
	pub relaunch_args: Vec<String>,
	pub relaunch: bool,
	pub close: bool,
	/// Задача установки или пробного запуска идёт.
	busy: bool,
	fresh: bool,
	/// Папку установки создала эта установка.
	created: bool,
	cancel: Arc<AtomicBool>,
	child: Arc<Mutex<Option<std::process::Child>>>,
	recheck_at: Option<f64>,
	spawn: Option<Spawner>,
	pub hwnd: isize,
}

impl App {
	pub fn new(mode: Mode, lang: Lang, spawn: Option<Spawner>) -> App {
		App {
			lang,
			mode,
			step: Step::Welcome,
			step_at: -1.0,
			now: 0.0,
			existing: None,
			choice: 0,
			dir: PathBuf::new(),
			dir_error: None,
			desktop: true,
			start_menu: true,
			free: None,
			checks: Vec::new(),
			checks_running: false,
			need_webview: false,
			webview: None,
			wv: Wv::Explain,
			progress: 0.0,
			phases: [Status::Waiting; 3],
			test: [Status::Waiting; 3],
			test_hint: Default::default(),
			failure: None,
			details_open: false,
			copied_at: None,
			modal: None,
			un: Un::Confirm,
			delete_data: false,
			data_size: None,
			silent_error: None,
			relaunch_args: Vec::new(),
			relaunch: true,
			close: false,
			busy: false,
			fresh: true,
			created: false,
			cancel: Arc::new(AtomicBool::new(false)),
			child: Arc::new(Mutex::new(None)),
			recheck_at: None,
			spawn,
			hwnd: 0,
		}
	}

	pub fn size(&self) -> (f32, f32) {
		match self.mode {
			Mode::Silent if self.silent_error.is_some() => (SILENT_W, SILENT_ERR_H),
			Mode::Silent => (SILENT_W, SILENT_H),
			_ => (W, H),
		}
	}

	fn run(&self, job: impl FnOnce(Send) + std::marker::Send + 'static) {
		if let Some(spawn) = &self.spawn {
			spawn(Box::new(job));
		}
	}

	fn go(&mut self, step: Step) {
		self.step = step;
		self.step_at = self.now;
		self.failure = None;
		self.details_open = false;
		match step {
			Step::Checks => self.start_checks(),
			Step::Install => self.start_install(),
			Step::Test => self.start_test(),
			Step::Folder => self.refresh_free(),
			_ => {}
		}
	}

	fn refresh_free(&mut self) {
		if self.spawn.is_some() {
			self.free = sys::disk_free(&self.dir);
		}
	}

	pub fn update_mode(&self) -> bool {
		self.existing.is_some() && self.choice == 0
	}

	// --- ввод ---

	pub fn click(&mut self, id: &str) {
		match id {
			"win:min" => {}
			"win:close" | "esc" => self.escape(),
			"details" => self.details_open = !self.details_open,
			"copy" => {
				if let Some(f) = &self.failure {
					sys::set_clipboard(None, &f.details);
				} else if let Wv::Error(d) = &self.wv {
					sys::set_clipboard(None, d);
				} else if let Some(e) = self.silent_error.clone().or_else(|| match &self.un {
					Un::Error(e) => Some(e.clone()),
					_ => None,
				}) {
					sys::set_clipboard(None, &e);
				}
				self.copied_at = Some(self.now);
			}
			"lang:0" => self.lang = Lang::Ru,
			"lang:1" => self.lang = Lang::En,
			"choice:0" => self.choice = 0,
			"choice:1" => self.choice = 1,
			"desktop" => self.desktop = !self.desktop,
			"startMenu" => self.start_menu = !self.start_menu,
			"change" => self.change_folder(),
			"deleteData" => self.delete_data = !self.delete_data,
			"wvAbout" | "wvSite" => sys::open(sys::WEBVIEW2_ABOUT_URL),
			"closeApp" => {
				let pids = sys::launcher_pids(&self.dir);
				sys::close_windows_of(&pids);
				self.recheck_at = Some(self.now + 1.0);
			}
			"otherFolder" => self.go(Step::Folder),
			"logs" => sys::open(&sys::data_dirs()[0].join("logs").to_string_lossy()),
			"finishAnyway" => self.finish_install(),
			"wvReinstall" => {
				self.wv = Wv::Explain;
				self.go(Step::WebView);
			}
			"modal:stay" => self.modal = None,
			"modal:stop" => self.abort(),
			"openFolder" => sys::open(&sys::data_dirs()[0].to_string_lossy()),
			"openPrevious" => {
				self.launch();
				self.close = true;
			}
			"back" => self.back(),
			"next" => self.next(),
			"cancel" => self.escape(),
			_ => {}
		}
	}

	/// Enter без кнопки в фокусе.
	pub fn default_action(&mut self) {
		match self.mode {
			Mode::Uninstall => match self.un {
				Un::Confirm => self.next(),
				Un::Done | Un::Error(_) => self.next(),
				_ => {}
			},
			Mode::Silent => {
				if self.silent_error.is_some() {
					self.next();
				}
			}
			Mode::Wizard => {
				if self.modal.is_none() && self.next_enabled() {
					self.next();
				}
			}
		}
	}

	pub fn escape(&mut self) {
		if self.modal.is_some() {
			self.modal = None;
			return;
		}
		match self.mode {
			Mode::Silent => {}
			Mode::Uninstall => {
				if !matches!(self.un, Un::Running(_)) {
					self.close = true;
				}
			}
			Mode::Wizard => match self.step {
				Step::WebView if matches!(self.wv, Wv::Running(_)) => {
					self.modal = Some(Modal::WaitWebView)
				}
				Step::Install | Step::Test if self.failure.is_none() => {
					self.modal = Some(Modal::Cancel)
				}
				Step::Install | Step::Test => self.abort(),
				_ => self.close = true,
			},
		}
	}

	/// Стрелки на сегментах и вариантах.
	pub fn arrow(&mut self, focus: &str, forward: bool) -> Option<String> {
		if focus.starts_with("lang:") {
			self.lang = if forward { Lang::En } else { Lang::Ru };
			return Some(format!("lang:{}", if forward { 1 } else { 0 }));
		}
		if focus.starts_with("choice:") {
			self.choice = if forward { 1 } else { 0 };
			return Some(format!("choice:{}", self.choice));
		}
		None
	}

	fn change_folder(&mut self) {
		let owner = (self.hwnd != 0).then_some(windows::Win32::Foundation::HWND(self.hwnd as _));
		if let Some(chosen) = sys::pick_folder(owner, &self.dir) {
			let mut protected = sys::program_files();
			protected.retain(|p| !p.as_os_str().is_empty());
			match install::place(&chosen, &sys::data_dirs(), &protected) {
				Placement::Ok(dir) => {
					self.dir = dir;
					self.dir_error = None;
				}
				Placement::DataFolder => self.dir_error = Some("folder.errData"),
				Placement::NeedsAdmin => self.dir_error = Some("folder.errAdmin"),
				Placement::DriveRoot => self.dir_error = Some("folder.errRoot"),
			}
			self.refresh_free();
		}
	}

	fn back(&mut self) {
		match self.step {
			Step::Folder => self.go(Step::Welcome),
			Step::Checks => self.go(if self.existing.is_some() {
				Step::Welcome
			} else {
				Step::Folder
			}),
			Step::WebView if !matches!(self.wv, Wv::Running(_)) => self.go(Step::Checks),
			_ => {}
		}
	}

	pub fn next_enabled(&self) -> bool {
		match self.step {
			Step::Folder => self.dir_error.is_none(),
			Step::Checks => {
				!self.checks_running
					&& !self
						.checks
						.iter()
						.any(|(s, _)| *s == Status::Error || *s == Status::Running)
					&& !(self.need_webview
						&& self.checks.get(5).is_some_and(|(s, _)| *s != Status::Ok))
			}
			Step::WebView => !matches!(self.wv, Wv::Running(_) | Wv::Done(..)),
			Step::Install | Step::Test => self.failure.is_some(),
			_ => true,
		}
	}

	fn next(&mut self) {
		match self.mode {
			Mode::Uninstall => {
				match self.un {
					Un::Confirm => self.start_uninstall(),
					Un::Error(_) => self.start_uninstall(),
					Un::Done => self.close = true,
					_ => {}
				}
				return;
			}
			Mode::Silent => {
				self.silent_error = None;
				self.start_silent();
				return;
			}
			Mode::Wizard => {}
		}
		if !self.next_enabled() {
			return;
		}
		match self.step {
			Step::Welcome => {
				if !install::has_payload()
					&& self.spawn.is_some()
					&& !(self.existing.is_some() && self.choice == 1)
				{
					self.failure = Some(Failure {
						title: "err.copyTitle",
						text: t(self.lang, "err.noPayload").to_owned(),
						details: "payload".into(),
						second: None,
					});
					return;
				}
				match (&self.existing, self.choice) {
					(Some(e), 1) => {
						self.dir = e.dir.clone();
						self.mode = Mode::Uninstall;
						self.un = Un::Confirm;
						self.start_data_size();
					}
					(Some(e), _) => {
						self.dir = e.dir.clone();
						self.go(Step::Checks);
					}
					_ => self.go(Step::Folder),
				}
			}
			Step::Folder => self.go(Step::Checks),
			Step::Checks => {
				if self.need_webview {
					self.wv = Wv::Explain;
					self.go(Step::WebView);
				} else {
					self.go(Step::Install);
				}
			}
			Step::WebView => self.start_webview(),
			Step::Install => self.go(Step::Install),
			Step::Test => self.go(Step::Test),
			Step::Done => {
				self.launch();
				self.close = true;
			}
		}
	}

	pub fn launch(&self) {
		let exe = self.dir.join(sys::EXE);
		let mut cmd = std::process::Command::new(&exe);
		if self.mode == Mode::Silent {
			cmd.args(&self.relaunch_args);
		} else {
			cmd.args(["--lang", self.lang.code()]);
		}
		cmd.current_dir(&self.dir);
		let _ = cmd.spawn();
	}

	/// Прервать установку: откатить файлы и закрыть окно.
	fn abort(&mut self) {
		self.modal = None;
		self.cancel.store(true, Ordering::SeqCst);
		if let Ok(mut child) = self.child.lock()
			&& let Some(c) = child.as_mut()
		{
			let _ = c.kill();
			let _ = c.wait();
		}
		if !self.busy {
			rollback_all(
				&self.dir,
				self.fresh,
				self.created,
				self.existing.as_ref().map(|e| e.version.as_str()),
			);
			self.close = true;
		}
	}

	// --- проверки ---

	fn start_checks(&mut self) {
		self.checks = vec![(Status::Waiting, String::new()); 5];
		self.checks_running = true;
		self.need_webview = false;
		let dir = self.dir.clone();
		let lang = self.lang;
		self.run(move |send| {
			let pause = Duration::from_millis(300);
			let row = |i: usize, s: Status, h: String| send(Event::Check(i, s, h));
			row(0, Status::Running, t(lang, "checks.checking").into());
			std::thread::sleep(pause);
			let (_, build, name) = sys::windows_version();
			if sys::windows_supported() {
				row(
					0,
					Status::Ok,
					tf(
						lang,
						"checks.windowsOk",
						&[("ver", &format!("{name} ({build})"))],
					),
				);
			} else {
				row(0, Status::Error, t(lang, "checks.windowsErr").into());
			}
			row(1, Status::Running, t(lang, "checks.checking").into());
			std::thread::sleep(pause);
			let need = size(lang, install::NEED_BYTES);
			match sys::disk_free(&dir) {
				Some(free) if free >= install::NEED_BYTES => row(
					1,
					Status::Ok,
					tf(
						lang,
						"checks.diskOk",
						&[("free", &size(lang, free)), ("need", &need)],
					),
				),
				_ => row(
					1,
					Status::Error,
					tf(lang, "checks.diskErr", &[("need", &need)]),
				),
			}
			row(2, Status::Running, t(lang, "checks.checking").into());
			std::thread::sleep(pause);
			if sys::can_write(&dir) {
				row(2, Status::Ok, t(lang, "checks.accessOk").into());
			} else {
				row(2, Status::Error, t(lang, "checks.accessErr").into());
			}
			row(3, Status::Running, t(lang, "checks.checking").into());
			std::thread::sleep(pause);
			match sys::webview2_version() {
				Some(v) => row(3, Status::Ok, tf(lang, "checks.webviewOk", &[("v", &v)])),
				None => row(3, Status::Warn, t(lang, "checks.webviewWarn").into()),
			}
			row(4, Status::Running, t(lang, "checks.checking").into());
			std::thread::sleep(pause);
			if sys::launcher_pids(&dir).is_empty() {
				row(4, Status::Ok, t(lang, "checks.closedOk").into());
			} else {
				row(4, Status::Error, t(lang, "checks.closedErr").into());
			}
			if sys::webview2_version().is_none() {
				row(5, Status::Running, t(lang, "checks.checking").into());
				std::thread::sleep(pause);
				if sys::network_ok() {
					row(5, Status::Ok, t(lang, "checks.netOk").into());
				} else {
					row(5, Status::Warn, t(lang, "checks.netWarn").into());
				}
			}
			send(Event::ChecksDone);
		});
	}

	/// Повторные проверки: лаунчер закрыт (раз в 2 с), сеть (раз в 5 с).
	pub fn tick(&mut self, now: f64) {
		self.now = now;
		if self.copied_at.is_some_and(|t| now - t > 1.5) {
			self.copied_at = None;
		}
		if let Wv::Done(_, at) = self.wv
			&& self.step == Step::WebView
			&& now - at > 0.6
		{
			self.wv = Wv::Explain;
			self.need_webview = false;
			self.go(Step::Install);
		}
		let Some(at) = self.recheck_at else {
			return;
		};
		if now < at || self.step != Step::Checks || self.checks_running {
			return;
		}
		self.recheck_at = None;
		let dir = self.dir.clone();
		let lang = self.lang;
		let launcher = self.checks.get(4).is_some_and(|(s, _)| *s == Status::Error);
		let net = self.checks.get(5).is_some_and(|(s, _)| *s == Status::Warn);
		self.run(move |send| {
			if launcher {
				if sys::launcher_pids(&dir).is_empty() {
					send(Event::Recheck(
						4,
						Status::Ok,
						t(lang, "checks.closedOk").into(),
					));
				} else {
					send(Event::Recheck(
						4,
						Status::Error,
						t(lang, "checks.closedErr").into(),
					));
				}
			}
			if net {
				if sys::network_ok() {
					send(Event::Recheck(
						5,
						Status::Ok,
						t(lang, "checks.netOk").into(),
					));
				} else {
					send(Event::Recheck(
						5,
						Status::Warn,
						t(lang, "checks.netWarn").into(),
					));
				}
			}
		});
	}

	pub fn needs_tick(&self) -> bool {
		self.recheck_at.is_some()
			|| self.copied_at.is_some()
			|| matches!(self.wv, Wv::Done(..) | Wv::Running(_))
	}

	fn schedule_recheck(&mut self) {
		let launcher = self.checks.get(4).is_some_and(|(s, _)| *s == Status::Error);
		let net = self.checks.get(5).is_some_and(|(s, _)| *s == Status::Warn);
		if launcher {
			self.recheck_at = Some(self.now + 2.0);
		} else if net {
			self.recheck_at = Some(self.now + 5.0);
		}
	}

	// --- WebView2 ---

	fn start_webview(&mut self) {
		self.wv = Wv::Running(self.now);
		let exe = std::env::current_exe().ok();
		self.run(move |send| send(Event::WebView(install_webview(exe.as_deref()))));
	}

	// --- установка ---

	fn start_install(&mut self) {
		self.progress = 0.0;
		self.phases = [Status::Running, Status::Waiting, Status::Waiting];
		self.fresh = !self.dir.join(sys::EXE).exists();
		self.created = !self.dir.exists();
		self.busy = true;
		let dir = self.dir.clone();
		let created = self.created;
		let shortcuts = if self.update_mode() {
			install::current_shortcuts()
		} else {
			Shortcuts {
				desktop: self.desktop,
				start_menu: self.start_menu,
			}
		};
		self.run(move |send| {
			let result = install_all(&dir, shortcuts, created, &send);
			send(Event::Installed(result));
		});
	}

	fn start_test(&mut self) {
		self.test = [Status::Running, Status::Waiting, Status::Waiting];
		self.test_hint = Default::default();
		self.busy = true;
		let dir = self.dir.clone();
		let lang = self.lang;
		let child = self.child.clone();
		self.run(move |send| send(Event::Tested(self_test(&dir, lang, &child))));
	}

	fn finish_install(&mut self) {
		install::commit(&self.dir);
		self.go(Step::Done);
	}

	// --- удаление ---

	pub fn start_data_size(&mut self) {
		self.run(move |send| {
			let total: u64 = sys::data_dirs()
				.iter()
				.chain(sys::old_data_dirs().iter())
				.map(|d| install::dir_size(d))
				.sum();
			send(Event::DataSize(total));
		});
	}

	fn start_uninstall(&mut self) {
		if !sys::launcher_pids(&self.dir).is_empty() && self.spawn.is_some() {
			self.un = Un::Error(t(self.lang, "checks.closedErr").into());
			return;
		}
		self.un = Un::Running(0.0);
		let dir = self.dir.clone();
		let data = self.delete_data;
		self.run(move |send| send(Event::Uninstalled(uninstall(&dir, data))));
	}

	// --- тихое обновление ---

	pub fn start_silent(&mut self) {
		self.progress = 0.0;
		self.silent_error = None;
		let dir = self.dir.clone();
		let child = self.child.clone();
		self.run(move |send| send(Event::SilentDone(silent_update(&dir, &send, &child))));
	}

	pub fn on_event(&mut self, ev: Event) {
		match ev {
			Event::Check(i, s, h) | Event::Recheck(i, s, h) => {
				if self.checks.len() <= i {
					self.checks.resize(i + 1, (Status::Waiting, String::new()));
				}
				self.checks[i] = (s, h);
				if i == 3 {
					self.need_webview = s == Status::Warn;
				}
				if !self.checks_running {
					self.schedule_recheck();
				}
			}
			Event::ChecksDone => {
				self.checks_running = false;
				self.schedule_recheck();
			}
			Event::WebView(r) => match r {
				Ok(v) => {
					self.webview = Some(v.clone());
					self.wv = Wv::Done(v, self.now);
				}
				Err(d) => self.wv = Wv::Error(d),
			},
			Event::Progress(p) => self.progress = p,
			Event::Phase(i, s) => {
				if i < 3 {
					self.phases[i] = s;
				}
			}
			Event::Installed(r) => {
				self.busy = false;
				if self.cancel.load(Ordering::SeqCst) {
					rollback_all(
						&self.dir,
						self.fresh,
						self.created,
						self.existing.as_ref().map(|e| e.version.as_str()),
					);
					self.close = true;
					return;
				}
				match r {
					Ok(()) => {
						if self.mode == Mode::Silent {
							return;
						}
						self.go(Step::Test);
					}
					Err((no_space, details)) => {
						let drive = sys::drive_of(&self.dir);
						let need = size(self.lang, install::NEED_BYTES);
						self.failure = Some(if no_space {
							Failure {
								title: "err.spaceTitle",
								text: tf(
									self.lang,
									"err.spaceText",
									&[("drive", &drive), ("need", &need)],
								),
								details,
								second: Some(("otherFolder", "checks.otherFolder")),
							}
						} else {
							Failure {
								title: "err.copyTitle",
								text: t(self.lang, "err.copyText").into(),
								details,
								second: Some(("otherFolder", "checks.otherFolder")),
							}
						});
					}
				}
			}
			Event::Tested(r) => {
				self.busy = false;
				if self.cancel.load(Ordering::SeqCst) {
					rollback_all(
						&self.dir,
						self.fresh,
						self.created,
						self.existing.as_ref().map(|e| e.version.as_str()),
					);
					self.close = true;
					return;
				}
				match r {
					Ok((ms, webview)) => {
						let secs = format!("{:.1}", ms as f64 / 1000.0);
						let secs = if self.lang == Lang::Ru {
							secs.replace('.', ",")
						} else {
							secs
						};
						self.test = [Status::Ok; 3];
						self.test_hint = [
							tf(self.lang, "test.startsOk", &[("t", &secs)]),
							tf(self.lang, "checks.webviewOk", &[("v", &webview)]),
							String::new(),
						];
						self.finish_install();
					}
					Err((code, details)) => {
						self.test = [Status::Error, Status::Waiting, Status::Waiting];
						self.failure = Some(if code == 3 {
							Failure {
								title: "err.wvTitle",
								text: t(self.lang, "err.wvText").into(),
								details,
								second: Some(("wvReinstall", "err.wvReinstall")),
							}
						} else {
							Failure {
								title: "err.testTitle",
								text: t(self.lang, "err.testText").into(),
								details,
								second: Some(("logs", "err.logs")),
							}
						});
					}
				}
			}
			Event::DataSize(s) => self.data_size = Some(s),
			Event::Uninstalled(r) => {
				self.un = match r {
					Ok(()) => Un::Done,
					Err(e) => Un::Error(e),
				}
			}
			Event::SilentDone(r) => match r {
				Ok(()) => {
					if self.relaunch {
						self.launch();
					}
					self.close = true;
				}
				Err(e) => self.silent_error = Some(e),
			},
		}
	}

	// --- отрисовка ---

	pub fn paint(&mut self, ui: &mut Ui) {
		self.now = ui.now;
		match self.mode {
			Mode::Silent => self.paint_silent(ui),
			_ => self.paint_main(ui),
		}
	}

	fn paint_main(&mut self, ui: &mut Ui) {
		let p = ui.p;
		ui.c.clear(p.background);
		// Боковая колонка.
		ui.c.fill(Rect::new(0.0, 0.0, SIDE, H), p.sidebar);
		ui.c.line(SIDE - 0.5, 0.0, SIDE - 0.5, H, 1.0, p.border);
		ui.drag(Rect::new(0.0, 0.0, SIDE, H));
		ui.drag(Rect::new(SIDE, 0.0, W - SIDE, 40.0));
		ui.c.lockup(24.0, 24.0, 20.0, p.brand, p.foreground);
		if self.mode == Mode::Wizard {
			self.paint_steps(ui);
		}
		let version = tf(self.lang, "version", &[("v", install::VERSION)]);
		ui.c.text(
			&version,
			Rect::new(24.0, H - 20.0 - 16.0, SIDE - 48.0, 16.0),
			&ui.style(12.0, 16.0, Weight::Regular, p.muted_foreground),
		);
		ui.window_buttons(W, 40.0, false);

		// Содержимое шага: появляется с прозрачностью и сдвигом.
		let k = if ui.reduced || self.step_at < 0.0 {
			1.0
		} else {
			((self.now - self.step_at) / 0.2).clamp(0.0, 1.0) as f32
		};
		if k < 1.0 {
			ui.animating = true;
		}
		let ease = 1.0 - (1.0 - k) * (1.0 - k);
		ui.c.opacity = ease;
		ui.c.offset(0.0, 8.0 * (1.0 - ease));
		match self.mode {
			Mode::Uninstall => self.paint_uninstall(ui),
			_ => self.paint_step(ui),
		}
		ui.c.offset(0.0, -8.0 * (1.0 - ease));
		ui.c.opacity = 1.0;
		self.paint_footer(ui);
		if let Some(m) = self.modal {
			self.paint_modal(ui, m);
		}
		if ui.win10 {
			ui.c.stroke_round(Rect::new(0.0, 0.0, W, H), 0.0, 1.0, p.border);
		}
	}

	fn step_status(&self, step: Step) -> (Status, bool) {
		let skipped =
			(step == Step::Folder && self.existing.is_some() && self.step > Step::Welcome)
				|| (step == Step::WebView
					&& self.step > Step::Checks
					&& !self.need_webview
					&& self.webview.is_none());
		let status = if step == self.step {
			if self.failure.is_some() || (step == Step::WebView && matches!(self.wv, Wv::Error(_)))
			{
				Status::Error
			} else {
				Status::Running
			}
		} else if step < self.step {
			Status::Ok
		} else {
			Status::Waiting
		};
		(status, skipped)
	}

	fn paint_steps(&self, ui: &mut Ui) {
		let p = ui.p;
		for (i, (step, key)) in STEPS.iter().enumerate() {
			let y = 88.0 + i as f32 * 36.0;
			let (status, skipped) = self.step_status(*step);
			let (cx, cy) = (24.0 + 10.0, y + 18.0);
			match status {
				Status::Ok => {
					ui.c.fill_circle(cx, cy, 10.0, p.primary);
					ui.c.icon("check", cx - 6.0, cy - 6.0, 12.0, p.primary_foreground, 0.0);
				}
				Status::Running => {
					ui.c.stroke_circle(cx, cy, 10.0, 2.0, p.primary);
					ui.c.fill_circle(cx, cy, 3.0, p.primary);
				}
				Status::Error => {
					ui.c.fill_circle(cx, cy, 10.0, p.destructive.alpha(0.14));
					ui.c.icon("x", cx - 6.0, cy - 6.0, 12.0, p.destructive, 0.0);
				}
				_ => ui.c.stroke_circle(cx, cy, 10.0, 1.0, p.control_track),
			}
			let (weight, color) = match status {
				Status::Running | Status::Error => (Weight::SemiBold, p.foreground),
				Status::Ok => (Weight::Medium, p.foreground),
				_ => (Weight::Medium, p.muted_foreground),
			};
			let style = ui.style(14.0, 20.0, weight, color);
			let label = t(self.lang, key);
			ui.c.text_mid(label, Rect::new(56.0, y, SIDE - 72.0, 36.0), &style);
			if skipped {
				let (lw, _) = ui.c.g.measure(label, &style, 200.0);
				ui.c.text_mid(
					t(self.lang, "step.skipped"),
					Rect::new(56.0 + lw + 6.0, y + 1.0, SIDE - 72.0 - lw - 6.0, 36.0),
					&ui.style(12.0, 16.0, Weight::Regular, p.muted_foreground),
				);
			}
		}
	}

	fn title(&self, ui: &mut Ui, title: &str, text: &str) -> f32 {
		let p = ui.p;
		let h = ui.c.text(
			title,
			Rect::new(X0, TOP, CW, 34.0),
			&ui.style(28.0, 34.0, Weight::SemiBold, p.foreground),
		);
		let h2 = ui.c.text(
			text,
			Rect::new(X0, TOP + h + 8.0, CW, 200.0),
			&ui.style(14.0, 22.0, Weight::Regular, p.muted_foreground)
				.wrap(),
		);
		TOP + h + 8.0 + h2 + 24.0
	}

	fn paint_failure(&mut self, ui: &mut Ui) {
		let Some(f) = self.failure.clone() else {
			return;
		};
		let copied = self.copied_at.is_some();
		ui.error_block(
			X0,
			TOP + 8.0,
			CW,
			t(self.lang, f.title),
			&f.text,
			Some(&f.details),
			self.details_open,
			copied,
		);
	}

	fn paint_step(&mut self, ui: &mut Ui) {
		if self.failure.is_some() {
			self.paint_failure(ui);
			return;
		}
		let p = ui.p;
		let lang = self.lang;
		match self.step {
			Step::Welcome => {
				if let Some(e) = self.existing.clone() {
					let y = self.title(
						ui,
						t(lang, "existing.title"),
						&tf(lang, "existing.text", &[("old", &e.version)]),
					);
					let order = install::version_key(install::VERSION)
						.cmp(&install::version_key(&e.version));
					let (title, text) = match order {
						std::cmp::Ordering::Greater => (
							tf(lang, "existing.update", &[("new", install::VERSION)]),
							t(lang, "existing.updateText"),
						),
						std::cmp::Ordering::Equal => (
							t(lang, "existing.reinstall").to_owned(),
							t(lang, "existing.reinstallText"),
						),
						std::cmp::Ordering::Less => (
							tf(
								lang,
								"existing.downgrade",
								&[("new", install::VERSION), ("old", &e.version)],
							),
							t(lang, "existing.downgradeText"),
						),
					};
					ui.choice(
						"choice:0",
						Rect::new(X0, y, CW, 64.0),
						&title,
						text,
						self.choice == 0,
					);
					ui.choice(
						"choice:1",
						Rect::new(X0, y + 72.0, CW, 64.0),
						t(lang, "existing.remove"),
						t(lang, "existing.removeText"),
						self.choice == 1,
					);
				} else {
					let y = self.title(ui, t(lang, "welcome.title"), t(lang, "welcome.text"));
					self.paint_language(ui, y);
				}
			}
			Step::Folder => {
				let y = self.title(ui, t(lang, "folder.title"), t(lang, "folder.text"));
				let change = t(lang, "folder.change");
				let bw = ui.button_width(change, false);
				let field = Rect::new(X0, y, CW - bw - 8.0, 36.0);
				ui.c.fill_round(field, 12.0, p.field);
				ui.c.stroke_round(
					field,
					12.0,
					1.0,
					if self.dir_error.is_some() {
						p.destructive
					} else {
						p.input
					},
				);
				let shown = middle_ellipsis(
					&self.dir.to_string_lossy(),
					((field.w - 24.0) / 7.2) as usize,
				);
				ui.c.text_mid(
					&shown,
					Rect::new(field.x + 12.0, field.y, field.w - 24.0, field.h),
					&ui.style(13.0, 18.0, Weight::Regular, p.foreground).mono(),
				);
				ui.button(
					"change",
					Rect::new(X0 + CW - bw, y, bw, BUTTON_H),
					change,
					Variant::Outline,
					true,
					None,
					false,
				);
				let mut cy = y + 44.0;
				if let Some(key) = self.dir_error {
					ui.c.icon(
						"circle-exclamation-point",
						X0,
						cy + 1.0,
						16.0,
						p.destructive,
						0.0,
					);
					ui.c.text(
						t(lang, key),
						Rect::new(X0 + 22.0, cy, CW - 22.0, 18.0),
						&ui.style(13.0, 18.0, Weight::Regular, p.destructive),
					);
				} else if let Some(free) = self.free {
					let text = tf(
						lang,
						"folder.space",
						&[
							("need", &size(lang, install::NEED_BYTES)),
							("free", &size(lang, free)),
							("drive", &sys::drive_of(&self.dir)),
						],
					);
					ui.c.text(
						&text,
						Rect::new(X0, cy, CW, 18.0),
						&ui.style(13.0, 18.0, Weight::Regular, p.muted_foreground),
					);
				}
				cy += 40.0;
				ui.checkbox(
					"desktop",
					X0,
					cy,
					t(lang, "folder.desktop"),
					self.desktop,
					CW,
				);
				ui.checkbox(
					"startMenu",
					X0,
					cy + 36.0,
					t(lang, "folder.startMenu"),
					self.start_menu,
					CW,
				);
			}
			Step::Checks => {
				let y = self.title(ui, t(lang, "checks.title"), t(lang, "checks.text"));
				let rows: [(&str, &str); 6] = [
					("monitor", "checks.windows"),
					("hard-drive", "checks.disk"),
					("folder-open", "checks.access"),
					("app-window", "checks.webview"),
					("rocket", "checks.closed"),
					("globe", "checks.net"),
				];
				let count = self.checks.len().min(rows.len());
				let row_h = if count > 5 { 44.0 } else { 52.0 };
				let mut cy = y - 12.0;
				for (i, (icon, key)) in rows.iter().enumerate().take(count) {
					let (status, hint) = self.checks[i].clone();
					let hint = if status == Status::Waiting {
						t(lang, "checks.waiting").to_owned()
					} else {
						hint
					};
					let action = match (i, status) {
						(1 | 2, Status::Error) => Some(("otherFolder", "checks.otherFolder")),
						(4, Status::Error) => Some(("closeApp", "checks.closeApp")),
						_ => None,
					};
					let row = Rect::new(X0, cy, CW, row_h);
					let reserve = match action {
						Some((id, label)) => {
							let label = t(lang, label);
							let w = ui.button_width(label, false) - 8.0;
							let b = Rect::new(
								row.right() - 32.0 - w,
								cy + (row_h - 32.0) / 2.0,
								w,
								32.0,
							);
							ui.button(id, b, label, Variant::Outline, true, None, false);
							w + 12.0
						}
						None => 0.0,
					};
					ui.check_row(row, icon, t(lang, key), &hint, status, reserve);
					cy += row_h;
				}
			}
			Step::WebView => self.paint_webview(ui),
			Step::Install => {
				let y = self.title(ui, t(lang, "install.title"), t(lang, "install.text"));
				let pct = format!("{}%", (self.progress * 100.0).round() as u32);
				ui.c.text(
					&pct,
					Rect::new(X0, y, 80.0, 20.0),
					&ui.style(14.0, 20.0, Weight::SemiBold, p.foreground),
				);
				ui.c.text(
					sys::EXE,
					Rect::new(X0 + 80.0, y, CW - 80.0, 20.0),
					&ui.style(13.0, 20.0, Weight::Regular, p.muted_foreground)
						.align(Align::End),
				);
				ui.progress(Rect::new(X0, y + 28.0, CW, 8.0), Some(self.progress));
				let labels = ["install.files", "install.shortcuts", "install.registry"];
				for (i, key) in labels.iter().enumerate() {
					ui.phase_row(X0, y + 60.0 + i as f32 * 32.0, t(lang, key), self.phases[i]);
				}
			}
			Step::Test => {
				let y = self.title(ui, t(lang, "test.title"), t(lang, "test.text"));
				let labels = ["test.starts", "test.webview", "test.render"];
				for (i, key) in labels.iter().enumerate() {
					let hint = self.test_hint[i].clone();
					let hint = if hint.is_empty() && self.test[i] == Status::Running {
						t(lang, "checks.checking").to_owned()
					} else {
						hint
					};
					let icon = ["rocket", "app-window", "monitor"][i];
					let row = Rect::new(X0, y - 12.0 + i as f32 * 52.0, CW, 52.0);
					ui.check_row(row, icon, t(lang, key), &hint, self.test[i], 0.0);
				}
			}
			Step::Done => {
				let cx = X0 + CW / 2.0;
				ui.c.app_icon(
					cx - 36.0,
					112.0,
					72.0,
					p.brand_tile,
					p.brand_mark,
					p.brand_icon,
				);
				let title = if self.existing.is_some() && self.choice == 0 {
					tf(lang, "done.updated", &[("v", install::VERSION)])
				} else {
					t(lang, "done.title").to_owned()
				};
				ui.c.text(
					&title,
					Rect::new(X0, 204.0, CW, 34.0),
					&ui.style(28.0, 34.0, Weight::SemiBold, p.foreground)
						.align(Align::Center),
				);
				let hint = match (self.desktop, self.start_menu, self.existing.is_some()) {
					(_, _, true) => String::new(),
					(true, true, _) => t(lang, "done.shortcuts").to_owned(),
					(true, false, _) => t(lang, "done.desktop").to_owned(),
					(false, true, _) => t(lang, "done.startMenu").to_owned(),
					(false, false, _) => tf(
						lang,
						"done.folder",
						&[("path", &self.dir.to_string_lossy())],
					),
				};
				ui.c.text(
					&hint,
					Rect::new(X0, 246.0, CW, 44.0),
					&ui.style(14.0, 22.0, Weight::Regular, p.muted_foreground)
						.align(Align::Center)
						.wrap(),
				);
			}
		}
	}

	fn paint_language(&self, ui: &mut Ui, y: f32) {
		let p = ui.p;
		ui.c.text(
			t(self.lang, "welcome.language"),
			Rect::new(X0, y, CW, 18.0),
			&ui.style(13.0, 18.0, Weight::Regular, p.muted_foreground),
		);
		let selected = if self.lang == Lang::Ru { 0 } else { 1 };
		ui.segments(
			"lang",
			X0,
			y + 26.0,
			&["Русский", "English"],
			selected,
			96.0,
		);
	}

	fn paint_webview(&mut self, ui: &mut Ui) {
		let p = ui.p;
		let lang = self.lang;
		match self.wv.clone() {
			Wv::Explain => {
				let title = ui.c.text(
					t(lang, "webview.title"),
					Rect::new(X0, TOP, CW, 34.0),
					&ui.style(28.0, 34.0, Weight::SemiBold, p.foreground),
				);
				let mut y = TOP + title + 8.0;
				y += ui.c.text(
					t(lang, "webview.text"),
					Rect::new(X0, y, CW.min(460.0), 120.0),
					&ui.style(14.0, 22.0, Weight::Regular, p.muted_foreground)
						.wrap(),
				) + 20.0;
				for (icon, key) in [
					("download", "webview.size"),
					("shield-check", "webview.trust"),
					("info", "webview.time"),
				] {
					ui.c.icon(icon, X0, y + 2.0, 16.0, p.primary_text, 0.0);
					let h = ui.c.text(
						t(lang, key),
						Rect::new(X0 + 28.0, y, CW - 28.0, 60.0),
						&ui.style(14.0, 20.0, Weight::Regular, p.foreground).wrap(),
					);
					y += h + 12.0;
				}
				ui.link(
					"wvAbout",
					X0,
					y + 4.0,
					t(lang, "webview.about"),
					Some("external-link"),
					13.0,
				);
			}
			Wv::Running(started) | Wv::Done(_, started) => {
				let done = matches!(self.wv, Wv::Done(..));
				let y = self.title(ui, t(lang, "webview.runTitle"), t(lang, "webview.runText"));
				let label = if let Wv::Done(v, _) = &self.wv {
					tf(lang, "webview.done", &[("v", v)])
				} else {
					t(lang, "webview.runLabel").to_owned()
				};
				ui.c.text(
					&label,
					Rect::new(X0, y, CW, 20.0),
					&ui.style(14.0, 20.0, Weight::SemiBold, p.foreground),
				);
				let secs = (self.now - started).max(0.0) as u64;
				let elapsed = tf(
					lang,
					"webview.elapsed",
					&[("time", &format!("{}:{:02}", secs / 60, secs % 60))],
				);
				ui.c.text(
					&elapsed,
					Rect::new(X0, y, CW, 20.0),
					&ui.style(13.0, 20.0, Weight::Regular, p.muted_foreground)
						.align(Align::End),
				);
				ui.progress(
					Rect::new(X0, y + 28.0, CW, 8.0),
					if done { Some(1.0) } else { None },
				);
				ui.c.text(
					t(lang, "webview.usually"),
					Rect::new(X0, y + 48.0, CW, 18.0),
					&ui.style(13.0, 18.0, Weight::Regular, p.muted_foreground),
				);
				ui.animating = true;
			}
			Wv::Error(details) => {
				let copied = self.copied_at.is_some();
				ui.error_block(
					X0,
					TOP + 8.0,
					CW,
					t(lang, "webview.errTitle"),
					t(lang, "webview.errText"),
					Some(&details),
					self.details_open,
					copied,
				);
			}
		}
	}

	fn paint_uninstall(&mut self, ui: &mut Ui) {
		let p = ui.p;
		let lang = self.lang;
		match self.un.clone() {
			Un::Confirm => {
				let y = self.title(ui, t(lang, "uninstall.title"), t(lang, "uninstall.text"));
				let label = match self.data_size {
					Some(s) if s > 0 => tf(lang, "uninstall.dataSize", &[("size", &size(lang, s))]),
					_ => t(lang, "uninstall.data").to_owned(),
				};
				ui.checkbox("deleteData", X0, y, &label, self.delete_data, CW);
				ui.c.text(
					t(lang, "uninstall.dataHint"),
					Rect::new(X0 + 28.0, y + 26.0, CW - 28.0, 18.0),
					&ui.style(13.0, 18.0, Weight::Regular, p.muted_foreground),
				);
				if self.delete_data {
					ui.c.icon(
						"triangle-exclamation-point",
						X0 + 28.0,
						y + 55.0,
						16.0,
						p.destructive,
						0.0,
					);
					ui.c.text(
						t(lang, "uninstall.warn"),
						Rect::new(X0 + 50.0, y + 54.0, CW - 50.0, 36.0),
						&ui.style(13.0, 18.0, Weight::Regular, p.destructive).wrap(),
					);
				}
			}
			Un::Running(_) => {
				let y = self.title(ui, t(lang, "uninstall.running"), t(lang, "uninstall.text"));
				ui.progress(Rect::new(X0, y, CW, 8.0), None);
			}
			Un::Done => {
				let text = if self.delete_data {
					t(lang, "uninstall.doneAll").to_owned()
				} else {
					tf(
						lang,
						"uninstall.doneData",
						&[("path", &sys::data_dirs()[0].to_string_lossy())],
					)
				};
				self.title(ui, t(lang, "uninstall.done"), &text);
			}
			Un::Error(details) => {
				let copied = self.copied_at.is_some();
				ui.error_block(
					X0,
					TOP + 8.0,
					CW,
					t(lang, "uninstall.errTitle"),
					t(lang, "uninstall.errText"),
					Some(&details),
					self.details_open,
					copied,
				);
			}
		}
	}

	fn footer_buttons(&self) -> Vec<FooterButton> {
		let lang = self.lang;
		let s = |k: &str| t(lang, k).to_owned();
		let mut out = Vec::new();
		if self.mode == Mode::Uninstall {
			match self.un {
				Un::Confirm => {
					out.push((
						"cancel",
						s("common.cancel"),
						Variant::Ghost,
						true,
						None,
						false,
					));
					out.push((
						"next",
						s("uninstall.action"),
						Variant::Destructive,
						true,
						None,
						false,
					));
				}
				Un::Running(_) => {
					out.push((
						"next",
						s("uninstall.running"),
						Variant::Destructive,
						false,
						None,
						true,
					));
				}
				Un::Done => {
					if !self.delete_data {
						out.push((
							"openFolder",
							s("uninstall.openFolder"),
							Variant::Ghost,
							true,
							None,
							false,
						));
					}
					out.push((
						"next",
						s("common.close"),
						Variant::Primary,
						true,
						None,
						false,
					));
				}
				Un::Error(_) => {
					out.push((
						"cancel",
						s("common.close"),
						Variant::Ghost,
						true,
						None,
						false,
					));
					out.push((
						"next",
						s("common.retry"),
						Variant::Primary,
						true,
						None,
						false,
					));
				}
			}
			return out;
		}
		if let Some(f) = &self.failure {
			if self.step == Step::Test {
				out.push((
					"finishAnyway",
					s("err.finishAnyway"),
					Variant::Ghost,
					true,
					None,
					false,
				));
			}
			if let Some((id, label)) = f.second {
				out.push((id, s(label), Variant::Ghost, true, None, false));
			}
			let retry_id = if self.step == Step::Welcome {
				"cancel"
			} else {
				"next"
			};
			let label = if self.step == Step::Welcome {
				s("common.close")
			} else {
				s("common.retry")
			};
			out.push((retry_id, label, Variant::Primary, true, None, false));
			return out;
		}
		match self.step {
			Step::Welcome => {
				out.push((
					"cancel",
					s("common.cancel"),
					Variant::Ghost,
					true,
					None,
					false,
				));
				let label = match (&self.existing, self.choice) {
					(Some(_), 1) => s("common.next"),
					(Some(e), _) => match install::version_key(install::VERSION)
						.cmp(&install::version_key(&e.version))
					{
						std::cmp::Ordering::Greater => s("existing.updateAction"),
						std::cmp::Ordering::Equal => s("existing.reinstall"),
						std::cmp::Ordering::Less => s("existing.downgradeAction"),
					},
					_ => s("common.next"),
				};
				out.push(("next", label, Variant::Primary, true, None, false));
			}
			Step::Folder | Step::Checks => {
				out.push(("back", s("common.back"), Variant::Ghost, true, None, false));
				out.push((
					"next",
					s("common.next"),
					Variant::Primary,
					self.next_enabled(),
					None,
					false,
				));
			}
			Step::WebView => match self.wv {
				Wv::Explain => {
					out.push(("back", s("common.back"), Variant::Ghost, true, None, false));
					out.push((
						"next",
						s("webview.install"),
						Variant::Primary,
						true,
						None,
						false,
					));
				}
				Wv::Running(_) | Wv::Done(..) => {
					out.push((
						"next",
						s("webview.installing"),
						Variant::Primary,
						false,
						None,
						true,
					));
				}
				Wv::Error(_) => {
					out.push((
						"wvSite",
						s("webview.site"),
						Variant::Ghost,
						true,
						Some("external-link"),
						false,
					));
					out.push((
						"next",
						s("common.retry"),
						Variant::Primary,
						true,
						None,
						false,
					));
				}
			},
			Step::Install | Step::Test => {
				out.push((
					"cancel",
					s("common.cancel"),
					Variant::Ghost,
					true,
					None,
					false,
				));
			}
			Step::Done => {
				out.push((
					"cancel",
					s("common.close"),
					Variant::Ghost,
					true,
					None,
					false,
				));
				out.push((
					"next",
					s("done.launch"),
					Variant::Primary,
					true,
					Some("play"),
					false,
				));
			}
		}
		out
	}

	fn paint_footer(&self, ui: &mut Ui) {
		let buttons = self.footer_buttons();
		let mut x = RIGHT;
		for (id, label, variant, enabled, icon, spin) in buttons.iter().rev() {
			let main = *id == "next" && *variant != Variant::Ghost;
			let h = if main && self.step == Step::Done && self.mode == Mode::Wizard {
				40.0
			} else {
				BUTTON_H
			};
			let w = ui.button_width(label, icon.is_some() || *spin);
			x -= w;
			ui.button(
				id,
				Rect::new(x, FOOT_Y + (BUTTON_H - h) / 2.0, w, h),
				label,
				*variant,
				*enabled,
				*icon,
				*spin,
			);
			x -= 8.0;
		}
	}

	fn paint_modal(&self, ui: &mut Ui, m: Modal) {
		let p = ui.p;
		let lang = self.lang;
		ui.modal(Rect::new(0.0, 0.0, W, H));
		ui.c.fill(
			Rect::new(0.0, 0.0, W, H),
			crate::theme::Color::hex(0).alpha(if p.dark { 0.6 } else { 0.3 }),
		);
		let (title, text) = match m {
			Modal::Cancel => ("cancel.title", "cancel.text"),
			Modal::WaitWebView => ("webview.waitTitle", "webview.waitText"),
		};
		let cw = 400.0;
		let body = ui
			.style(14.0, 22.0, Weight::Regular, p.muted_foreground)
			.wrap();
		let (_, th) = ui.c.g.measure(t(lang, text), &body, cw - 48.0);
		let ch = 24.0 + 24.0 + 8.0 + th + 24.0 + 36.0 + 24.0;
		let card = Rect::new((W - cw) / 2.0, (H - ch) / 2.0, cw, ch);
		ui.c.fill_round(card, 22.0, p.popover);
		ui.c.stroke_round(card, 22.0, 1.0, p.popover_border);
		ui.c.text(
			t(lang, title),
			Rect::new(card.x + 24.0, card.y + 24.0, cw - 48.0, 24.0),
			&ui.style(18.0, 24.0, Weight::SemiBold, p.foreground),
		);
		ui.c.text(
			t(lang, text),
			Rect::new(card.x + 24.0, card.y + 56.0, cw - 48.0, th),
			&body,
		);
		let by = card.bottom() - 24.0 - 36.0;
		let mut x = card.right() - 24.0;
		let buttons: Vec<(&str, &str, Variant)> = match m {
			Modal::Cancel => vec![
				("modal:stop", "cancel.stop", Variant::Destructive),
				("modal:stay", "cancel.keep", Variant::Primary),
			],
			Modal::WaitWebView => vec![("modal:stay", "webview.waitOk", Variant::Primary)],
		};
		for (id, key, variant) in buttons.iter().rev() {
			let label = t(lang, key);
			let w = ui.button_width(label, false);
			x -= w;
			ui.button(
				id,
				Rect::new(x, by, w, BUTTON_H),
				label,
				*variant,
				true,
				None,
				false,
			);
			x -= 8.0;
		}
	}

	fn paint_silent(&mut self, ui: &mut Ui) {
		let p = ui.p;
		let lang = self.lang;
		let (w, h) = self.size();
		ui.c.clear(p.background);
		ui.drag(Rect::new(0.0, 0.0, w, 36.0));
		ui.c.lockup(20.0, 11.0, 14.0, p.brand, p.foreground);
		ui.window_buttons(w, 36.0, true);
		if let Some(err) = self.silent_error.clone() {
			let _ = err;
			ui.c.icon("octagon-x", 24.0, 52.0, 20.0, p.destructive, 0.0);
			ui.c.text(
				t(lang, "silent.errTitle"),
				Rect::new(56.0, 52.0, w - 80.0, 20.0),
				&ui.style(14.0, 20.0, Weight::SemiBold, p.foreground),
			);
			ui.c.text(
				t(lang, "silent.errText"),
				Rect::new(56.0, 74.0, w - 80.0, 40.0),
				&ui.style(13.0, 18.0, Weight::Regular, p.muted_foreground)
					.wrap(),
			);
			let retry = t(lang, "common.retry");
			let open = t(lang, "silent.openPrevious");
			let rw = ui.button_width(retry, false) - 8.0;
			let ow = ui.button_width(open, false) - 8.0;
			ui.button(
				"next",
				Rect::new(w - 24.0 - rw, h - 24.0 - 32.0, rw, 32.0),
				retry,
				Variant::Primary,
				true,
				None,
				false,
			);
			ui.button(
				"openPrevious",
				Rect::new(w - 24.0 - rw - 8.0 - ow, h - 24.0 - 32.0, ow, 32.0),
				open,
				Variant::Ghost,
				true,
				None,
				false,
			);
		} else {
			ui.c.text(
				&tf(lang, "silent.title", &[("v", install::VERSION)]),
				Rect::new(24.0, 52.0, w - 48.0, 20.0),
				&ui.style(14.0, 20.0, Weight::SemiBold, p.foreground),
			);
			ui.c.text(
				t(lang, "silent.text"),
				Rect::new(24.0, 74.0, w - 48.0, 18.0),
				&ui.style(13.0, 18.0, Weight::Regular, p.muted_foreground),
			);
			ui.progress(Rect::new(24.0, 112.0, w - 48.0, 4.0), Some(self.progress));
			ui.c.text(
				&tf(
					lang,
					"silent.progress",
					&[(
						"percent",
						&((self.progress * 100.0).round() as u32).to_string(),
					)],
				),
				Rect::new(24.0, 128.0, w - 48.0, 16.0),
				&ui.style(12.0, 16.0, Weight::Regular, p.muted_foreground),
			);
		}
		if ui.win10 {
			ui.c.stroke_round(Rect::new(0.0, 0.0, w, h), 0.0, 1.0, p.border);
		}
	}
}

// --- фоновые задачи ---

fn rollback_all(dir: &Path, fresh: bool, created: bool, old_version: Option<&str>) {
	install::rollback(dir, created);
	if fresh {
		install::remove_system(dir);
	} else if let Some(v) = old_version {
		restore_version(v);
	}
}

/// Файлы вернулись к прежней версии — и запись в «Приложениях» тоже.
fn restore_version(v: &str) {
	let _ = sys::reg_write(
		sys::UNINSTALL_KEY,
		&[("DisplayVersion", sys::RegValue::Str(v))],
	);
}

fn uninstaller_bytes() -> Result<Vec<u8>, String> {
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	std::fs::read(&exe).map_err(|e| format!("{}: {e}", exe.display()))
}

fn install_all(
	dir: &Path,
	shortcuts: Shortcuts,
	created: bool,
	send: &Send,
) -> Result<(), (bool, String)> {
	sys::com_init();
	let launcher = install::payload().map_err(|e| (false, e.to_string()))?;
	let uninstaller = uninstaller_bytes().map_err(|e| (false, e))?;
	let files = install::Files {
		launcher: &launcher,
		uninstaller: &uninstaller,
	};
	let mut last = 0.0f32;
	install::install_files(dir, &files, &mut |p| {
		if p - last >= 0.02 || p >= 1.0 {
			last = p;
			send(Event::Progress(p));
		}
	})
	.map_err(|e| {
		(
			matches!(e, install::InstallError::NoSpace(_)),
			e.details().to_owned(),
		)
	})?;
	send(Event::Phase(0, Status::Ok));
	send(Event::Phase(1, Status::Running));
	install::apply_shortcuts(dir, shortcuts).map_err(|e| {
		install::rollback(dir, created);
		(false, e)
	})?;
	send(Event::Phase(1, Status::Ok));
	send(Event::Phase(2, Status::Running));
	install::apply_registry(dir, created).map_err(|e| (false, e))?;
	send(Event::Phase(2, Status::Ok));
	Ok(())
}

/// Пробный запуск лаунчера: `Ok((мс, версия WebView2))` или `Err((код, подробности))`.
pub fn self_test(
	dir: &Path,
	lang: Lang,
	slot: &Arc<Mutex<Option<std::process::Child>>>,
) -> Result<(u64, String), (i32, String)> {
	use std::io::Read;
	use std::os::windows::process::CommandExt;
	const CREATE_NO_WINDOW: u32 = 0x0800_0000;
	let started = std::time::Instant::now();
	let child = std::process::Command::new(dir.join(sys::EXE))
		.args(["--self-test", "--lang", lang.code()])
		.current_dir(dir)
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::null())
		.creation_flags(CREATE_NO_WINDOW)
		.spawn()
		.map_err(|e| (1, format!("запуск: {e}")))?;
	let pid = child.id();
	*slot.lock().unwrap() = Some(child);
	// Процессы WebView2 лаунчера: их запоминаем, пока он жив, и завершаем после, иначе они
	// держат временную папку пробного запуска.
	let mut tree: Vec<(u32, u64)> = Vec::new();
	let code = loop {
		std::thread::sleep(Duration::from_millis(100));
		let mut guard = slot.lock().unwrap();
		let Some(c) = guard.as_mut() else {
			break None;
		};
		for p in sys::descendants(pid) {
			if !tree.contains(&p) {
				tree.push(p);
			}
		}
		match c.try_wait() {
			Ok(Some(status)) => break status.code(),
			Ok(None) if started.elapsed() > Duration::from_secs(60) => {
				let _ = c.kill();
				break Some(2);
			}
			Ok(None) => {}
			Err(_) => break None,
		}
	};
	let mut out = String::new();
	if let Some(mut c) = slot.lock().unwrap().take()
		&& let Some(s) = c.stdout.as_mut()
	{
		let _ = s.read_to_string(&mut out);
	}
	sys::kill_processes(&tree);
	let temp = std::env::temp_dir().join(format!("LeeLauncher-selftest-{pid}"));
	std::thread::spawn(move || {
		for _ in 0..20 {
			if !temp.exists() || std::fs::remove_dir_all(&temp).is_ok() {
				break;
			}
			std::thread::sleep(Duration::from_millis(500));
		}
	});
	let field = |name: &str| {
		out.lines()
			.find_map(|l| l.strip_prefix(&format!("{name}=")))
			.map(str::to_owned)
	};
	match code {
		Some(0) => Ok((
			field("ms")
				.and_then(|m| m.parse().ok())
				.unwrap_or(started.elapsed().as_millis() as u64),
			field("webview").unwrap_or_default(),
		)),
		code => {
			let code = code.unwrap_or(1);
			Err((code, format!("exit code {code}\n{}", out.trim())))
		}
	}
}

/// Ставит WebView2: офлайн-установщик рядом, иначе встроенный загрузчик, иначе скачанный.
fn install_webview(own_exe: Option<&Path>) -> Result<String, String> {
	let temp = std::env::temp_dir();
	let offline = own_exe
		.and_then(Path::parent)
		.map(|d| d.join("MicrosoftEdgeWebView2RuntimeInstallerX64.exe"))
		.filter(|p| p.is_file());
	let (exe, cleanup) = if let Some(p) = offline {
		(p, false)
	} else {
		let p = temp.join(format!(
			"MicrosoftEdgeWebview2Setup-{}.exe",
			std::process::id()
		));
		if !install::WEBVIEW2.is_empty() {
			std::fs::write(&p, install::WEBVIEW2).map_err(|e| e.to_string())?;
		} else {
			sys::download(sys::WEBVIEW2_BOOTSTRAPPER_URL, &p)?;
		}
		(p, true)
	};
	let status = std::process::Command::new(&exe)
		.args(["/silent", "/install"])
		.status()
		.map_err(|e| format!("{}: {e}", exe.display()));
	if cleanup {
		let _ = std::fs::remove_file(&exe);
	}
	let status = status?;
	if !status.success() {
		return Err(format!(
			"exit code {:#x}",
			status.code().unwrap_or(-1) as u32
		));
	}
	sys::webview2_version().ok_or_else(|| "WebView2 не найден после установки".to_owned())
}

fn uninstall(dir: &Path, data: bool) -> Result<(), String> {
	sys::com_init();
	let me = std::env::current_exe().ok();
	// Признак читаем до того, как уберём запись в реестре.
	let ours = sys::install_dir_created();
	install::remove_system(dir);
	let mut errors = Vec::new();
	if let Err(e) = install::remove_files(dir, me.as_deref(), ours) {
		errors.push(e);
	}
	if data {
		for d in sys::data_dirs().into_iter().chain(sys::old_data_dirs()) {
			if let Err(e) = install::remove_data_dir(&d) {
				errors.push(e);
			}
		}
	}
	if errors.is_empty() {
		Ok(())
	} else {
		Err(errors.join("\n"))
	}
}

/// Тихое обновление из лаунчера: ждём его выхода, ставим, проверяем, при ошибке откатываем.
fn silent_update(
	dir: &Path,
	send: &Send,
	slot: &Arc<Mutex<Option<std::process::Child>>>,
) -> Result<(), String> {
	sys::com_init();
	if !sys::wait_launcher_closed(dir, Duration::from_secs(15)) {
		sys::close_windows_of(&sys::launcher_pids(dir));
		if !sys::wait_launcher_closed(dir, Duration::from_secs(5)) {
			return Err("LeeLauncher.exe не закрылся".into());
		}
	}
	let old_version = sys::existing_install().map(|e| e.version);
	let shortcuts = install::current_shortcuts();
	install_all(dir, shortcuts, false, send).map_err(|(_, e)| e)?;
	if let Err((code, details)) = self_test(dir, Lang::system(), slot) {
		install::rollback(dir, false);
		if let Some(v) = &old_version {
			restore_version(v);
		}
		return Err(format!("self-test {code}: {details}"));
	}
	install::commit(dir);
	Ok(())
}
