//! Установка и удаление файлов с откатом. Реестр и ярлыки — отдельно (`apply_system`).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::sys::{self, EXE, RegValue, UNINSTALL_EXE};

static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.bin"));
pub static WEBVIEW2: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/webview2.bin"));
pub const VERSION: &str = env!("APP_VERSION");

/// Место под лаунчер и запас на временные копии.
pub const NEED_BYTES: u64 = 200 * 1024 * 1024;

pub fn has_payload() -> bool {
	PAYLOAD.len() > 8
}

/// Распакованный лаунчер.
pub fn payload() -> io::Result<Vec<u8>> {
	if !has_payload() {
		return Err(io::Error::other("no payload"));
	}
	let size = u64::from_le_bytes(PAYLOAD[..8].try_into().unwrap()) as usize;
	let data = miniz_oxide::inflate::decompress_to_vec_with_limit(&PAYLOAD[8..], size)
		.map_err(|e| io::Error::other(format!("распаковка: {e:?}")))?;
	if data.len() != size {
		return Err(io::Error::other("распаковка: не тот размер"));
	}
	Ok(data)
}

/// Версия `x.y.z` как кортеж; лишнее после `-`/`+` не учитывается.
pub fn version_key(v: &str) -> (u64, u64, u64) {
	let core = v
		.trim()
		.trim_start_matches('v')
		.split(['-', '+'])
		.next()
		.unwrap_or("");
	let mut it = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
	(
		it.next().unwrap_or(0),
		it.next().unwrap_or(0),
		it.next().unwrap_or(0),
	)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Placement {
	Ok(PathBuf),
	/// Папка с данными лаунчера или профилем WebView2.
	DataFolder,
	/// Нужны права администратора.
	NeedsAdmin,
	/// Корень диска.
	DriveRoot,
}

/// Проверяет выбранную пользователем папку. Чужая непустая папка получает `\LeeLauncher` в конце.
pub fn place(chosen: &Path, data: &[PathBuf], protected: &[PathBuf]) -> Placement {
	let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
	if chosen.parent().is_none() || norm(chosen).len() <= 2 {
		return Placement::DriveRoot;
	}
	let mut dir = chosen.to_path_buf();
	let ours = dir.join(EXE).is_file();
	let empty = fs::read_dir(&dir)
		.map(|mut r| r.next().is_none())
		.unwrap_or(true);
	if !ours
		&& !empty
		&& !dir
			.file_name()
			.is_some_and(|n| n.eq_ignore_ascii_case(sys::PRODUCT))
	{
		dir.push(sys::PRODUCT);
	}
	let d = norm(&dir);
	for p in data {
		let p = norm(p);
		if d == p || d.starts_with(&format!("{p}\\")) {
			return Placement::DataFolder;
		}
	}
	for p in protected {
		let p = norm(p);
		if !p.is_empty() && (d == p || d.starts_with(&format!("{p}\\"))) {
			return Placement::NeedsAdmin;
		}
	}
	Placement::Ok(dir)
}

#[derive(Debug)]
pub enum InstallError {
	NoSpace(String),
	Io(String),
}

impl InstallError {
	pub fn details(&self) -> &str {
		match self {
			InstallError::NoSpace(s) | InstallError::Io(s) => s,
		}
	}
}

fn io_err(what: &str, path: &Path, e: io::Error) -> InstallError {
	let text = format!("{what} {}: {e}", path.display());
	// ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL.
	if matches!(e.raw_os_error(), Some(39 | 112)) {
		InstallError::NoSpace(text)
	} else {
		InstallError::Io(text)
	}
}

/// Файлы, которые кладёт установщик: (имя, содержимое).
pub struct Files<'a> {
	pub launcher: &'a [u8],
	pub uninstaller: &'a [u8],
}

const NEW: &str = ".new";
const OLD: &str = ".old";

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
	let mut s = path.as_os_str().to_owned();
	s.push(suffix);
	PathBuf::from(s)
}

