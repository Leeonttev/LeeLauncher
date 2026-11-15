//! Библиотека скинов: `skins/library.json` и файлы `skins/<id>.png`. Сеть здесь не нужна.
//! Старые записи `worn` (версия 1) не читаются и исчезают при следующей записи.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::png::{self, Rejection};
use super::textures::safe_key;
use super::{RejectReason, RejectedFile, SkinEntry, SkinModel, SkinPatch, SkinsAdded};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;

const FILE_NAME: &str = "library.json";
const CURRENT_VERSION: u32 = 2;
const RECOVERED_NAME: &str = "skin";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryFile {
	#[serde(default)]
	version: u32,
	/// Новые сверху.
	#[serde(default)]
	skins: Vec<StoredSkin>,
	/// Ключ текстуры Mojang → скин, загруженный через лаунчер.
	#[serde(default)]
	keys: BTreeMap<String, String>,
}

impl Default for LibraryFile {
	fn default() -> Self {
		Self {
			version: CURRENT_VERSION,
			skins: Vec::new(),
			keys: BTreeMap::new(),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSkin {
	id: String,
	name: String,
	model: SkinModel,
	#[serde(default)]
	legacy: bool,
	added_at: String,
}

impl LibraryFile {
	fn find(&self, id: &str) -> Option<&StoredSkin> {
		self.skins.iter().find(|skin| skin.id == id)
	}
}

/// `library.json` как есть: записи разбираются по одной, поэтому неверная не сбрасывает всю библиотеку.
#[derive(Default, Deserialize)]
struct RawFile {
	#[serde(default)]
	version: u32,
	#[serde(default)]
	skins: Vec<Value>,
	#[serde(default)]
	keys: BTreeMap<String, Value>,
}

struct ParsedIndex {
	file: LibraryFile,
	/// Названия из отброшенных записей: файл скина найдётся при сканировании, а имя взять уже неоткуда.
	names: HashMap<String, String>,
	/// Часть записей отброшена.
	damaged: bool,
}

impl RawFile {
	/// Поднимает версию и убирает то, чему нельзя верить: неполные записи, чужие id
	/// (они становятся именем файла) и повторы.
	fn parse(self) -> ParsedIndex {
		if self.version > CURRENT_VERSION {
			log::warn!(
				"{FILE_NAME} версии {} новее поддерживаемой {CURRENT_VERSION}, читаем как есть",
				self.version
			);
		}
		let mut names = HashMap::new();
		let mut damaged = false;
		let mut seen = HashSet::new();
		let mut skins = Vec::new();
		for value in self.skins {
			let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
			let salvage = text("id").zip(text("name"));
			match serde_json::from_value::<StoredSkin>(value) {
				Ok(skin) if fs_util::is_sha1_hex(&skin.id) && seen.insert(skin.id.clone()) => {
					skins.push(skin);
				}
				_ => {
					log::warn!("{FILE_NAME}: запись скина отброшена");
					damaged = true;
					if let Some((id, name)) = salvage {
						names.entry(id).or_insert(name);
					}
				}
			}
		}
		let keys = self
			.keys
			.into_iter()
			.filter_map(|(key, id)| Some((key, id.as_str()?.to_owned())))
			.collect();
		ParsedIndex {
			file: LibraryFile {
				version: self.version.max(CURRENT_VERSION),
				skins,
				keys,
			},
			names,
			damaged,
		}
	}
}

pub struct Library {
	dir: PathBuf,
	/// `None` — файл не прочитан; каждая команда пробует снова.
	file: Mutex<Option<LibraryFile>>,
}

fn now_iso() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn data_url(png: &[u8]) -> String {
	format!("data:image/png;base64,{}", BASE64_STANDARD.encode(png))
}

fn entry(skin: &StoredSkin, texture: String) -> SkinEntry {
	SkinEntry {
		id: skin.id.clone(),
		name: skin.name.clone(),
		model: skin.model,
		legacy: skin.legacy,
		added_at: skin.added_at.clone(),
		texture,
	}
}

fn storage(path: &Path, err: impl std::fmt::Display) -> AppError {
	AppError::storage(format!("{}: {err}", path.display()))
}

fn not_found() -> AppError {
	AppError::new(ErrorCode::SkinNotFound)
}

impl Library {
	pub fn new(dir: PathBuf) -> Self {
		Self {
			dir,
			file: Mutex::new(None),
		}
	}

	fn index_path(&self) -> PathBuf {
		self.dir.join(FILE_NAME)
	}

	fn png_path(&self, id: &str) -> Result<PathBuf, AppError> {
		if !fs_util::is_sha1_hex(id) {
			return Err(not_found());
		}
		fs_util::safe_join(&self.dir, &format!("{id}.png")).ok_or_else(not_found)
	}

	fn lock(&self) -> MutexGuard<'_, Option<LibraryFile>> {
		self.file.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn loaded<'a>(
		&self,
		slot: &'a mut Option<LibraryFile>,
	) -> Result<&'a mut LibraryFile, AppError> {
		if slot.is_none() {
			*slot = Some(self.read_index()?);
		}
		slot.as_mut()
			.ok_or_else(|| AppError::storage("библиотека скинов не загружена"))
	}

	/// Восстанавливает записи по файлам скинов, если индекс потерян.
	fn read_index(&self) -> Result<LibraryFile, AppError> {
		let path = self.index_path();
		let raw =
			fs_util::read_json_or_backup::<RawFile>(&path).map_err(|err| storage(&path, err))?;
		let lost = raw.is_none();
		let ParsedIndex {
			mut file,
			names,
			damaged,
		} = raw.unwrap_or_default().parse();
		if damaged {
			std::fs::read(&path)
				.and_then(|bytes| fs_util::backup_copy(&path, &bytes))
				.map_err(|err| storage(&path, err))?;
		}
		let recovered = if lost || damaged {
			self.recover(&mut file, &names)
		} else {
			0
		};
		let known: HashSet<String> = file.skins.iter().map(|skin| skin.id.clone()).collect();
		file.keys
			.retain(|key, id| safe_key(key) && known.contains(id));
		if recovered > 0 {
			log::warn!("{FILE_NAME}: восстановлено скинов по файлам: {recovered}");
		}
		if (damaged || recovered > 0 || (lost && path.exists()))
			&& let Err(err) = self.save(&file)
		{
			log::warn!("не удалось записать восстановленную библиотеку: {err}");
		}
		Ok(file)
	}

	/// Добавляет записи для `<sha1>.png`, которых нет в индексе: файл должен быть настоящим скином,
	/// а его хеш — совпадать с именем. Остальное не трогается.
	fn recover(&self, file: &mut LibraryFile, names: &HashMap<String, String>) -> usize {
		let Ok(entries) = std::fs::read_dir(&self.dir) else {
			return 0;
		};
		let mut found = Vec::new();
		for item in entries.flatten() {
			let file_name = item.file_name();
			let Some(id) = file_name
				.to_str()
				.and_then(|name| name.strip_suffix(".png"))
				.filter(|id| fs_util::is_sha1_hex(id) && !id.contains(char::is_uppercase))
			else {
				continue;
			};
			if file.find(id).is_some() {
				continue;
			}
			let path = item.path();
			match png::read_file(&path) {
				Ok(skin) if skin.id == id => {
					let added_at = item
						.metadata()
						.and_then(|meta| meta.modified())
						.map(|time| {
							DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Secs, true)
						})
						.unwrap_or_else(|_| now_iso());
					found.push(StoredSkin {
						id: skin.id,
						name: names
							.get(id)
							.and_then(|name| png::clean_name(name))
							.unwrap_or_else(|| RECOVERED_NAME.to_owned()),
						model: skin.model,
						legacy: skin.legacy,
						added_at,
					});
				}
				_ => log::warn!("не удалось восстановить скин: {}", path.display()),
			}
		}
		found.sort_by(|a, b| b.added_at.cmp(&a.added_at));
		let count = found.len();
		file.skins.extend(found);
		count
	}

