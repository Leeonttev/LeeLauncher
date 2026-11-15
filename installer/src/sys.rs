//! Обращения к Windows: версия, место, реестр, процессы, ярлыки, папки, буфер обмена.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::{
	CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
	CoTaskMemFree, IPersistFile,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
	CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::*;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::{
	OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{
	EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_SHOWNORMAL, WM_CLOSE,
};
use windows::core::{BOOL, GUID, HSTRING, Interface, PCWSTR, PWSTR, w};

pub const PRODUCT: &str = "LeeLauncher";
pub const EXE: &str = "LeeLauncher.exe";
pub const UNINSTALL_EXE: &str = "uninstall.exe";
pub const PUBLISHER: &str = "leeonttev";
pub const APP_ID: &str = "com.leelauncher.app";
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\LeeLauncher";
pub const PRODUCT_KEY: &str = r"Software\leeonttev\LeeLauncher";
const WEBVIEW2_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
pub const WEBVIEW2_BOOTSTRAPPER_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
pub const WEBVIEW2_ABOUT_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";
pub const HOMEPAGE: &str = "https://github.com/leeonttev/LeeLauncher";

fn wide(s: &str) -> Vec<u16> {
	s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_os(p: &Path) -> Vec<u16> {
	p.as_os_str()
		.encode_wide()
		.chain(std::iter::once(0))
		.collect()
}

pub fn com_init() {
	unsafe {
		let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
	}
}

/// Номер сборки Windows и «10» или «11».
pub fn windows_version() -> (u32, u32, &'static str) {
	let mut info = OSVERSIONINFOW {
		dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
		..Default::default()
	};
	unsafe {
		let _ = windows::Wdk::System::SystemServices::RtlGetVersion(&mut info);
	}
	let name = if info.dwBuildNumber >= 22000 {
		"11"
	} else {
		"10"
	};
	(info.dwMajorVersion, info.dwBuildNumber, name)
}

/// Windows 10 1809 или новее.
pub fn windows_supported() -> bool {
	let (major, build, _) = windows_version();
	major > 10 || (major == 10 && build >= 17763)
}

/// Ближайшая существующая папка вверх по пути.
pub fn existing_ancestor(path: &Path) -> Option<PathBuf> {
	let mut p = path.to_path_buf();
	loop {
		if p.is_dir() {
			return Some(p);
		}
		if !p.pop() {
			return None;
		}
	}
}

pub fn disk_free(path: &Path) -> Option<u64> {
	let dir = existing_ancestor(path)?;
	let w = wide_os(&dir);
	let mut free = 0u64;
	unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), None, None) }.ok()?;
	Some(free)
}

/// «C:» для пути.
pub fn drive_of(path: &Path) -> String {
	path.components()
		.next()
		.map(|c| {
			c.as_os_str()
				.to_string_lossy()
				.trim_end_matches('\\')
				.to_owned()
		})
		.unwrap_or_default()
}

/// Можно ли писать туда, где появится папка: пробный файл в ближайшей существующей папке.
pub fn can_write(path: &Path) -> bool {
	let Some(dir) = existing_ancestor(path) else {
		return false;
	};
	let probe = dir.join(format!(".leelauncher-probe-{}", std::process::id()));
	let ok = std::fs::write(&probe, b"ok").is_ok();
	let _ = std::fs::remove_file(&probe);
	ok
}

pub fn known_folder(id: &GUID) -> Option<PathBuf> {
	unsafe {
		let p: PWSTR = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
		let s = p.to_string().ok();
		CoTaskMemFree(Some(p.0 as _));
		s.map(PathBuf::from)
	}
}

