//! Сборки: список, создание, изменение, удаление, копирование.

pub mod commands;
pub mod fresh;
pub mod install;
#[cfg(test)]
pub(crate) mod live_tests;
mod processors;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::meta::LoaderKind;
use crate::paths::Layout;
use crate::settings::{Memory, nullable};

pub const FILE_NAME: &str = "instance.json";
const CURRENT_VERSION: u32 = 1;
const NAME_MAX_CHARS: usize = 64;
const GROUP_MAX_CHARS: usize = 32;
/// Папки удаляемых сборок: сначала переименование (мгновенно), потом удаление.
const TRASH_PREFIX: &str = ".trash-";
/// Папка копии сборки, пока идёт копирование: недоделанная копия не попадёт в список после перезапуска.
const COPY_PREFIX: &str = ".copy-";

/// Обложка сборки. Порядок не менять: по нему выбирается обложка по умолчанию.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceCover {
	Meadow,
	Forest,
	Peaks,
	Ocean,
	Desert,
	Sakura,
	Village,
	Castle,
	Caves,
	Volcano,
	Cosmos,
}

impl InstanceCover {
	pub const ALL: [Self; 11] = [
		Self::Meadow,
		Self::Forest,
		Self::Peaks,
		Self::Ocean,
		Self::Desert,
		Self::Sakura,
		Self::Village,
		Self::Castle,
		Self::Caves,
		Self::Volcano,
		Self::Cosmos,
	];

	pub fn name(self) -> &'static str {
		match self {
			Self::Meadow => "meadow",
			Self::Forest => "forest",
			Self::Peaks => "peaks",
			Self::Ocean => "ocean",
			Self::Desert => "desert",
			Self::Sakura => "sakura",
			Self::Village => "village",
			Self::Castle => "castle",
			Self::Caves => "caves",
			Self::Volcano => "volcano",
			Self::Cosmos => "cosmos",
		}
	}

	pub fn from_name(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|c| c.name() == name)
	}

	/// Имя из команды: неизвестное → `invalid_value`.
	pub fn parse(name: &str) -> Result<Self, AppError> {
		Self::from_name(name)
			.ok_or_else(|| AppError::coded(ErrorCode::InvalidValue, format!("cover: {name}")))
	}

	/// Обложка по умолчанию для сборки: FNV-1a (32 бита) от байтов id по модулю 11, индекс в [`Self::ALL`].
	pub fn for_id(id: &str) -> Self {
		Self::ALL[fnv1a(id) as usize % Self::ALL.len()]
	}

	/// Обложка новой сборки без выбора: из наименее занятых другими сборками первая, начиная
	/// с [`Self::for_id`] и дальше по кругу по [`Self::ALL`].
	fn pick_for_new(id: &str, taken: &[Self]) -> Self {
		let count = |c: Self| taken.iter().filter(|t| **t == c).count();
		let least = Self::ALL.into_iter().map(count).min().unwrap_or(0);
		let start = Self::ALL
			.iter()
			.position(|c| *c == Self::for_id(id))
			.unwrap_or(0);
		(0..Self::ALL.len())
			.map(|i| Self::ALL[(start + i) % Self::ALL.len()])
			.find(|c| count(*c) == least)
			.unwrap_or_else(|| Self::for_id(id))
	}
}

fn fnv1a(text: &str) -> u32 {
	text.bytes().fold(0x811c_9dc5_u32, |hash, b| {
		(hash ^ u32::from(b)).wrapping_mul(0x0100_0193)
	})
}

/// Обложка из файла: нет поля, `null` или неизвестное имя → `None` (потом обложка по id).
fn stored_cover<'de, D: serde::Deserializer<'de>>(
	de: D,
) -> Result<Option<InstanceCover>, D::Error> {
	let value = Option::<serde_json::Value>::deserialize(de)?;
	Ok(value
		.as_ref()
		.and_then(serde_json::Value::as_str)
		.and_then(InstanceCover::from_name))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Loader {
	pub kind: LoaderKind,
	/// Версия загрузчика. `null` — рекомендуемая; при установке сюда записывается выбранная.
	#[serde(default)]
	pub version: Option<String>,
}

/// Размер окна игры.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
	pub width: u32,
	pub height: u32,
}

impl Resolution {
	pub fn is_valid(self) -> bool {
		(1..=16384).contains(&self.width) && (1..=16384).contains(&self.height)
	}
}

/// Окно игры: во весь экран (`"fullscreen"`) или своего размера (`{ width, height }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GameWindow {
	Fullscreen(FullscreenTag),
	Size(Resolution),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullscreenTag {
	Fullscreen,
}

impl GameWindow {
	pub fn size(self) -> Option<Resolution> {
		match self {
			Self::Size(r) => Some(r),
			Self::Fullscreen(_) => None,
		}
	}

	pub fn is_fullscreen(self) -> bool {
		matches!(self, Self::Fullscreen(_))
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
	NotInstalled,
	Installing,
	Installed,
	Failed,
}

/// Что установлено (нужно для запуска).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
	/// Id версии в `meta/versions`: `1.20.1` или `1.20.1-fabric-0.16.14`.
	pub version_id: String,
	pub java_major: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
	pub id: String,
	pub name: String,
	pub game_version: String,
	pub loader: Loader,
	/// Обложка. В файле может не быть (старые сборки) — тогда [`InstanceCover::for_id`], см. [`load_one`].
	#[serde(skip_deserializing, default = "placeholder_cover")]
	pub cover: InstanceCover,
	/// Группа (один уровень). `null` — без группы.
	#[serde(default)]
	pub group: Option<String>,
	pub created_at: String,
	#[serde(default)]
	pub last_played_at: Option<String>,
	/// Наиграно, секунд.
	#[serde(default)]
	pub playtime: u64,
	/// Память. `null` — из настроек лаунчера.
	#[serde(default)]
	pub memory: Option<Memory>,
	/// Своя Java. `null` — лаунчер выбирает сам.
	#[serde(default)]
	pub java_path: Option<String>,
	/// Дополнительные аргументы JVM одной строкой.
	#[serde(default)]
	pub jvm_args: Option<String>,
	/// Окно игры. `null` — размер по умолчанию, `options.txt` не трогается.
	#[serde(default)]
	pub resolution: Option<GameWindow>,
	pub install_state: InstallState,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub installed: Option<Installed>,
	/// Сборка поставлена из модпака и связана с ним (обновление, отвязка). `null` — обычная сборка.
	#[serde(default)]
	pub modpack: Option<crate::modpacks::ModpackLink>,
	/// Сборка перенесена из другого лаунчера (по этому «Уже перенесена»).
	#[serde(default)]
	pub imported_from: Option<crate::modpacks::launchers::ImportedFrom>,
	/// В установленном описании версии уязвимый `lwjgl-freetype`. Считается по описанию на диске,
	/// в `instance.json` не хранится.
	#[serde(default, skip_deserializing)]
	pub font_vulnerable: bool,
}

/// Заглушка до [`load_one`], который ставит обложку из файла или по id.
fn placeholder_cover() -> InstanceCover {
	InstanceCover::Meadow
}