	fn save(&self, file: &LibraryFile) -> Result<(), AppError> {
		let path = self.index_path();
		let bytes = serde_json::to_vec_pretty(file).map_err(|err| storage(&path, err))?;
		fs_util::write_atomic(&path, &bytes).map_err(|err| {
			log::error!("не удалось записать {}: {err}", path.display());
			storage(&path, err)
		})
	}

	/// Копия файла → запись на диск → замена в памяти: при ошибке записи состояние не меняется.
	fn update<T>(
		&self,
		change: impl FnOnce(&mut LibraryFile) -> Result<T, AppError>,
	) -> Result<T, AppError> {
		let mut slot = self.lock();
		let file = self.loaded(&mut slot)?;
		let mut next = file.clone();
		let result = change(&mut next)?;
		self.save(&next)?;
		*file = next;
		Ok(result)
	}

	fn snapshot(&self) -> Result<LibraryFile, AppError> {
		let mut slot = self.lock();
		Ok(self.loaded(&mut slot)?.clone())
	}

	/// Библиотека читается, иначе `storage`. Вызывать перед запросом, который уже не отменить.
	pub fn ensure_readable(&self) -> Result<(), AppError> {
		let mut slot = self.lock();
		self.loaded(&mut slot).map(|_| ())
	}

	/// Текстуры без замка. Скин без файла скрыт, но запись остаётся: файл может вернуться.
	fn textures<'a>(&self, ids: impl Iterator<Item = &'a str>) -> HashMap<String, String> {
		let mut textures = HashMap::new();
		for id in ids {
			let Ok(path) = self.png_path(id) else {
				continue;
			};
			match std::fs::read(&path) {
				Ok(bytes) => {
					textures.insert(id.to_owned(), data_url(&bytes));
				}
				Err(err) => log::warn!("файл скина не читается: {}: {err}", path.display()),
			}
		}
		textures
	}

	pub fn skins(&self) -> Result<Vec<SkinEntry>, AppError> {
		let file = self.snapshot()?;
		let mut textures = self.textures(file.skins.iter().map(|skin| skin.id.as_str()));
		Ok(file
			.skins
			.iter()
			.filter_map(|skin| Some(entry(skin, textures.remove(&skin.id)?)))
			.collect())
	}

	/// Скин библиотеки для текущего скина аккаунта: по ключу из ответа на загрузку, иначе по `id`
	/// скачанной текстуры. Скин без файла не считается: его не показать.
	pub fn skin_id_for(&self, key: &str, texture_id: Option<&str>) -> Option<String> {
		let file = match self.snapshot() {
			Ok(file) => file,
			Err(err) => {
				log::warn!("библиотека скинов не читается: {err}");
				return None;
			}
		};
		file.keys
			.get(key)
			.map(String::as_str)
			.into_iter()
			.chain(texture_id)
			.find(|id| {
				file.find(id).is_some() && self.png_path(id).is_ok_and(|path| path.is_file())
			})
			.map(str::to_owned)
	}

	pub fn remember_key(&self, key: &str, skin_id: &str) -> Result<(), AppError> {
		if !safe_key(key) {
			return Ok(());
		}
		{
			let mut slot = self.lock();
			let file = self.loaded(&mut slot)?;
			if file.find(skin_id).is_none() || file.keys.get(key).is_some_and(|id| id == skin_id) {
				return Ok(());
			}
		}
		self.update(|file| {
			file.keys.insert(key.to_owned(), skin_id.to_owned());
			Ok(())
		})
	}

	pub fn model_of(&self, id: &str) -> Result<SkinModel, AppError> {
		self.snapshot()?
			.find(id)
			.map(|skin| skin.model)
			.ok_or_else(not_found)
	}

	/// Скин из готовых байт (текстура из кэша). Такой уже есть — та же запись, имя не меняется.
	pub fn add_bytes(
		&self,
		bytes: &[u8],
		name: &str,
		key: Option<&str>,
	) -> Result<SkinEntry, AppError> {
		let skin = png::parse(bytes).map_err(|_| not_found())?;
		let name = png::clean_name(name).unwrap_or_else(|| RECOVERED_NAME.to_owned());
		let path = self.png_path(&skin.id)?;
		let texture = data_url(&skin.png);
		self.update(|file| {
			if !path.exists() {
				fs_util::write_atomic(&path, &skin.png).map_err(|err| storage(&path, err))?;
			}
			if let Some(key) = key.filter(|key| safe_key(key)) {
				file.keys.insert(key.to_owned(), skin.id.clone());
			}
			if file.find(&skin.id).is_none() {
				file.skins.insert(
					0,
					StoredSkin {
						id: skin.id.clone(),
						name,
						model: skin.model,
						legacy: skin.legacy,
						added_at: now_iso(),
					},
				);
			}
			let stored = file.find(&skin.id).ok_or_else(not_found)?;
			Ok(entry(stored, texture))
		})
	}

	pub fn add_files(&self, paths: &[String]) -> Result<SkinsAdded, AppError> {
		let mut parsed = Vec::new();
		let mut rejected = Vec::new();
		for text in paths {
			let path = Path::new(text);
			match png::read_file(path) {
				Ok(skin) => parsed.push((png::display_name(path), skin)),
				Err(rejection) => rejected.push(rejected_file(text, rejection)),
			}
		}

		let mut slot = self.lock();
		let file = self.loaded(&mut slot)?;
		let mut next = file.clone();
		let mut written = Vec::new();
		let mut added = Vec::new();
		let mut duplicates = Vec::new();
		let result = (|| -> Result<(), AppError> {
			for (name, skin) in parsed {
				let path = self.png_path(&skin.id)?;
				let existing = next.find(&skin.id).cloned();
				if existing.is_none() || !path.exists() {
					fs_util::write_atomic(&path, &skin.png).map_err(|err| storage(&path, err))?;
					if existing.is_none() {
						written.push(path);
					}
				}
				let texture = data_url(&skin.png);
				match existing {
					Some(stored) => duplicates.push(entry(&stored, texture)),
					None => {
						let stored = StoredSkin {
							id: skin.id,
							name,
							model: skin.model,
							legacy: skin.legacy,
							added_at: now_iso(),
						};
						added.push(entry(&stored, texture));
						next.skins.insert(0, stored);
					}
				}
			}
			if added.is_empty() {
				Ok(())
			} else {
				self.save(&next)
			}
		})();
		if let Err(err) = result {
			for path in written {
				let _ = std::fs::remove_file(path);
			}
			return Err(err);
		}
		*file = next;
		Ok(SkinsAdded {
			added,
			duplicates,
			rejected,
		})
	}

	pub fn update_skin(&self, id: &str, patch: SkinPatch) -> Result<SkinEntry, AppError> {
		let name = match patch.name.as_deref() {
			Some(name) => Some(
				png::clean_name(name)
					.ok_or_else(|| AppError::coded(ErrorCode::InvalidName, "skin"))?,
			),
			None => None,
		};
		let bytes = self.read_png(id)?;
		self.update(|file| {
			let skin = file
				.skins
				.iter_mut()
				.find(|skin| skin.id == id)
				.ok_or_else(not_found)?;
			if let Some(name) = name {
				skin.name = name;
			}
			if let Some(model) = patch.model {
				skin.model = model;
			}
			Ok(entry(skin, data_url(&bytes)))
		})
	}

	/// Файл — в корзину без замка (это долго), потом запись. Не записалось — скин без файла просто скрыт.
	pub fn remove(
		&self,
		id: &str,
		delete: impl FnOnce(&Path) -> io::Result<()>,
	) -> Result<(), AppError> {
		if self.snapshot()?.find(id).is_none() {
			return Err(not_found());
		}
		let path = self.png_path(id)?;
		match delete(&path) {
			Ok(()) => {}
			Err(err) if err.kind() == io::ErrorKind::NotFound => {}
			Err(err) => return Err(crate::content::installed::io_error(&path, &err)),
		}
		self.update(|file| {
			file.skins.retain(|skin| skin.id != id);
			file.keys.retain(|_, skin_id| skin_id != id);
			Ok(())
		})
	}

	/// Файл скина. Скина нет или файл пропал — `skin_not_found`.
	pub fn read_png(&self, id: &str) -> Result<Vec<u8>, AppError> {
		{
			let mut slot = self.lock();
			if self.loaded(&mut slot)?.find(id).is_none() {
				return Err(not_found());
			}
		}
		let path = self.png_path(id)?;
		match std::fs::read(&path) {
			Ok(bytes) => Ok(bytes),
			Err(err) if err.kind() == io::ErrorKind::NotFound => {
				log::warn!("файл скина пропал: {}", path.display());
				Err(not_found())
			}
			Err(err) => Err(storage(&path, err)),
		}
	}

	pub fn reveal_path(&self, id: &str) -> Result<PathBuf, AppError> {
		self.read_png(id)?;
		self.png_path(id)
	}
}

