//! Файлы «только с сайта»: пользователь качает их браузером, мы сверяем размер и sha1 и кладём
//! копию в сборку. Пока открыт диалог, следим за папкой «Загрузки» (ничего там не трогаем).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::install::PlannedItem;
use super::installed::{self, SourceRef};
use super::{ProjectType, SourceKind};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::launcher::Launcher;

pub const STATE_FILE: &str = "manual.json";
const STATE_VERSION: u32 = 1;
/// Событие: у ждущих файлов сборки сменилось состояние (найден, добавлен, другой файл, ошибка).
pub const EVENT_CHANGED: &str = "instance-manual-changed";
/// Пачка событий папки «Загрузки» обрабатывается через это время (браузер дописывает файл).
const SETTLE: Duration = Duration::from_millis(700);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManualState {
	#[default]
	Waiting,
	Added,
	/// Скачан другой файл (`got`): похожее имя, не тот sha1.
	Wrong,
	/// Не удалось положить в сборку (`error`).
	Failed,
}

/// Файл, который нужно скачать на сайте каталога.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualFile {
	pub source: SourceKind,
	pub project_id: String,
	pub file_id: String,
	pub title: String,
	#[serde(default)]
	pub icon_url: Option<String>,
	/// Точное имя нужного файла (браузер может сохранить его под другим).
	pub file: String,
	pub size: u64,
	#[serde(rename = "type")]
	pub project_type: ProjectType,
	/// Датапак — в этот мир.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub world: Option<String>,
	/// Страница файла на сайте (скачивание начинается сразу).
	pub page_url: String,
	/// Ключ старого файла (обновление): уходит в корзину, когда новый добавлен.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub replaces: Option<String>,
	pub sha1: String,
	#[serde(default)]
	pub version_number: String,
	#[serde(default)]
	pub author: Option<String>,
	/// Поставлен как зависимость (для `content.json`).
	#[serde(default)]
	pub dependency: bool,
	#[serde(default)]
	pub state: ManualState,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub got: Option<String>,
	#[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
	pub error: Option<AppError>,
}

impl ManualFile {
	/// Файл плана «только с сайта» → ждущий файл.
	pub fn from_planned(item: &PlannedItem) -> Self {
		Self {
			source: item.hit.source,
			project_id: item.hit.id.clone(),
			file_id: item.version_id.clone(),
			title: item.hit.title.clone(),
			icon_url: item.hit.icon_url.clone(),
			file: item.file.file_name.clone(),
			size: item.file.size,
			project_type: item.kind,
			world: item.world.clone(),
			page_url: item
				.page_url
				.clone()
				.unwrap_or_else(|| super::project_url(item.hit.source, &item.hit.id)),
			replaces: item.replaces.clone(),
			sha1: item.file.sha1.clone(),
			version_number: item.version_number.clone(),
			author: Some(item.hit.author.clone()).filter(|a| !a.is_empty()),
			dependency: item.dependency,
			state: ManualState::Waiting,
			got: None,
			error: None,
		}
	}

	/// Проверка файла от фронта или из `manual.json`: имя, тип, мир, sha1.
	pub fn validate(&self) -> Result<(), AppError> {
		let bad = |what: &str| AppError::coded(ErrorCode::InvalidValue, format!("manual: {what}"));
		if !fs_util::is_sha1_hex(&self.sha1) {
			return Err(bad("sha1"));
		}
		if !fs_util::is_safe_file_name(&self.file)
			|| self.file.ends_with(installed::DISABLED_SUFFIX)
		{
			return Err(bad("file"));
		}
		let ext_ok = self
			.file
			.rsplit_once('.')
			.is_some_and(|(_, e)| e.eq_ignore_ascii_case(self.project_type.extension()));
		if !ext_ok || self.project_type == ProjectType::Modpack {
			return Err(bad("type"));
		}
		if self.project_type != ProjectType::World {
			self.dest_key()?;
		}
		if let Some(old) = &self.replaces {
			installed::parse_key(old)?;
		}
		super::check_ref(&self.project_id)?;
		super::check_ref(&self.file_id)?;
		Ok(())
	}

