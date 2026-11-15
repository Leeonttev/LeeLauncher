//! Проверка файла скина: размер читается из заголовка до распаковки, в библиотеку идёт чистый PNG.

use std::io::{Cursor, Read};
use std::path::Path;

use image::{ExtendedColorType, ImageEncoder, ImageFormat, ImageReader, Limits, RgbaImage};
use sha1::{Digest, Sha1};

use super::SkinModel;

pub const MAX_BYTES: u64 = 1024 * 1024;
const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
/// Подпись, длина чанка, «IHDR», ширина, высота.
const HEADER_LEN: usize = 24;
const NAME_MAX_CHARS: usize = 48;
const MAX_CAPE_SIDE: u32 = 1024;
/// Скину хватает с запасом; сжатые чанки вроде `iCCP` больше не раздуются.
const MAX_DECODE_ALLOC: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
	NotPng,
	BadSize { width: u32, height: u32 },
	TooLarge,
	Unreadable,
}

#[derive(Debug, Clone)]
pub struct ParsedSkin {
	/// sha1 пикселей RGBA и размеров.
	pub id: String,
	/// Заново записанный PNG, без чужих чанков.
	pub png: Vec<u8>,
	pub model: SkinModel,
	/// Файл 64×32.
	pub legacy: bool,
}

pub fn read_file(path: &Path) -> Result<ParsedSkin, Rejection> {
	let file = std::fs::File::open(path).map_err(|_| Rejection::Unreadable)?;
	// Устройства вроде `NUL` и каналы открываются, но файлами не являются.
	let is_file = file.metadata().is_ok_and(|meta| meta.is_file());
	if !is_file {
		return Err(Rejection::Unreadable);
	}
	let mut bytes = Vec::new();
	file.take(MAX_BYTES + 1)
		.read_to_end(&mut bytes)
		.map_err(|_| Rejection::Unreadable)?;
	if bytes.len() as u64 > MAX_BYTES {
		return Err(Rejection::TooLarge);
	}
	parse(&bytes)
}

pub fn parse(bytes: &[u8]) -> Result<ParsedSkin, Rejection> {
	if !bytes.starts_with(SIGNATURE) {
		return Err(Rejection::NotPng);
	}
	let (width, height) = header_size(bytes).ok_or(Rejection::Unreadable)?;
	if width != 64 || !(height == 64 || height == 32) {
		return Err(Rejection::BadSize { width, height });
	}
	let image = decode(bytes)?;
	if image.dimensions() != (width, height) {
		return Err(Rejection::Unreadable);
	}
	let png = encode_clean(&image)?;
	let mut hash = Sha1::new();
	hash.update(width.to_le_bytes());
	hash.update(height.to_le_bytes());
	hash.update(image.as_raw());
	Ok(ParsedSkin {
		id: crate::download::to_hex(&hash.finalize()),
		png,
		model: infer_model(&image),
		legacy: height == 32,
	})
}

/// Плащ Mojang: стороны до 1024, соотношение 2:1, 22:17 или 46:22 (как принимает skinview3d).
/// Размер проверяется до распаковки, результат — чистый PNG.
pub fn parse_cape(bytes: &[u8]) -> Result<Vec<u8>, Rejection> {
	if !bytes.starts_with(SIGNATURE) {
		return Err(Rejection::NotPng);
	}
	let (width, height) = header_size(bytes).ok_or(Rejection::Unreadable)?;
	let (w, h) = (u64::from(width), u64::from(height));
	let ratio_ok = w == 2 * h || w * 17 == h * 22 || w * 22 == h * 46;
	if width == 0 || height == 0 || width > MAX_CAPE_SIDE || height > MAX_CAPE_SIDE || !ratio_ok {
		return Err(Rejection::BadSize { width, height });
	}
	let image = decode(bytes)?;
	if image.dimensions() != (width, height) {
		return Err(Rejection::Unreadable);
	}
	encode_clean(&image)
}