fn rejected_file(path: &str, rejection: Rejection) -> RejectedFile {
	let (reason, width, height) = match rejection {
		Rejection::NotPng => (RejectReason::NotPng, None, None),
		Rejection::BadSize { width, height } => (RejectReason::BadSize, Some(width), Some(height)),
		Rejection::TooLarge => (RejectReason::TooLarge, None, None),
		Rejection::Unreadable => (RejectReason::Unreadable, None, None),
	};
	RejectedFile {
		path: path.to_owned(),
		reason,
		width,
		height,
	}
}

#[cfg(test)]
mod tests {
	use super::super::png::test_util::skin;
	use super::*;

	struct Fixture {
		dir: tempfile::TempDir,
		lib: Library,
	}

	fn fixture() -> Fixture {
		let dir = tempfile::tempdir().unwrap();
		let lib = Library::new(dir.path().join("skins"));
		Fixture { dir, lib }
	}

	impl Fixture {
		fn file(&self, name: &str, bytes: &[u8]) -> String {
			let path = self.dir.path().join(name);
			std::fs::write(&path, bytes).unwrap();
			path.to_string_lossy().into_owned()
		}

		fn add(&self, name: &str, seed: u8) -> SkinEntry {
			let path = self.file(name, &skin(64, 64, seed));
			self.lib.add_files(&[path]).unwrap().added.remove(0)
		}