	/// Куда положить (ключ индекса). У мира — нет.
	pub fn dest_key(&self) -> Result<String, AppError> {
		let world = self
			.world
			.as_deref()
			.filter(|_| self.project_type == ProjectType::Datapack);
		Ok(format!(
			"{}/{}",
			installed::folder_key(self.project_type, world)?,
			self.file
		))
	}

	fn same(&self, other: &Self) -> bool {
		self.source == other.source && self.file_id == other.file_id
	}
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
	#[serde(default)]
	version: u32,
	#[serde(default)]
	files: Vec<ManualFile>,
}

pub fn state_path(instance_dir: &Path) -> PathBuf {
	instance_dir.join(STATE_FILE)
}

/// Ждущие файлы сборки (неправильные записи пропускаются).
pub fn load(instance_dir: &Path) -> Vec<ManualFile> {
	let stored = fs_util::read_json_or_backup::<Stored>(&state_path(instance_dir))
		.ok()
		.flatten()
		.unwrap_or_default();
	stored
		.files
		.into_iter()
		.filter(|f| f.validate().is_ok())
		.map(|mut f| {
			f.state = ManualState::Waiting;
			f.got = None;
			f
		})
		.collect()
}

fn save(instance_dir: &Path, files: &[ManualFile]) -> Result<(), AppError> {
	let path = state_path(instance_dir);
	if files.is_empty() {
		return match std::fs::remove_file(&path) {
			Ok(()) => Ok(()),
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(e) => Err(AppError::storage(format!("{}: {e}", path.display()))),
		};
	}
	let stored = Stored {
		version: STATE_VERSION,
		files: files.to_vec(),
	};
	let json = serde_json::to_vec_pretty(&stored).map_err(|e| AppError::storage(e.to_string()))?;
	fs_util::write_atomic(&path, &json)
		.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))
}

/// Добавляет ждущие файлы (тот же файл каталога или файл с тем же назначением заменяется новым).
/// Вызывать под замком контента сборки.
pub fn add_waiting(instance_dir: &Path, new: &[ManualFile]) -> Result<Vec<ManualFile>, AppError> {
	let mut files = load(instance_dir);
	for f in new {
		if f.validate().is_err() {
			continue;
		}
		let dest = f.dest_key().ok();
		files.retain(|old| {
			!old.same(f)
				&& !(dest.is_some()
					&& old.project_type != ProjectType::World
					&& old.dest_key().ok() == dest)
		});
		let mut f = f.clone();
		f.state = ManualState::Waiting;
		f.got = None;
		f.error = None;
		files.push(f);
	}
	save(instance_dir, &files)?;
	Ok(files)
}

/// Убирает из ждущих файлы, которые уже лежат в сборке (их положили руками): по sha1 из индекса.
/// `true` — список изменился. Вызывать под замком контента сборки.
pub fn reconcile(instance_dir: &Path, present: &HashSet<String>) -> Result<bool, AppError> {
	let files = load(instance_dir);
	let left: Vec<ManualFile> = files
		.iter()
		.filter(|f| !present.contains(&f.sha1.to_ascii_lowercase()))
		.cloned()
		.collect();
	if left.len() == files.len() {
		return Ok(false);
	}
	save(instance_dir, &left)?;
	Ok(true)
}

fn remove_waiting(instance_dir: &Path, file: &ManualFile) -> Result<usize, AppError> {
	let mut files = load(instance_dir);
	files.retain(|f| !f.same(file));
	save(instance_dir, &files)?;
	Ok(files.len())
}

/// Имя без версии и расширения для «скачан другой файл»: `ae2-15.2.0.jar` → `ae2`.
fn stem(name: &str) -> String {
	let base = name.rsplit_once('.').map_or(name, |(b, _)| b);
	// Браузер дописывает « (1)».
	let base = base.split(" (").next().unwrap_or(base);
	base.split(['-', '_', '+', ' '])
		.take_while(|p| {
			!p.trim_start_matches(['v', 'V'])
				.starts_with(|c: char| c.is_ascii_digit())
		})
		.collect::<Vec<_>>()
		.join("-")
		.to_ascii_lowercase()
}

fn is_candidate(path: &Path) -> bool {
	path.extension()
		.and_then(|e| e.to_str())
		.is_some_and(|e| e.eq_ignore_ascii_case("jar") || e.eq_ignore_ascii_case("zip"))
}

