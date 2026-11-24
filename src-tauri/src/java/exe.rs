//! Разрядность исполняемого файла по заголовку (PE, Mach-O, ELF) — без запуска JVM.

use std::io::Read;
use std::path::Path;

/// Сколько байт заголовка читаем: хватает на PE (`e_lfanew` обычно < 0x200) и таблицу fat Mach-O.
const HEAD: usize = 4096;
/// Больше архитектур в universal-файле не бывает; иначе это не Mach-O (у class-файлов тот же magic).
const MAX_FAT_ARCHES: u32 = 16;

/// `x64`, `x86`, `arm64` по заголовку файла; неизвестный формат — `None`.
pub fn arch_of(path: &Path) -> Option<&'static str> {
	let mut file = std::fs::File::open(path).ok()?;
	let mut head = Vec::with_capacity(HEAD);
	file.by_ref()
		.take(HEAD as u64)
		.read_to_end(&mut head)
		.ok()?;
	parse(&head, crate::meta::rules::Platform::current().arch)
}

/// `host` — какую архитектуру предпочесть в universal Mach-O.
fn parse(head: &[u8], host: &str) -> Option<&'static str> {
	if head.starts_with(b"MZ") {
		return pe(head);
	}
	if head.starts_with(b"\x7fELF") {
		return elf(head);
	}
	let magic = u32_at(head, 0, false)?;
	match magic {
		0xfeed_facf | 0xfeed_face => mach_cpu(u32_at(head, 4, false)?),
		0xcffa_edfe | 0xcefa_edfe => mach_cpu(u32_at(head, 4, true)?),
		_ => fat(head, host),
	}
}

fn u16_at(b: &[u8], at: usize, big: bool) -> Option<u16> {
	let bytes: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
	Some(if big {
		u16::from_be_bytes(bytes)
	} else {
		u16::from_le_bytes(bytes)
	})
}

fn u32_at(b: &[u8], at: usize, big: bool) -> Option<u32> {
	let bytes: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
	Some(if big {
		u32::from_be_bytes(bytes)
	} else {
		u32::from_le_bytes(bytes)
	})
}

fn pe(head: &[u8]) -> Option<&'static str> {
	let at = u32_at(head, 0x3c, false)? as usize;
	if head.get(at..at + 4)? != b"PE\0\0" {
		return None;
	}
	match u16_at(head, at + 4, false)? {
		0x8664 => Some("x64"),
		0xaa64 => Some("arm64"),
		0x014c => Some("x86"),
		_ => None,
	}
}

fn elf(head: &[u8]) -> Option<&'static str> {
	let big = *head.get(5)? == 2;
	match u16_at(head, 0x12, big)? {
		0x3e => Some("x64"),
		0xb7 => Some("arm64"),
		0x03 => Some("x86"),
		_ => None,
	}
}

fn mach_cpu(cpu: u32) -> Option<&'static str> {
	match cpu {
		0x0100_0007 => Some("x64"),
		0x0100_000c => Some("arm64"),
		0x0000_0007 => Some("x86"),
		_ => None,
	}
}

/// Universal Mach-O: таблица архитектур в big-endian; `cafebabf` — с 64-битными смещениями.
fn fat(head: &[u8], host: &str) -> Option<&'static str> {
	let entry = match u32_at(head, 0, true)? {
		0xcafe_babe => 20,
		0xcafe_babf => 32,
		_ => return None,
	};
	let count = u32_at(head, 4, true)?;
	if count == 0 || count > MAX_FAT_ARCHES {
		return None;
	}
	let arches: Vec<&'static str> = (0..count as usize)
		.filter_map(|i| mach_cpu(u32_at(head, 8 + i * entry, true)?))
		.collect();
	arches
		.iter()
		.copied()
		.find(|a| *a == host)
		.or_else(|| arches.first().copied())
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	pub fn pe_header(machine: u16) -> Vec<u8> {
		let mut b = vec![0u8; 0x100];
		b[..2].copy_from_slice(b"MZ");
		b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
		b[0x80..0x84].copy_from_slice(b"PE\0\0");
		b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
		b
	}

	pub fn macho_header(cpu: u32) -> Vec<u8> {
		let mut b = vec![0u8; 32];
		b[..4].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
		b[4..8].copy_from_slice(&cpu.to_le_bytes());
		b
	}

	fn fat_header(cpus: &[u32]) -> Vec<u8> {
		let mut b = Vec::new();
		b.extend_from_slice(&0xcafe_babeu32.to_be_bytes());
		b.extend_from_slice(&(cpus.len() as u32).to_be_bytes());
		for (i, cpu) in cpus.iter().enumerate() {
			b.extend_from_slice(&cpu.to_be_bytes());
			b.extend_from_slice(&0u32.to_be_bytes());
			b.extend_from_slice(&(0x4000 * (i as u32 + 1)).to_be_bytes());
			b.extend_from_slice(&0x1000u32.to_be_bytes());
			b.extend_from_slice(&14u32.to_be_bytes());
		}
		b
	}

	#[test]
	fn reads_pe() {
		assert_eq!(parse(&pe_header(0x8664), "x64"), Some("x64"));
		assert_eq!(parse(&pe_header(0xaa64), "x64"), Some("arm64"));
		assert_eq!(parse(&pe_header(0x014c), "x64"), Some("x86"));
		assert_eq!(parse(&pe_header(0x01c4), "x64"), None);
		let mut broken = pe_header(0x8664);
		broken[0x80] = b'X';
		assert_eq!(parse(&broken, "x64"), None);
		assert_eq!(parse(b"MZ", "x64"), None);
	}

	#[test]
	fn reads_macho() {
		assert_eq!(parse(&macho_header(0x0100_000c), "x64"), Some("arm64"));
		assert_eq!(parse(&macho_header(0x0100_0007), "arm64"), Some("x64"));
		let both = fat_header(&[0x0100_0007, 0x0100_000c]);
		assert_eq!(parse(&both, "arm64"), Some("arm64"));
		assert_eq!(parse(&both, "x64"), Some("x64"));
		assert_eq!(parse(&fat_header(&[0x0100_0007]), "arm64"), Some("x64"));
		// Class-файл Java начинается так же (`cafebabe`), но дальше — версия, а не число архитектур.
		assert_eq!(parse(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52], "x64"), None);
	}

	#[test]
	fn reads_elf() {
		let mut b = vec![0u8; 64];
		b[..4].copy_from_slice(b"\x7fELF");
		b[5] = 1;
		b[0x12..0x14].copy_from_slice(&0xb7u16.to_le_bytes());
		assert_eq!(parse(&b, "x64"), Some("arm64"));
		assert_eq!(parse(b"#!/bin/sh\n", "x64"), None);
		assert_eq!(parse(&[], "x64"), None);
	}
}
