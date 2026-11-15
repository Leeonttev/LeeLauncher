//! Миры, скриншоты (с миниатюрами в `meta/cache/thumbs`) и логи игры для страницы сборки.

pub mod commands;
pub mod nbt;
pub mod watch;

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::game::logs::{BUFFER_LINES, LogLine, Parser, Stream};

/// Иконка мира больше этого не отдаётся (обычная — 64×64, единицы КБ).
const MAX_ICON_BYTES: u64 = 256 * 1024;
/// Из файла лога читаем не больше стольких последних байт.
const MAX_LOG_BYTES: u64 = 32 * 1024 * 1024;
/// Распакованный архив лога (`.log.gz`) больше этого не читаем.
const MAX_GZ_BYTES: u64 = 64 * 1024 * 1024;
/// Ширина миниатюры скриншота.
const THUMB_WIDTH: u32 = 480;
/// Сколько миниатюр готовится одновременно.
const THUMB_THREADS: usize = 4;
pub const SCREENSHOTS_LIMIT_DEFAULT: usize = 60;
pub const SCREENSHOTS_LIMIT_MAX: usize = 200;

fn storage(path: &Path, err: impl std::fmt::Display) -> AppError {
	AppError::storage(format!("{}: {err}", path.display()))
}

fn millis(time: std::io::Result<SystemTime>) -> u64 {
	time.ok()
		.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
		.map_or(0, |d| d.as_millis() as u64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
	Survival,
	Creative,
	Adventure,
	Spectator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct World {
	/// Имя папки в `saves/` — id мира в командах.
	pub folder: String,
	/// Название из `level.dat`; нет — имя папки.
	pub name: String,
	pub mode: GameMode,
	pub hardcore: bool,
	/// Разрешены читы (`allowCommands`).
	pub cheats: bool,
	/// Последняя игра, Unix мс (из `level.dat`, иначе время изменения папки).
	pub last_played_at: Option<u64>,
	/// Версия игры, в которой мир открывали последним.
	pub version: Option<String>,
	/// `icon.png` как `data:image/png;base64,…`.
	pub icon_url: Option<String>,
	/// Можно зайти сразу в мир (`instance_launch { world }`): версия сборки это умеет (1.20+).
	pub quick_play: bool,
	/// `level.dat` не прочитался.
	pub damaged: bool,
	/// Мир поставлен из каталога (`instances/<id>/worlds.json`); `null` — нет.
	pub source: Option<WorldSource>,
}

/// Откуда мир.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldSource {
	pub source: crate::content::SourceKind,
	pub project_id: String,
}

fn read_world(dir: &Path, folder: String, quick_play: bool) -> World {
	let mut world = World {
		name: folder.clone(),
		folder,
		mode: GameMode::Survival,
		hardcore: false,
		cheats: false,
		last_played_at: None,
		version: None,
		icon_url: None,
		quick_play,
		damaged: false,
		source: None,
	};
	match nbt::read_file(&dir.join("level.dat")) {
		Ok(root) => {
			if let Some(data) = root.get("Data") {
				if let Some(name) = data.get("LevelName").and_then(|t| t.as_str())
					&& !name.trim().is_empty()
				{
					world.name = name.to_owned();
				}
				world.last_played_at = data
					.get("LastPlayed")
					.and_then(|t| t.as_i64())
					.and_then(|t| u64::try_from(t).ok())
					.filter(|t| *t > 0);
				world.mode = match data.get("GameType").and_then(|t| t.as_i64()) {
					Some(1) => GameMode::Creative,
					Some(2) => GameMode::Adventure,
					Some(3) => GameMode::Spectator,
					_ => GameMode::Survival,
				};
				world.hardcore = data
					.get("difficulty_settings")
					.and_then(|s| s.get("hardcore"))
					.or_else(|| data.get("hardcore"))
					.and_then(|t| t.as_i64())
					.is_some_and(|v| v != 0);
				world.cheats = data
					.get("allowCommands")
					.and_then(|t| t.as_i64())
					.is_some_and(|v| v != 0);
				world.version = data
					.get("Version")
					.and_then(|v| v.get("Name"))
					.and_then(|t| t.as_str())
					.map(str::to_owned);
			} else {
				world.damaged = true;
			}
		}
		Err(err) => {
			log::debug!("мир {}: level.dat: {err}", dir.display());
			world.damaged = true;
		}
	}
	if world.last_played_at.is_none() {
		world.last_played_at =
			Some(millis(std::fs::metadata(dir).and_then(|m| m.modified()))).filter(|t| *t > 0);
	}
	let icon = dir.join("icon.png");
	if std::fs::metadata(&icon).is_ok_and(|m| m.is_file() && m.len() <= MAX_ICON_BYTES)
		&& let Ok(bytes) = std::fs::read(&icon)
		&& bytes.starts_with(b"\x89PNG")
	{
		world.icon_url = Some(format!(
			"data:image/png;base64,{}",
			base64::engine::general_purpose::STANDARD.encode(bytes)
		));
	}
	world
}

/// Миры сборки, последние сыгранные первыми. Папка без `level.dat` — не мир.
pub fn worlds(game_dir: &Path, quick_play: bool) -> Vec<World> {
	let Ok(entries) = std::fs::read_dir(game_dir.join("saves")) else {
		return Vec::new();
	};
	let mut list: Vec<World> = entries
		.flatten()
		.filter(|e| e.path().join("level.dat").is_file())
		.filter_map(|e| {
			let folder = e.file_name().to_str()?.to_owned();
			// Недораспакованный мир из каталога — не мир.
			(fs_util::is_safe_file_name(&folder)
				&& !folder.starts_with(crate::content::worlds::TMP_PREFIX))
			.then(|| read_world(&e.path(), folder, quick_play))
		})
		.collect();
	list.sort_by(|a, b| {
		b.last_played_at
			.cmp(&a.last_played_at)
			.then(a.name.cmp(&b.name))
	});
	list
}

/// Умеет ли версия сразу заходить в мир: в описании версии есть аргумент `quickPlaySingleplayer` (1.20+).
pub fn supports_quick_play(version_json: &Path) -> bool {
	std::fs::read_to_string(version_json).is_ok_and(|text| text.contains("quickPlaySingleplayer"))
}

/// Папка мира по id от фронта.
pub fn world_dir(game_dir: &Path, folder: &str) -> Result<PathBuf, AppError> {
	if !fs_util::is_safe_file_name(folder) {
		return Err(AppError::coded(
			ErrorCode::InvalidValue,
			format!("world: {folder}"),
		));
	}
	let dir = game_dir.join("saves").join(folder);
	if !dir.is_dir() {
		return Err(AppError::coded(
			ErrorCode::FileNotFound,
			format!("saves/{folder}"),
		));
	}
	Ok(dir)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Screenshot {
	/// Имя файла — id скриншота в командах.
	pub file: String,
	/// Время файла, Unix мс.
	pub taken_at: u64,
	/// 0 — размер не прочитался.
	pub width: u32,
	pub height: u32,
	/// Полный путь картинки — для `convertFileSrc` (asset protocol).
	pub path: String,
	/// Полный путь миниатюры 480px (`meta/cache/thumbs`); `null` — не получилось сделать.
	pub thumb_path: Option<String>,
	pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotPage {
	pub items: Vec<Screenshot>,
	pub total: usize,
}

pub fn screenshots_dir(game_dir: &Path) -> PathBuf {
	game_dir.join("screenshots")
}

fn is_image(name: &str) -> bool {
	let lower = name.to_ascii_lowercase();
	[".png", ".jpg", ".jpeg"].iter().any(|e| lower.ends_with(e))
}

/// Файл миниатюры: имя — sha1 от пути, размера и времени файла (изменился файл — новая миниатюра).
fn thumb_file(thumbs: &Path, path: &Path, size: u64, modified: u64) -> PathBuf {
	let key = format!("{}|{size}|{modified}", path.display());
	thumbs.join(format!("{}.jpg", crate::download::hex_sha1(key.as_bytes())))
}

/// Делает миниатюру (если её ещё нет) и возвращает размер картинки.
fn make_thumb(path: &Path, thumb: &Path) -> Result<(u32, u32), String> {
	if thumb.is_file() {
		return image::image_dimensions(path).map_err(|e| e.to_string());
	}
	let img = image::open(path).map_err(|e| e.to_string())?;
	let (width, height) = (img.width(), img.height());
	let small = if width > THUMB_WIDTH {
		img.resize(THUMB_WIDTH, u32::MAX, image::imageops::FilterType::Triangle)
	} else {
		img
	};
	if let Some(dir) = thumb.parent() {
		std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
	}
	let tmp = thumb.with_extension(format!("tmp-{:08x}", rand::random::<u32>()));
	let result = (|| {
		let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
		let mut out = std::io::BufWriter::new(file);
		image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
			.encode_image(&small.to_rgb8())
			.map_err(|e| e.to_string())?;
		std::io::Write::flush(&mut out).map_err(|e| e.to_string())?;
		drop(out);
		std::fs::rename(&tmp, thumb).map_err(|e| e.to_string())
	})();
	if result.is_err() {
		let _ = std::fs::remove_file(&tmp);
	}
	result.map(|()| (width, height))
}

/// Скриншоты порцией, новые первыми. Долго (PNG) — вызывать в `spawn_blocking`.
pub fn screenshots(game_dir: &Path, thumbs: &Path, offset: usize, limit: usize) -> ScreenshotPage {
	let mut all: Vec<(String, PathBuf, u64, u64)> = std::fs::read_dir(screenshots_dir(game_dir))
		.map(|entries| {
			entries
				.flatten()
				.filter_map(|e| {
					let meta = e.metadata().ok().filter(|m| m.is_file())?;
					let name = e.file_name().to_str()?.to_owned();
					(is_image(&name) && fs_util::is_safe_file_name(&name))
						.then(|| (name, e.path(), meta.len(), millis(meta.modified())))
				})
				.collect()
		})
		.unwrap_or_default();
	all.sort_by(|a, b| b.3.cmp(&a.3).then(b.0.cmp(&a.0)));
	let total = all.len();
	let page: Vec<_> = all
		.into_iter()
		.skip(offset)
		.take(limit.clamp(1, SCREENSHOTS_LIMIT_MAX))
		.collect();
	let mut items: Vec<Option<Screenshot>> = vec![None; page.len()];
	let chunk = page.len().div_ceil(THUMB_THREADS).max(1);
	std::thread::scope(|scope| {
		for (part, out) in page.chunks(chunk).zip(items.chunks_mut(chunk)) {
			scope.spawn(move || {
				for ((name, path, size, modified), slot) in part.iter().zip(out.iter_mut()) {
					let thumb = thumb_file(thumbs, path, *size, *modified);
					let (dims, thumb_path) = match make_thumb(path, &thumb) {
						Ok(dims) => (dims, Some(thumb.to_string_lossy().into_owned())),
						Err(err) => {
							log::debug!("миниатюра {}: {err}", path.display());
							((0, 0), None)
						}
					};
					*slot = Some(Screenshot {
						file: name.clone(),
						taken_at: *modified,
						width: dims.0,
						height: dims.1,
						path: path.to_string_lossy().into_owned(),
						thumb_path,
						size: *size,
					});
				}
			});
		}
	});
	ScreenshotPage {
		items: items.into_iter().flatten().collect(),
		total,
	}
}

/// Файл скриншота по имени от фронта.
pub fn screenshot_path(game_dir: &Path, name: &str) -> Result<PathBuf, AppError> {
	if !fs_util::is_safe_file_name(name) || !is_image(name) {
		return Err(AppError::coded(
			ErrorCode::InvalidValue,
			format!("screenshot: {name}"),
		));
	}
	let path = screenshots_dir(game_dir).join(name);
	if !path.is_file() {
		return Err(AppError::coded(
			ErrorCode::FileNotFound,
			format!("screenshots/{name}"),
		));
	}
	Ok(path)
}

/// Удаляет миниатюры старше `older_than` (их никто давно не просил). Вызывать в фоне.
pub fn remove_stale_thumbs(thumbs: &Path, older_than: std::time::Duration) -> usize {
	let now = SystemTime::now();
	let Ok(entries) = std::fs::read_dir(thumbs) else {
		return 0;
	};
	entries
		.flatten()
		.filter(|e| {
			e.metadata()
				.and_then(|m| m.modified())
				.ok()
				.and_then(|t| now.duration_since(t).ok())
				.is_some_and(|age| age >= older_than)
		})
		.filter(|e| std::fs::remove_file(e.path()).is_ok())
		.count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
	/// `logs/latest.log` — лог последнего запуска (пишет игра).
	Latest,
	/// `logs/debug.log` (Forge, Fabric с отладкой).
	Debug,
	/// `logs/launcher-output*.log` — вывод игры, записанный лаунчером.
	Launcher,
	/// `logs/*.log.gz` — прошлые запуски.
	Archive,
	/// `crash-reports/*.txt` — отчёты о вылете.
	Crash,
	/// Другие `logs/*.log`.
	Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
	/// `logs/latest.log`, `crash-reports/crash-….txt` — id файла в командах.
	pub name: String,
	pub kind: LogKind,
	pub size: u64,
	pub modified: u64,
}

fn log_kind(folder: &str, file: &str) -> Option<LogKind> {
	let lower = file.to_ascii_lowercase();
	match folder {
		"crash-reports" if lower.ends_with(".txt") => Some(LogKind::Crash),
		"logs" if lower == "latest.log" => Some(LogKind::Latest),
		"logs" if lower == "debug.log" => Some(LogKind::Debug),
		"logs" if lower.starts_with("launcher-output") && lower.ends_with(".log") => {
			Some(LogKind::Launcher)
		}
		"logs" if lower.ends_with(".log.gz") => Some(LogKind::Archive),
		"logs" if lower.ends_with(".log") => Some(LogKind::Other),
		_ => None,
	}
}

/// Файлы логов и отчёты о вылете, новые первыми.
pub fn log_list(game_dir: &Path) -> Vec<LogFile> {
	let mut list = Vec::new();
	for folder in ["logs", "crash-reports"] {
		let Ok(entries) = std::fs::read_dir(game_dir.join(folder)) else {
			continue;
		};
		for e in entries.flatten() {
			let Ok(meta) = e.metadata() else { continue };
			let Some(file) = e.file_name().to_str().map(str::to_owned) else {
				continue;
			};
			if !meta.is_file() || !fs_util::is_safe_file_name(&file) {
				continue;
			}
			if let Some(kind) = log_kind(folder, &file) {
				list.push(LogFile {
					name: format!("{folder}/{file}"),
					kind,
					size: meta.len(),
					modified: millis(meta.modified()),
				});
			}
		}
	}
	list.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)));
	list
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogContent {
	/// Последние строки (не больше 10 000).
	pub lines: Vec<LogLine>,
	/// Показано не всё: файл длиннее.
	pub truncated: bool,
}

/// Путь файла лога по имени от фронта (`logs/…` или `crash-reports/…`).
pub fn log_path(game_dir: &Path, name: &str) -> Result<(PathBuf, LogKind), AppError> {
	let bad = || AppError::coded(ErrorCode::InvalidValue, format!("log: {name}"));
	let (folder, file) = name.split_once('/').ok_or_else(bad)?;
	if !fs_util::is_safe_file_name(file) {
		return Err(bad());
	}
	let kind = log_kind(folder, file).ok_or_else(bad)?;
	let path = game_dir.join(folder).join(file);
	if !path.is_file() {
		return Err(AppError::coded(ErrorCode::FileNotFound, name.to_owned()));
	}
	Ok((path, kind))
}

/// Читает лог: последние 32 МБ (архив — распакованный, до 64 МБ), разбор строк как у живого лога.
pub fn read_log(path: &Path, kind: LogKind) -> Result<LogContent, AppError> {
	let mut truncated = false;
	let mut bytes = Vec::new();
	let mut file = std::fs::File::open(path).map_err(|e| storage(path, e))?;
	if kind == LogKind::Archive {
		flate2::read::GzDecoder::new(file)
			.take(MAX_GZ_BYTES + 1)
			.read_to_end(&mut bytes)
			.map_err(|e| storage(path, e))?;
		if bytes.len() as u64 > MAX_GZ_BYTES {
			truncated = true;
			bytes.truncate(MAX_GZ_BYTES as usize);
		}
	} else {
		let len = file.metadata().map_err(|e| storage(path, e))?.len();
		if len > MAX_LOG_BYTES {
			truncated = true;
			file.seek(SeekFrom::Start(len - MAX_LOG_BYTES))
				.map_err(|e| storage(path, e))?;
		}
		file.take(MAX_LOG_BYTES)
			.read_to_end(&mut bytes)
			.map_err(|e| storage(path, e))?;
	}
	let text = String::from_utf8_lossy(&bytes);
	let mut raw: Vec<&str> = text.lines().collect();
	if truncated && !raw.is_empty() {
		// Первая строка, скорее всего, обрезана посередине.
		raw.remove(0);
	}
	let mut parser = Parser::default();
	let mut lines: Vec<LogLine> = Vec::new();
	for line in raw {
		lines.extend(parser.push(line.to_owned(), Stream::Stdout));
	}
	lines.extend(parser.finish());
	if lines.len() > BUFFER_LINES {
		truncated = true;
		lines.drain(..lines.len() - BUFFER_LINES);
	}
	Ok(LogContent { lines, truncated })
}

#[cfg(test)]
mod tests {
	use super::nbt::tests::{W, encode, gzip};
	use super::*;

	fn write(path: &Path, data: &[u8]) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, data).unwrap();
	}

	#[test]
	fn lists_worlds() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path();
		let level = encode(W::Compound(vec![(
			"Data",
			W::Compound(vec![
				("LevelName", W::Str("Крепость")),
				("LastPlayed", W::Long(2_000)),
				("GameType", W::Int(2)),
				("allowCommands", W::Byte(1)),
				(
					"difficulty_settings",
					W::Compound(vec![("hardcore", W::Byte(1))]),
				),
				("Version", W::Compound(vec![("Name", W::Str("1.21.4"))])),
			]),
		)]));
		write(&game.join("saves/w1/level.dat"), &gzip(&level));
		write(&game.join("saves/w1/icon.png"), b"\x89PNGxx");
		write(&game.join("saves/broken/level.dat"), b"garbage");
		std::fs::create_dir_all(game.join("saves/not-a-world")).unwrap();
		let list = worlds(game, true);
		assert_eq!(list.len(), 2);
		let w = list.iter().find(|w| w.folder == "w1").unwrap();
		assert_eq!(w.name, "Крепость");
		assert_eq!(w.mode, GameMode::Adventure);
		assert!(w.hardcore && w.cheats && w.quick_play && !w.damaged);
		assert_eq!(w.last_played_at, Some(2_000));
		assert_eq!(w.version.as_deref(), Some("1.21.4"));
		assert!(
			w.icon_url
				.as_deref()
				.unwrap()
				.starts_with("data:image/png;base64,")
		);
		let json = serde_json::to_value(w).unwrap();
		assert_eq!(json["mode"], "adventure");
		assert_eq!(json["lastPlayedAt"], 2_000);
		let b = list.iter().find(|w| w.folder == "broken").unwrap();
		assert!(b.damaged && b.name == "broken" && b.mode == GameMode::Survival);
		// Мир без даты в level.dat (повреждённый) — по времени папки, он новее.
		assert_eq!(list[0].folder, "broken");

		assert!(world_dir(game, "w1").is_ok());
		assert_eq!(
			world_dir(game, "..").unwrap_err().code,
			ErrorCode::InvalidValue
		);
		assert_eq!(
			world_dir(game, "nope").unwrap_err().code,
			ErrorCode::FileNotFound
		);
		assert!(worlds(&game.join("none"), false).is_empty());

		write(
			&game.join("v.json"),
			br#"{"arguments":{"game":["--quickPlaySingleplayer"]}}"#,
		);
		assert!(supports_quick_play(&game.join("v.json")));
		write(
			&game.join("old.json"),
			br#"{"minecraftArguments":"--username"}"#,
		);
		assert!(!supports_quick_play(&game.join("old.json")));
	}

	fn png(path: &Path, width: u32, height: u32) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]))
			.save(path)
			.unwrap();
	}

	#[test]
	fn lists_screenshots_with_thumbs() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path().join("game");
		let thumbs = dir.path().join("thumbs");
		png(&game.join("screenshots/a.png"), 960, 540);
		png(&game.join("screenshots/b.png"), 100, 50);
		write(&game.join("screenshots/broken.png"), b"not png");
		write(&game.join("screenshots/readme.txt"), b"x");
		let page = screenshots(&game, &thumbs, 0, 60);
		assert_eq!(page.total, 3);
		let a = page.items.iter().find(|s| s.file == "a.png").unwrap();
		assert_eq!((a.width, a.height), (960, 540));
		let thumb = image::open(a.thumb_path.as_ref().unwrap()).unwrap();
		assert_eq!((thumb.width(), thumb.height()), (480, 270));
		let b = page.items.iter().find(|s| s.file == "b.png").unwrap();
		assert_eq!((b.width, b.height), (100, 50));
		let broken = page.items.iter().find(|s| s.file == "broken.png").unwrap();
		assert!(broken.thumb_path.is_none() && broken.width == 0);
		// Второй раз миниатюры берутся готовые.
		let again = screenshots(&game, &thumbs, 0, 60);
		assert_eq!(again.items.len(), 3);
		assert_eq!(screenshots(&game, &thumbs, 1, 1).items.len(), 1);
		assert!(screenshots(&game, &thumbs, 5, 10).items.is_empty());
		assert_eq!(
			remove_stale_thumbs(&thumbs, std::time::Duration::from_secs(3600)),
			0
		);

		assert!(screenshot_path(&game, "a.png").is_ok());
		assert!(screenshot_path(&game, "../x.png").is_err());
		assert!(screenshot_path(&game, "readme.txt").is_err());
	}

	#[test]
	fn lists_and_reads_logs() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path();
		write(
			&game.join("logs/latest.log"),
			"[10:00:00] [main/INFO]: Привет\n[10:00:01] [main/ERROR]: Ошибка\n\tat x.y\n"
				.as_bytes(),
		);
		write(
			&game.join("logs/2024-01-01-1.log.gz"),
			&gzip(b"[09:00:00] [main/WARN]: old\n"),
		);
		write(
			&game.join("crash-reports/crash-1.txt"),
			b"---- Minecraft Crash Report ----\n",
		);
		write(&game.join("logs/notes.md"), b"x");
		let files = log_list(game);
		let mut kinds: Vec<LogKind> = files.iter().map(|f| f.kind).collect();
		kinds.sort_by_key(|k| format!("{k:?}"));
		assert_eq!(
			kinds,
			vec![LogKind::Archive, LogKind::Crash, LogKind::Latest]
		);

		let (path, kind) = log_path(game, "logs/latest.log").unwrap();
		let content = read_log(&path, kind).unwrap();
		assert!(!content.truncated);
		assert_eq!(content.lines[0].text, "Привет");
		assert_eq!(content.lines[0].time.as_deref(), Some("10:00:00"));
		let (path, kind) = log_path(game, "logs/2024-01-01-1.log.gz").unwrap();
		assert_eq!(read_log(&path, kind).unwrap().lines[0].text, "old");
		let (path, kind) = log_path(game, "crash-reports/crash-1.txt").unwrap();
		assert_eq!(read_log(&path, kind).unwrap().lines.len(), 1);
		for bad in [
			"logs/../x.log",
			"config/a.log",
			"latest.log",
			"logs/notes.md",
		] {
			assert!(log_path(game, bad).is_err(), "{bad}");
		}
		assert_eq!(
			log_path(game, "logs/debug.log").unwrap_err().code,
			ErrorCode::FileNotFound
		);
	}
}