		fn index(&self) -> PathBuf {
			self.lib.index_path()
		}
	}

	#[test]
	fn add_order_duplicates_and_rejected() {
		let f = fixture();
		let knight = f.file("Рыцарь.png", &skin(64, 64, 1));
		let old = f.file("old.PNG", &skin(64, 32, 2));
		let same = f.file("Другое имя.png", &skin(64, 64, 1));
		let big = f.file("big.png", &skin(100, 100, 3));
		let square = f.file("square.png", &skin(128, 128, 3));
		let text = f.file("note.png", b"hello");
		let gone = f.dir.path().join("gone.png").display().to_string();

		let result = f
			.lib
			.add_files(&[knight, old, same, big, square, text, gone])
			.unwrap();

		assert_eq!(result.added.len(), 2);
		assert_eq!(result.added[0].name, "Рыцарь");
		assert!(!result.added[0].legacy);
		assert_eq!(result.added[1].name, "old");
		assert!(result.added[1].legacy);
		assert_eq!(result.added[1].model, SkinModel::Classic);
		assert!(
			result.added[0]
				.texture
				.starts_with("data:image/png;base64,")
		);
		// Дубликат — под прежним именем.
		assert_eq!(result.duplicates.len(), 1);
		assert_eq!(result.duplicates[0].name, "Рыцарь");

		let reasons: Vec<_> = result
			.rejected
			.iter()
			.map(|r| (r.reason, r.width, r.height))
			.collect();
		assert_eq!(
			reasons,
			[
				(RejectReason::BadSize, Some(100), Some(100)),
				(RejectReason::BadSize, Some(128), Some(128)),
				(RejectReason::NotPng, None, None),
				(RejectReason::Unreadable, None, None),
			]
		);

		// Новые сверху, пережили перезапуск.
		let names = |lib: &Library| -> Vec<String> {
			lib.skins()
				.unwrap()
				.iter()
				.map(|s| s.name.clone())
				.collect()
		};
		assert_eq!(names(&f.lib), ["old", "Рыцарь"]);
		let reloaded = Library::new(f.dir.path().join("skins"));
		assert_eq!(names(&reloaded), ["old", "Рыцарь"]);
		assert!(f.lib.png_path(&result.added[0].id).unwrap().is_file());
	}