/// Сеанс диалога «Скачайте вручную» у сборки.
#[derive(Default)]
struct Session {
	files: Vec<ManualFile>,
	/// Сейчас кладутся в сборку (`fileId`).
	placing: HashSet<String>,
	/// Держит слежение за «Загрузками», пока жив сеанс.
	_watcher: Option<notify::RecommendedWatcher>,
	/// Какие файлы «Загрузок» уже смотрели (путь → размер): повторное событие того же файла
	/// не пересчитывает sha1.
	seen: HashMap<PathBuf, u64>,
	/// Начало сеанса: «другой файл» — только у файлов, скачанных после него.
	started: Option<std::time::SystemTime>,
}

/// Сеансы по сборкам (`Launcher.manual`).
#[derive(Default)]
pub struct Sessions {
	inner: Mutex<HashMap<String, Session>>,
}

impl Sessions {
	fn with<T>(&self, id: &str, f: impl FnOnce(&mut Session) -> T) -> Option<T> {
		let mut map = self.inner.lock().ok()?;
		map.get_mut(id).map(f)
	}

	fn files(&self, id: &str) -> Option<Vec<ManualFile>> {
		self.with(id, |s| s.files.clone())
	}

	/// Все сеансы закрываются (слежение за «Загрузками» прекращается).
	pub fn forget_all(&self) {
		if let Ok(mut map) = self.inner.lock() {
			map.clear();
		}
	}

	/// Сборку удалили — сеанс закрывается.
	pub fn forget(&self, id: &str) {
		if let Ok(mut map) = self.inner.lock() {
			map.remove(id);
		}
	}
}

fn emit(app: &AppHandle, id: &str, files: &[ManualFile]) {
	#[derive(Clone, Serialize)]
	struct Changed<'a> {
		id: &'a str,
		files: &'a [ManualFile],
	}
	if let Err(err) = app.emit(EVENT_CHANGED, Changed { id, files }) {
		log::warn!("не удалось отправить {EVENT_CHANGED}: {err}");
	}
}

fn join_error(err: tokio::task::JoinError) -> AppError {
	AppError::unknown(err.to_string())
}

/// Ждущие файлы с диска после сверки с тем, что уже лежит в сборке.
async fn waiting(l: &Launcher, id: &str) -> Result<Vec<ManualFile>, AppError> {
	let _guard = l.content_locks.lock(id).await;
	let (index, _) = super::commands::sync_index(l, id).await?;
	let present: HashSet<String> = index.files.values().map(|e| e.sha1.clone()).collect();
	let dir = l.layout.instance_dir(id);
	if reconcile(&dir, &present)? && load(&dir).is_empty() {
		l.downloads.clear_manual(id);
	}
	Ok(load(&dir))
}

type L<'a> = State<'a, Arc<Launcher>>;

