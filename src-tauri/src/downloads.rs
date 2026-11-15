//! Список загрузок для панели «Загрузки». Только в памяти, до [`HISTORY_LIMIT`] завершённых.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::content::install::{ContentRequest, ContentTask};
use crate::download::Snapshot;
use crate::error::AppError;
use crate::instances::install::{InstallMode, InstallStage};
use crate::modpacks::launchers::LauncherKind;
use crate::modpacks::{PackAction, PackRequest, PackTask};

pub const EVENT_CHANGED: &str = "downloads-changed";
pub const EVENT_PROGRESS: &str = "download-progress";
/// Сколько завершённых задач (готово, ошибка) держать в списке.
pub const HISTORY_LIMIT: usize = 20;
/// Окно сглаживания скорости; раньше этого после начала скачивания время не оцениваем.
const SPEED_WINDOW: Duration = Duration::from_secs(3);
/// Прогресс одной задачи отправляется не чаще этого. Смена состава или состояния — сразу.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Пока что-то качается, прогресс отправляется хотя бы так часто (скорость падает, если загрузка встала).
const IDLE_TICK: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadKind {
	InstanceInstall,
	Java,
	Content,
	/// Установка или обновление модпака.
	Modpack,
	/// Импорт сборки из другого лаунчера.
	Import,
	/// Экспорт сборки в `.mrpack`.
	Export,
}

/// Режим задачи: у установки сборки — режим установки, у модпака — установка или обновление.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadMode {
	Install,
	Reinstall,
	Repair,
	Update,
}

impl From<InstallMode> for DownloadMode {
	fn from(mode: InstallMode) -> Self {
		match mode {
			InstallMode::Install => Self::Install,
			InstallMode::Reinstall => Self::Reinstall,
			InstallMode::Repair => Self::Repair,
		}
	}
}

/// Файл, который не скачался или не скопировался (задача всё равно `finished`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedFile {
	pub path: String,
	pub error: AppError,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Warnings {
	pub failed: Vec<FailedFile>,
	/// Файлы «только с сайта», которые ещё ждут ручной загрузки.
	pub manual: Vec<crate::content::manual::ManualFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
	Queued,
	Running,
	Finished,
	Failed,
}

impl DownloadState {
	pub fn is_done(self) -> bool {
		matches!(self, Self::Finished | Self::Failed)
	}
}

/// Что качается; по этому же задача повторяется (`download_retry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
	Instance {
		id: String,
		mode: InstallMode,
	},
	Java {
		major: u32,
	},
	/// Установка или обновление контента сборки; `info` — для показа в списке.
	Content {
		instance_id: String,
		request: ContentRequest,
		info: ContentTask,
	},
	/// Модпак (установка, обновление), импорт или экспорт сборки `instance_id`.
	Pack {
		instance_id: String,
		request: PackRequest,
		info: PackTask,
	},
}

impl Target {
	fn same(&self, other: &Self) -> bool {
		match (self, other) {
			(Self::Instance { id: a, .. }, Self::Instance { id: b, .. }) => a == b,
			(Self::Java { major: a }, Self::Java { major: b }) => a == b,
			(
				Self::Content {
					instance_id: a,
					info: x,
					..
				},
				Self::Content {
					instance_id: b,
					info: y,
					..
				},
			) => a == b && x.action == y.action && x.project_id == y.project_id,
			(
				Self::Pack {
					instance_id: a,
					info: x,
					..
				},
				Self::Pack {
					instance_id: b,
					info: y,
					..
				},
			) => a == b && x.kind() == y.kind(),
			_ => false,
		}
	}
}

