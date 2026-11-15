//! Миры из каталога: архив распаковывается в `saves/<имя>`, источник пишется в `worlds.json`.
//! Корень мира — папка с самым неглубоким `level.dat`.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::SourceKind;
use crate::error::{AppError, ErrorCode};
use crate::fs_util;

pub const STATE_FILE: &str = "worlds.json";
const STATE_VERSION: u32 = 1;
/// Недораспакованный мир: `saves/.world-tmp-*` (дочищается при запуске вместе с `.content-tmp-*`).
pub const TMP_PREFIX: &str = ".world-tmp-";

#[derive(Debug, Clone, Copy)]
pub struct Limits {
	pub files: usize,
	pub bytes: u64,
}

impl Limits {
	pub const DEFAULT: Self = Self {
		files: 100_000,
		bytes: 8 * 1024 * 1024 * 1024,
	};
}

/// Откуда мир (для `World.source`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldSource {
	pub source: SourceKind,
	pub project_id: String,
	#[serde(default)]
	pub version_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorldsState {
	#[serde(default)]
	pub version: u32,
	#[serde(default)]
	pub worlds: BTreeMap<String, WorldSource>,
}

pub fn state_path(instance_dir: &Path) -> PathBuf {
	instance_dir.join(STATE_FILE)
}

/// Нет файла или он повреждён (копия рядом) — пусто.
pub fn load(instance_dir: &Path) -> WorldsState {
	fs_util::read_json_or_backup::<WorldsState>(&state_path(instance_dir))
		.ok()
		.flatten()
		.unwrap_or_default()
}

pub fn save(instance_dir: &Path, state: &WorldsState) -> Result<(), AppError> {
	let mut state = state.clone();
	state.version = STATE_VERSION;
	let path = state_path(instance_dir);
	let json = serde_json::to_vec_pretty(&state).map_err(|e| AppError::storage(e.to_string()))?;
	fs_util::write_atomic(&path, &json)
		.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))
}

fn invalid(details: impl std::fmt::Display) -> AppError {
	AppError::coded(ErrorCode::ContentIncompatible, format!("world: {details}"))
}

/// Части пути из архива: только безопасные имена, без `..`, дисков и `\`.
fn parts(name: &str) -> Option<Vec<String>> {
	let name = name.replace('\\', "/");
	let parts: Vec<String> = name
		.split('/')
		.filter(|p| !p.is_empty())
		.map(str::to_owned)
		.collect();
	(!parts.is_empty() && parts.iter().all(|p| fs_util::is_safe_file_name(p))).then_some(parts)
}

/// Свободное имя папки в `saves/`: `Имя`, `Имя (2)`, …
pub fn unique_folder(saves: &Path, wanted: &str) -> String {
	let base: String = wanted
		.chars()
		.map(|c| {
			if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
				'_'
			} else {
				c
			}
		})
		.collect();
	let base = base.trim().trim_end_matches('.').trim().to_owned();
	let base = if fs_util::is_safe_file_name(&base) {
		base
	} else {
		"World".to_owned()
	};
	let mut name = base.clone();
	let mut n = 2;
	while saves.join(&name).exists() {
		name = format!("{base} ({n})");
		n += 1;
	}
	name
}

/// Распаковывает мир из `archive` в `saves/<имя>` (имя — папка мира в архиве, иначе `title`).
/// Возвращает имя папки. Синхронно — вызывать в `spawn_blocking`.
pub fn install(
	saves: &Path,
	archive: &Path,
	title: &str,
	limits: Limits,
) -> Result<String, AppError> {
	let file = std::fs::File::open(archive)
		.map_err(|e| AppError::storage(format!("{}: {e}", archive.display())))?;
	let mut zip = zip::ZipArchive::new(file).map_err(|e| invalid(format!("не zip: {e}")))?;
	let mut entries: Vec<(usize, Vec<String>)> = Vec::new();
	for i in 0..zip.len() {
		let entry = zip.by_index_raw(i).map_err(|e| invalid(e.to_string()))?;
		if entry.is_dir() {
			continue;
		}
		let name = entry
			.name()
			.map_err(|e| invalid(e.to_string()))?
			.to_string();
		let parts = parts(&name).ok_or_else(|| invalid(format!("небезопасный путь: {name}")))?;
		entries.push((i, parts));
	}
	// Корень — папка самого неглубокого `level.dat`; таких должно быть ровно одно.
	let depth = entries
		.iter()
		.filter(|(_, p)| p.last().is_some_and(|n| n == "level.dat"))
		.map(|(_, p)| p.len())
		.min()
		.ok_or_else(|| invalid("нет level.dat"))?;
	let roots: Vec<&Vec<String>> = entries
		.iter()
		.filter(|(_, p)| p.len() == depth && p.last().is_some_and(|n| n == "level.dat"))
		.map(|(_, p)| p)
		.collect();
	if roots.len() != 1 {
		return Err(invalid("в архиве несколько миров"));
	}
	let root: Vec<String> = roots[0][..depth - 1].to_vec();
	let chosen: Vec<(usize, Vec<String>)> = entries
		.into_iter()
		.filter(|(_, p)| p.len() > root.len() && p[..root.len()] == root[..])
		.map(|(i, p)| (i, p[root.len()..].to_vec()))
		.collect();
	if chosen.len() > limits.files {
		return Err(invalid(format!("слишком много файлов: {}", chosen.len())));
	}
	let wanted = root.last().cloned().unwrap_or_else(|| title.to_owned());
	std::fs::create_dir_all(saves)
		.map_err(|e| AppError::storage(format!("{}: {e}", saves.display())))?;
	let tmp = saves.join(format!("{TMP_PREFIX}{:08x}", rand::random::<u32>()));
	let result = extract(&mut zip, &chosen, &tmp, limits.bytes);
	if let Err(err) = result {
		let _ = std::fs::remove_dir_all(&tmp);
		return Err(err);
	}
	let folder = unique_folder(saves, &wanted);
	std::fs::rename(&tmp, saves.join(&folder)).map_err(|e| {
		let _ = std::fs::remove_dir_all(&tmp);
		AppError::storage(format!("{}: {e}", saves.display()))
	})?;
	Ok(folder)
}

