//! Перенос данных из `saLauncher` в `LeeLauncher` (лаунчер какое-то время назывался иначе).
//!
//! Папки данных и WebView2 переименовываются независимо; после сбоя следующий запуск доделывает.
//! Отметка [`MARKER`] едет вместе с папкой данных и снимается, когда пути в файлах переписаны.
//! Новая папка без единого файла убирается, с файлами — конфликт, работаем со старой.
//!
//! Вызывать до плагинов (плагин логов сразу создаёт `logs`). Логгера ещё нет, итог копится в [`Outcome`].
//!
//! Только Windows: на других ОС saLauncher не выходил, там сразу берутся новые папки.
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
use std::convert::Infallible;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use log::Level;
use serde_json::Value;

use crate::fs_util;

const OLD_DIR_NAME: &str = "saLauncher";
const NEW_DIR_NAME: &str = "LeeLauncher";
const MARKER: &str = ".rename-pending.json";
/// Копия до переписывания путей. Имя постоянное, чтобы повтор не плодил копий.
const BACKUP_SUFFIX: &str = ".bak-before-rename-back";
/// Замок ОС на время переноса: второй экземпляр ждёт здесь. После падения снимается сам.
const LOCK_FILE: &str = "LeeLauncher.migrate.lock";
/// Замок первого переноса: если тот упал перед уборкой, файл остался. Данных в нём нет.
const STALE_LOCK_FILE: &str = "saLauncher.migrate.lock";
/// Глубже пустые папки не ищем: такая папка считается непустой.
const HOLLOW_DEPTH: u32 = 16;
/// Антивирус и только что закрытый WebView2 держат файлы недолго: стоит подождать.
const RENAME_ATTEMPTS: u32 = 3;
const RENAME_PAUSE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
	pub old: PathBuf,
	pub new: PathBuf,
}