/// Задача в списке загрузок.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
	pub id: String,
	pub kind: DownloadKind,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub instance_id: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mode: Option<DownloadMode>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub java_major: Option<u32>,
	/// `content`: что ставится.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<ContentTask>,
	/// `modpack`, `import`, `export`: название (модпака, сборки, файла).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub icon_url: Option<String>,
	/// `import`: из какого лаунчера.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub launcher: Option<LauncherKind>,
	/// `export`: готовый файл.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_path: Option<String>,
	/// Готово, но часть файлов не скачалась или не скопировалась; `download_retry` докачает их.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub warnings: Option<Warnings>,
	pub state: DownloadState,
	/// Этап установки сборки; у `java` — `downloading`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stage: Option<InstallStage>,
	/// 0–100 за всю задачу; `null` — пока неизвестен.
	pub percent: Option<f64>,
	/// Скачано за всю задачу (без файлов, которые уже были).
	pub bytes_done: u64,
	/// Сколько надо скачать за всю задачу (без уже скачанного); `null` — ещё неизвестно.
	pub bytes_total: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub steps_done: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub steps_total: Option<u64>,
	/// `content`: сколько файлов скачано и всего.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub files_done: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub files_total: Option<u64>,
	/// Байт/с за последние 3 с; `null` — сейчас ничего не качается.
	pub speed: Option<u64>,
	pub eta_secs: Option<u64>,
	/// Unix, мс.
	pub queued_at: u64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub started_at: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub finished_at: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<AppError>,
}