pub fn local_app_data() -> PathBuf {
	known_folder(&FOLDERID_LocalAppData)
		.or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
		.unwrap_or_else(|| PathBuf::from(r"C:\"))
}

pub fn roaming_app_data() -> PathBuf {
	known_folder(&FOLDERID_RoamingAppData)
		.or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
		.unwrap_or_else(|| PathBuf::from(r"C:\"))
}

pub fn default_dir() -> PathBuf {
	local_app_data().join("Programs").join(PRODUCT)
}

/// Данные лаунчера и профиль WebView2.
pub fn data_dirs() -> [PathBuf; 2] {
	[
		roaming_app_data().join(PRODUCT),
		local_app_data().join(PRODUCT),
	]
}

/// Папки прежнего названия (saLauncher) и плагина window-state: удаляются вместе с данными.
pub fn old_data_dirs() -> [PathBuf; 5] {
	[
		roaming_app_data().join("saLauncher"),
		local_app_data().join("saLauncher"),
		roaming_app_data().join("com.salauncher.app"),
		roaming_app_data().join(APP_ID),
		local_app_data().join(APP_ID),
	]
}

/// Признак в реестре: папку установки создал наш установщик (её можно убрать при удалении).
pub const CREATED_DIR_VALUE: &str = "InstallDirCreated";

pub fn install_dir_created() -> bool {
	reg_read_dword(HKEY_CURRENT_USER, UNINSTALL_KEY, CREATED_DIR_VALUE) == Some(1)
}

/// Убирает папки пробных запусков, оставшиеся от прошлых установок (занятые — пропускает).
pub fn clean_self_test_dirs() {
	let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
		return;
	};
	for e in entries.flatten() {
		if e.file_name()
			.to_string_lossy()
			.starts_with("LeeLauncher-selftest-")
			&& e.path().is_dir()
		{
			let _ = std::fs::remove_dir_all(e.path());
		}
	}
}

pub fn desktop_link() -> Option<PathBuf> {
	known_folder(&FOLDERID_Desktop).map(|d| d.join(format!("{PRODUCT}.lnk")))
}

pub fn start_menu_link() -> Option<PathBuf> {
	known_folder(&FOLDERID_Programs).map(|d| d.join(format!("{PRODUCT}.lnk")))
}

pub fn program_files() -> Vec<PathBuf> {
	[
		FOLDERID_ProgramFiles,
		FOLDERID_ProgramFilesX86,
		FOLDERID_Windows,
	]
	.iter()
	.filter_map(known_folder)
	.collect()
}

// --- реестр ---

fn reg_read_string(root: HKEY, key: &str, name: &str) -> Option<String> {
	let key = HSTRING::from(key);
	let name_w = HSTRING::from(name);
	let value = if name.is_empty() {
		PCWSTR::null()
	} else {
		PCWSTR(name_w.as_ptr())
	};
	let mut size = 0u32;
	unsafe {
		if RegGetValueW(
			root,
			&key,
			value,
			RRF_RT_REG_SZ,
			None,
			None,
			Some(&mut size),
		)
		.is_err()
		{
			return None;
		}
		let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
		size = (buf.len() * 2) as u32;
		RegGetValueW(
			root,
			&key,
			value,
			RRF_RT_REG_SZ,
			None,
			Some(buf.as_mut_ptr().cast()),
			Some(&mut size),
		)
		.ok()
		.ok()?;
		let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
		Some(String::from_utf16_lossy(&buf[..len]))
	}
}

pub fn reg_read_dword(root: HKEY, key: &str, name: &str) -> Option<u32> {
	let mut v = 0u32;
	let mut size = 4u32;
	unsafe {
		RegGetValueW(
			root,
			&HSTRING::from(key),
			&HSTRING::from(name),
			RRF_RT_REG_DWORD,
			None,
			Some((&mut v as *mut u32).cast()),
			Some(&mut size),
		)
		.ok()
		.ok()?;
	}
	Some(v)
}

pub enum RegValue<'a> {
	Str(&'a str),
	Dword(u32),
}

pub fn reg_write(key: &str, values: &[(&str, RegValue)]) -> Result<(), String> {
	unsafe {
		let mut h = HKEY::default();
		RegCreateKeyExW(
			HKEY_CURRENT_USER,
			&HSTRING::from(key),
			None,
			PCWSTR::null(),
			REG_OPTION_NON_VOLATILE,
			KEY_WRITE,
			None,
			&mut h,
			None,
		)
		.ok()
		.map_err(|e| format!("реестр {key}: {e}"))?;
		let mut result = Ok(());
		for (name, value) in values {
			let name_w = HSTRING::from(*name);
			let name_p = if name.is_empty() {
				PCWSTR::null()
			} else {
				PCWSTR(name_w.as_ptr())
			};
			let err = match value {
				RegValue::Str(s) => {
					let data = wide(s);
					let bytes =
						std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
					RegSetValueExW(h, name_p, None, REG_SZ, Some(bytes))
				}
				RegValue::Dword(d) => {
					RegSetValueExW(h, name_p, None, REG_DWORD, Some(&d.to_le_bytes()))
				}
			};
			if let Err(e) = err.ok() {
				result = Err(format!("реестр {key}\\{name}: {e}"));
			}
		}
		let _ = RegCloseKey(h);
		result
	}
}

pub fn reg_delete(key: &str) {
	unsafe {
		let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key));
		let _ =
			windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(key));
	}
}

