use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Пишет во временный файл рядом и переименовывает поверх: при сбое старый файл остаётся целым.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
	if let Some(dir) = path.parent() {
		fs::create_dir_all(dir)?;
	}
	let mut tmp = path.as_os_str().to_owned();
	tmp.push(".tmp");
	let tmp = PathBuf::from(tmp);
	{
		let mut f = fs::File::create(&tmp)?;
		f.write_all(data)?;
		f.sync_all()?;
	}
	fs::rename(&tmp, path).inspect_err(|_| {
		let _ = fs::remove_file(&tmp);
	})
}

/// Пишет копию файла в `<файл>.bak-<ГГГГММДД-ччммсс>` (UTC), не перезаписывая прошлые копии.
pub fn backup_copy(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
	let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
	let base = format!("{}.bak-{stamp}", path.display());
	for n in 0..100 {
		let candidate = if n == 0 {
			PathBuf::from(&base)
		} else {
			PathBuf::from(format!("{base}-{n}"))
		};
		match fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&candidate)
		{
			Ok(mut f) => {
				f.write_all(bytes)?;
				f.sync_all()?;
				return Ok(candidate);
			}
			Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
			Err(err) => return Err(err),
		}
	}
	Err(io::Error::other("слишком много копий за одну секунду"))
}

/// Нет файла или он повреждён (копия рядом) — `Ok(None)`. `Err` — писать поверх нельзя.
pub fn read_json_or_backup<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
	let bytes = match fs::read(path) {
		Ok(bytes) => bytes,
		Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
		Err(err) => return Err(err),
	};
	match serde_json::from_slice(&bytes) {
		Ok(value) => Ok(Some(value)),
		Err(err) => {
			log::error!("{} повреждён: {err}", path.display());
			let backup = backup_copy(path, &bytes)?;
			log::error!("копия повреждённого файла: {}", backup.display());
			Ok(None)
		}
	}
}

/// Относительный путь из метаданных внутри `root`. `..`, абсолютные пути, диски и пустой путь — `None`.
pub fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
	let path = Path::new(rel);
	if rel.is_empty()
		|| path
			.components()
			.any(|c| !matches!(c, std::path::Component::Normal(_)))
	{
		return None;
	}
	Some(root.join(path))
}

/// Id из метаданных, который станет именем файла: `A–Z a–z 0–9 . _ + -`, без `..`.
pub fn is_safe_id(id: &str) -> bool {
	!id.is_empty()
		&& id.len() <= 128
		&& !id.contains("..")
		&& id
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// SHA-1 в hex: ровно 40 символов `0–9 a–f` (регистр любой).
pub fn is_sha1_hex(hash: &str) -> bool {
	hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Одна часть пути, допустимая в Windows: без запрещённых символов, не `CON`/`NUL`…, до 255 байт.
pub fn is_safe_file_name(name: &str) -> bool {
	if name.is_empty()
		|| name.len() > 255
		|| name.ends_with('.')
		|| name.ends_with(' ')
		|| name
			.chars()
			.any(|c| c.is_control() || r#"/\:*?"<>|"#.contains(c))
	{
		return false;
	}
	let stem = name.split('.').next().unwrap_or_default().trim_end();
	let upper = stem.to_ascii_uppercase();
	let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
		|| ((upper.starts_with("COM") || upper.starts_with("LPT"))
			&& upper.len() == 4
			&& upper.as_bytes()[3].is_ascii_digit());
	!reserved
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn safe_paths_and_ids() {
		let root = Path::new("root");
		assert_eq!(
			safe_join(root, "net/fabricmc/x.jar"),
			Some(root.join("net/fabricmc/x.jar"))
		);
		for bad in ["", "../evil", "a/../../b", "/abs", "./a"] {
			assert!(safe_join(root, bad).is_none(), "{bad}");
		}
		#[cfg(windows)]
		for bad in [r"a\..\..\b", "C:/Windows/x", "C:x", r"\\server\share\x"] {
			assert!(safe_join(root, bad).is_none(), "{bad}");
		}

		for good in [
			"1.20.1",
			"1.20.1-fabric-0.16.14",
			"client-1.12.xml",
			"47.4.0+x",
			"1_8",
		] {
			assert!(is_safe_id(good), "{good}");
		}
		for bad in ["", "..", "a..b", "a/b", r"a\b", "a b", "C:", "x\0"] {
			assert!(!is_safe_id(bad), "{bad}");
		}
		assert!(!is_safe_id(&"a".repeat(129)));

		assert!(is_sha1_hex("da39a3ee5e6b4b0d3255bfef95601890afd80709"));
		assert!(is_sha1_hex("DA39A3EE5E6B4B0D3255BFEF95601890AFD80709"));
		assert!(!is_sha1_hex("da39a3ee5e6b4b0d3255bfef95601890afd8070"));
		assert!(!is_sha1_hex("../9a3ee5e6b4b0d3255bfef95601890afd80709"));
		assert!(!is_sha1_hex("zz39a3ee5e6b4b0d3255bfef95601890afd80709"));

		for good in [
			"sodium-fabric-0.5.8+mc1.20.1.jar",
			"Мой мир",
			"pack (1).zip",
			".hidden",
		] {
			assert!(is_safe_file_name(good), "{good}");
		}
		for bad in [
			"", ".", "..", "a/b", r"a\b", "C:x", "a?.jar", "a.", "a ", "CON", "nul.txt",
			"com1.jar", "x\0",
		] {
			assert!(!is_safe_file_name(bad), "{bad}");
		}
		assert!(is_safe_file_name("console.jar") && is_safe_file_name("com10.jar"));
		assert!(!is_safe_file_name(&"a".repeat(256)));
	}
}
