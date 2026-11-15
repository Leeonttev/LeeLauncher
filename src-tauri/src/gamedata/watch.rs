//! Наблюдение за папками открытой сборки, события пачкой через 300 мс.

use std::collections::HashSet;
use std::path::{Component, Path};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const EVENT_WORLDS: &str = "instance-worlds-changed";
pub const EVENT_SCREENSHOTS: &str = "instance-screenshots-changed";
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Данные событий `instance-*-changed`.
#[derive(Debug, Clone, Serialize)]
pub struct InstanceRef {
	pub id: String,
}

pub fn emit(app: &AppHandle, event: &str, id: &str) {
	if let Err(err) = app.emit(event, InstanceRef { id: id.to_owned() }) {
		log::warn!("не удалось отправить {event}: {err}");
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Change {
	Content,
	Worlds,
	Screenshots,
}

impl Change {
	fn event(self) -> &'static str {
		match self {
			Self::Content => crate::content::EVENT_CHANGED,
			Self::Worlds => EVENT_WORLDS,
			Self::Screenshots => EVENT_SCREENSHOTS,
		}
	}
}

/// Что изменилось по пути относительно папки игры. Остальное (логи, регионы мира, конфиги) — `None`.
pub fn classify(rel: &Path) -> Option<Change> {
	let parts: Vec<&str> = rel
		.components()
		.map(|c| match c {
			Component::Normal(s) => s.to_str(),
			_ => None,
		})
		.collect::<Option<_>>()?;
	match parts.as_slice() {
		["mods" | "resourcepacks" | "shaderpacks" | "datapacks", ..] if parts.len() <= 2 => {
			Some(Change::Content)
		}
		["saves", _, "datapacks", ..] if parts.len() <= 4 => Some(Change::Content),
		["saves"] | ["saves", _] | ["saves", _, "level.dat" | "icon.png"] => Some(Change::Worlds),
		["screenshots", ..] if parts.len() <= 2 => Some(Change::Screenshots),
		_ => None,
	}
}

/// Текущее наблюдение (одна сборка за раз).
#[derive(Default)]
pub struct Watch {
	/// Номер последнего вызова: запоздавший старый ничего не меняет.
	requested: AtomicU64,
	current: Mutex<Option<(String, notify::RecommendedWatcher)>>,
}

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
	#[error("сборки нет")]
	Gone,
	#[error("{0}")]
	Other(String),
}

impl Watch {
	/// Номер нового вызова; его передают в [`Self::set`].
	pub fn begin(&self) -> u64 {
		self.requested.fetch_add(1, Ordering::SeqCst) + 1
	}

	/// Открытая папка не даёт Windows переименовать папку сборки.
	pub fn stop_if(&self, id: &str) {
		if let Ok(mut current) = self.current.lock()
			&& current.as_ref().is_some_and(|(old, _)| old == id)
		{
			*current = None;
		}
	}

	/// Следить за сборкой (`None` — перестать). Устаревший `generation` ничего не делает.
	pub fn set(
		&self,
		app: &AppHandle,
		generation: u64,
		target: Option<(String, std::path::PathBuf)>,
	) -> Result<(), WatchError> {
		let mut current = self
			.current
			.lock()
			.map_err(|e| WatchError::Other(e.to_string()))?;
		if generation != self.requested.load(Ordering::SeqCst) {
			return Ok(());
		}
		if let (Some((old, _)), Some((new, _))) = (current.as_ref(), target.as_ref())
			&& old == new
		{
			return Ok(());
		}
		// Старый наблюдатель закрывается вместе с отправителем событий: его задача завершится.
		*current = None;
		let Some((id, game_dir)) = target else {
			return Ok(());
		};
		let exists = game_dir
			.parent()
			.is_some_and(|dir| dir.join(crate::instances::FILE_NAME).is_file());
		if !exists {
			return Err(WatchError::Gone);
		}
		let other = |e: &dyn std::fmt::Display| WatchError::Other(e.to_string());
		std::fs::create_dir_all(&game_dir).map_err(|e| other(&e))?;
		let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
		let root = game_dir.clone();
		let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
			let Ok(event) = res else { return };
			for path in event.paths {
				if let Some(change) = path.strip_prefix(&root).ok().and_then(classify) {
					let _ = tx.send(change);
				}
			}
		})
		.map_err(|e| other(&e))?;
		watcher
			.watch(&game_dir, RecursiveMode::Recursive)
			.map_err(|e| other(&e))?;
		tauri::async_runtime::spawn(debounce(app.clone(), id.clone(), rx));
		*current = Some((id, watcher));
		Ok(())
	}
}

async fn debounce(
	app: AppHandle,
	id: String,
	mut rx: tokio::sync::mpsc::UnboundedReceiver<Change>,
) {
	while let Some(first) = rx.recv().await {
		let mut pending = HashSet::from([first]);
		let deadline = tokio::time::sleep(DEBOUNCE);
		tokio::pin!(deadline);
		let mut closed = false;
		loop {
			tokio::select! {
				_ = &mut deadline => break,
				next = rx.recv() => match next {
					Some(change) => {
						pending.insert(change);
					}
					None => {
						closed = true;
						break;
					}
				},
			}
		}
		if closed {
			return;
		}
		for change in pending {
			emit(&app, change.event(), &id);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn classifies_paths() {
		let c = |p: &str| classify(Path::new(p));
		assert_eq!(c("mods/a.jar"), Some(Change::Content));
		assert_eq!(c("mods"), Some(Change::Content));
		assert_eq!(c("mods/sub/a.jar"), None);
		assert_eq!(c("saves/W/datapacks/p.zip"), Some(Change::Content));
		assert_eq!(c("saves/W"), Some(Change::Worlds));
		assert_eq!(c("saves/W/level.dat"), Some(Change::Worlds));
		assert_eq!(c("saves/W/region/r.0.0.mca"), None);
		assert_eq!(c("saves/W/level.dat_old"), None);
		assert_eq!(c("screenshots/a.png"), Some(Change::Screenshots));
		assert_eq!(c("logs/latest.log"), None);
		assert_eq!(c("options.txt"), None);
	}
}
