//! Правка `options.txt` перед запуском. `fullscreen` понимают все версии с 1.3.1.

use std::io;
use std::path::Path;

use crate::fs_util;

pub const FILE_NAME: &str = "options.txt";

/// Ставит ключи, не трогая остальные строки. Ничего не поменялось — файл не переписывается.
pub fn set(game_dir: &Path, values: &[(&str, &str)]) -> io::Result<()> {
	let path = game_dir.join(FILE_NAME);
	let text = match std::fs::read(&path) {
		Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
		Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
		Err(err) => return Err(err),
	};
	let updated = apply(&text, values);
	if updated == text {
		return Ok(());
	}
	fs_util::write_atomic(&path, updated.as_bytes())
}

fn apply(text: &str, values: &[(&str, &str)]) -> String {
	let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
	for (key, value) in values {
		let line = format!("{key}:{value}");
		match lines
			.iter_mut()
			.find(|l| l.split_once(':').is_some_and(|(k, _)| k == *key))
		{
			Some(existing) => *existing = line,
			None => lines.push(line),
		}
	}
	let mut out = lines.join("\n");
	out.push('\n');
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn replaces_and_appends() {
		let text = "version:3465\nfullscreen:false\nlang:ru_ru\n";
		assert_eq!(
			apply(text, &[("fullscreen", "true")]),
			"version:3465\nfullscreen:true\nlang:ru_ru\n"
		);
		assert_eq!(
			apply("lang:ru_ru", &[("fullscreen", "true")]),
			"lang:ru_ru\nfullscreen:true\n"
		);
		// Похожий ключ не путается с нужным.
		assert_eq!(
			apply("fullscreenResolution:x\n", &[("fullscreen", "false")]),
			"fullscreenResolution:x\nfullscreen:false\n"
		);
		assert_eq!(apply("", &[("fullscreen", "true")]), "fullscreen:true\n");
	}

	#[test]
	fn writes_file() {
		let dir = tempfile::tempdir().unwrap();
		set(dir.path(), &[("fullscreen", "true")]).unwrap();
		let path = dir.path().join(FILE_NAME);
		assert_eq!(std::fs::read_to_string(&path).unwrap(), "fullscreen:true\n");
		std::fs::write(&path, "fov:0.5\r\nfullscreen:true\r\n").unwrap();
		set(dir.path(), &[("fullscreen", "false")]).unwrap();
		assert_eq!(
			std::fs::read_to_string(&path).unwrap(),
			"fov:0.5\nfullscreen:false\n"
		);
	}
}
