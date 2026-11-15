//! Минимальное чтение NBT (формат `level.dat`): только то, что нужно для списка миров.
//! Массивы не храним (только длину), глубина и размеры ограничены.

use std::io::Read;

/// Распакованный `level.dat` больше этого не читаем.
const MAX_SIZE: u64 = 32 * 1024 * 1024;
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
	Byte(i8),
	Short(i16),
	Int(i32),
	Long(i64),
	Float(f32),
	Double(f64),
	/// Массив байт, int или long: только длина.
	Array(usize),
	String(String),
	List(Vec<Tag>),
	Compound(Vec<(String, Tag)>),
}

impl Tag {
	pub fn get(&self, name: &str) -> Option<&Tag> {
		match self {
			Self::Compound(items) => items.iter().find(|(n, _)| n == name).map(|(_, t)| t),
			_ => None,
		}
	}

	pub fn as_str(&self) -> Option<&str> {
		match self {
			Self::String(s) => Some(s),
			_ => None,
		}
	}

	pub fn as_i64(&self) -> Option<i64> {
		match *self {
			Self::Byte(v) => Some(v.into()),
			Self::Short(v) => Some(v.into()),
			Self::Int(v) => Some(v.into()),
			Self::Long(v) => Some(v),
			_ => None,
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub enum NbtError {
	#[error("файл обрывается")]
	Eof,
	#[error("неизвестный тип {0}")]
	BadType(u8),
	#[error("слишком глубоко")]
	TooDeep,
	#[error("не NBT: корень не compound")]
	NotCompound,
	#[error("{0}")]
	Io(#[from] std::io::Error),
}

struct Reader<'a> {
	data: &'a [u8],
	pos: usize,
}

impl Reader<'_> {
	fn take(&mut self, n: usize) -> Result<&[u8], NbtError> {
		let end = self.pos.checked_add(n).ok_or(NbtError::Eof)?;
		let slice = self.data.get(self.pos..end).ok_or(NbtError::Eof)?;
		self.pos = end;
		Ok(slice)
	}

	fn array<const N: usize>(&mut self) -> Result<[u8; N], NbtError> {
		let mut out = [0u8; N];
		out.copy_from_slice(self.take(N)?);
		Ok(out)
	}

	fn u8(&mut self) -> Result<u8, NbtError> {
		Ok(self.take(1)?[0])
	}

	fn i32(&mut self) -> Result<i32, NbtError> {
		Ok(i32::from_be_bytes(self.array()?))
	}

	fn len(&mut self, item: usize) -> Result<usize, NbtError> {
		let n = usize::try_from(self.i32()?.max(0)).map_err(|_| NbtError::Eof)?;
		// Длина не может быть больше оставшихся байт.
		if n.saturating_mul(item.max(1)) > self.data.len() - self.pos {
			return Err(NbtError::Eof);
		}
		Ok(n)
	}

	fn string(&mut self) -> Result<String, NbtError> {
		let n = u16::from_be_bytes(self.array()?) as usize;
		Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
	}

	fn payload(&mut self, kind: u8, depth: usize) -> Result<Tag, NbtError> {
		if depth > MAX_DEPTH {
			return Err(NbtError::TooDeep);
		}
		Ok(match kind {
			1 => Tag::Byte(self.u8()? as i8),
			2 => Tag::Short(i16::from_be_bytes(self.array()?)),
			3 => Tag::Int(self.i32()?),
			4 => Tag::Long(i64::from_be_bytes(self.array()?)),
			5 => Tag::Float(f32::from_be_bytes(self.array()?)),
			6 => Tag::Double(f64::from_be_bytes(self.array()?)),
			7 | 11 | 12 => {
				let size = match kind {
					7 => 1,
					11 => 4,
					_ => 8,
				};
				let n = self.len(size)?;
				self.take(n * size)?;
				Tag::Array(n)
			}
			8 => Tag::String(self.string()?),
			9 => {
				let item = self.u8()?;
				let n = self.len(1)?;
				if item == 0 {
					Tag::List(Vec::new())
				} else {
					let mut list = Vec::with_capacity(n.min(1024));
					for _ in 0..n {
						list.push(self.payload(item, depth + 1)?);
					}
					Tag::List(list)
				}
			}
			10 => {
				let mut items = Vec::new();
				loop {
					let kind = self.u8()?;
					if kind == 0 {
						break;
					}
					let name = self.string()?;
					items.push((name, self.payload(kind, depth + 1)?));
				}
				Tag::Compound(items)
			}
			other => return Err(NbtError::BadType(other)),
		})
	}
}

/// Разбирает несжатый NBT: корневой compound.
pub fn parse(data: &[u8]) -> Result<Tag, NbtError> {
	let mut r = Reader { data, pos: 0 };
	if r.u8()? != 10 {
		return Err(NbtError::NotCompound);
	}
	r.string()?;
	r.payload(10, 0)
}

/// Читает файл NBT: сжатый gzip (как `level.dat`) или нет.
pub fn read_file(path: &std::path::Path) -> Result<Tag, NbtError> {
	let raw = std::fs::read(path)?;
	if raw.starts_with(&[0x1f, 0x8b]) {
		let mut data = Vec::new();
		flate2::read::GzDecoder::new(raw.as_slice())
			.take(MAX_SIZE)
			.read_to_end(&mut data)?;
		parse(&data)
	} else {
		parse(&raw)
	}
}

#[cfg(test)]
pub mod tests {
	use super::*;

	/// Пишет простой NBT для тестов.
	pub enum W<'a> {
		Byte(i8),
		Int(i32),
		Long(i64),
		Str(&'a str),
		Compound(Vec<(&'a str, W<'a>)>),
		Ints(Vec<i32>),
	}

	impl W<'_> {
		fn id(&self) -> u8 {
			match self {
				Self::Byte(_) => 1,
				Self::Int(_) => 3,
				Self::Long(_) => 4,
				Self::Str(_) => 8,
				Self::Compound(_) => 10,
				Self::Ints(_) => 11,
			}
		}

		fn write(&self, out: &mut Vec<u8>) {
			match self {
				Self::Byte(v) => out.push(*v as u8),
				Self::Int(v) => out.extend(v.to_be_bytes()),
				Self::Long(v) => out.extend(v.to_be_bytes()),
				Self::Str(s) => {
					out.extend((s.len() as u16).to_be_bytes());
					out.extend(s.as_bytes());
				}
				Self::Compound(items) => {
					for (name, tag) in items {
						out.push(tag.id());
						W::Str(name).write(out);
						tag.write(out);
					}
					out.push(0);
				}
				Self::Ints(v) => {
					out.extend((v.len() as i32).to_be_bytes());
					for i in v {
						out.extend(i.to_be_bytes());
					}
				}
			}
		}
	}

	pub fn encode(root: W) -> Vec<u8> {
		let mut out = vec![10];
		W::Str("").write(&mut out);
		root.write(&mut out);
		out
	}

	pub fn gzip(data: &[u8]) -> Vec<u8> {
		use std::io::Write;
		let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
		enc.write_all(data).unwrap();
		enc.finish().unwrap()
	}

	#[test]
	fn parses_level_dat() {
		let data = encode(W::Compound(vec![(
			"Data",
			W::Compound(vec![
				("LevelName", W::Str("Мой мир")),
				("LastPlayed", W::Long(1_700_000_000_000)),
				("GameType", W::Int(1)),
				("hardcore", W::Byte(1)),
				("WanderingTraderId", W::Ints(vec![1, 2, 3, 4])),
			]),
		)]));
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("level.dat");
		std::fs::write(&path, gzip(&data)).unwrap();
		let root = read_file(&path).unwrap();
		let level = root.get("Data").unwrap();
		assert_eq!(level.get("LevelName").unwrap().as_str(), Some("Мой мир"));
		assert_eq!(
			level.get("LastPlayed").unwrap().as_i64(),
			Some(1_700_000_000_000)
		);
		assert_eq!(level.get("GameType").unwrap().as_i64(), Some(1));
		assert_eq!(level.get("WanderingTraderId"), Some(&Tag::Array(4)));

		std::fs::write(&path, &data).unwrap();
		assert!(read_file(&path).is_ok());
		assert!(matches!(parse(&data[..data.len() - 3]), Err(NbtError::Eof)));
		assert!(matches!(parse(&[8, 0, 0]), Err(NbtError::NotCompound)));
		// Огромная длина массива при маленьком файле — ошибка, а не выделение памяти.
		let bad = [10, 0, 0, 11, 0, 1, b'a', 0x7f, 0xff, 0xff, 0xff, 0];
		assert!(parse(&bad).is_err());
	}
}