/// Прежняя установка: папка и версия (наша или NSIS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Existing {
	pub dir: PathBuf,
	pub version: String,
}

pub fn existing_install() -> Option<Existing> {
	let location = reg_read_string(HKEY_CURRENT_USER, UNINSTALL_KEY, "InstallLocation")
		.map(|s| s.trim_matches('"').to_owned())
		.filter(|s| !s.is_empty())
		.or_else(|| reg_read_string(HKEY_CURRENT_USER, PRODUCT_KEY, ""))?;
	let dir = PathBuf::from(location);
	if !dir.join(EXE).is_file() {
		return None;
	}
	let version =
		reg_read_string(HKEY_CURRENT_USER, UNINSTALL_KEY, "DisplayVersion").unwrap_or_default();
	Some(Existing { dir, version })
}

/// Версия WebView2 Evergreen, если он установлен.
pub fn webview2_version() -> Option<String> {
	let keys = [
		(
			HKEY_LOCAL_MACHINE,
			format!(r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_GUID}"),
		),
		(
			HKEY_LOCAL_MACHINE,
			format!(r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_GUID}"),
		),
		(
			HKEY_CURRENT_USER,
			format!(r"Software\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_GUID}"),
		),
	];
	keys.iter()
		.filter_map(|(root, key)| reg_read_string(*root, key, "pv"))
		.find(|v| !v.is_empty() && v != "0.0.0.0")
}

/// Светлая тема приложений Windows.
pub fn light_theme() -> bool {
	reg_read_dword(
		HKEY_CURRENT_USER,
		r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
		"AppsUseLightTheme",
	)
	.is_none_or(|v| v != 0)
}

// --- процессы ---

fn process_path(pid: u32) -> Option<PathBuf> {
	unsafe {
		let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
		let mut buf = vec![0u16; 1024];
		let mut len = buf.len() as u32;
		let r =
			QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
		let _ = CloseHandle(h);
		r.ok()?;
		Some(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
	}
}

fn same_path(a: &Path, b: &Path) -> bool {
	a.to_string_lossy()
		.eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Запущенные `LeeLauncher.exe` из этой папки. Путь не прочитался — считаем, что наш.
pub fn launcher_pids(dir: &Path) -> Vec<u32> {
	let target = dir.join(EXE);
	let mut out = Vec::new();
	unsafe {
		let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
			return out;
		};
		let mut e = PROCESSENTRY32W {
			dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
			..Default::default()
		};
		let mut ok = Process32FirstW(snap, &mut e).is_ok();
		while ok {
			let len = e
				.szExeFile
				.iter()
				.position(|&c| c == 0)
				.unwrap_or(e.szExeFile.len());
			let name = String::from_utf16_lossy(&e.szExeFile[..len]);
			if name.eq_ignore_ascii_case(EXE)
				&& process_path(e.th32ProcessID).is_none_or(|p| same_path(&p, &target))
			{
				out.push(e.th32ProcessID);
			}
			ok = Process32NextW(snap, &mut e).is_ok();
		}
		let _ = CloseHandle(snap);
	}
	out
}

/// Просит окна этих процессов закрыться (как крестик). Игру это не трогает.
pub fn close_windows_of(pids: &[u32]) {
	struct Ctx<'a> {
		pids: &'a [u32],
	}
	unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
		unsafe {
			let ctx = &*(lparam.0 as *const Ctx);
			let mut pid = 0u32;
			GetWindowThreadProcessId(hwnd, Some(&mut pid));
			if ctx.pids.contains(&pid) && IsWindowVisible(hwnd).as_bool() {
				let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
			}
		}
		BOOL(1)
	}
	let ctx = Ctx { pids };
	unsafe {
		let _ = EnumWindows(Some(each), LPARAM(&ctx as *const Ctx as isize));
	}
}