#[derive(Debug, Serialize, Deserialize)]
struct InstanceFile {
	#[serde(default)]
	version: u32,
	/// Обложка читается здесь, а не в [`Instance`]: для обложки по умолчанию нужен id из имени папки.
	#[serde(default, skip_serializing, deserialize_with = "stored_cover")]
	cover: Option<InstanceCover>,
	#[serde(flatten)]
	instance: Instance,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstancePatch {
	#[serde(default)]
	pub name: Option<String>,
	#[serde(default)]
	pub game_version: Option<String>,
	#[serde(default)]
	pub loader: Option<Loader>,
	/// `null` — обложка по умолчанию ([`InstanceCover::for_id`]).
	#[serde(default, deserialize_with = "nullable")]
	pub cover: Option<Option<String>>,
	/// `null` или пустая строка — без группы.
	#[serde(default, deserialize_with = "nullable")]
	pub group: Option<Option<String>>,
	#[serde(default, deserialize_with = "nullable")]
	pub memory: Option<Option<Memory>>,
	#[serde(default, deserialize_with = "nullable")]
	pub java_path: Option<Option<String>>,
	#[serde(default, deserialize_with = "nullable")]
	pub jvm_args: Option<Option<String>>,
	#[serde(default, deserialize_with = "nullable")]
	pub resolution: Option<Option<GameWindow>>,
}

impl InstancePatch {
	/// Меняет версию игры или загрузчик: файлы надо ставить заново.
	pub fn changes_version(&self, current: &Instance) -> bool {
		self.game_version
			.as_ref()
			.is_some_and(|v| v != &current.game_version)
			|| self.loader.as_ref().is_some_and(|l| l != &current.loader)
	}
}

pub fn now_iso() -> String {
	Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn validate_name(name: &str) -> Result<String, AppError> {
	let name = name.trim();
	let count = name.chars().count();
	if count == 0 || count > NAME_MAX_CHARS || name.chars().any(char::is_control) {
		return Err(AppError::new(ErrorCode::InvalidName));
	}
	Ok(name.to_owned())
}

/// Группа: пробелы по краям обрезаются, повторные пробелы схлопываются, пусто → без группы.
/// Длиннее 32 символов или с управляющими символами — `invalid_name` (`details: "group"`).
pub fn normalize_group(group: Option<&str>) -> Result<Option<String>, AppError> {
	let Some(group) = group else {
		return Ok(None);
	};
	if group.chars().any(char::is_control) {
		return Err(AppError::coded(ErrorCode::InvalidName, "group"));
	}
	let group = group.split_whitespace().collect::<Vec<_>>().join(" ");
	if group.is_empty() {
		return Ok(None);
	}
	if group.chars().count() > GROUP_MAX_CHARS {
		return Err(AppError::coded(ErrorCode::InvalidName, "group"));
	}
	Ok(Some(group))
}

/// Группы сравниваются без учёта регистра.
fn same_group(a: &str, b: &str) -> bool {
	a == b || a.to_lowercase() == b.to_lowercase()
}

/// Написание группы: если такая (без учёта регистра) уже есть у сборок, кроме `skip`, — её написание.
fn canonical_group(
	items: &[Instance],
	group: Option<String>,
	skip: impl Fn(&Instance) -> bool,
) -> Option<String> {
	let group = group?;
	items
		.iter()
		.filter(|i| !skip(i))
		.filter_map(|i| i.group.as_deref())
		.find(|g| same_group(g, &group))
		.map(str::to_owned)
		.or(Some(group))
}

fn not_found(id: &str) -> AppError {
	AppError::coded(ErrorCode::InstanceNotFound, id.to_owned())
}

fn busy(id: &str) -> AppError {
	AppError::coded(ErrorCode::InstanceBusy, id.to_owned())
}

/// Что сейчас происходит со сборкой. Пока есть занятие, сборку нельзя ставить, удалять и копировать.
#[derive(Debug, Clone)]
enum Activity {
	/// Идёт установка; токен — её отмена.
	Install(CancellationToken),
	/// Игра запускается или запущена; токен — остановка процесса.
	Run(CancellationToken),
	/// Сборку копируют (`instance_duplicate`).
	Copy,
	/// Сборку удаляют (`instance_remove`): папка переименовывается.
	Remove,
	/// Меняются версия игры или загрузчик (`instance_update`).
	Update,
}

/// Id сборки — имя её папки: только `a–z 0–9`, чтобы путь был безопасным.
fn is_valid_id(id: &str) -> bool {
	!id.is_empty()
		&& id.len() <= 32
		&& id
			.bytes()
			.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn new_id(layout: &Layout) -> String {
	loop {
		let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_owned();
		if !layout.instance_dir(&id).exists() {
			return id;
		}
	}
}

fn save(layout: &Layout, instance: &Instance) -> Result<(), AppError> {
	write_instance(&layout.instance_dir(&instance.id).join(FILE_NAME), instance)
}

fn write_instance(path: &Path, instance: &Instance) -> Result<(), AppError> {
	let mut stored = instance.clone();
	// «Устанавливается» живёт только в памяти: после перезапуска такая установка считается прерванной.
	if stored.install_state == InstallState::Installing {
		stored.install_state = InstallState::NotInstalled;
	}
	let file = InstanceFile {
		version: CURRENT_VERSION,
		cover: None,
		instance: stored,
	};
	let mut value = serde_json::to_value(&file).map_err(|e| AppError::storage(e.to_string()))?;
	if let Some(map) = value.as_object_mut() {
		map.remove("fontVulnerable");
	}
	let json = serde_json::to_vec_pretty(&value).map_err(|e| AppError::storage(e.to_string()))?;
	fs_util::write_atomic(path, &json)
		.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))
}

/// Читает сборку из папки. `None` — папка не сборка или `instance.json` повреждён (тогда рядом его копия).
fn load_one(dir: &Path) -> Option<Instance> {
	let id = dir.file_name()?.to_str()?.to_owned();
	if !is_valid_id(&id) {
		return None;
	}
	let path = dir.join(FILE_NAME);
	let file = match fs_util::read_json_or_backup::<InstanceFile>(&path) {
		Ok(Some(file)) => file,
		Ok(None) => {
			if path.exists() {
				log::error!("сборка {id} пропущена: {FILE_NAME} повреждён");
			}
			return None;
		}
		Err(err) => {
			log::error!("сборка {id} пропущена: {}: {err}", path.display());
			return None;
		}
	};
	let mut instance = file.instance;
	instance.cover = file.cover.unwrap_or_else(|| InstanceCover::for_id(&id));
	if file.version > CURRENT_VERSION {
		log::warn!(
			"{} версии {} новее поддерживаемой",
			path.display(),
			file.version
		);
	}
	// Папка — источник истины для id (папку могли скопировать руками).
	instance.id = id;
	if instance.install_state == InstallState::Installing {
		instance.install_state = InstallState::Failed;
	}
	Some(instance)
}

/// Сборки в памяти; кладётся в состояние лаунчера.
pub struct Instances {
	layout: Layout,
	items: Mutex<Vec<Instance>>,
	/// Идущие установки и запущенные игры: id сборки → занятие.
	tasks: std::sync::Mutex<HashMap<String, Activity>>,
	/// Недоудалённые и недокопированные папки с прошлого запуска; удаляются в фоне.
	leftovers: std::sync::Mutex<Vec<PathBuf>>,
	font: fresh::FontCache,
}

impl Instances {
	pub fn load(layout: Layout) -> Self {
		let dir = layout.instances_dir();
		let mut items = Vec::new();
		let mut leftovers = Vec::new();
		if let Ok(entries) = std::fs::read_dir(&dir) {
			for entry in entries.flatten() {
				let path = entry.path();
				let name = entry.file_name().to_string_lossy().into_owned();
				if name.starts_with(TRASH_PREFIX) || name.starts_with(COPY_PREFIX) {
					leftovers.push(path);
					continue;
				}
				if path.is_dir()
					&& let Some(instance) = load_one(&path)
				{
					items.push(instance);
				}
			}
		}
		items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
		log::info!("сборок загружено: {}", items.len());
		Self {
			layout,
			items: Mutex::new(items),
			tasks: std::sync::Mutex::new(HashMap::new()),
			leftovers: std::sync::Mutex::new(leftovers),
			font: fresh::FontCache::default(),
		}
	}