fn decode(bytes: &[u8]) -> Result<RgbaImage, Rejection> {
	let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
	let mut limits = Limits::default();
	limits.max_alloc = Some(MAX_DECODE_ALLOC);
	reader.limits(limits);
	Ok(reader
		.decode()
		.map_err(|_| Rejection::Unreadable)?
		.to_rgba8())
}

fn encode_clean(image: &RgbaImage) -> Result<Vec<u8>, Rejection> {
	let mut png = Vec::new();
	image::codecs::png::PngEncoder::new(&mut png)
		.write_image(
			image.as_raw(),
			image.width(),
			image.height(),
			ExtendedColorType::Rgba8,
		)
		.map_err(|_| Rejection::Unreadable)?;
	Ok(png)
}

fn header_size(bytes: &[u8]) -> Option<(u32, u32)> {
	if bytes.len() < HEADER_LEN || bytes.get(8..12)? != 13u32.to_be_bytes() {
		return None;
	}
	if bytes.get(12..16)? != b"IHDR" {
		return None;
	}
	let number = |range: std::ops::Range<usize>| -> Option<u32> {
		Some(u32::from_be_bytes(bytes.get(range)?.try_into().ok()?))
	};
	Some((number(16..20)?, number(20..24)?))
}

/// Столбцы, которых нет у тонких рук: `inferModelType` из `skinview-utils` (x, y, ширина, высота).
const SLIM_GAPS: [(u32, u32, u32, u32); 4] = [
	(50, 16, 2, 4),
	(54, 20, 2, 12),
	(42, 48, 2, 4),
	(46, 52, 2, 12),
];

fn infer_model(image: &RgbaImage) -> SkinModel {
	if image.height() != 64 {
		return SkinModel::Classic;
	}
	let pixels = || {
		SLIM_GAPS.iter().flat_map(|&(x0, y0, w, h)| {
			(y0..y0 + h).flat_map(move |y| (x0..x0 + w).map(move |x| (x, y)))
		})
	};
	let all = |color: [u8; 4]| pixels().all(|(x, y)| image.get_pixel(x, y).0 == color);
	let transparent = pixels().any(|(x, y)| image.get_pixel(x, y).0[3] != 0xff);
	if transparent || all([0, 0, 0, 0xff]) || all([0xff, 0xff, 0xff, 0xff]) {
		SkinModel::Slim
	} else {
		SkinModel::Classic
	}
}

/// Имя файла без `.png`, без управляющих символов, до 48 символов.
pub fn display_name(path: &Path) -> String {
	let file = path
		.file_name()
		.map(|name| name.to_string_lossy().into_owned())
		.unwrap_or_default();
	let stem = match file
		.len()
		.checked_sub(4)
		.and_then(|cut| file.split_at_checked(cut))
	{
		Some((stem, ext)) if ext.eq_ignore_ascii_case(".png") => stem,
		_ => file.as_str(),
	};
	let clean: String = stem
		.chars()
		.filter(|c| !c.is_control())
		.take(NAME_MAX_CHARS)
		.collect();
	match clean.trim() {
		"" => "skin".to_owned(),
		name => name.to_owned(),
	}
}

/// Имя от пользователя: без пробелов по краям, 1–48 символов, без управляющих.
pub fn clean_name(name: &str) -> Option<String> {
	let name = name.trim();
	let count = name.chars().count();
	if count == 0 || count > NAME_MAX_CHARS || name.chars().any(char::is_control) {
		return None;
	}
	Some(name.to_owned())
}

#[cfg(test)]
pub(crate) mod test_util {
	use image::{ImageEncoder, Rgba, RgbaImage};

	pub fn image_of(width: u32, height: u32, seed: u8) -> RgbaImage {
		RgbaImage::from_fn(width, height, |x, y| {
			Rgba([seed, (x * 3) as u8, (y * 3) as u8, 0xff])
		})
	}