/// Кладёт файлы рядом как `.new`, затем меняет местами: прежние становятся `.old`.
/// При ошибке всё возвращается как было. `progress` получает долю 0..1.
pub fn install_files(
	dir: &Path,
	files: &Files,
	progress: &mut dyn FnMut(f32),
) -> Result<(), InstallError> {
	fs::create_dir_all(dir).map_err(|e| io_err("папка", dir, e))?;
	let items = [(EXE, files.launcher), (UNINSTALL_EXE, files.uninstaller)];
	let total: usize = items.iter().map(|(_, d)| d.len()).sum::<usize>().max(1);
	let mut done = 0usize;
	for (name, data) in items {
		let new = with_suffix(&dir.join(name), NEW);
		let mut write = || -> io::Result<()> {
			let mut f = fs::File::create(&new)?;
			for chunk in data.chunks(256 * 1024) {
				f.write_all(chunk)?;
				done += chunk.len();
				progress(done as f32 / total as f32 * 0.9);
			}
			f.sync_all()
		};
		if let Err(e) = write() {
			discard_new(dir);
			return Err(io_err("запись", &new, e));
		}
	}
	let mut swapped: Vec<&str> = Vec::new();
	for (name, _) in items {
		let target = dir.join(name);
		let new = with_suffix(&target, NEW);
		let old = with_suffix(&target, OLD);
		let step = || -> io::Result<()> {
			let _ = fs::remove_file(&old);
			if target.exists() {
				fs::rename(&target, &old)?;
			}
			if let Err(e) = fs::rename(&new, &target) {
				let _ = fs::rename(&old, &target);
				return Err(e);
			}
			Ok(())
		};
		if let Err(e) = step() {
			restore(dir, &swapped);
			discard_new(dir);
			return Err(io_err("замена", &target, e));
		}
		swapped.push(name);
	}
	progress(1.0);
	Ok(())
}

fn discard_new(dir: &Path) {
	for name in [EXE, UNINSTALL_EXE] {
		let _ = fs::remove_file(with_suffix(&dir.join(name), NEW));
	}
}

fn restore(dir: &Path, names: &[&str]) {
	for name in names {
		let target = dir.join(name);
		let old = with_suffix(&target, OLD);
		if old.exists() {
			let _ = fs::remove_file(&target);
			let _ = fs::rename(&old, &target);
		} else {
			let _ = fs::remove_file(&target);
		}
	}
}

/// Откат после установки (отмена, неудачный пробный запуск при обновлении).
/// `created` — папку создала эта установка: пустую её можно убрать.
pub fn rollback(dir: &Path, created: bool) {
	restore(dir, &[EXE, UNINSTALL_EXE]);
	discard_new(dir);
	if created {
		let _ = fs::remove_dir(dir);
	}
}

/// Убирает прежние копии после удачной установки. Занятую файл удалит перезагрузка.
pub fn commit(dir: &Path) {
	for name in [EXE, UNINSTALL_EXE] {
		let old = with_suffix(&dir.join(name), OLD);
		if old.exists() && fs::remove_file(&old).is_err() {
			delete_on_reboot(&old);
		}
	}
}

fn delete_on_reboot(path: &Path) {
	use windows::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
	let w = windows::core::HSTRING::from(path.as_os_str());
	unsafe {
		let _ = MoveFileExW(&w, None, MOVEFILE_DELAY_UNTIL_REBOOT);
	}
}

/// Ярлыки, которые надо создать.
#[derive(Clone, Copy, Debug)]
pub struct Shortcuts {
	pub desktop: bool,
	pub start_menu: bool,
}

/// Ярлыки (создаются заново по пути к новому exe).
pub fn apply_shortcuts(dir: &Path, s: Shortcuts) -> Result<(), String> {
	let target = dir.join(EXE);
	for (want, link) in [
		(s.desktop, sys::desktop_link()),
		(s.start_menu, sys::start_menu_link()),
	] {
		if let (true, Some(link)) = (want, link) {
			sys::create_shortcut(&link, &target, "")?;
		}
	}
	Ok(())
}

/// Какие ярлыки уже есть (при обновлении ставим те же).
pub fn current_shortcuts() -> Shortcuts {
	Shortcuts {
		desktop: sys::desktop_link().is_some_and(|p| p.exists()),
		start_menu: sys::start_menu_link().is_some_and(|p| p.exists()),
	}
}

/// Запись в «Установленных приложениях» — тот же ключ, что у NSIS Tauri.
pub fn apply_registry(dir: &Path, created: bool) -> Result<(), String> {
	let exe = dir.join(EXE);
	let uninstall = dir.join(UNINSTALL_EXE);
	let size_kb = fs::metadata(&exe).map(|m| m.len() / 1024).unwrap_or(0)
		+ fs::metadata(&uninstall)
			.map(|m| m.len() / 1024)
			.unwrap_or(0);
	let icon = format!("\"{}\"", exe.display());
	let location = format!("\"{}\"", dir.display());
	let un = format!("\"{}\" --uninstall", uninstall.display());
	let quiet = format!("\"{}\" --uninstall --quiet", uninstall.display());
	sys::reg_write(
		sys::UNINSTALL_KEY,
		&[
			("DisplayName", RegValue::Str(sys::PRODUCT)),
			("DisplayIcon", RegValue::Str(&icon)),
			("DisplayVersion", RegValue::Str(VERSION)),
			("Publisher", RegValue::Str(sys::PUBLISHER)),
			("InstallLocation", RegValue::Str(&location)),
			("UninstallString", RegValue::Str(&un)),
			("QuietUninstallString", RegValue::Str(&quiet)),
			("MainBinaryName", RegValue::Str(EXE)),
			("URLInfoAbout", RegValue::Str(sys::HOMEPAGE)),
			("HelpLink", RegValue::Str(sys::HOMEPAGE)),
			("NoModify", RegValue::Dword(1)),
			("NoRepair", RegValue::Dword(1)),
			("EstimatedSize", RegValue::Dword(size_kb as u32)),
		],
	)?;
	if created {
		sys::reg_write(
			sys::UNINSTALL_KEY,
			&[(sys::CREATED_DIR_VALUE, RegValue::Dword(1))],
		)?;
	}
	let dir_s = dir.display().to_string();
	sys::reg_write(sys::PRODUCT_KEY, &[("", RegValue::Str(&dir_s))])
}