/// Ждёт, пока лаунчер из папки закроется.
pub fn wait_launcher_closed(dir: &Path, timeout: Duration) -> bool {
	let start = std::time::Instant::now();
	while start.elapsed() < timeout {
		if launcher_pids(dir).is_empty() {
			return true;
		}
		std::thread::sleep(Duration::from_millis(250));
	}
	launcher_pids(dir).is_empty()
}

/// Есть ли связь с серверами Microsoft (для WebView2).
pub fn network_ok() -> bool {
	use std::net::{TcpStream, ToSocketAddrs};
	let Ok(addrs) = "go.microsoft.com:443".to_socket_addrs() else {
		return false;
	};
	addrs
		.take(2)
		.any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(4)).is_ok())
}

// --- ярлыки ---

pub fn create_shortcut(link: &Path, target: &Path, args: &str) -> Result<(), String> {
	unsafe {
		let shell: IShellLinkW =
			CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
		let target_w = HSTRING::from(target.as_os_str());
		shell.SetPath(&target_w).map_err(|e| e.to_string())?;
		if let Some(dir) = target.parent() {
			shell
				.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))
				.map_err(|e| e.to_string())?;
		}
		if !args.is_empty() {
			shell
				.SetArguments(&HSTRING::from(args))
				.map_err(|e| e.to_string())?;
		}
		shell
			.SetIconLocation(&target_w, 0)
			.map_err(|e| e.to_string())?;
		// Тот же AppUserModelID, что у окна лаунчера: значок в панели задач не двоится.
		if let Ok(store) = shell.cast::<IPropertyStore>() {
			let key = PROPERTYKEY {
				fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
				pid: 5,
			};
			let value = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::from(APP_ID);
			let _ = store.SetValue(&key, &value);
			let _ = store.Commit();
		}
		if let Some(dir) = link.parent() {
			let _ = std::fs::create_dir_all(dir);
		}
		let file: IPersistFile = shell.cast().map_err(|e| e.to_string())?;
		file.Save(&HSTRING::from(link.as_os_str()), true)
			.map_err(|e| e.to_string())?;
		Ok(())
	}
}

// --- оболочка ---

pub fn open(target: &str) {
	unsafe {
		ShellExecuteW(
			None,
			w!("open"),
			&HSTRING::from(target),
			PCWSTR::null(),
			PCWSTR::null(),
			SW_SHOWNORMAL,
		);
	}
}

/// Системное окно выбора папки.
pub fn pick_folder(owner: Option<HWND>, start: &Path) -> Option<PathBuf> {
	unsafe {
		let dialog: IFileOpenDialog =
			CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
		let options = dialog.GetOptions().ok()?;
		dialog
			.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
			.ok()?;
		if let Some(dir) = existing_ancestor(start)
			&& let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
				&HSTRING::from(dir.as_os_str()),
				None,
			) {
			let _ = dialog.SetFolder(&item);
		}
		dialog.Show(owner).ok()?;
		let item = dialog.GetResult().ok()?;
		let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
		let s = p.to_string().ok();
		CoTaskMemFree(Some(p.0 as _));
		s.map(PathBuf::from)
	}
}