impl Folder {
	fn under(parent: &Path) -> Self {
		Self {
			old: parent.join(OLD_DIR_NAME),
			new: parent.join(NEW_DIR_NAME),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
	pub data: Folder,
	pub webview: Folder,
	lock: PathBuf,
	stale_lock: PathBuf,
}

impl Plan {
	/// `roaming` — `%APPDATA%`, `local` — `%LOCALAPPDATA%`.
	pub fn under(roaming: &Path, local: &Path) -> Self {
		Self {
			data: Folder::under(roaming),
			webview: Folder::under(local),
			lock: roaming.join(LOCK_FILE),
			stale_lock: roaming.join(STALE_LOCK_FILE),
		}
	}

	pub fn system() -> Option<Self> {
		Some(Self::under(&dirs::data_dir()?, &dirs::data_local_dir()?))
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderState {
	/// Ни старой, ни новой: первый запуск на этом компьютере.
	Fresh,
	/// Новая уже есть, старой нет: обычный запуск.
	Ready,
	/// Переименована на этом запуске.
	Moved,
	/// Есть обе, в новой есть файлы: ничего не тронуто, работаем со старой (в ней данные).
	Conflict,
	/// Старая занята или не переименовалась: на этом запуске работаем с ней, снова попробуем при следующем.
	Busy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
	pub data: FolderState,
	pub webview: FolderState,
	/// С какими папками работает этот запуск. Других источников путей быть не должно.
	pub data_dir: PathBuf,
	pub webview_dir: PathBuf,
	/// Сколько файлов с путями переписано на этом запуске. Вне тестов итог уже есть в записи для лога.
	#[cfg_attr(not(test), allow(dead_code))]
	pub rewritten: usize,
	/// Файлы, которые не удалось переписать: попробуем при следующем запуске.
	pub rewrite_failed: Vec<PathBuf>,
	/// Пары папок, где есть и старая, и новая: само это не разрешится, решать пользователю.
	pub conflicts: Vec<Folder>,
	notes: Vec<(Level, String)>,
}

impl Outcome {
	/// Перенос не закончен, следующий запуск его продолжит.
	pub fn unfinished(&self) -> bool {
		self.data == FolderState::Busy
			|| self.webview == FolderState::Busy
			|| !self.rewrite_failed.is_empty()
	}

	pub fn write_log(&self) {
		for (level, text) in &self.notes {
			log::log!(*level, "{text}");
		}
	}
}

/// На других ОС переносить нечего.
#[cfg(not(windows))]
pub fn run(plan: &Plan) -> Outcome {
	let state = if present(&plan.data.new) {
		FolderState::Ready
	} else {
		FolderState::Fresh
	};
	Outcome {
		data: state,
		webview: state,
		data_dir: plan.data.new.clone(),
		webview_dir: plan.webview.new.clone(),
		rewritten: 0,
		rewrite_failed: Vec::new(),
		conflicts: Vec::new(),
		notes: Vec::new(),
	}
}

/// Выполняет или продолжает перенос. Не падает: при любой ошибке отвечает, с какой папкой работать.
#[cfg(windows)]
pub fn run(plan: &Plan) -> Outcome {
	match run_with::<Infallible>(plan, &mut |_| Ok(())) {
		Ok(outcome) => outcome,
		Err(never) => match never {},
	}
}

/// `hook` вызывается после каждого шага: тесты обрывают работу на любом из них.
fn run_with<E>(plan: &Plan, hook: &mut dyn FnMut(&str) -> Result<(), E>) -> Result<Outcome, E> {
	let has_work = present(&plan.data.old)
		|| present(&plan.webview.old)
		|| present(&plan.data.new.join(MARKER));
	let mut notes = Vec::new();
	let lock = if has_work {
		take_lock(&plan.lock)
	} else {
		None
	};
	if has_work {
		hook("lock")?;
	}

	let data = settle(&plan.data, true, &mut notes, hook)?;
	let webview = settle(&plan.webview, false, &mut notes, hook)?;

	let (mut rewritten, mut rewrite_failed) = (0, Vec::new());
	let marker = plan.data.new.join(MARKER);
	if present(&marker) {
		(rewritten, rewrite_failed) = rewrite_paths(&plan.data, &mut notes, hook)?;
		if rewrite_failed.is_empty() {
			if let Err(err) = fs::remove_file(&marker) {
				notes.push((
					Level::Warn,
					format!("не удалось снять отметку переноса: {err}"),
				));
			}
			hook("marker removed")?;
			notes.push((
				Level::Info,
				format!("пути в файлах данных обновлены, файлов изменено: {rewritten}"),
			));
		} else {
			notes.push((
				Level::Warn,
				format!(
					"перенос: не переписано файлов: {}, продолжим при следующем запуске",
					rewrite_failed.len()
				),
			));
		}
	}

	drop(lock);
	// И при `has_work = false`: прошлый запуск мог упасть после всех шагов, но до этой строки.
	let _ = fs::remove_file(&plan.lock);
	let _ = fs::remove_file(&plan.stale_lock);

	let active = |state, folder: &Folder| match state {
		FolderState::Busy | FolderState::Conflict => folder.old.clone(),
		_ => folder.new.clone(),
	};
	let conflicts = [(data, &plan.data), (webview, &plan.webview)]
		.into_iter()
		.filter(|(state, _)| *state == FolderState::Conflict)
		.map(|(_, folder)| folder.clone())
		.collect();
	Ok(Outcome {
		conflicts,
		data_dir: active(data, &plan.data),
		webview_dir: active(webview, &plan.webview),
		data,
		webview,
		rewritten,
		rewrite_failed,
		notes,
	})
}

fn present(path: &Path) -> bool {
	!matches!(fs::symlink_metadata(path), Err(err) if err.kind() == io::ErrorKind::NotFound)
}

/// Не вышло — идём без замка: все шаги можно повторять, замок нужен только против гонки двух запусков.
fn take_lock(path: &Path) -> Option<fs::File> {
	let file = fs::OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(false)
		.open(path)
		.ok()?;
	file.lock().ok()?;
	Some(file)
}

fn settle<E>(
	folder: &Folder,
	with_marker: bool,
	notes: &mut Vec<(Level, String)>,
	hook: &mut dyn FnMut(&str) -> Result<(), E>,
) -> Result<FolderState, E> {
	let (old, mut new) = (present(&folder.old), present(&folder.new));
	if old && new && hollow(&folder.new, HOLLOW_DEPTH) {
		match remove_hollow(&folder.new) {
			Ok(()) => {
				notes.push((
					Level::Info,
					format!(
						"пустая папка без файлов убрана перед переносом: {}",
						folder.new.display()
					),
				));
				new = false;
			}
			Err(err) => notes.push((
				Level::Warn,
				format!(
					"не удалось убрать пустую папку {}: {err}",
					folder.new.display()
				),
			)),
		}
		hook("hollow removed")?;
	}
	match (old, new) {
		(false, false) => Ok(FolderState::Fresh),
		(false, true) => Ok(FolderState::Ready),
		(true, true) => {
			notes.push((
				Level::Warn,
				format!(
					"есть обе папки, ничего не трогаем, работаем со старой: {} и {}",
					folder.old.display(),
					folder.new.display()
				),
			));
			Ok(FolderState::Conflict)
		}
		(true, false) => {
			let marker = folder.old.join(MARKER);
			if with_marker {
				let body = serde_json::json!({
					"from": folder.old.to_string_lossy(),
					"to": folder.new.to_string_lossy(),
				})
				.to_string();
				if let Err(err) = fs_util::write_atomic(&marker, body.as_bytes()) {
					notes.push((
						Level::Warn,
						format!(
							"не удалось начать перенос {}: {err}; работаем со старой папкой",
							folder.old.display()
						),
					));
					return Ok(FolderState::Busy);
				}
				hook("marker written")?;
			}
			match rename_retrying(&folder.old, &folder.new) {
				Ok(()) => {
					hook("renamed")?;
					notes.push((
						Level::Info,
						format!(
							"папка перенесена: {} → {}",
							folder.old.display(),
							folder.new.display()
						),
					));
					Ok(FolderState::Moved)
				}
				// Другой запуск успел раньше (замок не взялся).
				Err(_) if !present(&folder.old) && present(&folder.new) => Ok(FolderState::Ready),
				Err(err) => {
					if with_marker {
						let _ = fs::remove_file(&marker);
					}
					notes.push((
						Level::Warn,
						format!(
							"папка занята, работаем со старой, перенос при следующем запуске: {}: {err}",
							folder.old.display()
						),
					));
					Ok(FolderState::Busy)
				}
			}
		}
	}
}

/// В папке нет ни одного файла, ссылки или точки повторной обработки, только пустые папки.
/// Не прочиталась — не пустая.
fn hollow(dir: &Path, depth: u32) -> bool {
	let Ok(meta) = fs::symlink_metadata(dir) else {
		return false;
	};
	if !meta.file_type().is_dir() || depth == 0 {
		return false;
	}
	let Ok(entries) = fs::read_dir(dir) else {
		return false;
	};
	entries
		.into_iter()
		.all(|entry| entry.is_ok_and(|entry| hollow(&entry.path(), depth - 1)))
}

/// Только `remove_dir`: если за это время в папке появился файл, удаление просто не пройдёт.
fn remove_hollow(dir: &Path) -> io::Result<()> {
	for entry in fs::read_dir(dir)? {
		let path = entry?.path();
		if fs::symlink_metadata(&path)?.file_type().is_dir() {
			remove_hollow(&path)?;
		}
	}
	fs::remove_dir(dir)
}

fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
	let mut attempt = 1;
	loop {
		match fs::rename(from, to) {
			Ok(()) => return Ok(()),
			Err(err) if attempt >= RENAME_ATTEMPTS || err.kind() == io::ErrorKind::NotFound => {
				return Err(err);
			}
			Err(_) => {
				attempt += 1;
				std::thread::sleep(RENAME_PAUSE);
			}
		}
	}
}

#[derive(Clone, Copy)]
enum FileKind {
	Settings,
	Instance,
}

/// Файлы, где бывают абсолютные пути (`launch-args.txt` пересоздаётся при запуске).
fn path_files(root: &Path) -> io::Result<Vec<(PathBuf, FileKind)>> {
	let mut files = vec![(root.join("settings.json"), FileKind::Settings)];
	let entries = match fs::read_dir(root.join("instances")) {
		Ok(entries) => entries,
		Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(files),
		Err(err) => return Err(err),
	};
	let mut instances = Vec::new();
	for entry in entries {
		let entry = entry?;
		// `.trash-*` и `.copy-*` удаляются при запуске.
		if !entry.file_name().to_string_lossy().starts_with('.') {
			instances.push((entry.path().join("instance.json"), FileKind::Instance));
		}
	}
	instances.sort_by(|a, b| a.0.cmp(&b.0));
	files.append(&mut instances);
	Ok(files)
}

fn rewrite_paths<E>(
	folder: &Folder,
	notes: &mut Vec<(Level, String)>,
	hook: &mut dyn FnMut(&str) -> Result<(), E>,
) -> Result<(usize, Vec<PathBuf>), E> {
	let (old, new) = (folder.old.to_string_lossy(), folder.new.to_string_lossy());
	let mut failed = Vec::new();
	let mut rewritten = 0;
	let instances = folder.new.join("instances");
	let files = match path_files(&folder.new) {
		Ok(files) => files,
		Err(err) => {
			notes.push((
				Level::Warn,
				format!("не удалось прочитать {}: {err}", instances.display()),
			));
			failed.push(instances);
			vec![(folder.new.join("settings.json"), FileKind::Settings)]
		}
	};
	for (path, kind) in files {
		let mut fail = |err: io::Error| {
			notes.push((Level::Warn, format!("{}: {err}", path.display())));
			failed.push(path.clone());
		};
		let change = match prepare(&path, kind, &old, &new) {
			Ok(change) => change,
			Err(err) => {
				fail(err);
				continue;
			}
		};
		let Some((original, updated)) = change else {
			continue;
		};
		if let Err(err) = write_backup(&path, &original) {
			fail(err);
			continue;
		}
		hook("backup written")?;
		if let Err(err) = fs_util::write_atomic(&path, &updated) {
			fail(err);
			continue;
		}
		hook("file rewritten")?;
		rewritten += 1;
	}
	Ok((rewritten, failed))
}

/// `Some((было, стало))`, если есть что менять. Повреждённый файл не трогаем.
fn prepare(
	path: &Path,
	kind: FileKind,
	old: &str,
	new: &str,
) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
	let bytes = match fs::read(path) {
		Ok(bytes) => bytes,
		Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
		Err(err) => return Err(err),
	};
	let Ok(mut json) = serde_json::from_slice::<Value>(&bytes) else {
		return Ok(None);
	};
	let mut changed = false;
	let mut edit = |value: &mut Value| {
		if let Some(text) = value.as_str()
			&& let Some(next) = replace_root(text, old, new)
		{
			*value = Value::String(next);
			changed = true;
		}
	};
	match kind {
		FileKind::Settings => {
			if let Some(paths) = json.get_mut("javaPaths").and_then(Value::as_object_mut) {
				paths.values_mut().for_each(&mut edit);
			}
		}
		FileKind::Instance => {
			for field in ["javaPath", "jvmArgs"] {
				if let Some(value) = json.get_mut(field) {
					edit(value);
				}
			}
		}
	}
	if !changed {
		return Ok(None);
	}
	let updated = serde_json::to_vec_pretty(&json).map_err(io::Error::other)?;
	Ok(Some((bytes, updated)))
}

/// Первая копия и есть исходная: если она уже есть, значит это повтор после сбоя.
fn write_backup(path: &Path, original: &[u8]) -> io::Result<()> {
	let mut backup = path.as_os_str().to_owned();
	backup.push(BACKUP_SUFFIX);
	let backup = PathBuf::from(backup);
	if present(&backup) {
		return Ok(());
	}
	fs_util::write_atomic(&backup, original)
}

/// Меняет корень `old` на `new` без учёта регистра и вида слэшей. `…\saLauncher2` не трогает.
fn replace_root(text: &str, old: &str, new: &str) -> Option<String> {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	let mut previous: Option<char> = None;
	let mut changed = false;
	while let Some(next) = rest.chars().next() {
		if !previous.is_some_and(char::is_alphanumeric)
			&& let Some((len, slashes)) = match_root(rest, old)
			&& is_root_end(rest[len..].chars().next())
		{
			if slashes {
				out.push_str(&new.replace('\\', "/"));
			} else {
				out.push_str(new);
			}
			rest = &rest[len..];
			previous = None;
			changed = true;
			continue;
		}
		out.push(next);
		rest = &rest[next.len_utf8()..];
		previous = Some(next);
	}
	changed.then_some(out)
}

/// Длина совпадения в байтах и был ли в нём прямой слэш.
fn match_root(text: &str, root: &str) -> Option<(usize, bool)> {
	let mut chars = text.chars();
	let mut len = 0;
	let mut slashes = false;
	for wanted in root.chars() {
		let found = chars.next()?;
		let separator = |c| c == '/' || c == '\\';
		if !(separator(found) && separator(wanted) || same_letter(found, wanted)) {
			return None;
		}
		slashes |= found == '/';
		len += found.len_utf8();
	}
	Some((len, slashes))
}

fn same_letter(a: char, b: char) -> bool {
	a == b || a.to_lowercase().eq(b.to_lowercase())
}

fn is_root_end(next: Option<char>) -> bool {
	matches!(next, None | Some('\\' | '/' | '"' | '\'' | ';'))
}

#[cfg(all(test, not(windows)))]
mod unix_tests {
	use super::*;

	#[test]
	fn nothing_to_move() {
		let tmp = tempfile::tempdir().unwrap();
		std::fs::create_dir_all(tmp.path().join(OLD_DIR_NAME)).unwrap();
		let plan = Plan::under(tmp.path(), tmp.path());
		let outcome = run(&plan);
		assert_eq!(outcome.data, FolderState::Fresh);
		assert_eq!(outcome.data_dir, tmp.path().join(NEW_DIR_NAME));
		assert!(!outcome.unfinished());
		assert!(tmp.path().join(OLD_DIR_NAME).exists());
	}
}

#[cfg(all(test, windows))]
mod tests {
	use std::collections::{BTreeMap, BTreeSet};

	use serde_json::json;

	use super::*;

	struct Fixture {
		_tmp: tempfile::TempDir,
		roaming: PathBuf,
		local: PathBuf,
		plan: Plan,
	}

	/// Папка пользователя с кириллицей и пробелом: такие пути ломаются первыми.
	fn fixture() -> Fixture {
		let tmp = tempfile::tempdir().unwrap();
		let home = tmp.path().join("Лена Иванова").join("AppData");
		let (roaming, local) = (home.join("Roaming"), home.join("Local"));
		fs::create_dir_all(&roaming).unwrap();
		fs::create_dir_all(&local).unwrap();
		let plan = Plan::under(&roaming, &local);
		Fixture {
			_tmp: tmp,
			roaming,
			local,
			plan,
		}
	}

	fn write(path: &Path, bytes: &[u8]) {
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(path, bytes).unwrap();
	}

	fn java_in(root: &Path) -> String {
		root.join(r"java\java-runtime-gamma\bin\javaw.exe")
			.to_string_lossy()
			.into_owned()
	}

	impl Fixture {
		/// Данные, как у настоящего пользователя: пути на свою Java и на чужую.
		fn with_old_data(self) -> Self {
			let (data, webview) = (&self.plan.data.old, &self.plan.webview.old);
			let java = java_in(data);
			write(
				&data.join("settings.json"),
				json!({ "version": 1, "javaPaths": { "17": java, "8": r"C:\Program Files\Java\javaw.exe" } })
					.to_string()
					.as_bytes(),
			);
			write(
				&data.join(r"instances\a\instance.json"),
				json!({
					"id": "a",
					"name": "Сборка с пробелом",
					"javaPath": java,
					"jvmArgs": format!("-Dx=\"{}\\x y\" -Xss1M", data.display()),
					"importedFrom": { "launcher": "prism", "path": "D:\\Games\\Prism" },
				})
				.to_string()
				.as_bytes(),
			);
			write(
				&data.join(r"instances\b\instance.json"),
				json!({ "id": "b", "javaPath": null })
					.to_string()
					.as_bytes(),
			);
			write(
				&data.join(r"instances\a\minecraft\saves\Мир\level.dat"),
				b"world",
			);
			write(&data.join("accounts.json"), b"{\"secret\":\"dpapi:x\"}");
			write(
				&data.join(r"java\java-runtime-gamma\.leelauncher-runtime.json"),
				b"{}",
			);
			write(
				&webview.join(r"EBWebView\Default\Local Storage\leveldb\000003.log"),
				b"ls",
			);
			self
		}
	}

	fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
		fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
			let Ok(entries) = fs::read_dir(dir) else {
				return;
			};
			for entry in entries.flatten() {
				let path = entry.path();
				let rel = path
					.strip_prefix(root)
					.unwrap()
					.to_string_lossy()
					.into_owned();
				if path.is_dir() {
					out.insert(format!("{rel}\\"), Vec::new());
					walk(root, &path, out);
				} else {
					out.insert(rel, fs::read(&path).unwrap());
				}
			}
		}
		let mut out = BTreeMap::new();
		walk(root, root, &mut out);
		out
	}