	/// Остатки `.trash-*` и `.copy-*` с прошлого запуска (отдаются один раз).
	pub fn take_leftovers(&self) -> Vec<PathBuf> {
		self.leftovers
			.lock()
			.map(|mut l| std::mem::take(&mut *l))
			.unwrap_or_default()
	}

	fn activity(&self, id: &str) -> Option<Activity> {
		self.tasks.lock().ok().and_then(|t| t.get(id).cloned())
	}

	pub fn is_installing(&self, id: &str) -> bool {
		matches!(self.activity(id), Some(Activity::Install(_)))
	}

	fn overlay(&self, mut instance: Instance) -> Instance {
		if self.is_installing(&instance.id) {
			instance.install_state = InstallState::Installing;
		}
		instance.font_vulnerable = match (&instance.installed, instance.install_state) {
			(Some(installed), InstallState::Installed) => {
				self.font.vulnerable(&self.layout, &installed.version_id)
			}
			_ => false,
		};
		instance
	}

	pub async fn list(&self) -> Vec<Instance> {
		let items = self.items.lock().await.clone();
		items.into_iter().map(|i| self.overlay(i)).collect()
	}

	pub async fn get(&self, id: &str) -> Result<Instance, AppError> {
		let items = self.items.lock().await;
		let found = items.iter().find(|i| i.id == id).cloned();
		drop(items);
		found.map(|i| self.overlay(i)).ok_or_else(|| not_found(id))
	}

	pub fn game_dir(&self, id: &str) -> PathBuf {
		self.layout.game_dir(id)
	}

	pub async fn create(
		&self,
		name: &str,
		game_version: &str,
		loader: Loader,
		cover: Option<InstanceCover>,
		group: Option<&str>,
	) -> Result<Instance, AppError> {
		let name = validate_name(name)?;
		let group = normalize_group(group)?;
		let mut items = self.items.lock().await;
		let group = canonical_group(&items, group, |_| false);
		let id = new_id(&self.layout);
		let cover = cover.unwrap_or_else(|| {
			let taken: Vec<_> = items.iter().map(|i| i.cover).collect();
			InstanceCover::pick_for_new(&id, &taken)
		});
		let instance = Instance {
			id: id.clone(),
			name,
			game_version: game_version.to_owned(),
			loader,
			cover,
			group,
			created_at: now_iso(),
			last_played_at: None,
			playtime: 0,
			memory: None,
			java_path: None,
			jvm_args: None,
			resolution: None,
			install_state: InstallState::NotInstalled,
			installed: None,
			modpack: None,
			imported_from: None,
			font_vulnerable: false,
		};
		let game_dir = self.layout.game_dir(&id);
		std::fs::create_dir_all(&game_dir)
			.map_err(|e| AppError::storage(format!("{}: {e}", game_dir.display())))?;
		if let Err(err) = save(&self.layout, &instance) {
			let _ = std::fs::remove_dir_all(self.layout.instance_dir(&id));
			return Err(err);
		}
		items.push(instance.clone());
		Ok(instance)
	}

	/// Изменяет сборку: копия → проверка → запись на диск → замена в памяти.
	pub async fn modify(
		&self,
		id: &str,
		change: impl FnOnce(&mut Instance) -> Result<(), AppError>,
	) -> Result<Instance, AppError> {
		self.modify_with(id, |inst, _| change(inst)).await
	}

	/// Как [`Self::modify`], но изменение видит весь список (под тем же замком, что и запись).
	async fn modify_with(
		&self,
		id: &str,
		change: impl FnOnce(&mut Instance, &[Instance]) -> Result<(), AppError>,
	) -> Result<Instance, AppError> {
		let mut items = self.items.lock().await;
		let index = items
			.iter()
			.position(|i| i.id == id)
			.ok_or_else(|| not_found(id))?;
		let mut next = items[index].clone();
		change(&mut next, &items)?;
		next.id = id.to_owned();
		save(&self.layout, &next)?;
		items[index] = next.clone();
		drop(items);
		Ok(self.overlay(next))
	}

	pub async fn update(&self, id: &str, patch: InstancePatch) -> Result<Instance, AppError> {
		let group = patch
			.group
			.as_ref()
			.map(|g| normalize_group(g.as_deref()))
			.transpose()?;
		// Смена версии занимает сборку; остальное можно менять и во время игры.
		let mut _updating = None;
		// Под тем же замком, иначе «Моды» и «моды» одновременно дали бы две группы.
		self.modify_with(id, |inst, items| {
			if let Some(group) = group {
				inst.group = canonical_group(items, group, |i| i.id == id);
			}
			if patch.changes_version(inst) {
				_updating = Some(self.hold(id, Activity::Update)?);
				inst.install_state = InstallState::NotInstalled;
				inst.installed = None;
			}
			if let Some(name) = &patch.name {
				inst.name = validate_name(name)?;
			}
			if let Some(v) = &patch.game_version {
				inst.game_version = v.clone();
			}
			if let Some(l) = &patch.loader {
				inst.loader = l.clone();
			}
			if let Some(cover) = &patch.cover {
				inst.cover = match cover {
					Some(cover) => InstanceCover::parse(cover)?,
					None => InstanceCover::for_id(id),
				};
			}
			if let Some(memory) = patch.memory {
				inst.memory = memory.map(Memory::validate).transpose()?;
			}
			if let Some(path) = &patch.java_path {
				inst.java_path = path.clone().filter(|p| !p.trim().is_empty());
			}
			if let Some(args) = &patch.jvm_args {
				inst.jvm_args = args.clone().filter(|a| !a.trim().is_empty());
			}
			if let Some(res) = patch.resolution {
				if res
					.and_then(GameWindow::size)
					.is_some_and(|r| !r.is_valid())
				{
					return Err(AppError::coded(ErrorCode::InvalidValue, "resolution"));
				}
				inst.resolution = res;
			}
			Ok(())
		})
		.await
	}

	/// Переносит все сборки группы `from` в `to` (`None` — без группы). Ошибка записи одной сборки
	/// не останавливает остальные, возвращается первая.
	pub async fn set_group_of(
		&self,
		from: &str,
		to: Option<String>,
	) -> (Vec<Instance>, Option<AppError>) {
		let mut changed = Vec::new();
		let mut error = None;
		let Some(from) = normalize_group(Some(from)).ok().flatten() else {
			return (changed, error);
		};
		let in_from = |i: &Instance| i.group.as_deref().is_some_and(|g| same_group(g, &from));
		let mut items = self.items.lock().await;
		let to = canonical_group(&items, to, in_from);
		for slot in items.iter_mut().filter(|i| in_from(i)) {
			if slot.group == to {
				continue;
			}
			let mut next = slot.clone();
			next.group = to.clone();
			match save(&self.layout, &next) {
				Ok(()) => {
					*slot = next.clone();
					changed.push(next);
				}
				Err(err) => {
					log::warn!("группа сборки {}: {:?}", slot.id, err.details);
					error.get_or_insert(err);
				}
			}
		}
		drop(items);
		let changed = changed.into_iter().map(|i| self.overlay(i)).collect();
		(changed, error)
	}

