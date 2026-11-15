//! Удаление файлов пользователя (миры, скриншоты, моды) в корзину ОС, чтобы удаление можно было отменить.

use std::io;
use std::path::Path;

/// В корзину без окон. Если корзина недоступна (сетевой диск), Windows удаляет сразу.
pub fn delete(path: &Path) -> io::Result<()> {
	if !path.exists() {
		return Err(io::Error::new(
			io::ErrorKind::NotFound,
			format!("{}: нет такого файла", path.display()),
		));
	}
	platform_delete(path)
}

/// Файл занят другим процессом (игрой) — ошибка с кодом Windows 32, чтобы её узнал `io_error`.
#[cfg(windows)]
fn check_not_locked(path: &Path) -> io::Result<()> {
	use std::os::windows::fs::OpenOptionsExt;
	if path.is_dir() {
		return Ok(());
	}
	match std::fs::OpenOptions::new()
		.read(true)
		.share_mode(0)
		.open(path)
	{
		Err(err) if matches!(err.raw_os_error(), Some(32 | 33)) => Err(err),
		_ => Ok(()),
	}
}

#[cfg(windows)]
fn platform_delete(path: &Path) -> io::Result<()> {
	use std::os::windows::ffi::OsStrExt;
	use windows_sys::Win32::UI::Shell::{
		FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
		SHFileOperationW,
	};

	check_not_locked(path)?;
	let absolute = std::path::absolute(path)?;
	// Список путей для SHFileOperationW заканчивается двумя нулями.
	let mut from: Vec<u16> = absolute.as_os_str().encode_wide().collect();
	if from.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]) {
		from.drain(..4);
	}
	from.extend([0, 0]);
	let mut op = SHFILEOPSTRUCTW {
		hwnd: std::ptr::null_mut(),
		wFunc: FO_DELETE,
		pFrom: from.as_ptr(),
		pTo: std::ptr::null(),
		fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
		fAnyOperationsAborted: 0,
		hNameMappings: std::ptr::null_mut(),
		lpszProgressTitle: std::ptr::null(),
	};
	// SAFETY: структура заполнена, строка `from` живёт до конца вызова и заканчивается двумя нулями.
	let code = unsafe { SHFileOperationW(&mut op) };
	// SHFileOperationW часто отдаёт обычный код Win32 (32 — файл занят): пробрасываем его.
	if matches!(code, 32 | 33) {
		return Err(io::Error::from_raw_os_error(code));
	}
	if code != 0 || op.fAnyOperationsAborted != 0 {
		return Err(io::Error::other(format!(
			"{}: не удалось удалить в корзину (код {code:#x})",
			absolute.display()
		)));
	}
	if path.exists() {
		return Err(io::Error::other(format!(
			"{}: файл остался на месте",
			absolute.display()
		)));
	}
	Ok(())
}

#[cfg(not(windows))]
fn platform_delete(path: &Path) -> io::Result<()> {
	if path.is_dir() {
		std::fs::remove_dir_all(path)
	} else {
		std::fs::remove_file(path)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[cfg(windows)]
	#[test]
	fn locked_file_is_in_use() {
		use std::os::windows::fs::OpenOptionsExt;
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("mod.jar");
		std::fs::write(&file, b"x").unwrap();
		let lock = std::fs::OpenOptions::new()
			.read(true)
			.share_mode(0)
			.open(&file)
			.unwrap();
		// До корзины дело не доходит: занятый файл распознаётся заранее.
		let err = delete(&file).unwrap_err();
		assert_eq!(err.raw_os_error(), Some(32));
		let app = crate::content::installed::io_error(&file, &err);
		assert_eq!(app.code, crate::error::ErrorCode::FileInUse);
		drop(lock);
		assert!(file.exists());
	}

	/// Кладёт файлы в настоящую корзину: `cargo test trash -- --ignored`.
	#[test]
	#[ignore]
	fn deletes_file_and_dir() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("a.txt");
		std::fs::write(&file, b"x").unwrap();
		delete(&file).unwrap();
		assert!(!file.exists());
		let sub = dir.path().join("Мир 1");
		std::fs::create_dir_all(sub.join("region")).unwrap();
		std::fs::write(sub.join("level.dat"), b"x").unwrap();
		delete(&sub).unwrap();
		assert!(!sub.exists());
		assert_eq!(
			delete(&dir.path().join("none")).unwrap_err().kind(),
			io::ErrorKind::NotFound
		);
	}
}