	pub fn encode(image: &RgbaImage) -> Vec<u8> {
		let mut out = Vec::new();
		image::codecs::png::PngEncoder::new(&mut out)
			.write_image(
				image.as_raw(),
				image.width(),
				image.height(),
				image::ExtendedColorType::Rgba8,
			)
			.unwrap();
		out
	}

	/// Непрозрачный скин с широкими руками.
	pub fn skin(width: u32, height: u32, seed: u8) -> Vec<u8> {
		encode(&image_of(width, height, seed))
	}

	pub fn paint_gaps(image: &mut RgbaImage, color: [u8; 4]) {
		for (x0, y0, w, h) in super::SLIM_GAPS {
			for y in y0..y0 + h {
				for x in x0..x0 + w {
					image.put_pixel(x, y, Rgba(color));
				}
			}
		}
	}

	/// Чанк PNG с настоящей контрольной суммой.
	pub fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
		let mut crc = flate2::Crc::new();
		crc.update(kind);
		crc.update(data);
		let mut out = (data.len() as u32).to_be_bytes().to_vec();
		out.extend_from_slice(kind);
		out.extend_from_slice(data);
		out.extend_from_slice(&crc.sum().to_be_bytes());
		out
	}
}

#[cfg(test)]
mod tests {
	use super::test_util::*;
	use super::*;

	#[test]
	fn accepts_only_skin_sizes() {
		assert!(!parse(&skin(64, 64, 1)).unwrap().legacy);
		assert!(parse(&skin(64, 32, 1)).unwrap().legacy);
		for (width, height) in [(100, 100), (128, 128), (64, 128), (32, 32), (64, 48)] {
			assert_eq!(
				parse(&skin(width, height, 1)).unwrap_err(),
				Rejection::BadSize { width, height },
			);
		}
	}

	#[test]
	fn cape_sizes() {
		for (width, height) in [
			(64, 32),
			(128, 64),
			(22, 17),
			(44, 34),
			(46, 22),
			(1024, 512),
		] {
			let png = parse_cape(&skin(width, height, 1)).unwrap();
			let back = image::load_from_memory(&png).unwrap();
			assert_eq!((back.width(), back.height()), (width, height));
		}
		for (width, height) in [(64, 64), (100, 40), (2048, 1024)] {
			assert_eq!(
				parse_cape(&skin(width, height, 1)).unwrap_err(),
				Rejection::BadSize { width, height },
			);
		}
		assert_eq!(parse_cape(b"text").unwrap_err(), Rejection::NotPng);
	}

	#[test]
	fn rejects_other_files() {
		assert_eq!(parse(b"GIF89a").unwrap_err(), Rejection::NotPng);
		assert_eq!(parse(b"").unwrap_err(), Rejection::NotPng);
		// Подпись есть, заголовка нет.
		assert_eq!(parse(SIGNATURE).unwrap_err(), Rejection::Unreadable);
		let mut no_ihdr = SIGNATURE.to_vec();
		no_ihdr.extend_from_slice(&chunk(b"tEXt", &[0; 13]));
		assert_eq!(parse(&no_ihdr).unwrap_err(), Rejection::Unreadable);
		// Заголовок верный, данные — мусор.
		let mut broken = skin(64, 64, 1);
		broken.truncate(HEADER_LEN + 4);
		assert_eq!(parse(&broken).unwrap_err(), Rejection::Unreadable);
	}

	#[test]
	fn huge_size_in_header_is_not_unpacked() {
		// Настоящий распаковщик на этом файле дал бы ошибку данных (Unreadable) или занял бы гигабайты.
		let mut ihdr = Vec::new();
		ihdr.extend_from_slice(&40_000u32.to_be_bytes());
		ihdr.extend_from_slice(&40_000u32.to_be_bytes());
		ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
		let mut png = SIGNATURE.to_vec();
		png.extend_from_slice(&chunk(b"IHDR", &ihdr));
		png.extend_from_slice(&chunk(b"IDAT", b"garbage"));
		png.extend_from_slice(&chunk(b"IEND", b""));
		assert_eq!(
			parse(&png).unwrap_err(),
			Rejection::BadSize {
				width: 40_000,
				height: 40_000
			},
		);
	}