	pub async fn remove(&self, id: &str) -> Result<(), AppError> {
		let _removing = self.hold(id, Activity::Remove)?;
		let mut items = self.items.lock().await;
		let index = items
			.iter()
			.position(|i| i.id == id)
			.ok_or_else(|| not_found(id))?;
		let dir = self.layout.instance_dir(id);
		let trash = self
			.layout
			.instances_dir()
			.join(format!("{TRASH_PREFIX}{id}"));
		// Наблюдатель папки или антивирус могут отпускать её с задержкой: несколько попыток.
		let mut attempt = 0;
		loop {
			match std::fs::rename(&dir, &trash) {
				Ok(()) => break,
				Err(e)
					if attempt < RENAME_RETRIES
						&& e.kind() == std::io::ErrorKind::PermissionDenied =>
				{
					attempt += 1;
					tokio::time::sleep(std::time::Duration::from_millis(100 * u64::from(attempt)))
						.await;
				}
				Err(e) => return Err(AppError::storage(format!("{}: {e}", dir.display()))),
			}
		}
		items.remove(index);
		drop(items);
		tokio::task::spawn_blocking(move || {
			if let Err(err) = std::fs::remove_dir_all(&trash) {
				log::warn!("не удалось удалить {}: {err}", trash.display());
			}
		});
		Ok(())
	}

	/// Копирует сборку в фоне; исходная на это время занята.
	pub async fn duplicate(&self, id: &str, name: &str) -> Result<Instance, AppError> {
		let name = validate_name(name)?;
		let _copying = self.hold(id, Activity::Copy)?;
		self.copy_instance(id, name).await
	}

	/// Копия сборки, которую уже занимает вызывающий (копия перед обновлением модпака).
	pub async fn duplicate_while_busy(&self, id: &str, name: &str) -> Result<Instance, AppError> {
		let name = validate_name(name)?;
		self.copy_instance(id, name).await
	}

	async fn copy_instance(&self, id: &str, name: String) -> Result<Instance, AppError> {
		let mut source = self.get(id).await?;
		if source.install_state == InstallState::Installing {
			source.install_state = if source.installed.is_some() {
				InstallState::Installed
			} else {
				InstallState::NotInstalled
			};
		}
		let new_id = new_id(&self.layout);
		let instance = Instance {
			id: new_id.clone(),
			name,
			created_at: now_iso(),
			last_played_at: None,
			playtime: 0,
			..source
		};
		let layout = self.layout.clone();
		let from = layout.instance_dir(id);
		let to = layout.instance_dir(&new_id);
		let temp = layout
			.instances_dir()
			.join(format!("{COPY_PREFIX}{new_id}"));
		let copy = instance.clone();
		tokio::task::spawn_blocking(move || -> Result<(), AppError> {
			let result = copy_dir(&from, &temp)
				.map_err(|e| AppError::storage(format!("{}: {e}", from.display())))
				.and_then(|()| {
					// `instance.json` пишется в копию, пока она ещё скрыта.
					write_instance(&temp.join(FILE_NAME), &copy)
				})
				.and_then(|()| {
					std::fs::rename(&temp, &to)
						.map_err(|e| AppError::storage(format!("{}: {e}", to.display())))
				});
			if result.is_err() {
				let _ = std::fs::remove_dir_all(&temp);
			}
			result
		})
		.await
		.map_err(|e| AppError::unknown(e.to_string()))??;
		self.items.lock().await.push(instance.clone());
		Ok(instance)
	}

	/// Сборка занята, пока жив guard.
	fn hold(&self, id: &str, activity: Activity) -> Result<HoldGuard<'_>, AppError> {
		let mut tasks = self
			.tasks
			.lock()
			.map_err(|_| AppError::unknown("tasks lock"))?;
		if tasks.contains_key(id) {
			return Err(busy(id));
		}
		let kind = std::mem::discriminant(&activity);
		tasks.insert(id.to_owned(), activity);
		Ok(HoldGuard {
			instances: self,
			id: id.to_owned(),
			kind,
		})
	}

	/// Начало установки: сборка становится занятой. Установка или игра уже идёт — `instance_busy`.
	pub fn begin_task(&self, id: &str) -> Result<CancellationToken, AppError> {
		let mut tasks = self
			.tasks
			.lock()
			.map_err(|_| AppError::unknown("tasks lock"))?;
		if tasks.contains_key(id) {
			return Err(busy(id));
		}
		let token = CancellationToken::new();
		tasks.insert(id.to_owned(), Activity::Install(token.clone()));
		Ok(token)
	}

	pub fn end_task(&self, id: &str) {
		if let Ok(mut tasks) = self.tasks.lock()
			&& matches!(tasks.get(id), Some(Activity::Install(_)))
		{
			tasks.remove(id);
		}
	}

	/// `true` — установка шла и отменена.
	pub fn cancel_task(&self, id: &str) -> bool {
		match self.activity(id) {
			Some(Activity::Install(token)) => {
				token.cancel();
				true
			}
			_ => false,
		}
	}

	/// Начало запуска игры. Игра уже запущена — `already_running`, идёт установка — `instance_busy`.
	/// Токен отменяется командой остановки.
	pub fn begin_run(&self, id: &str) -> Result<CancellationToken, AppError> {
		let mut tasks = self
			.tasks
			.lock()
			.map_err(|_| AppError::unknown("tasks lock"))?;
		match tasks.get(id) {
			Some(Activity::Run(_)) => {
				Err(AppError::coded(ErrorCode::AlreadyRunning, id.to_owned()))
			}
			Some(_) => Err(busy(id)),
			None => {
				let token = CancellationToken::new();
				tasks.insert(id.to_owned(), Activity::Run(token.clone()));
				Ok(token)
			}
		}
	}

	pub fn end_run(&self, id: &str) {
		if let Ok(mut tasks) = self.tasks.lock()
			&& matches!(tasks.get(id), Some(Activity::Run(_)))
		{
			tasks.remove(id);
		}
	}

	/// Игра запускается или запущена.
	pub fn is_running(&self, id: &str) -> bool {
		matches!(self.activity(id), Some(Activity::Run(_)))
	}

	/// Хоть одна игра запускается или запущена.
	pub fn any_running(&self) -> bool {
		self.tasks
			.lock()
			.map(|tasks| tasks.values().any(|a| matches!(a, Activity::Run(_))))
			.unwrap_or(false)
	}

	/// Останавливает все игры. Возвращает, сколько их было.
	pub fn kill_all(&self) -> usize {
		let Ok(tasks) = self.tasks.lock() else {
			return 0;
		};
		let mut count = 0;
		for activity in tasks.values() {
			if let Activity::Run(token) = activity {
				token.cancel();
				count += 1;
			}
		}
		count
	}