/// Ждущие файлы сборки.
#[tauri::command]
pub async fn instance_manual(l: L<'_>, id: String) -> Result<Vec<ManualFile>, AppError> {
	l.instances.get(&id).await?;
	if let Some(files) = l.manual.files(&id) {
		return Ok(files);
	}
	waiting(&l, &id).await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualSession {
	/// Папка «Загрузки», за которой следим; `null` — не нашли.
	pub path: Option<String>,
	/// Слежение работает.
	pub watching: bool,
	pub files: Vec<ManualFile>,
}

/// Начинает сеанс диалога и слежение за «Загрузками».
#[tauri::command]
pub async fn instance_manual_begin(
	app: AppHandle,
	l: L<'_>,
	id: String,
	files: Option<Vec<ManualFile>>,
) -> Result<ManualSession, AppError> {
	l.instances.get(&id).await?;
	let extra = files.unwrap_or_default();
	for f in &extra {
		f.validate()?;
	}
	let mut list = waiting(&l, &id).await?;
	for f in extra {
		if !list.iter().any(|x| x.same(&f)) {
			let mut f = f;
			f.state = ManualState::Waiting;
			list.push(f);
		}
	}
	let folder = dirs::download_dir().filter(|d| d.is_dir());
	let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
	let watcher = folder.as_ref().and_then(|dir| watch(dir, tx.clone()));
	let watching = watcher.is_some();
	if let Ok(mut map) = l.manual.inner.lock() {
		map.insert(
			id.clone(),
			Session {
				files: list.clone(),
				placing: HashSet::new(),
				_watcher: watcher,
				seen: HashMap::new(),
				started: Some(std::time::SystemTime::now()),
			},
		);
	}
	if let Some(dir) = &folder {
		// Уже лежащие файлы — после ответа, в той же очереди, что и события.
		if let Ok(entries) = std::fs::read_dir(dir) {
			for entry in entries.flatten() {
				let _ = tx.send(entry.path());
			}
		}
		spawn_listener(app, l.inner().clone(), id.clone(), rx);
	} else {
		log::warn!("папка «Загрузки» не найдена");
	}
	Ok(ManualSession {
		path: folder.map(|d| d.display().to_string()),
		watching,
		files: list,
	})
}

/// Заканчивает сеанс: слежение останавливается, файлы сеанса забываются.
#[tauri::command]
pub async fn instance_manual_end(l: L<'_>, id: String) -> Result<(), AppError> {
	l.manual.forget(&id);
	Ok(())
}

/// Заканчивает все сеансы (окно перезагрузилось — прежние сеансы некому закрыть).
#[tauri::command]
pub async fn instance_manual_end_all(l: L<'_>) -> Result<(), AppError> {
	l.manual.forget_all();
	Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualAdded {
	/// Все ждущие файлы сеанса (или сборки) с состояниями.
	pub files: Vec<ManualFile>,
	/// Пути, которые не подошли ни к одному ждущему файлу.
	pub rejected: Vec<String>,
}

/// Файлы из «Указать файлы…» и перетаскивания: сверка и добавление.
#[tauri::command]
pub async fn instance_manual_add(
	app: AppHandle,
	l: L<'_>,
	id: String,
	paths: Vec<String>,
) -> Result<ManualAdded, AppError> {
	add_paths(&app, l.inner(), &id, paths).await
}

/// Системное окно выбора файлов; выбранное — как в `instance_manual_add`.
#[tauri::command]
pub async fn instance_manual_pick(
	app: AppHandle,
	l: L<'_>,
	id: String,
	lang: Option<crate::lang::Lang>,
) -> Result<Option<ManualAdded>, AppError> {
	use crate::modpacks::commands::{dialog_title, with_parent};
	use tauri_plugin_dialog::DialogExt;
	l.instances.get(&id).await?;
	let lang = lang.unwrap_or_default();
	let mut dialog = app
		.dialog()
		.file()
		.set_title(dialog_title(
			lang,
			"Выберите скачанные файлы",
			"Choose downloaded files",
		))
		.add_filter(
			dialog_title(lang, "Файлы (*.jar, *.zip)", "Files (*.jar, *.zip)"),
			&["jar", "zip"],
		);
	if let Some(dir) = dirs::download_dir().filter(|d| d.is_dir()) {
		dialog = dialog.set_directory(dir);
	}
	let (tx, rx) = tokio::sync::oneshot::channel();
	with_parent(&app, dialog).pick_files(move |files| {
		let _ = tx.send(files);
	});
	let Some(files) = rx.await.ok().flatten() else {
		return Ok(None);
	};
	let paths: Vec<String> = files
		.into_iter()
		.filter_map(|f| f.into_path().ok())
		.map(|p| p.to_string_lossy().into_owned())
		.collect();
	add_paths(&app, l.inner(), &id, paths).await.map(Some)
}

async fn add_paths(
	app: &AppHandle,
	l: &Arc<Launcher>,
	id: &str,
	paths: Vec<String>,
) -> Result<ManualAdded, AppError> {
	let id = id.to_owned();
	let l = l.clone();
	l.instances.get(&id).await?;
	let had_session = l.manual.files(&id).is_some();
	if !had_session {
		let files = waiting(&l, &id).await?;
		if let Ok(mut map) = l.manual.inner.lock() {
			map.insert(
				id.clone(),
				Session {
					files,
					..Session::default()
				},
			);
		}
	}
	let mut rejected = Vec::new();
	for p in paths {
		let path = PathBuf::from(&p);
		if !path.is_absolute() || !path.is_file() || !check(app, &l, &id, &path, true).await {
			rejected.push(p);
		}
	}
	let files = l.manual.files(&id).unwrap_or_default();
	if !had_session {
		l.manual.forget(&id);
	}
	Ok(ManualAdded { files, rejected })
}

fn watch(
	dir: &Path,
	tx: tokio::sync::mpsc::UnboundedSender<PathBuf>,
) -> Option<notify::RecommendedWatcher> {
	use notify::{EventKind, RecursiveMode, Watcher};
	let handler = move |res: notify::Result<notify::Event>| {
		if let Ok(event) = res
			&& matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
		{
			for path in event.paths {
				let _ = tx.send(path);
			}
		}
	};
	match notify::recommended_watcher(handler) {
		Ok(mut w) => match w.watch(dir, RecursiveMode::NonRecursive) {
			Ok(()) => Some(w),
			Err(err) => {
				log::warn!("слежение за «Загрузками» не началось: {err}");
				None
			}
		},
		Err(err) => {
			log::warn!("слежение за «Загрузками» не началось: {err}");
			None
		}
	}
}

/// Обработка путей из «Загрузок» пачками, пока открыт сеанс.
fn spawn_listener(
	app: AppHandle,
	l: Arc<Launcher>,
	id: String,
	mut rx: tokio::sync::mpsc::UnboundedReceiver<PathBuf>,
) {
	tauri::async_runtime::spawn(async move {
		while let Some(first) = rx.recv().await {
			let mut batch = vec![first];
			tokio::time::sleep(SETTLE).await;
			while let Ok(p) = rx.try_recv() {
				batch.push(p);
			}
			batch.sort();
			batch.dedup();
			if l.manual.files(&id).is_none() {
				break;
			}
			for path in batch {
				if is_candidate(&path) && path.is_file() {
					check(&app, &l, &id, &path, false).await;
				}
			}
		}
	});
}

/// Подошёл к ждущему файлу — кладёт копию в сборку и отвечает `true`.
/// `explicit` — файл указал пользователь.
async fn check(app: &AppHandle, l: &Arc<Launcher>, id: &str, path: &Path, explicit: bool) -> bool {
	let Ok(meta) = std::fs::metadata(path) else {
		return false;
	};
	let size = meta.len();
	let name = path
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_default();
	let seen_before = l
		.manual
		.with(id, |s| {
			s.seen.insert(path.to_path_buf(), size) == Some(size)
		})
		.unwrap_or(true);
	if seen_before && !explicit {
		return false;
	}
	let Some(files) = l.manual.files(id) else {
		return false;
	};
	let open: Vec<&ManualFile> = files
		.iter()
		.filter(|f| f.state != ManualState::Added)
		.collect();
	// Уже добавленные тоже сверяются: тот же файл ещё раз — не «не подошёл».
	let same_size: Vec<&ManualFile> = files.iter().filter(|f| f.size == size).collect();
	let found = if same_size.is_empty() {
		None
	} else {
		let p = path.to_path_buf();
		match tokio::task::spawn_blocking(move || installed::sha1_file(&p)).await {
			Ok(Ok(sha1)) => same_size
				.iter()
				.find(|f| f.sha1.eq_ignore_ascii_case(&sha1))
				.map(|f| (*f).clone()),
			_ => None,
		}
	};
	if found
		.as_ref()
		.is_some_and(|f| f.state == ManualState::Added)
	{
		return true;
	}
	let Some(file) = found else {
		// Не тот файл, но похож по имени — «скачан другой файл».
		let wanted = stem(&name);
		let similar = open
			.iter()
			.find(|f| !wanted.is_empty() && stem(&f.file) == wanted)
			.map(|f| (*f).clone());
		let started = l.manual.with(id, |s| s.started).flatten();
		let fresh = match (meta.modified(), started) {
			(Ok(m), Some(s)) => m >= s,
			_ => false,
		};
		if let Some(f) = similar
			&& (explicit || fresh)
		{
			update(app, l, id, &f, |x| {
				x.state = ManualState::Wrong;
				x.got = Some(name.clone());
				x.error = Some(AppError::coded(
					ErrorCode::FingerprintMismatch,
					name.clone(),
				));
			});
		}
		return false;
	};
	let started = l
		.manual
		.with(id, |s| s.placing.insert(file.file_id.clone()))
		.unwrap_or(false);
	if !started {
		return true;
	}
	let result = place(l, id, &file, path).await;
	l.manual.with(id, |s| s.placing.remove(&file.file_id));
	let mut fitted = true;
	match result {
		Ok(()) => {
			log::info!("сборка {id}: добавлен скачанный вручную {}", file.file);
			update(app, l, id, &file, |x| {
				x.state = ManualState::Added;
				x.got = None;
				x.error = None;
			});
			crate::gamedata::watch::emit(app, super::EVENT_CHANGED, id);
			if file.project_type == ProjectType::World {
				crate::gamedata::watch::emit(app, "instance-worlds-changed", id);
			}
		}
		Err(err) => {
			log::warn!("сборка {id}: {} не добавлен: {:?}", file.file, err.code);
			let wrong = err.code == ErrorCode::FingerprintMismatch;
			fitted = !wrong;
			update(app, l, id, &file, |x| {
				if wrong {
					x.state = ManualState::Wrong;
					x.got = Some(name.clone());
				} else {
					x.state = ManualState::Failed;
				}
				x.error = Some(err.clone());
			});
		}
	}
	fitted
}

fn update(
	app: &AppHandle,
	l: &Launcher,
	id: &str,
	file: &ManualFile,
	change: impl FnOnce(&mut ManualFile),
) {
	let files = l.manual.with(id, |s| {
		if let Some(x) = s.files.iter_mut().find(|x| x.same(file)) {
			change(x);
		}
		s.files.clone()
	});
	if let Some(files) = files {
		emit(app, id, &files);
	}
}

/// Кладёт копию файла в сборку и записывает его источник в индекс; убирает из ждущих.
pub async fn place(l: &Launcher, id: &str, file: &ManualFile, src: &Path) -> Result<(), AppError> {
	let _guard = l.content_locks.lock(id).await;
	super::commands::ensure_exists(l, id)?;
	if l.downloads.has_active_pack(id) {
		return Err(AppError::coded(ErrorCode::InstanceBusy, id.to_owned()));
	}
	let game = l.layout.game_dir(id);
	let instance_dir = l.layout.instance_dir(id);
	let f = file.clone();
	let from = src.to_path_buf();
	if file.project_type == ProjectType::World {
		let dir = instance_dir.clone();
		tokio::task::spawn_blocking(move || {
			// Распаковываем проверенную копию: исходный файл в «Загрузках» могли подменить.
			let staging = dir.join(format!(
				"{}{:08x}",
				crate::instances::CONTENT_TMP_PREFIX,
				rand::random::<u32>()
			));
			std::fs::create_dir_all(&staging)
				.map_err(|e| AppError::storage(format!("{}: {e}", staging.display())))?;
			let copy = staging.join("world.zip");
			let folder = installed::copy_checked(&from, &copy, Some(&f.sha1)).and_then(|()| {
				super::worlds::install(
					&game.join("saves"),
					&copy,
					&f.title,
					super::worlds::Limits::DEFAULT,
				)
			});
			let _ = std::fs::remove_dir_all(&staging);
			let folder = folder?;
			let mut state = super::worlds::load(&dir);
			state.worlds.insert(
				folder,
				super::worlds::WorldSource {
					source: f.source,
					project_id: f.project_id.clone(),
					version_id: f.file_id.clone(),
				},
			);
			super::worlds::save(&dir, &state)
		})
		.await
		.map_err(join_error)??;
	} else {
		let key = file.dest_key()?;
		tokio::task::spawn_blocking(move || -> Result<(), AppError> {
			let dest = installed::disk_path(&game, &key, true)?;
			let mut disabled = installed::import_file_checked(&from, &dest, Some(&f.sha1))?;
			if let Some(old) = f.replaces.as_ref().filter(|o| **o != key) {
				let on = installed::disk_path(&game, old, true)?;
				let off = installed::disk_path(&game, old, false)?;
				let old_disabled = off.exists() && !on.exists();
				for p in [on, off] {
					if p.exists() {
						crate::trash::delete(&p).map_err(|e| installed::io_error(&p, &e))?;
					}
				}
				if old_disabled && !disabled {
					let off = installed::disk_path(&game, &key, false)?;
					std::fs::rename(&dest, &off).map_err(|e| installed::io_error(&dest, &e))?;
					disabled = true;
				}
			}
			let _ = disabled;
			Ok(())
		})
		.await
		.map_err(join_error)??;
		let (mut index, _) = super::commands::sync_index(l, id).await?;
		let key = file.dest_key()?;
		if let Some(entry) = index.files.get_mut(&key) {
			entry.source = Some(SourceRef {
				kind: file.source,
				project_id: file.project_id.clone(),
				version_id: file.file_id.clone(),
				title: Some(file.title.clone()),
				icon_url: file.icon_url.clone(),
				version_number: Some(file.version_number.clone()),
				author: file.author.clone(),
				slug: None,
			});
			entry.dependency = file.dependency;
			entry.update = None;
			entry.checked_at = None;
		}
		let path = installed::index_path(&instance_dir);
		let idx = index.clone();
		tokio::task::spawn_blocking(move || installed::save_index(&path, &idx))
			.await
			.map_err(join_error)??;
	}
	let dir = l.layout.instance_dir(id);
	let f = file.clone();
	let left = tokio::task::spawn_blocking(move || remove_waiting(&dir, &f))
		.await
		.map_err(join_error)??;
	if left == 0 {
		l.downloads.clear_manual(id);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn file(id: &str, name: &str) -> ManualFile {
		ManualFile {
			source: SourceKind::Curseforge,
			project_id: "1".into(),
			file_id: id.into(),
			title: "Mod".into(),
			icon_url: None,
			file: name.into(),
			size: 3,
			project_type: ProjectType::Mod,
			world: None,
			page_url: "https://www.curseforge.com/minecraft/mc-mods/m/download/1".into(),
			replaces: None,
			sha1: "a9993e364706816aba3e25717850c26c9cd0d89d".into(),
			version_number: "1".into(),
			author: None,
			dependency: true,
			state: ManualState::Waiting,
			got: None,
			error: None,
		}
	}

	#[test]
	fn stores_and_validates() {
		let dir = tempfile::tempdir().unwrap();
		assert!(load(dir.path()).is_empty());
		add_waiting(
			dir.path(),
			&[file("10", "a-1.0.jar"), file("11", "b-2.jar")],
		)
		.unwrap();
		// Тот же файл ещё раз и файл с тем же назначением — заменяют прежние.
		let list = add_waiting(
			dir.path(),
			&[file("10", "a-1.0.jar"), file("12", "b-2.jar")],
		)
		.unwrap();
		let ids: Vec<&str> = list.iter().map(|f| f.file_id.as_str()).collect();
		assert_eq!(ids, vec!["10", "12"]);
		// Небезопасные записи не попадают в список.
		let mut bad = file("13", "../evil.jar");
		assert!(bad.validate().is_err());
		bad.file = "ok.zip".into();
		assert!(bad.validate().is_err());
		bad.file = "ok.jar".into();
		bad.world = Some("..".into());
		bad.project_type = ProjectType::Datapack;
		assert!(bad.validate().is_err());
		add_waiting(dir.path(), &[bad]).unwrap();
		assert_eq!(load(dir.path()).len(), 2);

		let present = HashSet::from(["a9993e364706816aba3e25717850c26c9cd0d89d".to_owned()]);
		assert!(reconcile(dir.path(), &present).unwrap());
		assert!(load(dir.path()).is_empty());
		assert!(!state_path(dir.path()).exists());

		let json = serde_json::to_value(file("10", "a.jar")).unwrap();
		assert_eq!(json["type"], "mod");
		assert_eq!(json["state"], "waiting");
		assert!(json.get("error").is_none());
	}

	#[test]
	fn stems() {
		assert_eq!(stem("ae2-15.2.0.jar"), stem("ae2-15.2.1.jar"));
		assert_eq!(stem("ae2-15.2.1 (1).jar"), "ae2");
		assert_eq!(stem("Mekanism-1.20.1-10.4.8.jar"), "mekanism");
		assert!(is_candidate(Path::new("a.JAR")) && !is_candidate(Path::new("a.crdownload")));
	}
}