/// Убирает ярлыки и записи в реестре.
pub fn remove_system(dir: &Path) {
	let target = dir.join(EXE);
	for link in [sys::desktop_link(), sys::start_menu_link()]
		.into_iter()
		.flatten()
	{
		if link.exists() && shortcut_points_to(&link, &target) {
			let _ = fs::remove_file(&link);
		}
	}
	sys::reg_delete(sys::UNINSTALL_KEY);
	sys::reg_delete(sys::PRODUCT_KEY);
}

/// Ярлык ведёт на наш exe. Не прочитался — считаем, что наш (имя совпадает).
fn shortcut_points_to(link: &Path, target: &Path) -> bool {
	use windows::Win32::System::Com::{
		CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
	};
	use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
	use windows::core::Interface;
	unsafe {
		let read = || -> Option<String> {
			let shell: IShellLinkW =
				CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
			let file: IPersistFile = shell.cast().ok()?;
			file.Load(&windows::core::HSTRING::from(link.as_os_str()), STGM_READ)
				.ok()?;
			let mut buf = [0u16; 1024];
			shell.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
			let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
			Some(String::from_utf16_lossy(&buf[..len]))
		};
		read().is_none_or(|p| p.eq_ignore_ascii_case(&target.to_string_lossy()))
	}
}

/// Файлы программы при удалении. Чужие файлы в папке остаются, папка — если опустела.
/// `remove_dir` — убрать и опустевшую папку (только если её создал наш установщик).
pub fn remove_files(
	dir: &Path,
	keep_running_self: Option<&Path>,
	remove_dir: bool,
) -> Result<(), String> {
	let mut failed = Vec::new();
	for name in [EXE, UNINSTALL_EXE] {
		for path in [
			dir.join(name),
			with_suffix(&dir.join(name), OLD),
			with_suffix(&dir.join(name), NEW),
		] {
			if keep_running_self.is_some_and(|s| s == path) {
				continue;
			}
			if path.exists()
				&& let Err(e) = fs::remove_file(&path)
			{
				failed.push(format!("{}: {e}", path.display()));
			}
		}
	}
	if remove_dir {
		let _ = fs::remove_dir(dir);
	}
	if failed.is_empty() {
		Ok(())
	} else {
		Err(failed.join("\n"))
	}
}

/// Удаляет папку данных: сначала переименование, чтобы прерванное удаление не оставило
/// полусломанные данные под прежним именем.
pub fn remove_data_dir(dir: &Path) -> Result<(), String> {
	if !dir.exists() {
		return Ok(());
	}
	let stamp = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	let doomed = with_suffix(dir, &format!(".deleting-{stamp}"));
	fs::rename(dir, &doomed).map_err(|e| format!("{}: {e}", dir.display()))?;
	fs::remove_dir_all(&doomed).map_err(|e| format!("{}: {e}", doomed.display()))
}

/// Размер папки (для флажка «и мои данные»).
pub fn dir_size(dir: &Path) -> u64 {
	let mut total = 0u64;
	let mut stack = vec![dir.to_path_buf()];
	while let Some(d) = stack.pop() {
		let Ok(entries) = fs::read_dir(&d) else {
			continue;
		};
		for e in entries.flatten() {
			let Ok(meta) = e.metadata() else {
				continue;
			};
			if meta.is_dir() && !meta.file_type().is_symlink() {
				stack.push(e.path());
			} else {
				total += meta.len();
			}
		}
	}
	total
}

#[cfg(test)]
mod tests {
	use super::*;

	fn files<'a>(a: &'a [u8], b: &'a [u8]) -> Files<'a> {
		Files {
			launcher: a,
			uninstaller: b,
		}
	}