	/// Останавливает игру. `true` — игра шла.
	pub fn kill(&self, id: &str) -> bool {
		match self.activity(id) {
			Some(Activity::Run(token)) => {
				token.cancel();
				true
			}
			_ => false,
		}
	}
}

struct HoldGuard<'a> {
	instances: &'a Instances,
	id: String,
	kind: std::mem::Discriminant<Activity>,
}

impl Drop for HoldGuard<'_> {
	fn drop(&mut self) {
		if let Ok(mut tasks) = self.instances.tasks.lock()
			&& tasks
				.get(&self.id)
				.is_some_and(|a| std::mem::discriminant(a) == self.kind)
		{
			tasks.remove(&self.id);
		}
	}
}

/// Удаляет папки из [`Instances::take_leftovers`]. Долго на больших сборках — вызывать в фоне.
pub fn remove_leftovers(dirs: Vec<PathBuf>) {
	for path in dirs {
		match std::fs::remove_dir_all(&path) {
			Ok(()) => log::info!("дочищено: {}", path.display()),
			Err(err) => log::warn!("не удалось дочистить {}: {err}", path.display()),
		}
	}
}

/// Повторов переименования папки сборки при удалении.
const RENAME_RETRIES: u32 = 5;
/// Временные папки загрузки контента (`instances/<id>/.content-tmp-*`).
pub const CONTENT_TMP_PREFIX: &str = ".content-tmp-";

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
	std::fs::create_dir_all(to)?;
	for entry in std::fs::read_dir(from)? {
		let entry = entry?;
		if entry
			.file_name()
			.to_string_lossy()
			.starts_with(CONTENT_TMP_PREFIX)
		{
			continue;
		}
		let target = to.join(entry.file_name());
		let kind = entry.file_type()?;
		if kind.is_dir() {
			copy_dir(&entry.path(), &target)?;
		} else if kind.is_file() {
			std::fs::copy(entry.path(), &target)?;
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn copy_skips_content_tmp() {
		let dir = tempfile::tempdir().unwrap();
		let from = dir.path().join("a");
		std::fs::create_dir_all(from.join(".content-tmp-1")).unwrap();
		std::fs::write(from.join(".content-tmp-1/0"), b"x").unwrap();
		std::fs::create_dir_all(from.join("minecraft/mods")).unwrap();
		std::fs::write(from.join("minecraft/mods/m.jar"), b"m").unwrap();
		let to = dir.path().join("b");
		copy_dir(&from, &to).unwrap();
		assert!(to.join("minecraft/mods/m.jar").is_file());
		assert!(!to.join(".content-tmp-1").exists());
	}

	fn vanilla() -> Loader {
		Loader {
			kind: LoaderKind::Vanilla,
			version: None,
		}
	}

	#[tokio::test]
	async fn create_update_reload() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let created = instances
			.create("  Мой мир  ", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		assert_eq!(created.name, "Мой мир");
		assert!(layout.game_dir(&created.id).is_dir());
		assert!(is_valid_id(&created.id));

		let patch: InstancePatch = serde_json::from_value(serde_json::json!({
			"name": "Новое",
			"memory": { "minMb": 512, "maxMb": 2048 },
			"jvmArgs": "  ",
			"resolution": { "width": 1280, "height": 720 }
		}))
		.unwrap();
		let updated = instances.update(&created.id, patch).await.unwrap();
		assert_eq!(updated.name, "Новое");
		assert_eq!(updated.memory.unwrap().max_mb, 2048);
		assert!(updated.jvm_args.is_none());

		let text =
			std::fs::read_to_string(layout.instance_dir(&created.id).join(FILE_NAME)).unwrap();
		assert!(text.contains("\"gameVersion\": \"1.20.1\""));
		assert!(text.contains("\"installState\": \"not_installed\""));
		assert!(text.contains("\"version\": 1"));

		let reloaded = Instances::load(layout.clone());
		assert_eq!(reloaded.list().await, vec![updated.clone()]);

		let err = instances.update(
			&created.id,
			InstancePatch {
				name: Some(" ".into()),
				..Default::default()
			},
		);
		assert_eq!(err.await.unwrap_err().code, ErrorCode::InvalidName);
	}

	#[tokio::test]
	async fn version_change_resets_install() {
		let dir = tempfile::tempdir().unwrap();
		let instances = Instances::load(Layout::new(dir.path().to_path_buf()));
		let created = instances
			.create("a", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		instances
			.modify(&created.id, |i| {
				i.install_state = InstallState::Installed;
				i.installed = Some(Installed {
					version_id: "1.20.1".into(),
					java_major: 17,
				});
				Ok(())
			})
			.await
			.unwrap();
		let same = instances
			.update(
				&created.id,
				InstancePatch {
					game_version: Some("1.20.1".into()),
					..Default::default()
				},
			)
			.await
			.unwrap();
		assert_eq!(same.install_state, InstallState::Installed);
		let changed = instances
			.update(
				&created.id,
				InstancePatch {
					loader: Some(Loader {
						kind: LoaderKind::Fabric,
						version: None,
					}),
					..Default::default()
				},
			)
			.await
			.unwrap();
		assert_eq!(changed.install_state, InstallState::NotInstalled);
		assert!(changed.installed.is_none());

		let token = instances.begin_task(&created.id).unwrap();
		assert_eq!(
			instances.get(&created.id).await.unwrap().install_state,
			InstallState::Installing
		);
		assert_eq!(
			instances.begin_task(&created.id).unwrap_err().code,
			ErrorCode::InstanceBusy
		);
		assert_eq!(
			instances.remove(&created.id).await.unwrap_err().code,
			ErrorCode::InstanceBusy
		);
		assert!(instances.cancel_task(&created.id));
		assert!(token.is_cancelled());
		instances.end_task(&created.id);
		assert!(!instances.cancel_task(&created.id));
	}

	#[tokio::test]
	async fn running_game_blocks_changes() {
		let dir = tempfile::tempdir().unwrap();
		let instances = Instances::load(Layout::new(dir.path().to_path_buf()));
		let a = instances
			.create("a", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		let b = instances
			.create("b", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();

		let kill = instances.begin_run(&a.id).unwrap();
		// Разные сборки можно запускать одновременно.
		instances.begin_run(&b.id).unwrap();
		assert_eq!(
			instances.begin_run(&a.id).unwrap_err().code,
			ErrorCode::AlreadyRunning
		);
		for code in [
			instances.begin_task(&a.id).unwrap_err().code,
			instances.remove(&a.id).await.unwrap_err().code,
			instances.duplicate(&a.id, "c").await.unwrap_err().code,
		] {
			assert_eq!(code, ErrorCode::InstanceBusy);
		}
		let version_change = InstancePatch {
			game_version: Some("1.21".into()),
			..Default::default()
		};
		assert_eq!(
			instances
				.update(&a.id, version_change)
				.await
				.unwrap_err()
				.code,
			ErrorCode::InstanceBusy
		);
		// Запущенная игра — не «устанавливается».
		assert_eq!(
			instances.get(&a.id).await.unwrap().install_state,
			InstallState::NotInstalled
		);
		// Отмена установки не трогает игру, остановка — трогает.
		assert!(!instances.cancel_task(&a.id));
		assert!(instances.kill(&a.id));
		assert!(kill.is_cancelled());
		instances.end_task(&a.id);
		assert!(instances.begin_task(&a.id).is_err());
		instances.end_run(&a.id);
		let install = instances.begin_task(&a.id).unwrap();
		assert_eq!(
			instances.begin_run(&a.id).unwrap_err().code,
			ErrorCode::InstanceBusy
		);
		assert!(!instances.kill(&a.id));
		instances.end_run(&a.id);
		assert!(instances.cancel_task(&a.id));
		assert!(install.is_cancelled());
	}

	#[tokio::test]
	async fn covers() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let b = instances
			.create("b", "1.20.1", vanilla(), Some(InstanceCover::Cosmos), None)
			.await
			.unwrap();
		assert_eq!(b.cover, InstanceCover::Cosmos);
		assert_eq!(serde_json::to_value(&b).unwrap()["cover"], "cosmos");

		// Без выбора — разные обложки, пока все не заняты.
		let mut seen = vec![b.cover];
		for i in 0..InstanceCover::ALL.len() - 1 {
			let c = instances
				.create(&format!("n{i}"), "1.20.1", vanilla(), None, None)
				.await
				.unwrap();
			assert!(!seen.contains(&c.cover), "{:?} повторилась", c.cover);
			seen.push(c.cover);
		}

		let patch: InstancePatch =
			serde_json::from_value(serde_json::json!({ "cover": "castle" })).unwrap();
		assert_eq!(
			instances.update(&b.id, patch).await.unwrap().cover,
			InstanceCover::Castle
		);
		let reset: InstancePatch =
			serde_json::from_value(serde_json::json!({ "cover": null })).unwrap();
		assert_eq!(
			instances.update(&b.id, reset).await.unwrap().cover,
			InstanceCover::for_id(&b.id)
		);
		let bad: InstancePatch =
			serde_json::from_value(serde_json::json!({ "cover": "x" })).unwrap();
		assert_eq!(
			instances.update(&b.id, bad).await.unwrap_err().code,
			ErrorCode::InvalidValue
		);
		assert_eq!(
			InstanceCover::parse("dragon").unwrap_err().code,
			ErrorCode::InvalidValue
		);
	}

	#[test]
	fn default_cover_is_stable() {
		// Значения закреплены: обложки старых сборок не должны меняться.
		assert_eq!(fnv1a(""), 0x811c_9dc5);
		assert_eq!(fnv1a("a"), 0xe40c_292c);
		assert_eq!(
			InstanceCover::for_id("a"),
			InstanceCover::ALL[0xe40c_292c_usize % 11]
		);
		for id in ["0123456789ab", "deadbeef0000", "zzzzzzzzzzzz"] {
			assert_eq!(InstanceCover::for_id(id), InstanceCover::for_id(id));
		}
		let ids: Vec<String> = (0..200).map(|i| format!("{i:012}")).collect();
		let distinct: std::collections::HashSet<_> =
			ids.iter().map(|id| InstanceCover::for_id(id)).collect();
		assert_eq!(distinct.len(), InstanceCover::ALL.len());
	}

	#[tokio::test]
	async fn old_files_icon_to_cover() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let a = instances
			.create("a", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		let b = instances
			.create("b", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		let c = instances
			.create("c", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		let rewrite = |id: &str, cover: serde_json::Value| {
			let path = layout.instance_dir(id).join(FILE_NAME);
			let mut json: serde_json::Value =
				serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
			let obj = json.as_object_mut().unwrap();
			obj.remove("cover");
			if !cover.is_null() {
				obj.insert("cover".into(), cover);
			}
			// Файлы до обложек: поле `icon`.
			obj.insert("icon".into(), "rocket".into());
			std::fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
		};
		rewrite(&a.id, serde_json::Value::Null);
		rewrite(&b.id, "dragon".into());
		rewrite(&c.id, "ocean".into());

		for _ in 0..2 {
			let reloaded = Instances::load(layout.clone());
			assert_eq!(
				reloaded.get(&a.id).await.unwrap().cover,
				InstanceCover::for_id(&a.id)
			);
			assert_eq!(
				reloaded.get(&b.id).await.unwrap().cover,
				InstanceCover::for_id(&b.id)
			);
			assert_eq!(
				reloaded.get(&c.id).await.unwrap().cover,
				InstanceCover::Ocean
			);
		}

		// При следующей записи `icon` пропадает, `cover` появляется.
		let reloaded = Instances::load(layout.clone());
		let renamed = InstancePatch {
			name: Some("a2".into()),
			..Default::default()
		};
		let saved = reloaded.update(&a.id, renamed).await.unwrap();
		let text = std::fs::read_to_string(layout.instance_dir(&a.id).join(FILE_NAME)).unwrap();
		assert!(!text.contains("\"icon\""));
		assert_eq!(text.matches("\"cover\"").count(), 1);
		assert!(text.contains(&format!("\"cover\": \"{}\"", saved.cover.name())));
		assert_eq!(Instances::load(layout).get(&a.id).await.unwrap(), saved);
	}

	#[tokio::test]
	async fn corrupt_instance_is_skipped_and_backed_up() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let good = instances
			.create("good", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		let bad_dir = layout.instance_dir("broken01");
		std::fs::create_dir_all(&bad_dir).unwrap();
		std::fs::write(bad_dir.join(FILE_NAME), b"{ nope").unwrap();
		std::fs::create_dir_all(layout.instance_dir("Not-An-Id")).unwrap();

		let reloaded = Instances::load(layout.clone());
		let list = reloaded.list().await;
		assert_eq!(list.len(), 1);
		assert_eq!(list[0].id, good.id);
		let backups = std::fs::read_dir(&bad_dir)
			.unwrap()
			.flatten()
			.filter(|e| {
				e.file_name()
					.to_string_lossy()
					.starts_with("instance.json.bak-")
			})
			.count();
		assert_eq!(backups, 1);
		// Сам файл не трогаем: пользователь может его поправить.
		assert_eq!(std::fs::read(bad_dir.join(FILE_NAME)).unwrap(), b"{ nope");
	}

	#[tokio::test]
	async fn interrupted_install_loads_as_failed() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let created = instances
			.create("x", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		let path = layout.instance_dir(&created.id).join(FILE_NAME);
		let text = std::fs::read_to_string(&path)
			.unwrap()
			.replace("not_installed", "installing");
		std::fs::write(&path, text).unwrap();
		let reloaded = Instances::load(layout);
		assert_eq!(reloaded.list().await[0].install_state, InstallState::Failed);
	}

	#[tokio::test]
	async fn font_vulnerable_is_computed_not_stored() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let created = instances
			.create("x", "1.21.4", vanilla(), None, None)
			.await
			.unwrap();
		assert!(!created.font_vulnerable);
		let json =
			serde_json::json!({ "libraries": [ { "name": "org.lwjgl:lwjgl-freetype:3.3.3" } ] });
		fs_util::write_atomic(&layout.version_json("1.21.4"), json.to_string().as_bytes()).unwrap();
		// Не установлена — не уязвима, даже если описание есть.
		assert!(!instances.get(&created.id).await.unwrap().font_vulnerable);
		let installed = instances
			.modify(&created.id, |i| {
				i.install_state = InstallState::Installed;
				i.installed = Some(Installed {
					version_id: "1.21.4".into(),
					java_major: 21,
				});
				Ok(())
			})
			.await
			.unwrap();
		assert!(installed.font_vulnerable);
		let sent = serde_json::to_value(&installed).unwrap();
		assert_eq!(sent["fontVulnerable"], true);
		let text =
			std::fs::read_to_string(layout.instance_dir(&created.id).join(FILE_NAME)).unwrap();
		assert!(!text.contains("fontVulnerable"));
		let reloaded = Instances::load(layout.clone());
		assert!(reloaded.list().await[0].font_vulnerable);

		let fixed = serde_json::json!({ "libraries": [ { "name": "org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0" } ] });
		fs_util::write_atomic(&layout.version_json("1.21.4"), fixed.to_string().as_bytes())
			.unwrap();
		assert!(!reloaded.list().await[0].font_vulnerable);
	}

	#[tokio::test]
	async fn duplicate_and_remove() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let a = instances
			.create("a", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		let saves = layout.game_dir(&a.id).join("saves").join("World");
		std::fs::create_dir_all(&saves).unwrap();
		std::fs::write(saves.join("level.dat"), b"x").unwrap();

		let b = instances.duplicate(&a.id, "b").await.unwrap();
		assert_ne!(a.id, b.id);
		assert_eq!(b.name, "b");
		assert!(
			layout
				.game_dir(&b.id)
				.join("saves/World/level.dat")
				.is_file()
		);

		instances.remove(&a.id).await.unwrap();
		assert!(!layout.instance_dir(&a.id).exists());
		assert_eq!(instances.list().await.len(), 1);
		assert_eq!(
			instances.get(&a.id).await.unwrap_err().code,
			ErrorCode::InstanceNotFound
		);
		assert_eq!(Instances::load(layout).list().await.len(), 1);
	}

	#[tokio::test]
	async fn fullscreen_window() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let a = instances
			.create("a", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		let patch = |v: serde_json::Value| -> InstancePatch {
			serde_json::from_value(serde_json::json!({ "resolution": v })).unwrap()
		};

		let full = instances
			.update(&a.id, patch(serde_json::json!("fullscreen")))
			.await
			.unwrap();
		assert!(full.resolution.unwrap().is_fullscreen());
		assert_eq!(full.resolution.unwrap().size(), None);
		let json = serde_json::to_value(&full).unwrap();
		assert_eq!(json["resolution"], "fullscreen");
		assert_eq!(
			Instances::load(layout.clone()).get(&a.id).await.unwrap(),
			full
		);

		let sized = instances
			.update(
				&a.id,
				patch(serde_json::json!({ "width": 1280, "height": 720 })),
			)
			.await
			.unwrap();
		assert_eq!(
			sized.resolution.unwrap().size(),
			Some(Resolution {
				width: 1280,
				height: 720
			})
		);
		assert_eq!(
			instances
				.update(
					&a.id,
					patch(serde_json::json!({ "width": 0, "height": 720 }))
				)
				.await
				.unwrap_err()
				.code,
			ErrorCode::InvalidValue
		);
		let reset = instances
			.update(&a.id, patch(serde_json::Value::Null))
			.await
			.unwrap();
		assert!(reset.resolution.is_none());

		for bad in [
			serde_json::json!("window"),
			serde_json::json!({ "width": 5 }),
		] {
			assert!(
				serde_json::from_value::<InstancePatch>(serde_json::json!({ "resolution": bad }))
					.is_err()
			);
		}
	}

	#[tokio::test]
	async fn copying_blocks_source() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let a = instances
			.create("a", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();
		{
			let _copying = instances.hold(&a.id, Activity::Copy).unwrap();
			for code in [
				instances.begin_run(&a.id).unwrap_err().code,
				instances.begin_task(&a.id).unwrap_err().code,
				instances.remove(&a.id).await.unwrap_err().code,
				instances.duplicate(&a.id, "b").await.unwrap_err().code,
			] {
				assert_eq!(code, ErrorCode::InstanceBusy);
			}
			// Копирование — не установка и не игра.
			assert!(!instances.cancel_task(&a.id));
			assert!(!instances.kill(&a.id));
		}
		instances.end_run(&a.id);
		instances.begin_run(&a.id).unwrap();
		instances.end_run(&a.id);

		let b = instances.duplicate(&a.id, "b").await.unwrap();
		assert!(instances.begin_task(&a.id).is_ok());
		let names: Vec<_> = std::fs::read_dir(layout.instances_dir())
			.unwrap()
			.flatten()
			.map(|e| e.file_name().to_string_lossy().into_owned())
			.collect();
		assert_eq!(names.len(), 2);
		assert!(names.contains(&b.id));
		let stored = std::fs::read_to_string(layout.instance_dir(&b.id).join(FILE_NAME)).unwrap();
		assert!(stored.contains("\"name\": \"b\""));

		// Недоделанная копия (лаунчер закрыли во время копирования) удаляется при запуске.
		let leftover = layout.instances_dir().join(format!("{COPY_PREFIX}abc"));
		std::fs::create_dir_all(leftover.join("minecraft")).unwrap();
		std::fs::copy(
			layout.instance_dir(&a.id).join(FILE_NAME),
			leftover.join(FILE_NAME),
		)
		.unwrap();
		let reloaded = Instances::load(layout);
		assert_eq!(reloaded.list().await.len(), 2);
		let leftovers = reloaded.take_leftovers();
		assert_eq!(leftovers, vec![leftover.clone()]);
		assert!(reloaded.take_leftovers().is_empty());
		remove_leftovers(leftovers);
		assert!(!leftover.exists());
	}

	#[tokio::test]
	async fn remove_and_version_change_hold_instance() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let a = instances
			.create("a", "1.20.1", vanilla(), None, None)
			.await
			.unwrap();

		// Пока идёт удаление или смена версии, запуск и установка не начнутся (и наоборот).
		for activity in [Activity::Remove, Activity::Update] {
			let held = instances.hold(&a.id, activity).unwrap();
			for code in [
				instances.begin_run(&a.id).unwrap_err().code,
				instances.begin_task(&a.id).unwrap_err().code,
				instances.remove(&a.id).await.unwrap_err().code,
			] {
				assert_eq!(code, ErrorCode::InstanceBusy);
			}
			drop(held);
			assert!(instances.tasks.lock().unwrap().is_empty());
		}

		// Во время игры можно переименовать, но не сменить версию; занятость после — снова игры.
		instances.begin_run(&a.id).unwrap();
		let renamed = InstancePatch {
			name: Some("b".into()),
			game_version: Some("1.20.1".into()),
			..Default::default()
		};
		assert_eq!(instances.update(&a.id, renamed).await.unwrap().name, "b");
		let change = InstancePatch {
			game_version: Some("1.21".into()),
			..Default::default()
		};
		assert_eq!(
			instances
				.update(&a.id, change.clone())
				.await
				.unwrap_err()
				.code,
			ErrorCode::InstanceBusy
		);
		assert!(instances.kill(&a.id));
		instances.end_run(&a.id);

		// Смена версии без занятости проходит и освобождает сборку.
		let changed = instances.update(&a.id, change).await.unwrap();
		assert_eq!(changed.game_version, "1.21");
		assert!(instances.tasks.lock().unwrap().is_empty());
		// Неудачное удаление (папки нет) тоже освобождает сборку.
		std::fs::rename(layout.instance_dir(&a.id), dir.path().join("moved-away")).unwrap();
		assert_eq!(
			instances.remove(&a.id).await.unwrap_err().code,
			ErrorCode::Storage
		);
		assert!(instances.tasks.lock().unwrap().is_empty());
		instances.begin_run(&a.id).unwrap();
	}

	#[tokio::test]
	async fn groups() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let a = instances
			.create("a", "1.20.1", vanilla(), None, Some("  Мои   моды  "))
			.await
			.unwrap();
		assert_eq!(a.group.as_deref(), Some("Мои моды"));
		// Совпадение без учёта регистра — написание существующей группы.
		let b = instances
			.create("b", "1.20.1", vanilla(), None, Some("МОИ МОДЫ"))
			.await
			.unwrap();
		assert_eq!(b.group.as_deref(), Some("Мои моды"));
		let c = instances
			.create("c", "1.20.1", vanilla(), None, Some("   "))
			.await
			.unwrap();
		assert_eq!(c.group, None);
		assert_eq!(
			serde_json::to_value(&c).unwrap()["group"],
			serde_json::Value::Null
		);
		let long = "я".repeat(33);
		assert_eq!(
			instances
				.create("d", "1.20.1", vanilla(), None, Some(&long))
				.await
				.unwrap_err()
				.code,
			ErrorCode::InvalidName
		);
		assert!(normalize_group(Some(&"я".repeat(32))).unwrap().is_some());
		assert!(normalize_group(Some("a\nb")).is_err());

		let patch = |v: serde_json::Value| -> InstancePatch {
			serde_json::from_value(serde_json::json!({ "group": v })).unwrap()
		};
		let c = instances
			.update(&c.id, patch(serde_json::json!(" мои моды ")))
			.await
			.unwrap();
		assert_eq!(c.group.as_deref(), Some("Мои моды"));
		// Единственная сборка группы может сменить написание.
		instances
			.update(&c.id, patch(serde_json::json!("Ванилла")))
			.await
			.unwrap();
		assert_eq!(
			instances
				.update(&c.id, patch(serde_json::json!("ВАНИЛЛА")))
				.await
				.unwrap()
				.group
				.as_deref(),
			Some("ВАНИЛЛА")
		);
		// Без поля в патче группа не меняется.
		let renamed = instances
			.update(
				&c.id,
				InstancePatch {
					name: Some("c2".into()),
					..Default::default()
				},
			)
			.await
			.unwrap();
		assert_eq!(renamed.group.as_deref(), Some("ВАНИЛЛА"));
		assert_eq!(
			instances
				.update(&c.id, patch(serde_json::json!(long)))
				.await
				.unwrap_err()
				.code,
			ErrorCode::InvalidName
		);
		for empty in [serde_json::Value::Null, serde_json::json!("")] {
			instances
				.update(&c.id, patch(serde_json::json!("x")))
				.await
				.unwrap();
			assert_eq!(
				instances.update(&c.id, patch(empty)).await.unwrap().group,
				None
			);
		}

		// Копия сохраняет группу.
		let copy = instances.duplicate(&a.id, "a2").await.unwrap();
		assert_eq!(copy.group.as_deref(), Some("Мои моды"));

		// Переименование: все три сборки группы, во время игры тоже.
		instances.begin_run(&b.id).unwrap();
		let (changed, error) = instances
			.set_group_of(" мои МОДЫ ", Some("Fabric".into()))
			.await;
		assert_eq!(changed.len(), 3);
		assert!(error.is_none());
		let reloaded = Instances::load(layout.clone());
		for id in [&a.id, &b.id, &copy.id] {
			assert_eq!(
				reloaded.get(id).await.unwrap().group.as_deref(),
				Some("Fabric")
			);
		}
		assert_eq!(reloaded.get(&c.id).await.unwrap().group, None);
		assert!(instances.set_group_of("нет такой", None).await.0.is_empty());

		// Совпадение с другой группой — объединение с её написанием.
		instances
			.update(&c.id, patch(serde_json::json!("Сервер")))
			.await
			.unwrap();
		let (changed, _) = instances
			.set_group_of("fabric", Some("СЕРВЕР".into()))
			.await;
		assert_eq!(changed.len(), 3);
		assert!(changed.iter().all(|i| i.group.as_deref() == Some("Сервер")));

		// Не записавшаяся сборка остаётся в старой группе, остальные меняются, ответ — ошибка.
		let file = layout.instance_dir(&b.id).join(FILE_NAME);
		std::fs::remove_file(&file).unwrap();
		std::fs::create_dir_all(file.join("x")).unwrap();
		let (changed, error) = instances.set_group_of("Сервер", None).await;
		assert_eq!(changed.len(), 3);
		assert_eq!(error.unwrap().code, ErrorCode::Storage);
		assert_eq!(
			instances.get(&b.id).await.unwrap().group.as_deref(),
			Some("Сервер")
		);
		assert_eq!(instances.get(&a.id).await.unwrap().group, None);
	}

	#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
	async fn concurrent_group_updates_keep_one_spelling() {
		let dir = tempfile::tempdir().unwrap();
		let instances = std::sync::Arc::new(Instances::load(Layout::new(dir.path().to_path_buf())));
		for round in 0..20 {
			let mut ids = Vec::new();
			for i in 0..4 {
				let created = instances
					.create(&format!("r{round}-{i}"), "1.20.1", vanilla(), None, None)
					.await
					.unwrap();
				ids.push(created.id);
			}
			let group = format!("Группа{round}");
			let tasks: Vec<_> = ids
				.iter()
				.enumerate()
				.map(|(i, id)| {
					let instances = instances.clone();
					let id = id.clone();
					let name = if i % 2 == 0 {
						group.clone()
					} else {
						group.to_uppercase()
					};
					tokio::spawn(async move {
						let patch: InstancePatch =
							serde_json::from_value(serde_json::json!({ "group": name })).unwrap();
						instances.update(&id, patch).await.unwrap();
					})
				})
				.collect();
			for task in tasks {
				task.await.unwrap();
			}
			let mut spellings = std::collections::HashSet::new();
			for id in &ids {
				spellings.insert(instances.get(id).await.unwrap().group.unwrap());
			}
			assert_eq!(spellings.len(), 1, "{spellings:?}");
		}
	}

	#[tokio::test]
	async fn old_files_without_group() {
		let dir = tempfile::tempdir().unwrap();
		let layout = Layout::new(dir.path().to_path_buf());
		let instances = Instances::load(layout.clone());
		let a = instances
			.create("a", "1.20.1", vanilla(), None, Some("x"))
			.await
			.unwrap();
		let path = layout.instance_dir(&a.id).join(FILE_NAME);
		let mut json: serde_json::Value =
			serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
		json.as_object_mut().unwrap().remove("group");
		std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
		assert_eq!(
			Instances::load(layout).get(&a.id).await.unwrap().group,
			None
		);
	}

	#[tokio::test]
	async fn duplicate_missing_instance() {
		let dir = tempfile::tempdir().unwrap();
		let instances = Instances::load(Layout::new(dir.path().to_path_buf()));
		assert_eq!(
			instances.duplicate("nope", "b").await.unwrap_err().code,
			ErrorCode::InstanceNotFound
		);
		// Неудачная попытка не оставляет сборку «занятой».
		assert!(instances.tasks.lock().unwrap().is_empty());
	}
}