fn extract<R: Read + std::io::Seek>(
	zip: &mut zip::ZipArchive<R>,
	chosen: &[(usize, Vec<String>)],
	dir: &Path,
	mut budget: u64,
) -> Result<(), AppError> {
	let mut buf = vec![0u8; 256 * 1024];
	for (i, parts) in chosen {
		let rel = parts.join("/");
		let target = fs_util::safe_join(dir, &rel)
			.ok_or_else(|| invalid(format!("небезопасный путь: {rel}")))?;
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)
				.map_err(|e| AppError::storage(format!("{}: {e}", parent.display())))?;
		}
		let mut entry = zip.by_index(*i).map_err(|e| invalid(e.to_string()))?;
		let declared = entry.size();
		let mut out = std::fs::File::create(&target)
			.map_err(|e| AppError::storage(format!("{}: {e}", target.display())))?;
		let mut read = 0u64;
		loop {
			let n = entry
				.read(&mut buf)
				.map_err(|e| invalid(format!("{rel}: {e}")))?;
			if n == 0 {
				break;
			}
			read += n as u64;
			if read > declared || n as u64 > budget {
				return Err(invalid(format!("{rel}: распаковано больше, чем заявлено")));
			}
			budget -= n as u64;
			std::io::Write::write_all(&mut out, &buf[..n])
				.map_err(|e| AppError::storage(format!("{}: {e}", target.display())))?;
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;

	fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
		let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
		for (name, data) in entries {
			zip.start_file(*name, zip::write::SimpleFileOptions::default())
				.unwrap();
			zip.write_all(data).unwrap();
		}
		zip.finish().unwrap();
	}

	#[test]
	fn installs_world_from_nested_root() {
		let dir = tempfile::tempdir().unwrap();
		let saves = dir.path().join("saves");
		let archive = dir.path().join("w.zip");
		zip_with(
			&archive,
			&[
				("__MACOSX/._x", b"junk"),
				("My Map/level.dat", b"nbt"),
				("My Map/region/r.0.0.mca", b"r"),
				("My Map/DIM1/level.dat", b"inner"),
				("readme.txt", b"hi"),
			],
		);
		let folder = install(&saves, &archive, "Title", Limits::DEFAULT).unwrap();
		assert_eq!(folder, "My Map");
		assert!(saves.join("My Map/level.dat").is_file());
		assert!(saves.join("My Map/region/r.0.0.mca").is_file());
		assert!(saves.join("My Map/DIM1/level.dat").is_file());
		assert!(!saves.join("My Map/readme.txt").exists());
		// Второй раз — копия рядом.
		assert_eq!(
			install(&saves, &archive, "Title", Limits::DEFAULT).unwrap(),
			"My Map (2)"
		);
		// Мир прямо в корне архива — имя по названию.
		let flat = dir.path().join("f.zip");
		zip_with(&flat, &[("level.dat", b"n"), ("data/x.dat", b"x")]);
		assert_eq!(
			install(&saves, &flat, "Sky: Block", Limits::DEFAULT).unwrap(),
			"Sky_ Block"
		);
		let leftovers: Vec<_> = std::fs::read_dir(&saves)
			.unwrap()
			.flatten()
			.filter(|e| e.file_name().to_string_lossy().starts_with(TMP_PREFIX))
			.collect();
		assert!(leftovers.is_empty());
	}

	#[test]
	fn rejects_bad_archives() {
		let dir = tempfile::tempdir().unwrap();
		let saves = dir.path().join("saves");
		let cases: [&[(&str, &[u8])]; 3] = [
			&[("a/region/x", b"x")],
			&[("a/level.dat", b"1"), ("b/level.dat", b"2")],
			&[("w/level.dat", b"1"), ("w/../../evil.txt", b"x")],
		];
		for (n, entries) in cases.iter().enumerate() {
			let archive = dir.path().join(format!("{n}.zip"));
			zip_with(&archive, entries);
			let err = install(&saves, &archive, "T", Limits::DEFAULT).unwrap_err();
			assert_eq!(err.code, ErrorCode::ContentIncompatible, "{n}");
		}
		assert!(!dir.path().join("evil.txt").exists());
		let big = dir.path().join("big.zip");
		zip_with(&big, &[("w/level.dat", &[0u8; 64]), ("w/b", &[1u8; 64])]);
		let limits = Limits {
			files: 10,
			bytes: 100,
		};
		assert!(install(&saves, &big, "T", limits).is_err());
		assert!(!saves.join("w").exists());
	}
}