	fn names(root: &Path) -> BTreeSet<String> {
		tree(root).into_keys().collect()
	}

	fn read_json(path: &Path) -> Value {
		serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
	}

	/// Всё перенесено и дописано, лишнего нет. Проверяется и после прямого переноса, и после сбоев.
	fn assert_done(f: &Fixture, outcome: &Outcome) {
		let (old, new) = (&f.plan.data.old, &f.plan.data.new);
		assert!(
			!outcome.unfinished() && outcome.conflicts.is_empty(),
			"{outcome:?}"
		);
		assert_eq!(outcome.data_dir, *new);
		assert_eq!(outcome.webview_dir, f.plan.webview.new);
		assert!(!old.exists() && !f.plan.webview.old.exists());

		let settings = read_json(&new.join("settings.json"));
		assert_eq!(settings["javaPaths"]["17"], json!(java_in(new)));
		assert_eq!(
			settings["javaPaths"]["8"],
			r"C:\Program Files\Java\javaw.exe"
		);
		let a = read_json(&new.join(r"instances\a\instance.json"));
		assert_eq!(a["javaPath"], json!(java_in(new)));
		assert_eq!(
			a["jvmArgs"],
			json!(format!("-Dx=\"{}\\x y\" -Xss1M", new.display()))
		);
		assert_eq!(a["name"], "Сборка с пробелом");
		assert_eq!(a["importedFrom"]["path"], r"D:\Games\Prism");
		assert_eq!(
			fs::read(new.join(r"instances\a\minecraft\saves\Мир\level.dat")).unwrap(),
			b"world"
		);
		assert_eq!(
			fs::read(new.join("accounts.json")).unwrap(),
			b"{\"secret\":\"dpapi:x\"}"
		);
		assert!(
			new.join(r"java\java-runtime-gamma\.leelauncher-runtime.json")
				.is_file()
		);
		assert_eq!(
			fs::read(
				f.plan
					.webview
					.new
					.join(r"EBWebView\Default\Local Storage\leveldb\000003.log")
			)
			.unwrap(),
			b"ls"
		);

		let backup = read_json(&new.join("settings.json.bak-before-rename-back"));
		assert_eq!(backup["javaPaths"]["17"], json!(java_in(old)));
		let backup = read_json(&new.join(r"instances\a\instance.json.bak-before-rename-back"));
		assert_eq!(backup["javaPath"], json!(java_in(old)));

		let expected: BTreeSet<String> = [
			"accounts.json",
			"settings.json",
			"settings.json.bak-before-rename-back",
			"instances\\",
			"instances\\a\\",
			"instances\\a\\instance.json",
			"instances\\a\\instance.json.bak-before-rename-back",
			"instances\\a\\minecraft\\",
			"instances\\a\\minecraft\\saves\\",
			"instances\\a\\minecraft\\saves\\Мир\\",
			"instances\\a\\minecraft\\saves\\Мир\\level.dat",
			"instances\\b\\",
			"instances\\b\\instance.json",
			"java\\",
			"java\\java-runtime-gamma\\",
			"java\\java-runtime-gamma\\.leelauncher-runtime.json",
		]
		.into_iter()
		.map(String::from)
		.collect();
		assert_eq!(names(new), expected);
		assert_eq!(names(&f.roaming), {
			let mut top = BTreeSet::from(["LeeLauncher\\".to_owned()]);
			top.extend(expected.iter().map(|n| format!("LeeLauncher\\{n}")));
			top
		});
	}