	#[test]
	fn duplicate_restores_lost_file() {
		let f = fixture();
		let first = f.add("a.png", 1);
		std::fs::remove_file(f.lib.png_path(&first.id).unwrap()).unwrap();
		let path = f.file("b.png", &skin(64, 64, 1));
		let result = f.lib.add_files(&[path]).unwrap();
		assert_eq!(result.duplicates[0].id, first.id);
		assert!(f.lib.png_path(&first.id).unwrap().is_file());
	}

	#[test]
	fn update_name_and_model() {
		let f = fixture();
		let knight = f.add("a.png", 1);
		let patch = |name: Option<&str>, model| SkinPatch {
			name: name.map(str::to_owned),
			model,
		};
		let updated = f
			.lib
			.update_skin(&knight.id, patch(Some(" Маг "), Some(SkinModel::Slim)))
			.unwrap();
		assert_eq!(updated.name, "Маг");
		assert_eq!(updated.model, SkinModel::Slim);
		assert_eq!(updated.texture, knight.texture);

		let long = "я".repeat(49);
		let err = f
			.lib
			.update_skin(&knight.id, patch(Some(&long), None))
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::InvalidName);
		assert_eq!(
			f.lib
				.update_skin(&"0".repeat(40), patch(None, None))
				.unwrap_err()
				.code,
			ErrorCode::SkinNotFound
		);
		assert_eq!(
			f.lib
				.update_skin("../../evil", patch(None, None))
				.unwrap_err()
				.code,
			ErrorCode::SkinNotFound
		);
	}

	#[test]
	fn remove_busy_file_keeps_skin() {
		let f = fixture();
		let a = f.add("a.png", 1);
		let err = f
			.lib
			.remove(&a.id, |_| Err(io::Error::from_raw_os_error(32)))
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::FileInUse);
		assert_eq!(f.lib.skins().unwrap().len(), 1);
	}

	/// Кладёт настоящий скин в папку библиотеки под его настоящим id.
	fn place(f: &Fixture, seed: u8) -> String {
		let parsed = png::parse(&skin(64, 64, seed)).unwrap();
		std::fs::create_dir_all(&f.lib.dir).unwrap();
		std::fs::write(f.lib.png_path(&parsed.id).unwrap(), parsed.png).unwrap();
		parsed.id
	}

	#[test]
	fn keys_resolve_and_are_cleared_with_skin() {
		let f = fixture();
		let a = f.add("a.png", 1);
		let b = f.add("b.png", 2);
		f.lib.remember_key("key-a", &a.id).unwrap();
		f.lib.remember_key("../evil", &a.id).unwrap();
		f.lib.remember_key("key-none", &"0".repeat(40)).unwrap();

		// По ключу, по id картинки, ключ важнее.
		assert_eq!(f.lib.skin_id_for("key-a", None), Some(a.id.clone()));
		assert_eq!(f.lib.skin_id_for("key-a", Some(&b.id)), Some(a.id.clone()));
		assert_eq!(f.lib.skin_id_for("other", Some(&b.id)), Some(b.id.clone()));
		assert_eq!(f.lib.skin_id_for("other", Some(&"0".repeat(40))), None);
		assert_eq!(f.lib.skin_id_for("key-none", None), None);

		let saved: Value = serde_json::from_slice(&std::fs::read(f.index()).unwrap()).unwrap();
		assert_eq!(saved["version"], 2);
		assert_eq!(saved["keys"].as_object().unwrap().len(), 1);

		let mut deleted = Vec::new();
		f.lib
			.remove(&a.id, |path| {
				deleted.push(path.to_path_buf());
				std::fs::remove_file(path)
			})
			.unwrap();
		assert_eq!(deleted.len(), 1);
		assert_eq!(f.lib.skin_id_for("key-a", None), None);
		let saved: Value = serde_json::from_slice(&std::fs::read(f.index()).unwrap()).unwrap();
		assert!(saved["keys"].as_object().unwrap().is_empty());
		assert_eq!(f.lib.skins().unwrap().len(), 1);
		assert_eq!(
			f.lib.remove(&a.id, |_| Ok(())).unwrap_err().code,
			ErrorCode::SkinNotFound
		);
	}

	#[test]
	fn add_bytes_is_idempotent() {
		let f = fixture();
		let png = skin(64, 64, 8);
		let first = f.lib.add_bytes(&png, "Steve", Some("k")).unwrap();
		assert_eq!(first.name, "Steve");
		let again = f.lib.add_bytes(&png, "Другой", Some("k2")).unwrap();
		assert_eq!(again.id, first.id);
		assert_eq!(again.name, "Steve");
		assert_eq!(f.lib.skins().unwrap().len(), 1);
		assert_eq!(f.lib.skin_id_for("k2", None), Some(first.id.clone()));
		assert_eq!(f.lib.model_of(&first.id).unwrap(), SkinModel::Classic);
		// Не картинка скина.
		assert_eq!(
			f.lib.add_bytes(b"x", "a", None).unwrap_err().code,
			ErrorCode::SkinNotFound
		);
		// Плохое имя не мешает.
		let other = f.lib.add_bytes(&skin(64, 64, 9), "  ", None).unwrap();
		assert_eq!(other.name, "skin");
	}

	#[test]
	fn lost_file_hides_skin_until_it_returns() {
		let f = fixture();
		let a = f.add("a.png", 1);
		let b = f.add("b.png", 2);
		f.lib.remember_key("key-a", &a.id).unwrap();
		let path = f.lib.png_path(&a.id).unwrap();
		let bytes = std::fs::read(&path).unwrap();
		std::fs::remove_file(&path).unwrap();

		let skins = f.lib.skins().unwrap();
		assert_eq!(skins.len(), 1);
		assert_eq!(skins[0].id, b.id);
		assert_eq!(
			f.lib.read_png(&a.id).unwrap_err().code,
			ErrorCode::SkinNotFound
		);
		assert_eq!(f.lib.skin_id_for("key-a", Some(&a.id)), None);
		// После перезапуска то же, индекс не тронут.
		let reloaded = Library::new(f.dir.path().join("skins"));
		assert_eq!(reloaded.skins().unwrap().len(), 1);

		// Файл вернулся — вернулись и имя, и ключ.
		std::fs::write(&path, bytes).unwrap();
		for lib in [&f.lib, &reloaded] {
			let skins = lib.skins().unwrap();
			assert_eq!(skins.len(), 2);
			assert!(skins.iter().any(|skin| skin.name == "a"));
			assert_eq!(lib.skin_id_for("key-a", None), Some(a.id.clone()));
		}
	}

	#[test]
	fn removal_of_hidden_skin_works() {
		let f = fixture();
		let a = f.add("a.png", 1);
		std::fs::remove_file(f.lib.png_path(&a.id).unwrap()).unwrap();
		f.lib.remove(&a.id, |_| Ok(())).unwrap();
		let path = f.file("again.png", &skin(64, 64, 1));
		// Запись стёрта, поэтому это снова новый скин.
		assert_eq!(f.lib.add_files(&[path]).unwrap().added.len(), 1);
	}

	/// Файл версии 1, какой уже мог появиться у пользователя.
	#[test]
	fn version_1_keeps_skins_and_drops_worn() {
		let f = fixture();
		let id = place(&f, 1);
		let json = serde_json::json!({
			"version": 1,
			"skins": [{ "id": id, "name": "Рыцарь", "model": "slim", "legacy": false, "addedAt": "t" }],
			"worn": { "acc": { "skinId": id, "model": "slim", "wornAt": "t" } },
		});
		std::fs::write(f.index(), serde_json::to_vec(&json).unwrap()).unwrap();
		let saved =
			|| -> Value { serde_json::from_slice(&std::fs::read(f.index()).unwrap()).unwrap() };

		let skins = f.lib.skins().unwrap();
		assert_eq!(skins.len(), 1);
		assert_eq!(skins[0].name, "Рыцарь");
		assert_eq!(skins[0].model, SkinModel::Slim);
		// Чтение файл не меняет: он перепишется при первом изменении.
		assert_eq!(saved()["version"], 1);
		assert!(saved().get("worn").is_some());
		assert!(
			!std::fs::read_dir(&f.lib.dir)
				.unwrap()
				.flatten()
				.any(|e| e.file_name().to_string_lossy().contains(".bak-"))
		);

		let patch = SkinPatch {
			name: Some("Маг".to_owned()),
			model: None,
		};
		f.lib.update_skin(&id, patch).unwrap();
		assert_eq!(saved()["version"], 2);
		assert!(saved().get("worn").is_none());
		assert!(saved()["keys"].as_object().unwrap().is_empty());
		assert_eq!(saved()["skins"][0]["name"], "Маг");
		assert!(f.lib.png_path(&id).unwrap().is_file());
	}

	#[test]
	fn foreign_keys_are_dropped() {
		let f = fixture();
		let good = place(&f, 1);
		let json = serde_json::json!({
			"version": 2,
			"skins": [{ "id": good, "name": "Ok", "model": "slim", "addedAt": "t" }],
			"keys": {
				"fine": good,
				"../evil": good,
				"none": "0".repeat(40),
				"number": 5,
			},
		});
		std::fs::write(f.index(), serde_json::to_vec(&json).unwrap()).unwrap();
		assert_eq!(f.lib.skin_id_for("fine", None), Some(good.clone()));
		assert_eq!(f.lib.skin_id_for("../evil", None), None);
		assert_eq!(f.lib.skin_id_for("none", None), None);
		assert_eq!(f.lib.skin_id_for("number", None), None);
	}

	#[test]
	fn one_bad_record_keeps_the_rest() {
		let f = fixture();
		let good = place(&f, 1);
		let unknown_model = place(&f, 2);
		let no_date = place(&f, 3);
		let json = serde_json::json!({
			"version": 2,
			"skins": [
				{ "id": good, "name": "Рыцарь", "model": "slim", "addedAt": "t" },
				{ "id": unknown_model, "name": "Маг", "model": "wide", "addedAt": "t" },
				{ "id": no_date, "name": "Робот", "model": "slim" },
				"мусор",
				{ "name": "Без id" },
			],
			"keys": { "k": good },
		});
		let text = serde_json::to_vec(&json).unwrap();
		std::fs::write(f.index(), &text).unwrap();

		let skins = f.lib.skins().unwrap();
		let name_of = |id: &str| skins.iter().find(|s| s.id == id).unwrap().name.clone();
		assert_eq!(skins.len(), 3);
		assert_eq!(skins[0].id, good);
		assert_eq!(skins[0].model, SkinModel::Slim);
		// Файлы найдены, названия взяты из испорченных записей.
		assert_eq!(name_of(&unknown_model), "Маг");
		assert_eq!(name_of(&no_date), "Робот");
		assert_eq!(f.lib.skin_id_for("k", None), Some(good.clone()));

		// Прежний файл сохранён копией до перезаписи.
		let backup = std::fs::read_dir(&f.lib.dir)
			.unwrap()
			.flatten()
			.find(|e| e.file_name().to_string_lossy().contains(".bak-"))
			.unwrap();
		assert_eq!(std::fs::read(backup.path()).unwrap(), text);
		let saved: Value = serde_json::from_slice(&std::fs::read(f.index()).unwrap()).unwrap();
		assert_eq!(saved["skins"].as_array().unwrap().len(), 3);
	}

	#[test]
	fn failed_write_keeps_state_and_files() {
		let f = fixture();
		let a = f.add("a.png", 1);
		// Временный файл записи занят папкой: `write_atomic` падает.
		let tmp = f.lib.dir.join("library.json.tmp");
		std::fs::create_dir(&tmp).unwrap();

		let path = f.file("b.png", &skin(64, 64, 2));
		let err = f.lib.add_files(&[path]).unwrap_err();
		assert_eq!(err.code, ErrorCode::Storage);
		let pngs = std::fs::read_dir(&f.lib.dir)
			.unwrap()
			.flatten()
			.filter(|e| e.path().extension().is_some_and(|x| x == "png"))
			.count();
		assert_eq!(pngs, 1);

		assert_eq!(
			f.lib.remember_key("k", &a.id).unwrap_err().code,
			ErrorCode::Storage
		);
		std::fs::remove_dir(&tmp).unwrap();
		assert_eq!(f.lib.skin_id_for("k", None), None);
		assert_eq!(f.lib.skins().unwrap().len(), 1);
	}

	#[test]
	fn unreadable_index_is_storage_until_fixed() {
		let f = fixture();
		// Папка вместо файла: чтение падает не с «файла нет».
		std::fs::create_dir_all(f.index()).unwrap();
		std::fs::write(f.index().join("keep"), b"x").unwrap();

		assert_eq!(f.lib.skins().unwrap_err().code, ErrorCode::Storage);
		assert_eq!(
			f.lib.ensure_readable().unwrap_err().code,
			ErrorCode::Storage
		);
		assert_eq!(f.lib.skin_id_for("k", None), None);
		let path = f.file("a.png", &skin(64, 64, 1));
		assert_eq!(
			f.lib
				.add_files(std::slice::from_ref(&path))
				.unwrap_err()
				.code,
			ErrorCode::Storage
		);
		assert!(f.index().join("keep").exists());

		// «Повторить» без перезапуска.
		std::fs::remove_dir_all(f.index()).unwrap();
		assert!(f.lib.skins().unwrap().is_empty());
		assert_eq!(f.lib.add_files(&[path]).unwrap().added.len(), 1);
	}

	#[test]
	fn migrates_foreign_records() {
		let f = fixture();
		let good = "a".repeat(40);
		std::fs::create_dir_all(&f.lib.dir).unwrap();
		std::fs::write(f.lib.png_path(&good).unwrap(), skin(64, 64, 1)).unwrap();
		// Без `version`; чужой id с выходом из папки и повтор.
		let json = serde_json::json!({
			"skins": [
				{ "id": good, "name": "Ok", "model": "slim", "addedAt": "t" },
				{ "id": good, "name": "Повтор", "model": "slim", "addedAt": "t" },
				{ "id": "../../evil", "name": "Evil", "model": "slim", "addedAt": "t" },
			],
		});
		std::fs::write(f.index(), serde_json::to_vec(&json).unwrap()).unwrap();

		let skins = f.lib.skins().unwrap();
		assert_eq!(skins.len(), 1);
		assert_eq!(skins[0].name, "Ok");
		assert!(!skins[0].legacy);

		// Мусор убран с диска сразу, копия — рядом.
		let saved: Value = serde_json::from_slice(&std::fs::read(f.index()).unwrap()).unwrap();
		assert_eq!(saved["version"], 2);
		assert_eq!(saved["skins"].as_array().unwrap().len(), 1);
		assert!(saved.to_string().find("evil").is_none());
		assert!(saved["skins"][0].get("texture").is_none());
	}

	#[test]
	fn missing_index_is_recovered_from_files() {
		let f = fixture();
		let id = place(&f, 1);
		let legacy = png::parse(&skin(64, 32, 2)).unwrap();
		std::fs::write(f.lib.png_path(&legacy.id).unwrap(), &legacy.png).unwrap();

		// Не должны попасть: чужой хеш, не PNG, имя не из sha1, заглавные буквы, папка.
		std::fs::write(
			f.lib.dir.join(format!("{}.png", "a".repeat(40))),
			skin(64, 64, 3),
		)
		.unwrap();
		std::fs::write(f.lib.dir.join(format!("{}.png", "b".repeat(40))), b"text").unwrap();
		std::fs::write(f.lib.dir.join("my skin.png"), skin(64, 64, 4)).unwrap();
		let upper = png::parse(&skin(64, 64, 5)).unwrap();
		std::fs::write(
			f.lib.dir.join(format!("{}.png", upper.id.to_uppercase())),
			&upper.png,
		)
		.unwrap();
		std::fs::create_dir(f.lib.dir.join(format!("{}.png", "c".repeat(40)))).unwrap();

		let skins = f.lib.skins().unwrap();
		assert_eq!(skins.len(), 2);
		let knight = skins.iter().find(|skin| skin.id == id).unwrap();
		assert_eq!(knight.name, "skin");
		assert_eq!(knight.model, SkinModel::Classic);
		assert!(!knight.legacy);
		assert!(DateTime::parse_from_rfc3339(&knight.added_at).is_ok());
		assert!(skins.iter().any(|skin| skin.id == legacy.id && skin.legacy));
		// Чужие файлы на месте, индекс записан.
		assert!(f.lib.dir.join("my skin.png").exists());
		assert!(f.index().is_file());
		let reloaded = Library::new(f.dir.path().join("skins"));
		assert_eq!(reloaded.skins().unwrap().len(), 2);
	}

	#[test]
	fn empty_folder_writes_nothing() {
		let f = fixture();
		assert!(f.lib.skins().unwrap().is_empty());
		assert!(!f.lib.dir.exists());
	}

	#[test]
	fn corrupt_index_is_recovered_from_files() {
		let f = fixture();
		let id = place(&f, 1);
		std::fs::write(f.index(), b"{ broken").unwrap();

		let skins = f.lib.skins().unwrap();
		assert_eq!(skins.len(), 1);
		assert_eq!(skins[0].id, id);
		// Индекс снова целый, и новой копии при следующем чтении не будет.
		let saved: Value = serde_json::from_slice(&std::fs::read(f.index()).unwrap()).unwrap();
		assert_eq!(saved["version"], 2);
		let reloaded = Library::new(f.dir.path().join("skins"));
		assert_eq!(reloaded.skins().unwrap().len(), 1);
		let backups = std::fs::read_dir(&f.lib.dir)
			.unwrap()
			.flatten()
			.filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
			.count();
		assert_eq!(backups, 1);
	}

	#[test]
	fn bad_record_without_file_is_dropped() {
		let f = fixture();
		let good = place(&f, 1);
		let json = serde_json::json!({
			"skins": [
				{ "id": good, "name": "Рыцарь", "model": "slim", "addedAt": "t" },
				{ "id": "d".repeat(40), "name": "Нет файла", "model": "wide" },
			],
		});
		std::fs::write(f.index(), serde_json::to_vec(&json).unwrap()).unwrap();
		assert_eq!(f.lib.skins().unwrap().len(), 1);
	}

	#[test]
	fn corrupt_index_is_backed_up() {
		let f = fixture();
		std::fs::create_dir_all(f.lib.dir.clone()).unwrap();
		std::fs::write(f.index(), b"{ broken").unwrap();

		assert!(f.lib.skins().unwrap().is_empty());
		// Чинить нечего, но файл уже перезаписан целым: следующий запуск копии не сделает.
		let reloaded = Library::new(f.dir.path().join("skins"));
		assert!(reloaded.skins().unwrap().is_empty());
		f.add("a.png", 1);
		let backups: Vec<_> = std::fs::read_dir(&f.lib.dir)
			.unwrap()
			.flatten()
			.filter(|e| {
				e.file_name()
					.to_string_lossy()
					.starts_with("library.json.bak-")
			})
			.collect();
		assert_eq!(backups.len(), 1);
		assert_eq!(std::fs::read(backups[0].path()).unwrap(), b"{ broken");
	}
}