	/// `chunk` сразу за IHDR.
	fn with_chunk(source: &[u8], extra: &[u8]) -> Vec<u8> {
		let after_ihdr = 8 + 25;
		let mut out = source[..after_ihdr].to_vec();
		out.extend_from_slice(extra);
		out.extend_from_slice(&source[after_ihdr..]);
		out
	}

	fn iccp(unpacked: usize) -> Vec<u8> {
		use std::io::Write;
		let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
		let zeros = vec![0u8; 1024 * 1024];
		for _ in 0..unpacked / zeros.len() {
			zlib.write_all(&zeros).unwrap();
		}
		let mut data = b"x\0\0".to_vec();
		data.extend_from_slice(&zlib.finish().unwrap());
		chunk(b"iCCP", &data)
	}

	/// Считает память только своего потока: тесты идут параллельно.
	struct Counting;

	thread_local! {
		static TRACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
		static NOW: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
		static PEAK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
	}

	fn note(grown: usize, shrunk: usize) {
		let _ = TRACK.try_with(|track| {
			if track.get() {
				let now = NOW.get().saturating_add(grown).saturating_sub(shrunk);
				NOW.set(now);
				PEAK.set(PEAK.get().max(now));
			}
		});
	}

	// SAFETY: все вызовы передаются системному распределителю без изменений.
	unsafe impl std::alloc::GlobalAlloc for Counting {
		unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
			note(layout.size(), 0);
			unsafe { std::alloc::System.alloc(layout) }
		}

		unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
			note(0, layout.size());
			unsafe { std::alloc::System.dealloc(ptr, layout) }
		}

		unsafe fn realloc(
			&self,
			ptr: *mut u8,
			layout: std::alloc::Layout,
			new_size: usize,
		) -> *mut u8 {
			note(new_size, layout.size());
			unsafe { std::alloc::System.realloc(ptr, layout, new_size) }
		}
	}

	#[global_allocator]
	static ALLOCATOR: Counting = Counting;

	fn peak_of<T>(work: impl FnOnce() -> T) -> (T, usize) {
		NOW.set(0);
		PEAK.set(0);
		TRACK.set(true);
		let result = work();
		TRACK.set(false);
		(result, PEAK.get())
	}

	#[test]
	fn iccp_bomb_stays_within_memory_limit() {
		const UNPACKED: usize = 64 * 1024 * 1024;
		let bomb = with_chunk(&skin(64, 64, 1), &iccp(UNPACKED));
		assert!((bomb.len() as u64) < MAX_BYTES, "{}", bomb.len());
		// Без лимита тот же файл занимал 69 МБ, с лимитом — 17 МБ.
		let (result, peak) = peak_of(|| parse(&bomb));
		assert!(peak < 2 * MAX_DECODE_ALLOC as usize, "пик {peak} байт");
		// Файл либо отвергнут, либо записан заново уже без бомбы.
		if let Ok(parsed) = result {
			assert!(parsed.png.len() < 64 * 1024);
		}
	}

	#[cfg(windows)]
	#[test]
	fn device_is_not_a_file() {
		assert_eq!(
			read_file(Path::new("NUL")).unwrap_err(),
			Rejection::Unreadable
		);
	}

	#[test]
	fn file_limit() {
		let dir = tempfile::tempdir().unwrap();
		let large = dir.path().join("large.png");
		let mut bytes = skin(64, 64, 1);
		bytes.resize(MAX_BYTES as usize + 1, 0);
		std::fs::write(&large, &bytes).unwrap();
		assert_eq!(read_file(&large).unwrap_err(), Rejection::TooLarge);

		// Ровно предел: хвост после IEND не мешает, файл принимается.
		let edge = dir.path().join("edge.png");
		bytes.truncate(MAX_BYTES as usize);
		std::fs::write(&edge, &bytes).unwrap();
		assert!(read_file(&edge).is_ok());

		assert_eq!(
			read_file(&dir.path().join("none.png")).unwrap_err(),
			Rejection::Unreadable
		);
		assert_eq!(read_file(dir.path()).unwrap_err(), Rejection::Unreadable);
	}

	#[test]
	fn rewrites_clean_png() {
		let source = skin(64, 64, 7);
		let end = source.len() - 12;
		let mut dirty = source[..end].to_vec();
		dirty.extend_from_slice(&chunk(b"tEXt", b"Comment\0secret-note"));
		dirty.extend_from_slice(&source[end..]);

		let parsed = parse(&dirty).unwrap();
		let contains = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);
		assert!(contains(&dirty, b"secret-note"));
		assert!(!contains(&parsed.png, b"secret-note"));
		// Пиксели те же, значит и id тот же.
		assert_eq!(parsed.id, parse(&source).unwrap().id);
		assert_eq!(parsed.id.len(), 40);
		assert_ne!(parsed.id, parse(&skin(64, 64, 8)).unwrap().id);
		let back = image::load_from_memory(&parsed.png).unwrap().to_rgba8();
		assert_eq!(back, image_of(64, 64, 7));
	}

	#[test]
	fn infers_arms() {
		let model = |image: &RgbaImage| parse(&encode(image)).unwrap().model;
		let mut image = image_of(64, 64, 1);
		assert_eq!(model(&image), SkinModel::Classic);

		paint_gaps(&mut image, [0, 0, 0, 0]);
		assert_eq!(model(&image), SkinModel::Slim);
		paint_gaps(&mut image, [0, 0, 0, 0xff]);
		assert_eq!(model(&image), SkinModel::Slim);
		paint_gaps(&mut image, [0xff, 0xff, 0xff, 0xff]);
		assert_eq!(model(&image), SkinModel::Slim);
		paint_gaps(&mut image, [10, 10, 10, 0xff]);
		assert_eq!(model(&image), SkinModel::Classic);

		// Один полупрозрачный пиксель — уже тонкие; один чёрный среди цветных — ещё нет.
		let mut one = image_of(64, 64, 1);
		one.put_pixel(46, 52, image::Rgba([1, 1, 1, 0x80]));
		assert_eq!(model(&one), SkinModel::Slim);
		let mut one = image_of(64, 64, 1);
		one.put_pixel(50, 16, image::Rgba([0, 0, 0, 0xff]));
		assert_eq!(model(&one), SkinModel::Classic);

		// 64×32: всегда широкие, даже с прозрачностью.
		let mut legacy = image_of(64, 32, 1);
		legacy.put_pixel(50, 16, image::Rgba([0, 0, 0, 0]));
		assert_eq!(model(&legacy), SkinModel::Classic);
	}

	#[test]
	fn names() {
		let name = |text: &str| display_name(Path::new(text));
		assert_eq!(name(r"C:\skins\Рыцарь.png"), "Рыцарь");
		assert_eq!(name("Knight.PNG"), "Knight");
		assert_eq!(name("a.b.png"), "a.b");
		assert_eq!(name("noext"), "noext");
		assert_eq!(name("яя"), "яя");
		assert_eq!(name(".png"), "skin");
		assert_eq!(name("a\u{7}b.png"), "ab");
		assert_eq!(name(&format!("{}.png", "я".repeat(60))).chars().count(), 48);

		assert_eq!(clean_name("  Маг "), Some("Маг".to_owned()));
		assert_eq!(clean_name("   "), None);
		assert_eq!(clean_name("a\nb"), None);
		assert!(clean_name(&"я".repeat(48)).is_some());
		assert!(clean_name(&"я".repeat(49)).is_none());
	}
}
