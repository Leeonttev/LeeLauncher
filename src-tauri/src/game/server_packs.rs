//! Защита от ресурспаков серверов, пока в версии уязвимый FreeType: у серверов в `servers.dat`
//! снимается «принимать ресурспак автоматически», игра снова спрашивает перед загрузкой.

use std::path::Path;

use crate::fs_util;
use crate::gamedata::nbt::{self, Node};

pub const EVENT: &str = "server-packs-guarded";
pub const FILE_NAME: &str = "servers.dat";

#[derive(Debug, Clone, serde::Serialize)]
pub struct Guarded {
	pub id: String,
	pub servers: usize,
}

/// Убирает `acceptTextures: 1` у всех серверов. Возвращает, сколько серверов изменено.
/// Нет файла — 0; файл не разобрался или не записался — ошибка (запуск она не срывает).
pub fn guard(game_dir: &Path) -> Result<usize, String> {
	let path = game_dir.join(FILE_NAME);
	let raw = match std::fs::read(&path) {
		Ok(raw) => raw,
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
		Err(err) => return Err(format!("{}: {err}", path.display())),
	};
	let mut doc = nbt::parse_document(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
	let changed = strip_auto_accept(&mut doc.root);
	if changed == 0 {
		return Ok(0);
	}
	let bytes = nbt::encode_document(&doc).map_err(|e| format!("{}: {e}", path.display()))?;
	fs_util::backup_copy(&path, &raw).map_err(|e| format!("копия {}: {e}", path.display()))?;
	fs_util::write_atomic(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
	Ok(changed)
}

fn strip_auto_accept(root: &mut Node) -> usize {
	let Some(Node::List(_, servers)) = root.get_mut("servers") else {
		return 0;
	};
	let mut changed = 0;
	for server in servers {
		let auto =
			matches!(server.get("acceptTextures"), Some(Node::Leaf(1, v)) if v.as_slice() == [1]);
		if auto && server.remove("acceptTextures") {
			changed += 1;
		}
	}
	changed
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::gamedata::nbt::tests::{W, encode, gzip};

	fn servers() -> Vec<u8> {
		let server = |name: &'static str, accept: Option<i8>| {
			let mut items = vec![("name", W::Str(name)), ("ip", W::Str("mc.example.org"))];
			if let Some(a) = accept {
				items.push(("acceptTextures", W::Byte(a)));
			}
			items.push(("icon", W::Str("iVBORw0KGgo=")));
			W::Compound(items)
		};
		encode(W::Compound(vec![(
			"servers",
			W::List(vec![
				server("Авто", Some(1)),
				server("Спрашивать", None),
				server("Никогда", Some(0)),
				server("Тоже авто", Some(1)),
			]),
		)]))
	}

	#[test]
	fn strips_auto_accept_and_keeps_rest() {
		for zipped in [false, true] {
			let dir = tempfile::tempdir().unwrap();
			let raw = if zipped { gzip(&servers()) } else { servers() };
			std::fs::write(dir.path().join(FILE_NAME), &raw).unwrap();
			assert_eq!(guard(dir.path()).unwrap(), 2);

			let written = std::fs::read(dir.path().join(FILE_NAME)).unwrap();
			assert_eq!(written.starts_with(&[0x1f, 0x8b]), zipped);
			let doc = nbt::parse_document(&written).unwrap();
			let Some(Node::List(10, list)) = doc.root.get("servers") else {
				panic!("нет servers");
			};
			assert_eq!(list.len(), 4);
			assert!(list[0].get("acceptTextures").is_none());
			assert!(list[1].get("acceptTextures").is_none());
			assert_eq!(list[2].get("acceptTextures"), Some(&Node::Leaf(1, vec![0])));
			assert!(list[3].get("acceptTextures").is_none());
			assert!(list[0].get("icon").is_some() && list[3].get("name").is_some());
			// Копия прежнего файла рядом.
			let backups: Vec<_> = std::fs::read_dir(dir.path())
				.unwrap()
				.flatten()
				.filter(|e| {
					e.file_name()
						.to_string_lossy()
						.starts_with("servers.dat.bak-")
				})
				.collect();
			assert_eq!(backups.len(), 1);
			assert_eq!(std::fs::read(backups[0].path()).unwrap(), raw);
			// Второй раз менять нечего — файл и копии не трогаются.
			assert_eq!(guard(dir.path()).unwrap(), 0);
			assert_eq!(std::fs::read(dir.path().join(FILE_NAME)).unwrap(), written);
		}
	}

	#[test]
	fn missing_and_broken_files() {
		let dir = tempfile::tempdir().unwrap();
		assert_eq!(guard(dir.path()).unwrap(), 0);
		let mut bad = servers();
		bad.truncate(bad.len() - 5);
		std::fs::write(dir.path().join(FILE_NAME), &bad).unwrap();
		assert!(guard(dir.path()).is_err());
		assert_eq!(std::fs::read(dir.path().join(FILE_NAME)).unwrap(), bad);
		// Без списка серверов — ничего не меняется.
		std::fs::write(dir.path().join(FILE_NAME), encode(W::Compound(vec![]))).unwrap();
		assert_eq!(guard(dir.path()).unwrap(), 0);
	}
}