	#[test]
	fn installs_and_updates_with_rollback() {
		let tmp = std::env::temp_dir().join(format!("ll-inst-test-{}", std::process::id()));
		let _ = fs::remove_dir_all(&tmp);
		let dir = tmp.join("LeeLauncher");
		install_files(&dir, &files(b"v1", b"u1"), &mut |_| {}).unwrap();
		commit(&dir);
		assert_eq!(fs::read(dir.join(EXE)).unwrap(), b"v1");
		install_files(&dir, &files(b"v2", b"u2"), &mut |_| {}).unwrap();
		assert_eq!(fs::read(dir.join(EXE)).unwrap(), b"v2");
		assert!(dir.join("LeeLauncher.exe.old").exists());
		rollback(&dir, false);
		assert_eq!(fs::read(dir.join(EXE)).unwrap(), b"v1");
		assert_eq!(fs::read(dir.join(UNINSTALL_EXE)).unwrap(), b"u1");
		assert!(!dir.join("LeeLauncher.exe.old").exists());
		install_files(&dir, &files(b"v3", b"u3"), &mut |_| {}).unwrap();
		commit(&dir);
		assert!(!dir.join("LeeLauncher.exe.old").exists());
		fs::write(dir.join("notes.txt"), b"mine").unwrap();
		remove_files(&dir, None, true).unwrap();
		assert!(!dir.join(EXE).exists());
		assert!(dir.join("notes.txt").exists());
		fs::remove_file(dir.join("notes.txt")).unwrap();
		// Папку, которую создавали не мы, оставляем даже пустой.
		remove_files(&dir, None, false).unwrap();
		assert!(dir.exists());
		remove_files(&dir, None, true).unwrap();
		assert!(!dir.exists());
		let _ = fs::remove_dir_all(&tmp);
	}

	#[test]
	fn fresh_rollback_removes_folder() {
		let tmp = std::env::temp_dir().join(format!("ll-inst-fresh-{}", std::process::id()));
		let _ = fs::remove_dir_all(&tmp);
		install_files(&tmp, &files(b"v1", b"u1"), &mut |_| {}).unwrap();
		rollback(&tmp, true);
		assert!(!tmp.exists());
	}

	#[test]
	fn placement_rules() {
		let tmp = std::env::temp_dir().join(format!("ll-place-{}", std::process::id()));
		let _ = fs::remove_dir_all(&tmp);
		let data = vec![
			tmp.join("Roaming").join("LeeLauncher"),
			tmp.join("Local").join("LeeLauncher"),
		];
		let protected = vec![tmp.join("Program Files")];
		assert_eq!(
			place(Path::new("C:\\"), &data, &protected),
			Placement::DriveRoot
		);
		assert_eq!(place(&data[1], &data, &protected), Placement::DataFolder);
		assert_eq!(
			place(&data[0].join("x"), &data, &protected),
			Placement::DataFolder
		);
		assert_eq!(
			place(&tmp.join("Program Files").join("A"), &data, &protected),
			Placement::NeedsAdmin
		);
		// Чужая непустая папка — внутрь неё в «LeeLauncher».
		let other = tmp.join("Games");
		fs::create_dir_all(&other).unwrap();
		fs::write(other.join("x.txt"), b"x").unwrap();
		assert_eq!(
			place(&other, &data, &protected),
			Placement::Ok(other.join("LeeLauncher"))
		);
		let empty = tmp.join("Empty");
		fs::create_dir_all(&empty).unwrap();
		assert_eq!(
			place(&empty, &data, &protected),
			Placement::Ok(empty.clone())
		);
		let _ = fs::remove_dir_all(&tmp);
	}

	/// Собран с `LEELAUNCHER_PAYLOAD` — распакованный лаунчер совпадает с исходным файлом.
	#[test]
	fn payload_roundtrip() {
		let Some(path) = option_env!("LEELAUNCHER_PAYLOAD") else {
			return;
		};
		assert_eq!(payload().unwrap(), fs::read(path).unwrap());
	}

	#[test]
	fn compares_versions() {
		assert!(version_key("0.10.0") > version_key("0.9.9"));
		assert_eq!(version_key("v1.2.3-beta.1"), (1, 2, 3));
		assert_eq!(version_key(""), (0, 0, 0));
	}

	#[test]
	fn data_dir_removed_via_rename() {
		let tmp = std::env::temp_dir().join(format!("ll-data-{}", std::process::id()));
		let _ = fs::remove_dir_all(&tmp);
		fs::create_dir_all(tmp.join("a").join("b")).unwrap();
		fs::write(tmp.join("a").join("b").join("f"), vec![0u8; 1000]).unwrap();
		assert_eq!(dir_size(&tmp), 1000);
		remove_data_dir(&tmp).unwrap();
		assert!(!tmp.exists());
		assert!(remove_data_dir(&tmp).is_ok());
	}
}