pub fn set_clipboard(hwnd: Option<HWND>, text: &str) {
	use windows::Win32::Foundation::HANDLE;
	use windows::Win32::System::DataExchange::{
		CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
	};
	use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
	const CF_UNICODETEXT: u32 = 13;
	let data = wide(text);
	unsafe {
		if OpenClipboard(hwnd).is_err() {
			return;
		}
		let _ = EmptyClipboard();
		if let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2) {
			let ptr = GlobalLock(mem);
			if !ptr.is_null() {
				std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast::<u16>(), data.len());
				let _ = GlobalUnlock(mem);
				let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0)));
			}
		}
		let _ = CloseClipboard();
	}
}

/// Скачивает файл по адресу (загрузчик WebView2, если его нет внутри установщика).
pub fn download(url: &str, to: &Path) -> Result<(), String> {
	unsafe {
		windows::Win32::System::Com::Urlmon::URLDownloadToFileW(
			None,
			&HSTRING::from(url),
			&HSTRING::from(to.as_os_str()),
			0,
			None,
		)
		.map_err(|e| e.to_string())
	}
}

/// Удаляет файл после выхода процесса (себя из `%TEMP%`).
pub fn delete_after_exit(path: &Path) {
	use std::os::windows::process::CommandExt;
	const CREATE_NO_WINDOW: u32 = 0x0800_0000;
	let cmd = format!(
		"ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"",
		path.display()
	);
	let _ = std::process::Command::new("cmd")
		.args(["/c", &cmd])
		.creation_flags(CREATE_NO_WINDOW)
		.spawn();
}

fn creation_time(pid: u32) -> Option<u64> {
	use windows::Win32::Foundation::FILETIME;
	use windows::Win32::System::Threading::GetProcessTimes;
	unsafe {
		let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
		let (mut c, mut e, mut k, mut u) = (
			FILETIME::default(),
			FILETIME::default(),
			FILETIME::default(),
			FILETIME::default(),
		);
		let r = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
		let _ = CloseHandle(h);
		r.ok()?;
		Some(((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
	}
}

/// Потомки процесса (pid, время создания) — дочерние, внуки и дальше.
pub fn descendants(root: u32) -> Vec<(u32, u64)> {
	let mut pairs = Vec::new();
	unsafe {
		let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
			return Vec::new();
		};
		let mut e = PROCESSENTRY32W {
			dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
			..Default::default()
		};
		let mut ok = Process32FirstW(snap, &mut e).is_ok();
		while ok {
			pairs.push((e.th32ProcessID, e.th32ParentProcessID));
			ok = Process32NextW(snap, &mut e).is_ok();
		}
		let _ = CloseHandle(snap);
	}
	let root_time = creation_time(root);
	let mut out: Vec<(u32, u64)> = Vec::new();
	let mut queue = vec![(root, root_time)];
	while let Some((parent, parent_time)) = queue.pop() {
		for &(pid, ppid) in &pairs {
			if ppid != parent || pid == root || out.iter().any(|(p, _)| *p == pid) {
				continue;
			}
			let Some(t) = creation_time(pid) else {
				continue;
			};
			// Номер родителя мог достаться другому процессу: потомок не старше родителя.
			if parent_time.is_some_and(|pt| t < pt) {
				continue;
			}
			out.push((pid, t));
			queue.push((pid, Some(t)));
		}
	}
	out
}

/// Завершает процессы, если это всё ещё те же (совпало время создания).
pub fn kill_processes(list: &[(u32, u64)]) {
	use windows::Win32::System::Threading::{PROCESS_TERMINATE, TerminateProcess};
	for &(pid, t) in list.iter().rev() {
		if creation_time(pid) != Some(t) {
			continue;
		}
		unsafe {
			if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
				let _ = TerminateProcess(h, 1);
				let _ = CloseHandle(h);
			}
		}
	}
}