fn now_ms() -> u64 {
	u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// Скачано и надо скачать за этап, без файлов, которые уже были.
fn net(snap: &Snapshot) -> (u64, u64) {
	(
		snap.bytes_done.saturating_sub(snap.bytes_skipped),
		snap.bytes_total.saturating_sub(snap.bytes_skipped),
	)
}

struct Entry {
	task: Download,
	target: Target,
	cancel: CancellationToken,
	/// Текущий этап и его счётчики.
	stage: Option<InstallStage>,
	snap: Snapshot,
	/// Сумма прошлых этапов: скачано, надо скачать, получено из сети.
	base: (u64, u64, u64),
	/// (время, получено из сети за всю задачу) — для скорости.
	samples: VecDeque<(Instant, u64)>,
	/// Когда пошли первые байты из сети.
	receiving_since: Option<Instant>,
}

impl Entry {
	fn received(&self) -> u64 {
		self.base.2 + self.snap.bytes_received
	}

	fn prune(&mut self, now: Instant) {
		// Оставляем одну точку старше окна — от неё считается скорость.
		while self.samples.len() > 1
			&& self
				.samples
				.get(1)
				.is_some_and(|(t, _)| now.duration_since(*t) >= SPEED_WINDOW)
		{
			self.samples.pop_front();
		}
	}

	fn speed(&self, now: Instant) -> Option<u64> {
		let (first, last) = (self.samples.front()?, self.samples.back()?);
		let bytes = last.1.saturating_sub(first.1);
		if bytes == 0 || now.duration_since(last.0) >= SPEED_WINDOW {
			return None;
		}
		let elapsed = now.duration_since(first.0).max(Duration::from_secs(1));
		Some((bytes as f64 / elapsed.as_secs_f64()) as u64)
	}

	fn view(&mut self, now: Instant) -> Download {
		let mut task = self.task.clone();
		if task.state != DownloadState::Running {
			return task;
		}
		let (done, total) = net(&self.snap);
		task.bytes_done = self.base.0 + done;
		let total = self.base.1 + total;
		task.bytes_total = (total > 0).then_some(total.max(task.bytes_done));
		if self.stage == Some(InstallStage::Processors) {
			task.steps_done = Some(self.snap.files_done);
			task.steps_total = Some(self.snap.files_total);
		}
		// Файлы: у контента — всегда, у модпака, импорта и экспорта — на этапах с файлами.
		if task.kind == DownloadKind::Content
			|| matches!(
				self.stage,
				Some(InstallStage::ModpackFiles | InstallStage::Copying | InstallStage::Packing)
			) {
			task.files_done = Some(self.snap.files_done);
			task.files_total = Some(self.snap.files_total);
		}
		self.prune(now);
		task.speed = self.speed(now);
		let settled = self
			.receiving_since
			.is_some_and(|t| now.duration_since(t) >= SPEED_WINDOW);
		task.eta_secs = match (task.bytes_total, task.speed) {
			(Some(total), Some(speed)) if settled && speed > 0 => {
				Some(total.saturating_sub(task.bytes_done).div_ceil(speed))
			}
			_ => None,
		};
		task
	}
}

/// Что отправить в интерфейс.
#[derive(Debug, Clone)]
pub enum Emit {
	/// `downloads-changed`: весь список.
	List(Vec<Download>),
	/// `download-progress`: одна задача.
	Progress(Box<Download>),
}

#[derive(Default)]
struct Inner {
	entries: Vec<Entry>,
	/// Изменился состав или состояние — отправить весь список.
	list_dirty: bool,
	/// Изменился прогресс этих задач.
	dirty: HashSet<String>,
}

#[derive(Default)]
pub struct Downloads {
	inner: Mutex<Inner>,
	seq: AtomicU64,
	changed: Notify,
	urgent: Notify,
}

impl Downloads {
	fn lock(&self) -> MutexGuard<'_, Inner> {
		self.inner.lock().unwrap_or_else(|e| e.into_inner())
	}

	fn notify(&self, urgent: bool) {
		self.changed.notify_one();
		if urgent {
			self.urgent.notify_one();
		}
	}

	/// Новая задача (в очереди). Завершённые задачи того же объекта (прошлая попытка) убираются.
	pub fn begin(&self, target: Target, cancel: CancellationToken) -> String {
		let n = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
		let (kind, prefix) = match &target {
			Target::Instance { .. } => (DownloadKind::InstanceInstall, "install"),
			Target::Java { .. } => (DownloadKind::Java, "java"),
			Target::Content { .. } => (DownloadKind::Content, "content"),
			Target::Pack { info, .. } => match info.kind() {
				DownloadKind::Import => (DownloadKind::Import, "import"),
				DownloadKind::Export => (DownloadKind::Export, "export"),
				_ => (DownloadKind::Modpack, "modpack"),
			},
		};
		let id = format!("{prefix}-{n}");
		let (instance_id, mode, java_major, stage) = match &target {
			Target::Instance { id, mode } => (
				Some(id.clone()),
				Some((*mode).into()),
				None,
				InstallStage::Queued,
			),
			Target::Java { major } => (None, None, Some(*major), InstallStage::Downloading),
			Target::Content { instance_id, .. } => (
				Some(instance_id.clone()),
				None,
				None,
				InstallStage::Downloading,
			),
			Target::Pack {
				instance_id, info, ..
			} => {
				let mode = match info.action {
					PackAction::Install => Some(DownloadMode::Install),
					PackAction::Update => Some(DownloadMode::Update),
					PackAction::Import | PackAction::Export => None,
				};
				let stage = if info.action == PackAction::Export {
					InstallStage::Packing
				} else {
					InstallStage::Queued
				};
				(Some(instance_id.clone()), mode, None, stage)
			}
		};
		let content = match &target {
			Target::Content { info, .. } => Some(info.clone()),
			_ => None,
		};
		let pack: Option<PackTask> = match &target {
			Target::Pack { info, .. } => Some(info.clone()),
			_ => None,
		};
		let task = Download {
			id: id.clone(),
			kind,
			instance_id,
			mode,
			java_major,
			content,
			title: pack.as_ref().map(|p| p.title.clone()),
			icon_url: pack.as_ref().and_then(|p| p.icon_url.clone()),
			launcher: pack.as_ref().and_then(|p| p.launcher),
			output_path: None,
			warnings: None,
			state: DownloadState::Queued,
			stage: Some(stage),
			percent: None,
			bytes_done: 0,
			bytes_total: None,
			steps_done: None,
			steps_total: None,
			files_done: None,
			files_total: None,
			speed: None,
			eta_secs: None,
			queued_at: now_ms(),
			started_at: None,
			finished_at: None,
			error: None,
		};
		let mut inner = self.lock();
		inner
			.entries
			.retain(|e| !(e.task.state.is_done() && e.target.same(&target)));
		inner.entries.push(Entry {
			task,
			target,
			cancel,
			stage: None,
			snap: Snapshot::default(),
			base: (0, 0, 0),
			samples: VecDeque::new(),
			receiving_since: None,
		});
		inner.list_dirty = true;
		drop(inner);
		self.notify(true);
		id
	}

	/// Прогресс. `stage` — этап установки сборки (`None` у Java); `percent` — общий 0–100,
	/// `None` — по байтам этапа.
	pub fn progress(
		&self,
		id: &str,
		stage: Option<InstallStage>,
		percent: Option<f64>,
		snap: Snapshot,
	) {
		let now = Instant::now();
		let urgent = {
			let mut inner = self.lock();
			let Some(e) = inner
				.entries
				.iter_mut()
				.find(|e| e.task.id == id && !e.task.state.is_done())
			else {
				return;
			};
			let queued = stage == Some(InstallStage::Queued);
			let state = if queued {
				DownloadState::Queued
			} else {
				DownloadState::Running
			};
			let mut urgent = e.task.state != state;
			e.task.state = state;
			if state == DownloadState::Running && e.task.started_at.is_none() {
				e.task.started_at = Some(now_ms());
			}
			if stage.is_some() && stage != e.stage {
				let (done, total) = net(&e.snap);
				e.base.0 += done;
				e.base.1 += total;
				e.base.2 += e.snap.bytes_received;
				e.stage = stage;
				e.task.stage = stage;
				urgent = true;
			}
			e.snap = snap;
			e.task.percent = if queued {
				None
			} else {
				percent
					.or_else(|| {
						(snap.bytes_total > 0)
							.then(|| snap.bytes_done as f64 * 100.0 / snap.bytes_total as f64)
					})
					.map(|p| p.clamp(0.0, 100.0))
			};
			let received = e.received();
			if received > 0 && e.receiving_since.is_none() {
				e.receiving_since = Some(now);
			}
			if e.samples.back().is_none_or(|(_, b)| *b != received) {
				e.samples.push_back((now, received));
			}
			e.prune(now);
			if urgent {
				inner.list_dirty = true;
			} else {
				inner.dirty.insert(id.to_owned());
			}
			urgent
		};
		self.notify(urgent);
	}

	/// Конец задачи: `Ok` — готово, `cancelled` — задача убирается из списка, другая ошибка — `failed`.
	pub fn finish(&self, id: &str, result: Result<(), AppError>) {
		{
			let mut inner = self.lock();
			let now = Instant::now();
			let Some(index) = inner.entries.iter().position(|e| e.task.id == id) else {
				return;
			};
			match result {
				Err(err) if err.code == crate::error::ErrorCode::Cancelled => {
					inner.entries.remove(index);
				}
				result => {
					let entry = &mut inner.entries[index];
					let mut task = entry.view(now);
					task.state = if result.is_ok() {
						DownloadState::Finished
					} else {
						DownloadState::Failed
					};
					if result.is_ok() {
						task.percent = Some(100.0);
					}
					task.error = result.err();
					task.finished_at = Some(now_ms());
					task.speed = None;
					task.eta_secs = None;
					entry.task = task;
					entry.samples.clear();
				}
			}
			let done = inner
				.entries
				.iter()
				.filter(|e| e.task.state.is_done())
				.count();
			let mut extra = done.saturating_sub(HISTORY_LIMIT);
			inner.entries.retain(|e| {
				if extra > 0 && e.task.state.is_done() {
					extra -= 1;
					return false;
				}
				true
			});
			inner.dirty.remove(id);
			inner.list_dirty = true;
		}
		self.notify(true);
	}

	/// Список: идущие и в очереди, затем завершённые; внутри — новые первыми.
	pub fn list(&self) -> Vec<Download> {
		let now = Instant::now();
		let mut inner = self.lock();
		let mut list: Vec<Download> = inner
			.entries
			.iter_mut()
			.rev()
			.map(|e| e.view(now))
			.collect();
		list.sort_by_key(|t| t.state.is_done());
		list
	}

	pub fn get(&self, id: &str) -> Option<Download> {
		let now = Instant::now();
		self.lock()
			.entries
			.iter_mut()
			.find(|e| e.task.id == id)
			.map(|e| e.view(now))
	}

	/// Что повторять: объект завершённой задачи (`finished`/`failed`). Идущая или нет такой — `None`.
	pub fn retry_target(&self, id: &str) -> Option<Target> {
		self.lock()
			.entries
			.iter()
			.find(|e| e.task.id == id && e.task.state.is_done())
			.map(|e| e.target.clone())
	}

	/// Отмена задачи. `true` — задача шла и отменена (из списка она уйдёт, когда остановится).
	pub fn cancel(&self, id: &str) -> bool {
		let token = self
			.lock()
			.entries
			.iter()
			.find(|e| e.task.id == id && !e.task.state.is_done())
			.map(|e| e.cancel.clone());
		token.map(|t| t.cancel()).is_some()
	}

	fn remove_where(&self, pred: impl Fn(&Entry) -> bool) {
		let mut inner = self.lock();
		let before = inner.entries.len();
		inner.entries.retain(|e| !pred(e));
		if inner.entries.len() != before {
			inner.list_dirty = true;
			drop(inner);
			self.notify(true);
		}
	}

	/// Убирает все завершённые задачи.
	pub fn clear(&self) {
		self.remove_where(|e| e.task.state.is_done());
	}

	/// Убирает одну завершённую задачу. Идущие не трогает.
	pub fn dismiss(&self, id: &str) {
		self.remove_where(|e| e.task.id == id && e.task.state.is_done());
	}

	/// Сборку удалили: её завершённые задачи больше не нужны (идущих у удаляемой сборки нет).
	pub fn forget_instance(&self, instance_id: &str) {
		self.remove_where(|e| {
			e.task.state.is_done() && e.task.instance_id.as_deref() == Some(instance_id)
		});
	}

	/// Дополняет задачу (предупреждения, путь готового файла) — до [`Self::finish`].
	pub fn annotate(&self, id: &str, change: impl FnOnce(&mut Download)) {
		let mut inner = self.lock();
		if let Some(e) = inner.entries.iter_mut().find(|e| e.task.id == id) {
			change(&mut e.task);
			inner.list_dirty = true;
		}
		drop(inner);
		self.notify(true);
	}

	/// Снимает предупреждение о ручных файлах с задач сборки.
	pub fn clear_manual(&self, instance_id: &str) {
		let mut inner = self.lock();
		let mut changed = false;
		for e in inner.entries.iter_mut() {
			if e.task.instance_id.as_deref() != Some(instance_id) {
				continue;
			}
			if let Some(w) = &mut e.task.warnings
				&& !w.manual.is_empty()
			{
				w.manual.clear();
				if w.failed.is_empty() {
					e.task.warnings = None;
				}
				changed = true;
			}
		}
		if changed {
			inner.list_dirty = true;
		}
		drop(inner);
		if changed {
			self.notify(true);
		}
	}

	/// У сборки идёт установка контента или экспорт (сборку нельзя удалить или переустановить).
	pub fn has_active_content(&self, instance_id: &str) -> bool {
		self.lock().entries.iter().any(|e| {
			matches!(e.task.kind, DownloadKind::Content | DownloadKind::Export)
				&& !e.task.state.is_done()
				&& e.task.instance_id.as_deref() == Some(instance_id)
		})
	}

	/// У сборки идёт установка или обновление модпака (или перенос): ставить контент нельзя.
	pub fn has_active_pack(&self, instance_id: &str) -> bool {
		self.lock().entries.iter().any(|e| {
			matches!(e.task.kind, DownloadKind::Modpack | DownloadKind::Import)
				&& !e.task.state.is_done()
				&& e.task.instance_id.as_deref() == Some(instance_id)
		})
	}

	/// Есть незавершённые задачи (идут или ждут очереди).
	pub fn has_active(&self) -> bool {
		self.lock().entries.iter().any(|e| !e.task.state.is_done())
	}

	fn has_running(&self) -> bool {
		self.lock()
			.entries
			.iter()
			.any(|e| e.task.state == DownloadState::Running)
	}

	fn take(&self, tick: bool) -> Vec<Emit> {
		let now = Instant::now();
		let mut inner = self.lock();
		if inner.list_dirty {
			inner.list_dirty = false;
			inner.dirty.clear();
			drop(inner);
			return vec![Emit::List(self.list())];
		}
		let dirty = std::mem::take(&mut inner.dirty);
		inner
			.entries
			.iter_mut()
			.filter(|e| {
				dirty.contains(&e.task.id) || (tick && e.task.state == DownloadState::Running)
			})
			.map(|e| Emit::Progress(Box::new(e.view(now))))
			.collect()
	}

	/// Рассылает изменения: состав списка — сразу, прогресс — не чаще [`PROGRESS_INTERVAL`].
	pub async fn run_emitter(self: Arc<Self>, emit: impl Fn(Emit)) {
		loop {
			let tick = if self.has_running() {
				tokio::select! {
					_ = self.changed.notified() => false,
					_ = tokio::time::sleep(IDLE_TICK) => true,
				}
			} else {
				self.changed.notified().await;
				false
			};
			for item in self.take(tick) {
				emit(item);
			}
			tokio::select! {
				_ = self.urgent.notified() => {}
				_ = tokio::time::sleep(PROGRESS_INTERVAL) => {}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::error::ErrorCode;

	fn instance(id: &str) -> Target {
		Target::Instance {
			id: id.into(),
			mode: InstallMode::Install,
		}
	}

	fn snap(done: u64, total: u64, received: u64, skipped: u64) -> Snapshot {
		Snapshot {
			files_done: 1,
			files_total: 4,
			bytes_done: done,
			bytes_total: total,
			bytes_received: received,
			bytes_skipped: skipped,
		}
	}

	#[test]
	fn lifecycle_and_json() {
		let d = Downloads::default();
		let token = CancellationToken::new();
		let id = d.begin(instance("abc"), token.clone());
		let t = d.get(&id).unwrap();
		assert_eq!(t.state, DownloadState::Queued);
		assert_eq!(t.stage, Some(InstallStage::Queued));
		assert_eq!(t.percent, None);
		let json = serde_json::to_value(&t).unwrap();
		assert_eq!(json["kind"], "instance_install");
		assert_eq!(json["instanceId"], "abc");
		assert_eq!(json["mode"], "install");
		assert_eq!(json["state"], "queued");
		assert!(json["percent"].is_null() && json["speed"].is_null() && json["etaSecs"].is_null());
		assert!(json.get("startedAt").is_none() && json.get("javaMajor").is_none());

		// Java: 100 надо скачать, 300 уже было.
		d.progress(
			&id,
			Some(InstallStage::Java),
			Some(5.0),
			snap(350, 400, 50, 300),
		);
		let t = d.get(&id).unwrap();
		assert_eq!(t.state, DownloadState::Running);
		assert!(t.started_at.is_some());
		assert_eq!((t.bytes_done, t.bytes_total), (50, Some(100)));
		// Файлы игры: байты считаются за всю задачу.
		d.progress(
			&id,
			Some(InstallStage::Downloading),
			Some(10.0),
			snap(0, 0, 0, 0),
		);
		d.progress(
			&id,
			Some(InstallStage::Downloading),
			Some(50.0),
			snap(500, 1000, 400, 100),
		);
		let t = d.get(&id).unwrap();
		assert_eq!(t.stage, Some(InstallStage::Downloading));
		assert_eq!(t.percent, Some(50.0));
		assert_eq!((t.bytes_done, t.bytes_total), (50 + 400, Some(100 + 900)));
		assert!(t.steps_done.is_none());
		d.progress(
			&id,
			Some(InstallStage::Processors),
			Some(90.0),
			snap(0, 0, 0, 0),
		);
		let steps = Snapshot {
			files_done: 2,
			files_total: 6,
			..Snapshot::default()
		};
		d.progress(&id, Some(InstallStage::Processors), Some(91.0), steps);
		let t = d.get(&id).unwrap();
		assert_eq!((t.steps_done, t.steps_total), (Some(2), Some(6)));
		assert_eq!(t.bytes_done, 450);

		d.finish(&id, Ok(()));
		let t = d.get(&id).unwrap();
		assert_eq!(t.state, DownloadState::Finished);
		assert_eq!(t.percent, Some(100.0));
		assert!(t.finished_at.is_some() && t.speed.is_none());
		assert_eq!(t.bytes_done, 450);
		assert!(!d.cancel(&id));
		assert!(!token.is_cancelled());
		// Прогресс после конца не меняет задачу.
		d.progress(&id, Some(InstallStage::Downloading), None, snap(1, 1, 1, 0));
		assert_eq!(d.get(&id).unwrap().state, DownloadState::Finished);
	}

	#[test]
	fn cancel_removes_and_failed_stays() {
		let d = Downloads::default();
		let token = CancellationToken::new();
		let id = d.begin(Target::Java { major: 21 }, token.clone());
		assert_eq!(d.get(&id).unwrap().stage, Some(InstallStage::Downloading));
		assert_eq!(d.get(&id).unwrap().java_major, Some(21));
		assert!(d.retry_target(&id).is_none());
		assert!(d.cancel(&id));
		assert!(token.is_cancelled());
		d.finish(&id, Err(AppError::new(ErrorCode::Cancelled)));
		assert!(d.list().is_empty());

		let id = d.begin(Target::Java { major: 21 }, CancellationToken::new());
		d.progress(&id, None, None, snap(25, 100, 25, 0));
		assert_eq!(d.get(&id).unwrap().percent, Some(25.0));
		d.finish(&id, Err(AppError::new(ErrorCode::DownloadFailed)));
		let json = serde_json::to_value(d.get(&id).unwrap()).unwrap();
		assert_eq!(json["state"], "failed");
		assert_eq!(json["error"]["code"], "download_failed");
		assert_eq!(d.retry_target(&id), Some(Target::Java { major: 21 }));
		// Повтор убирает прошлую строку.
		let again = d.begin(Target::Java { major: 21 }, CancellationToken::new());
		let ids: Vec<_> = d.list().into_iter().map(|t| t.id).collect();
		assert_eq!(ids, vec![again]);
	}

	#[test]
	fn speed_and_eta() {
		let d = Downloads::default();
		let id = d.begin(Target::Java { major: 17 }, CancellationToken::new());
		d.progress(&id, None, None, snap(0, 3_000_000, 0, 0));
		{
			let mut inner = d.lock();
			let e = &mut inner.entries[0];
			let now = Instant::now();
			e.samples.clear();
			e.samples.push_back((now - Duration::from_secs(2), 0));
			e.samples.push_back((now, 2_000_000));
			e.snap = snap(2_000_000, 3_000_000, 2_000_000, 0);
			e.receiving_since = Some(now - Duration::from_secs(1));
		}
		let t = d.get(&id).unwrap();
		let speed = t.speed.unwrap();
		assert!((900_000..=1_100_000).contains(&speed), "{speed}");
		// Меньше 3 с после начала — время не оцениваем.
		assert_eq!(t.eta_secs, None);
		d.lock().entries[0].receiving_since = Some(Instant::now() - Duration::from_secs(5));
		assert!(matches!(d.get(&id).unwrap().eta_secs, Some(1..=2)));

		// Встала: последняя точка старше окна — скорости нет.
		{
			let mut inner = d.lock();
			let e = &mut inner.entries[0];
			let old = Instant::now() - Duration::from_secs(10);
			e.samples.clear();
			e.samples.push_back((old - Duration::from_secs(1), 0));
			e.samples.push_back((old, 5_000));
		}
		let t = d.get(&id).unwrap();
		assert_eq!((t.speed, t.eta_secs), (None, None));

		// Уже скачанные файлы (не из сети) скорость не дают.
		let other = d.begin(Target::Java { major: 8 }, CancellationToken::new());
		d.progress(&other, None, None, snap(500, 1000, 0, 500));
		d.progress(&other, None, None, snap(900, 1000, 0, 900));
		let t = d.get(&other).unwrap();
		assert_eq!(t.speed, None);
		assert_eq!((t.bytes_done, t.bytes_total), (0, Some(100)));
	}

	#[test]
	fn history_clear_dismiss_forget() {
		let d = Downloads::default();
		let running = d.begin(instance("run"), CancellationToken::new());
		for i in 0..HISTORY_LIMIT + 5 {
			let id = d.begin(instance(&format!("i{i}")), CancellationToken::new());
			d.finish(&id, Ok(()));
		}
		let list = d.list();
		assert_eq!(list.len(), HISTORY_LIMIT + 1);
		// Сначала идущие, потом завершённые новые первыми; старые вытеснены.
		assert_eq!(list[0].id, running);
		assert_eq!(
			list[1].instance_id.as_deref(),
			Some(format!("i{}", HISTORY_LIMIT + 4).as_str())
		);

		d.dismiss(&running);
		assert!(d.get(&running).is_some());
		let done = list[1].id.clone();
		d.dismiss(&done);
		assert!(d.get(&done).is_none());
		let last = list[2].instance_id.clone().unwrap();
		d.forget_instance(&last);
		assert!(
			!d.list()
				.iter()
				.any(|t| t.instance_id.as_deref() == Some(&last))
		);
		d.clear();
		let list = d.list();
		assert_eq!(list.len(), 1);
		assert_eq!(list[0].id, running);
	}

	async fn next(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Emit>) -> Emit {
		tokio::time::timeout(Duration::from_secs(3), rx.recv())
			.await
			.unwrap()
			.unwrap()
	}

	#[tokio::test]
	async fn emitter_sends_list_and_progress() {
		let d = Arc::new(Downloads::default());
		let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
		let task = tokio::spawn(d.clone().run_emitter(move |e| {
			let _ = tx.send(e);
		}));
		let id = d.begin(instance("a"), CancellationToken::new());
		assert!(matches!(next(&mut rx).await, Emit::List(l) if l[0].id == id));
		d.progress(
			&id,
			Some(InstallStage::Metadata),
			Some(1.0),
			Snapshot::default(),
		);
		assert!(
			matches!(next(&mut rx).await, Emit::List(l) if l[0].state == DownloadState::Running)
		);
		d.progress(
			&id,
			Some(InstallStage::Metadata),
			Some(2.0),
			Snapshot::default(),
		);
		assert!(matches!(next(&mut rx).await, Emit::Progress(t) if t.percent == Some(2.0)));
		d.finish(&id, Ok(()));
		loop {
			if let Emit::List(l) = next(&mut rx).await
				&& l[0].state == DownloadState::Finished
			{
				break;
			}
		}
		task.abort();
	}
}
