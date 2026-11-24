//! Шифрование токенов на диске: на Windows DPAPI (`dpapi:<base64>`), на других ОС `plain:<base64>`.
//!
//! Keychain не берём: приложение без подписи Developer ID, и macOS спрашивала бы доступ после
//! каждого обновления. Вместо этого файл с секретами доступен только владельцу (`fs_util::write_private`).

use base64::Engine;
use base64::prelude::BASE64_STANDARD;

#[derive(Debug, thiserror::Error)]
pub enum SecureError {
	#[error("неизвестный формат защищённых данных")]
	Format,
	#[error("base64: {0}")]
	Base64(#[from] base64::DecodeError),
	#[cfg_attr(not(windows), allow(dead_code))]
	#[error("DPAPI: код ошибки {0}")]
	Os(i32),
}

#[cfg(windows)]
const PREFIX: &str = "dpapi:";
#[cfg(not(windows))]
const PREFIX: &str = "plain:";

pub fn protect(data: &[u8]) -> Result<String, SecureError> {
	let sealed = imp::protect(data)?;
	Ok(format!("{PREFIX}{}", BASE64_STANDARD.encode(sealed)))
}

pub fn unprotect(text: &str) -> Result<Vec<u8>, SecureError> {
	let encoded = text.strip_prefix(PREFIX).ok_or(SecureError::Format)?;
	let sealed = BASE64_STANDARD.decode(encoded)?;
	imp::unprotect(&sealed)
}

#[cfg(windows)]
mod imp {
	use std::ptr;

	use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
	use windows_sys::Win32::Security::Cryptography::{
		CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
	};

	use super::SecureError;

	/// Не менять: сохранённые токены перестанут расшифровываться.
	const ENTROPY: &[u8] = b"LeeLauncher/secrets/v1";

	fn blob(data: &[u8]) -> Result<CRYPT_INTEGER_BLOB, SecureError> {
		Ok(CRYPT_INTEGER_BLOB {
			cbData: u32::try_from(data.len()).map_err(|_| SecureError::Format)?,
			pbData: data.as_ptr().cast_mut(),
		})
	}

	/// # Safety
	/// `out` заполнен успешным вызовом `CryptProtectData`/`CryptUnprotectData`.
	unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
		if out.pbData.is_null() {
			return Vec::new();
		}
		// SAFETY: DPAPI вернула буфер длиной cbData, выделенный через LocalAlloc.
		unsafe {
			let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
			LocalFree(out.pbData.cast());
			bytes
		}
	}

	pub fn protect(data: &[u8]) -> Result<Vec<u8>, SecureError> {
		let input = blob(data)?;
		let entropy = blob(ENTROPY)?;
		let mut out = CRYPT_INTEGER_BLOB::default();
		// SAFETY: входные буферы живут до конца вызова, out заполняется системой.
		let ok = unsafe {
			CryptProtectData(
				&input,
				ptr::null(),
				&entropy,
				ptr::null(),
				ptr::null(),
				CRYPTPROTECT_UI_FORBIDDEN,
				&mut out,
			)
		};
		if ok == 0 {
			// SAFETY: простой вызов без аргументов.
			return Err(SecureError::Os(unsafe { GetLastError() } as i32));
		}
		// SAFETY: вызов успешен.
		Ok(unsafe { take(out) })
	}

	pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, SecureError> {
		let input = blob(sealed)?;
		let entropy = blob(ENTROPY)?;
		let mut out = CRYPT_INTEGER_BLOB::default();
		// SAFETY: как в protect.
		let ok = unsafe {
			CryptUnprotectData(
				&input,
				ptr::null_mut(),
				&entropy,
				ptr::null(),
				ptr::null(),
				CRYPTPROTECT_UI_FORBIDDEN,
				&mut out,
			)
		};
		if ok == 0 {
			// SAFETY: простой вызов без аргументов.
			return Err(SecureError::Os(unsafe { GetLastError() } as i32));
		}
		// SAFETY: вызов успешен.
		Ok(unsafe { take(out) })
	}
}

#[cfg(not(windows))]
mod imp {
	use super::SecureError;

	pub fn protect(data: &[u8]) -> Result<Vec<u8>, SecureError> {
		Ok(data.to_vec())
	}

	pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, SecureError> {
		Ok(sealed.to_vec())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn round_trip() {
		let sealed = protect(b"secret token").unwrap();
		assert!(sealed.starts_with(PREFIX));
		assert!(!sealed.contains("secret"));
		assert_eq!(unprotect(&sealed).unwrap(), b"secret token");
	}

	#[test]
	fn rejects_garbage() {
		assert!(unprotect("nope").is_err());
		assert!(unprotect(&format!("{PREFIX}AAAA")).is_err() || cfg!(not(windows)));
	}
}