	#[test]
	fn system_plan_uses_user_folders() {
		let plan = Plan::system().unwrap();
		assert!(plan.data.old.ends_with("Roaming/saLauncher"));
		assert!(plan.data.new.ends_with("Roaming/LeeLauncher"));
		assert!(plan.webview.old.ends_with("Local/saLauncher"));
		assert!(plan.webview.new.ends_with("Local/LeeLauncher"));
	}

	#[test]
	fn moves_and_rewrites() {
		let f = fixture().with_old_data();
		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Moved);
		assert_eq!(outcome.webview, FolderState::Moved);
		assert_eq!(outcome.rewritten, 2);
		assert_done(&f, &outcome);
		assert!(
			outcome
				.notes
				.iter()
				.any(|(l, t)| *l == Level::Info && t.contains("перенесена"))
		);
		outcome.write_log();
	}

	#[test]
	fn second_run_changes_nothing() {
		let f = fixture().with_old_data();
		run(&f.plan);
		let (roaming, local) = (tree(&f.roaming), tree(&f.local));
		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Ready);
		assert_eq!(outcome.webview, FolderState::Ready);
		assert_eq!(outcome.rewritten, 0);
		assert!(outcome.notes.is_empty());
		assert_done(&f, &outcome);
		assert_eq!(tree(&f.roaming), roaming);
		assert_eq!(tree(&f.local), local);
	}

	#[test]
	fn fresh_install_creates_nothing() {
		let f = fixture();
		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Fresh);
		assert_eq!(outcome.webview, FolderState::Fresh);
		assert_eq!(outcome.data_dir, f.plan.data.new);
		assert_eq!(outcome.webview_dir, f.plan.webview.new);
		assert!(tree(&f.roaming).is_empty() && tree(&f.local).is_empty());
	}

	#[test]
	fn both_folders_are_left_alone() {
		let f = fixture().with_old_data();
		write(&f.plan.data.new.join("settings.json"), b"{\"new\":true}");
		write(&f.plan.webview.new.join("x"), b"new");
		let (roaming, local) = (tree(&f.roaming), tree(&f.local));
		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Conflict);
		assert_eq!(outcome.webview, FolderState::Conflict);
		assert_eq!(
			outcome.conflicts,
			vec![f.plan.data.clone(), f.plan.webview.clone()]
		);
		assert!(!outcome.unfinished());
		// Данные — в старой папке: с ней и работаем, новую не трогаем.
		assert_eq!(outcome.data_dir, f.plan.data.old);
		assert_eq!(outcome.webview_dir, f.plan.webview.old);
		assert_eq!(tree(&f.roaming), roaming);
		assert_eq!(tree(&f.local), local);
		assert!(
			outcome
				.notes
				.iter()
				.any(|(l, t)| *l == Level::Warn && t.contains("обе папки"))
		);
	}

	#[test]
	fn empty_new_folders_are_replaced() {
		let f = fixture().with_old_data();
		// Пустой `LeeLauncher` рядом с полным `saLauncher`: файлов нет, данными не считается.
		fs::create_dir_all(&f.plan.data.new).unwrap();
		fs::create_dir_all(f.plan.webview.new.join(r"EBWebView\Default")).unwrap();
		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Moved);
		assert_eq!(outcome.webview, FolderState::Moved);
		assert_done(&f, &outcome);
	}

	#[test]
	fn crash_after_removing_empty_folder_finishes() {
		let f = fixture().with_old_data();
		fs::create_dir_all(&f.plan.data.new).unwrap();
		// lock, hollow removed — сбой сразу после уборки пустой папки.
		assert!(crash_at(&f.plan, 1));
		assert!(!f.plan.data.new.exists() && f.plan.data.old.is_dir());
		assert_done(&f, &run(&f.plan));
	}

	#[test]
	fn new_folder_with_any_file_is_a_conflict() {
		let f = fixture().with_old_data();
		write(&f.plan.data.new.join(r"logs\leelauncher.log"), b"");
		fs::create_dir_all(f.plan.data.new.join("empty")).unwrap();
		let (old, new) = (tree(&f.plan.data.old), tree(&f.plan.data.new));
		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Conflict);
		assert_eq!(outcome.data_dir, f.plan.data.old);
		assert_eq!(outcome.conflicts, vec![f.plan.data.clone()]);
		assert_eq!(tree(&f.plan.data.old), old);
		assert_eq!(
			tree(&f.plan.data.new),
			new,
			"в конфликте даже пустые папки не трогаем"
		);
		// Папки независимы: профиль WebView2 всё равно переехал.
		assert_eq!(outcome.webview, FolderState::Moved);
		assert_eq!(outcome.webview_dir, f.plan.webview.new);
	}

	#[test]
	fn plugin_folder_is_not_data() {
		// Пустую `com.leelauncher.app` создаёт плагин window-state, к переносу она не относится.
		let f = fixture().with_old_data();
		let plugin = f.roaming.join("com.leelauncher.app");
		fs::create_dir_all(&plugin).unwrap();
		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Moved);
		assert!(plugin.is_dir());
		fs::remove_dir(&plugin).unwrap();
		assert_done(&f, &outcome);
	}

	#[test]
	fn leftovers_of_first_rename_are_finished() {
		let f = fixture().with_old_data();
		let data = &f.plan.data.old;
		// Первый перенос не закончил: его отметка и копия, путь в сборке b ещё на `LeeLauncher`.
		write(&data.join(MARKER), b"{}");
		write(&data.join("settings.json.bak-before-rename"), b"first");
		let lee_java = java_in(&f.plan.data.new);
		write(
			&data.join(r"instances\b\instance.json"),
			json!({ "id": "b", "javaPath": lee_java })
				.to_string()
				.as_bytes(),
		);
		write(&f.roaming.join(STALE_LOCK_FILE), b"");

		let outcome = run(&f.plan);
		let new = &f.plan.data.new;
		assert!(!outcome.unfinished(), "{outcome:?}");
		assert_eq!(outcome.rewritten, 2);
		assert!(!new.join(MARKER).exists());
		assert!(!f.roaming.join(STALE_LOCK_FILE).exists());
		assert_eq!(
			fs::read(new.join("settings.json.bak-before-rename")).unwrap(),
			b"first",
			"копия первого переноса не перезаписана"
		);
		assert!(new.join("settings.json.bak-before-rename-back").is_file());
		let b = read_json(&new.join(r"instances\b\instance.json"));
		assert_eq!(b["javaPath"], json!(lee_java));
		assert!(
			!new.join(r"instances\b\instance.json.bak-before-rename-back")
				.exists()
		);
	}

	#[cfg(windows)]
	fn hold(path: &Path) -> fs::File {
		use std::os::windows::fs::OpenOptionsExt;
		fs::OpenOptions::new()
			.read(true)
			.share_mode(0)
			.open(path)
			.unwrap()
	}

	#[cfg(windows)]
	#[test]
	fn busy_data_folder_keeps_working_with_old() {
		let f = fixture().with_old_data();
		let before = tree(&f.plan.data.old);
		let held = hold(&f.plan.data.old.join("settings.json"));

		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Busy);
		assert_eq!(outcome.data_dir, f.plan.data.old);
		assert!(outcome.unfinished());
		assert!(!f.plan.data.new.exists(), "новая папка не должна появиться");
		// Отметка убрана, файлы не менялись: занятая папка осталась ровно такой же.
		drop(held);
		assert_eq!(tree(&f.plan.data.old), before);
		// Профиль WebView2 свободен и переехал: смешанное состояние рабочее.
		assert_eq!(outcome.webview, FolderState::Moved);
		assert_eq!(outcome.webview_dir, f.plan.webview.new);

		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Moved);
		assert_eq!(outcome.webview, FolderState::Ready);
		assert_done(&f, &outcome);
	}

	#[cfg(windows)]
	#[test]
	fn busy_webview_folder_keeps_working_with_old() {
		let f = fixture().with_old_data();
		let held = hold(
			&f.plan
				.webview
				.old
				.join(r"EBWebView\Default\Local Storage\leveldb\000003.log"),
		);
		let outcome = run(&f.plan);
		assert_eq!(outcome.webview, FolderState::Busy);
		assert_eq!(outcome.webview_dir, f.plan.webview.old);
		assert!(!f.plan.webview.new.exists());
		assert_eq!(outcome.data, FolderState::Moved);
		assert!(outcome.unfinished());
		drop(held);
		let outcome = run(&f.plan);
		assert_eq!(outcome.webview, FolderState::Moved);
		assert_done(&f, &outcome);
	}

	#[derive(Debug)]
	struct Crash;

	/// Сбой на шаге `at`: `true`, если до него дошло.
	fn crash_at(plan: &Plan, at: usize) -> bool {
		let mut step = 0;
		let result = run_with(plan, &mut |_| {
			let reached = step == at;
			step += 1;
			if reached { Err(Crash) } else { Ok(()) }
		});
		result.is_err()
	}

	/// Каждый файл лежит ровно в одном месте, потерь и копий нет.
	fn assert_nothing_lost(f: &Fixture) {
		for (folder, file) in [
			(&f.plan.data, r"instances\a\minecraft\saves\Мир\level.dat"),
			(&f.plan.data, "accounts.json"),
			(
				&f.plan.webview,
				r"EBWebView\Default\Local Storage\leveldb\000003.log",
			),
		] {
			assert!(
				folder.old.join(file).is_file() != folder.new.join(file).is_file(),
				"{file}"
			);
		}
	}

	#[test]
	fn crash_at_any_step_finishes_on_next_run() {
		let mut steps = 0;
		loop {
			let f = fixture().with_old_data();
			if !crash_at(&f.plan, steps) {
				break;
			}
			assert_nothing_lost(&f);
			assert_done(&f, &run(&f.plan));
			steps += 1;
		}
		// Замок, отметка, две папки, по копии и записи на два файла, снятие отметки.
		assert_eq!(steps, 9);
	}

	#[test]
	fn crash_twice_in_a_row_still_finishes() {
		for first in 0..9 {
			for second in 0..9 {
				let f = fixture().with_old_data();
				assert!(crash_at(&f.plan, first));
				assert_nothing_lost(&f);
				crash_at(&f.plan, second);
				assert_nothing_lost(&f);
				assert_done(&f, &run(&f.plan));
			}
		}
	}

	#[test]
	fn crash_after_rename_keeps_the_pending_marker() {
		let f = fixture().with_old_data();
		// lock, marker written, renamed — сбой сразу после переименования папки данных.
		assert!(crash_at(&f.plan, 2));
		assert!(!f.plan.data.old.exists());
		assert!(f.plan.data.new.join(MARKER).is_file());
		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Ready);
		assert_eq!(outcome.webview, FolderState::Moved);
		assert_eq!(outcome.rewritten, 2);
		assert_done(&f, &outcome);
	}

	#[test]
	fn leftovers_of_interrupted_write_are_consumed() {
		let f = fixture().with_old_data();
		// Обрыв внутри write_atomic: недописанные файлы рядом с настоящими.
		write(&f.plan.data.old.join(format!("{MARKER}.tmp")), b"{");
		write(&f.plan.data.old.join("settings.json.tmp"), b"{");
		write(
			&f.plan
				.data
				.old
				.join(format!("settings.json{BACKUP_SUFFIX}.tmp")),
			b"{",
		);
		assert_done(&f, &run(&f.plan));
	}

	#[test]
	fn existing_backup_is_not_overwritten() {
		let f = fixture().with_old_data();
		let backup = f
			.plan
			.data
			.old
			.join(format!("settings.json{BACKUP_SUFFIX}"));
		write(&backup, b"first");
		run(&f.plan);
		let backup = f
			.plan
			.data
			.new
			.join(format!("settings.json{BACKUP_SUFFIX}"));
		assert_eq!(fs::read(backup).unwrap(), b"first");
	}

	#[test]
	fn failed_file_is_retried_next_run() {
		let f = fixture().with_old_data();
		let file = f.plan.data.old.join(r"instances\a\instance.json");
		let original = fs::read(&file).unwrap();
		// Папка вместо файла — чтение не удаётся, как у занятого файла.
		fs::remove_file(&file).unwrap();
		fs::create_dir(&file).unwrap();

		let outcome = run(&f.plan);
		assert_eq!(outcome.data, FolderState::Moved);
		assert_eq!(outcome.data_dir, f.plan.data.new);
		assert!(outcome.unfinished());
		assert_eq!(outcome.rewritten, 1);
		let file = f.plan.data.new.join(r"instances\a\instance.json");
		assert_eq!(outcome.rewrite_failed, vec![file.clone()]);
		assert!(
			f.plan.data.new.join(MARKER).is_file(),
			"отметка остаётся до конца"
		);

		fs::remove_dir(&file).unwrap();
		fs::write(&file, original).unwrap();
		let outcome = run(&f.plan);
		assert_eq!(
			outcome.rewritten, 1,
			"уже переписанное второй раз не трогаем"
		);
		assert_done(&f, &outcome);
	}

	#[test]
	fn untouched_files_get_no_backup() {
		let f = fixture();
		let data = &f.plan.data.old;
		let damaged = b"{ not json".to_vec();
		let foreign =
			json!({ "id": "c", "javaPath": r"D:\saLauncher\java.exe", "jvmArgs": "-Xss1M" })
				.to_string();
		write(&data.join("settings.json"), &damaged);
		write(&data.join(r"instances\c\instance.json"), foreign.as_bytes());
		fs::create_dir_all(data.join(r"instances\d")).unwrap();
		let trash = json!({ "javaPath": java_in(data) }).to_string();
		write(
			&data.join(r"instances\.trash-e\instance.json"),
			trash.as_bytes(),
		);

		let outcome = run(&f.plan);
		let new = &f.plan.data.new;
		assert_eq!(outcome.rewritten, 0);
		assert!(!outcome.unfinished());
		assert_eq!(fs::read(new.join("settings.json")).unwrap(), damaged);
		assert_eq!(
			fs::read(new.join(r"instances\c\instance.json")).unwrap(),
			foreign.as_bytes()
		);
		assert_eq!(
			fs::read(new.join(r"instances\.trash-e\instance.json")).unwrap(),
			trash.as_bytes()
		);
		assert!(
			!names(new)
				.iter()
				.any(|n| n.contains(".bak") || n.contains(".tmp"))
		);
		assert!(!new.join(r"instances\d\instance.json").exists());
	}

	#[test]
	fn replace_root_cases() {
		let old = r"C:\Users\Лена Иванова\AppData\Roaming\saLauncher";
		let new = r"C:\Users\Лена Иванова\AppData\Roaming\LeeLauncher";
		let r = |text: &str| replace_root(text, old, new);
		let java = r"\java\javaw.exe";

		assert_eq!(r(old).as_deref(), Some(new));
		assert_eq!(r(&format!("{old}{java}")), Some(format!("{new}{java}")));
		// Регистр у Windows не важен; хвост пути остаётся как был.
		let upper = r"c:\users\ЛЕНА ИВАНОВА\appdata\roaming\salauncher\Java\A.exe";
		assert_eq!(r(upper), Some(format!(r"{new}\Java\A.exe")));
		// Прямые слэши остаются прямыми.
		assert_eq!(
			r("C:/Users/Лена Иванова/AppData/Roaming/saLauncher/java/a.exe").as_deref(),
			Some("C:/Users/Лена Иванова/AppData/Roaming/LeeLauncher/java/a.exe")
		);
		assert_eq!(
			r(&format!(r"\\?\{old}{java}")),
			Some(format!(r"\\?\{new}{java}"))
		);
		// Несколько путей в аргументах JVM.
		assert_eq!(
			r(&format!("-a=\"{old}\\x\" -b={old}\\y -cp {old};{old}\\z")),
			Some(format!("-a=\"{new}\\x\" -b={new}\\y -cp {new};{new}\\z"))
		);

		for same in [
			format!("{old}2{java}"),
			format!("{old} Backup{java}"),
			format!("{old}.old{java}"),
			format!("{old}_{java}"),
			format!("X{old}{java}"),
			r"D:\Games\saLauncher\java\a.exe".to_owned(),
			r"C:\Users\Лена Иванова\AppData\Local\saLauncher".to_owned(),
			new.to_owned(),
			String::new(),
		] {
			assert_eq!(r(&same), None, "{same}");
		}

		// Старое название выше по пути: меняется только сама папка данных.
		let old = r"C:\Users\saLauncher\AppData\Roaming\saLauncher";
		let new = r"C:\Users\saLauncher\AppData\Roaming\LeeLauncher";
		assert_eq!(
			replace_root(&format!(r"{old}\a"), old, new),
			Some(format!(r"{new}\a"))
		);
		assert_eq!(replace_root(&format!(r"{new}\a"), old, new), None);
	}

	#[test]
	fn two_starts_at_once_move_once() {
		let f = fixture().with_old_data();
		let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
			let spawned: Vec<_> = (0..4).map(|_| scope.spawn(|| run(&f.plan))).collect();
			spawned.into_iter().map(|t| t.join().unwrap()).collect()
		});
		let moved = |pick: fn(&Outcome) -> FolderState| {
			outcomes
				.iter()
				.filter(|o| pick(o) == FolderState::Moved)
				.count()
		};
		assert_eq!(moved(|o| o.data), 1);
		assert_eq!(moved(|o| o.webview), 1);
		for outcome in &outcomes {
			assert_eq!(outcome.data_dir, f.plan.data.new);
			assert_eq!(outcome.webview_dir, f.plan.webview.new);
		}
		assert_done(&f, &run(&f.plan));
	}
}
