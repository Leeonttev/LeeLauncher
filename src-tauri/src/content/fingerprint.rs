//! Отпечаток CurseForge: MurmurHash2 (seed 1) без байтов 9, 10, 13, 32 — как в Prism Launcher.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const M: u32 = 0x5bd1_e995;
const R: u32 = 24;
const BUFFER: usize = 1024 * 1024;

fn skipped(b: u8) -> bool {
	matches!(b, 9 | 10 | 13 | 32)
}

/// Отпечаток данных в памяти.
#[cfg(test)]
pub fn of_bytes(data: &[u8]) -> u32 {
	let mut hasher = Hasher::new(data.iter().filter(|b| !skipped(**b)).count() as u32);
	hasher.update(data);
	hasher.finish()
}

/// Отпечаток файла: два прохода (длина без пробельных байтов нужна до начала хеширования).
pub fn of_file(path: &Path) -> std::io::Result<u32> {
	let mut file = std::fs::File::open(path)?;
	of_reader(&mut file)
}

pub fn of_reader<R: Read + Seek>(reader: &mut R) -> std::io::Result<u32> {
	let mut buf = vec![0u8; BUFFER];
	let mut len: u32 = 0;
	loop {
		let n = reader.read(&mut buf)?;
		if n == 0 {
			break;
		}
		let counted = buf[..n].iter().filter(|b| !skipped(**b)).count() as u32;
		len = len.wrapping_add(counted);
	}
	reader.seek(SeekFrom::Start(0))?;
	let mut hasher = Hasher::new(len);
	loop {
		let n = reader.read(&mut buf)?;
		if n == 0 {
			break;
		}
		hasher.update(&buf[..n]);
	}
	Ok(hasher.finish())
}

struct Hasher {
	h: u32,
	tail: [u8; 4],
	filled: usize,
}

impl Hasher {
	fn new(len: u32) -> Self {
		Self {
			h: 1 ^ len,
			tail: [0; 4],
			filled: 0,
		}
	}

	fn update(&mut self, data: &[u8]) {
		for &b in data {
			if skipped(b) {
				continue;
			}
			self.tail[self.filled] = b;
			self.filled += 1;
			if self.filled == 4 {
				let mut k = u32::from_le_bytes(self.tail);
				k = k.wrapping_mul(M);
				k ^= k >> R;
				k = k.wrapping_mul(M);
				self.h = self.h.wrapping_mul(M);
				self.h ^= k;
				self.filled = 0;
			}
		}
	}

	fn finish(self) -> u32 {
		let mut h = self.h;
		let t = self.tail;
		if self.filled >= 3 {
			h ^= u32::from(t[2]) << 16;
		}
		if self.filled >= 2 {
			h ^= u32::from(t[1]) << 8;
		}
		if self.filled >= 1 {
			h ^= u32::from(t[0]);
			h = h.wrapping_mul(M);
		}
		h ^= h >> 13;
		h = h.wrapping_mul(M);
		h ^= h >> 15;
		h
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Значения посчитаны отдельной реализацией MurmurHash2 по исходному коду Остина Эпплби
	/// (seed 1, пробельные байты выброшены заранее).
	#[test]
	fn known_values() {
		let cases: [(&[u8], u32); 7] = [
			(b"", 1_540_447_798),
			(b"a", 626_045_324),
			(b"ab", 1_692_487_918),
			(b"abc", 1_621_425_345),
			(b"abcd", 3_376_380_438),
			(b"Hello, world!", 2_173_663_876),
			(b"hello world\r\n\tfoo", 1_776_490_315),
		];
		for (data, expected) in cases {
			assert_eq!(
				of_bytes(data),
				expected,
				"{:?}",
				String::from_utf8_lossy(data)
			);
		}
		let big: Vec<u8> = (0..=255u8).cycle().take(768).collect();
		assert_eq!(of_bytes(&big), 137_755_623);
		// Пробельные байты не влияют.
		assert_eq!(of_bytes(b"a b\tc\r\nd"), of_bytes(b"abcd"));
	}

	#[test]
	fn file_matches_bytes() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("f.jar");
		// Больше буфера, чтобы проверить чтение кусками.
		let data: Vec<u8> = (0..BUFFER * 2 + 7).map(|i| (i * 31 % 251) as u8).collect();
		std::fs::write(&path, &data).unwrap();
		assert_eq!(of_file(&path).unwrap(), of_bytes(&data));
	}
}
