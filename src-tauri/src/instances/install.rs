//! Установка сборки: метаданные → Java → файлы игры (клиент, библиотеки, natives, ассеты) → процессоры Forge/NeoForge.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::{InstallState, Installed, Instance};
use crate::download::{Job, Progress, Snapshot, Verify};
use crate::downloads::{Downloads, Target};
use crate::error::{AppError, ErrorCode};
use crate::fs_util;
use crate::java::{self, JavaInstall};
use crate::launcher::Launcher;
use crate::meta::model::{AssetIndex, VersionInfo};
use crate::meta::rules::{Platform, library_allowed, maven_path, native_download};
use crate::meta::{Resolved, check_version_info, unsafe_meta};
use crate::paths::Layout;

pub const EVENT_PROGRESS: &str = "instance-install-progress";
pub const EVENT_FINISHED: &str = "instance-install-finished";
pub const EVENT_JAVA_PROGRESS: &str = "java-install-progress";

const ASSETS_URL: &str = "https://resources.download.minecraft.net";
const LIBRARIES_URL: &str = "https://libraries.minecraft.net/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
	/// Скачать недостающее; уже скачанные файлы проверяются по размеру.
	#[default]
	Install,
	/// Заново: метаданные и описание версии скачиваются снова, все файлы проверяются по sha1, процессоры запускаются снова.
	Reinstall,
	/// Починить: все файлы проверяются по sha1, испорченные скачиваются снова, процессоры запускаются снова.
	Repair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStage {
	/// Ждёт, пока закончится установка другой сборки.
	Queued,
	Metadata,
	Java,
	Downloading,
	Processors,
	Finishing,
	/// Модпак: моды и файлы его индекса.
	ModpackFiles,
	/// Модпак: распаковка `overrides`.
	Overrides,
	/// Импорт: копирование папки игры из другого лаунчера.
	Copying,
	/// Экспорт: упаковка `.mrpack`.
	Packing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
	pub instance_id: String,
	pub stage: InstallStage,
	/// Общий процент установки 0–100.
	pub percent: f32,
	pub files_done: u64,
	pub files_total: u64,
	pub bytes_done: u64,
	pub bytes_total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallResult {
	Installed,
	Failed,
	Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallFinished {
	pub instance_id: String,
	pub result: InstallResult,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<AppError>,
}

/// Куда установка сообщает о себе (в приложении — события Tauri, в тестах — лог).
pub trait InstallEvents: Send + Sync {
	fn progress(&self, progress: InstallProgress);
	fn changed(&self, instances: Vec<Instance>);
	fn finished(&self, finished: InstallFinished);
}

/// Задача установки в общем списке загрузок.
#[derive(Clone)]
pub struct Tracked {
	pub downloads: Arc<Downloads>,
	pub task_id: String,
}

/// Пересчитывает прогресс этапа в общий процент.
pub struct Reporter {
	instance_id: String,
	events: Arc<dyn InstallEvents>,
	tracked: Option<Tracked>,
	/// Какую часть общего процента занимает этот отчёт (модпак: игра — только часть задачи).
	range: (f32, f32),
}

impl Reporter {
	pub fn new(instance_id: String, events: Arc<dyn InstallEvents>) -> Self {
		Self {
			instance_id,
			events,
			tracked: None,
			range: (0.0, 100.0),
		}
	}

	/// Проценты этапов `0–100` сжимаются в `from..to` общего процента.
	pub fn scaled(mut self, from: f32, to: f32) -> Self {
		self.range = (from, to);
		self
	}

	fn map(range: (f32, f32), percent: f32) -> f32 {
		range.0 + (range.1 - range.0) * percent.clamp(0.0, 100.0) / 100.0
	}

	pub fn tracked(mut self, tracked: Tracked) -> Self {
		self.tracked = Some(tracked);
		self
	}

	fn emit(&self, stage: InstallStage, percent: f32, snap: Snapshot) {
		let percent = Self::map(self.range, percent);
		if let Some(t) = &self.tracked {
			t.downloads
				.progress(&t.task_id, Some(stage), Some(f64::from(percent)), snap);
		}
		self.events.progress(InstallProgress {
			instance_id: self.instance_id.clone(),
			stage,
			percent,
			files_done: snap.files_done,
			files_total: snap.files_total,
			bytes_done: snap.bytes_done,
			bytes_total: snap.bytes_total,
		});
	}

	/// Начало этапа, занимающего диапазон `from..to` общего процента.
	pub fn stage(&self, stage: InstallStage, from: f32, to: f32) -> Arc<Progress> {
		self.emit(stage, from, Snapshot::default());
		let id = self.instance_id.clone();
		let events = self.events.clone();
		let tracked = self.tracked.clone();
		let range = self.range;
		Progress::new(move |snap| {
			let fraction = if snap.bytes_total > 0 {
				snap.bytes_done as f32 / snap.bytes_total as f32
			} else if snap.files_total > 0 {
				snap.files_done as f32 / snap.files_total as f32
			} else {
				0.0
			};
			let percent = Self::map(range, from + (to - from) * fraction.min(1.0));
			if let Some(t) = &tracked {
				t.downloads
					.progress(&t.task_id, Some(stage), Some(f64::from(percent)), snap);
			}
			events.progress(InstallProgress {
				instance_id: id.clone(),
				stage,
				percent,
				files_done: snap.files_done,
				files_total: snap.files_total,
				bytes_done: snap.bytes_done,
				bytes_total: snap.bytes_total,
			});
		})
	}
}

/// Java для версии игры: своя у сборки → выбранная в настройках для этой версии → найденная → скачанная.
pub async fn resolve_java(
	l: &Launcher,
	own: Option<&str>,
	major: u32,
	component: Option<&str>,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<JavaInstall, AppError> {
	let managed_root = l.layout.java_dir();
	let configured = l.settings.get().java_paths.get(&major.to_string()).cloned();
	if let Some(found) = java::select(
		own,
		configured.as_deref(),
		major,
		&managed_root,
		&l.java_found,
	)
	.await?
	{
		log::info!("Java {major}: {}", found.path);
		if found.managed
			&& let Some(updated) = update_java(l, &found, progress.clone(), cancel).await?
		{
			return Ok(updated);
		}
		return Ok(found);
	}
	let _guard = tokio::select! {
		_ = cancel.cancelled() => return Err(AppError::new(ErrorCode::Cancelled)),
		g = l.java_lock.lock() => g,
	};
	let in_use = |dir: &std::path::Path| l.games.java_in_use(dir);
	let java = java::runtime::install(
		&l.meta,
		&l.dl,
		&managed_root,
		component,
		major,
		progress,
		cancel,
		&in_use,
	)
	.await?;
	l.java_found.invalidate().await;
	Ok(java)
}

/// Скачанная нами Java устарела — ставим новую версию на её место. Пока ею пользуется игра,
/// обновление откладывается; сбой обновления не мешает установке сборки.
async fn update_java(
	l: &Launcher,
	found: &JavaInstall,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<Option<JavaInstall>, AppError> {
	let managed_root = l.layout.java_dir();
	let Some(home) = java::runtime::home_of(&found.path, &managed_root) else {
		return Ok(None);
	};
	let Some(version) = java::runtime::update_available(&l.meta, &home, cancel).await else {
		return Ok(None);
	};
	if l.games.java_in_use(&home) {
		log::info!(
			"Java {} → {version}: идёт игра на этой Java, обновим позже",
			found.version
		);
		return Ok(None);
	}
	let Some(marker) = java::runtime::marker(&home) else {
		return Ok(None);
	};
	let _guard = tokio::select! {
		_ = cancel.cancelled() => return Err(AppError::new(ErrorCode::Cancelled)),
		g = l.java_lock.lock() => g,
	};
	log::info!("Java {} → {version}: обновляем", found.version);
	let in_use = |dir: &std::path::Path| l.games.java_in_use(dir);
	let result = java::runtime::install(
		&l.meta,
		&l.dl,
		&managed_root,
		Some(&marker.component),
		found.major,
		progress,
		cancel,
		&in_use,
	)
	.await;
	l.java_found.invalidate().await;
	match result {
		Ok(java) => Ok(Some(java)),
		Err(err) if err.code == ErrorCode::Cancelled => Err(err),
		Err(err) => {
			log::warn!(
				"Java {}: обновление не удалось ({:?}), берём прежнюю",
				found.version,
				err.code
			);
			Ok(None)
		}
	}
}

/// Natives старого формата: jar и что из него не распаковывать.
#[derive(Debug, Clone)]
pub struct NativeJar {
	pub path: PathBuf,
	pub exclude: Vec<String>,
}

/// Файлы библиотек для платформы: classpath-библиотеки и natives старого формата.
pub fn library_jobs(
	info: &VersionInfo,
	layout: &Layout,
	platform: Platform,
) -> Result<(Vec<Job>, Vec<NativeJar>), AppError> {
	let libs_dir = layout.libraries_dir();
	let mut jobs = Vec::new();
	let mut natives = Vec::new();
	for lib in &info.libraries {
		if !lib.downloadable || !library_allowed(lib, platform) {
			continue;
		}
		// Распакованный серверный патч Forge нужен только серверным процессорам.
		if !lib.include_in_classpath && lib.name.ends_with(":server@lzma") {
			continue;
		}
		if let Some((classifier, dl)) = native_download(lib, platform) {
			let rel = match &dl.path {
				Some(p) => p.clone(),
				None => maven_path(&format!("{}:{classifier}", lib.name))
					.ok_or_else(|| bad_library(&lib.name))?,
			};
			let path = library_file(&libs_dir, &rel, &lib.name)?;
			natives.push(NativeJar {
				path: path.clone(),
				exclude: lib
					.extract
					.as_ref()
					.map(|e| e.exclude.clone())
					.unwrap_or_default(),
			});
			jobs.push(Job {
				url: dl.url.clone(),
				path,
				sha1: dl.sha1.clone(),
				size: dl.size,
			});
		}
		let artifact = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref());
		// У старых natives-библиотек основного артефакта нет.
		if lib.natives.is_some() && artifact.is_none() {
			continue;
		}
		let rel = match artifact.and_then(|a| a.path.clone()) {
			Some(p) => p,
			None => maven_path(&lib.name).ok_or_else(|| bad_library(&lib.name))?,
		};
		let url = match artifact.map(|a| a.url.as_str()).filter(|u| !u.is_empty()) {
			Some(url) => url.to_owned(),
			None => {
				let base = lib.url.as_deref().unwrap_or(LIBRARIES_URL);
				let sep = if base.ends_with('/') { "" } else { "/" };
				format!("{base}{sep}{rel}")
			}
		};
		jobs.push(Job {
			url,
			path: library_file(&libs_dir, &rel, &lib.name)?,
			sha1: artifact.and_then(|a| a.sha1.clone()),
			size: artifact.and_then(|a| a.size),
		});
	}
	Ok((jobs, natives))
}

fn bad_library(name: &str) -> AppError {
	AppError::coded(
		ErrorCode::DownloadFailed,
		format!("неверное имя библиотеки: {name}"),
	)
}

/// Файл библиотеки внутри `libraries`: путь из описания версии не должен выводить наружу.
fn library_file(libs_dir: &Path, rel: &str, name: &str) -> Result<PathBuf, AppError> {
	fs_util::safe_join(libs_dir, rel)
		.ok_or_else(|| unsafe_meta(format!("библиотека {name}: {rel}")))
}

/// Объекты ассетов. Хеш — имя файла, поэтому он обязан быть ровно 40 hex-символами.
pub fn asset_jobs(index: &AssetIndex, layout: &Layout) -> Result<Vec<Job>, AppError> {
	index
		.objects
		.iter()
		.map(|(name, a)| {
			if !fs_util::is_sha1_hex(&a.hash) {
				return Err(unsafe_meta(format!("ассет {name}: хеш {:?}", a.hash)));
			}
			Ok(Job {
				url: format!("{ASSETS_URL}/{}/{}", &a.hash[..2], a.hash),
				path: layout.asset_object(&a.hash),
				sha1: Some(a.hash.clone()),
				size: Some(a.size),
			})
		})
		.collect()
}

/// Скачивает и раскладывает файлы версии: клиент, конфиг логов, библиотеки, natives, ассеты.
pub async fn game_files(
	l: &Launcher,
	resolved: &Resolved,
	game_dir: &Path,
	platform: Platform,
	verify: Verify,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<(), AppError> {
	let info = &resolved.info;
	let layout = &l.layout;
	check_version_info(info)?;

	let index_ref = &info.asset_index;
	let index_job = Job {
		url: index_ref.url.clone(),
		path: layout.asset_index(&index_ref.id),
		sha1: Some(index_ref.sha1.clone()),
		size: Some(index_ref.size),
	};
	l.dl.file(&index_job, Verify::Hash, &Progress::silent(), cancel)
		.await?;
	let index_bytes = tokio::fs::read(&index_job.path)
		.await
		.map_err(|e| AppError::storage(format!("{}: {e}", index_job.path.display())))?;
	let index: AssetIndex = serde_json::from_slice(&index_bytes)
		.map_err(|e| AppError::coded(ErrorCode::DownloadFailed, format!("индекс ассетов: {e}")))?;

	let mut jobs = Vec::new();
	let client = info.downloads.get("client").ok_or_else(|| {
		AppError::coded(ErrorCode::DownloadFailed, "в описании версии нет клиента")
	})?;
	jobs.push(Job {
		url: client.url.clone(),
		path: layout.version_jar(&resolved.game_version),
		sha1: Some(client.sha1.clone()),
		size: Some(client.size),
	});
	if let Some(log) = info.logging.as_ref().and_then(|l| l.get("client")) {
		jobs.push(Job {
			url: log.file.url.clone(),
			path: layout.log_config(&log.file.id),
			sha1: Some(log.file.sha1.clone()),
			size: Some(log.file.size),
		});
	}
	let (libraries, natives) = library_jobs(info, layout, platform)?;
	jobs.extend(libraries);
	jobs.extend(asset_jobs(&index, layout)?);

	l.dl.files(jobs, verify, progress, cancel).await?;

	let natives_dir = layout.natives_dir(&resolved.version_id);
	let virtual_dir = index
		.is_virtual
		.then(|| layout.virtual_assets_dir(&index_ref.id));
	let resources_dir = index.map_to_resources.then(|| game_dir.join("resources"));
	let layout = layout.clone();
	tokio::task::spawn_blocking(move || -> Result<(), AppError> {
		std::fs::create_dir_all(&natives_dir)
			.map_err(|e| AppError::storage(format!("{}: {e}", natives_dir.display())))?;
		for jar in &natives {
			extract_natives(&jar.path, &natives_dir, &jar.exclude)
				.map_err(|e| AppError::storage(format!("natives {}: {e}", jar.path.display())))?;
		}
		for dir in [virtual_dir, resources_dir].into_iter().flatten() {
			copy_assets(&index, &layout, &dir)
				.map_err(|e| AppError::storage(format!("ассеты {}: {e}", dir.display())))?;
		}
		Ok(())
	})
	.await
	.map_err(|e| AppError::unknown(e.to_string()))?
}

/// Распаковывает natives (dll) из jar, кроме `META-INF/` и исключений из описания версии.
pub fn extract_natives(jar: &Path, dest: &Path, exclude: &[String]) -> std::io::Result<()> {
	let file = std::fs::File::open(jar)?;
	let mut archive = zip::ZipArchive::new(file).map_err(std::io::Error::other)?;
	for i in 0..archive.len() {
		let mut entry = archive.by_index(i).map_err(std::io::Error::other)?;
		if entry.is_dir() {
			continue;
		}
		let name = entry.name().map_err(std::io::Error::other)?.into_owned();
		if name.starts_with("META-INF/") || exclude.iter().any(|e| name.starts_with(e.as_str())) {
			continue;
		}
		let Some(rel) = entry.enclosed_name() else {
			continue;
		};
		let out = dest.join(rel);
		// Тот же файл уже на месте (и может быть занят запущенной игрой).
		if std::fs::metadata(&out).is_ok_and(|m| m.len() == entry.size()) {
			continue;
		}
		if let Some(parent) = out.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let mut target = std::fs::File::create(&out)?;
		std::io::copy(&mut entry, &mut target)?;
	}
	Ok(())
}

/// Старые версии читают ассеты по именам: копия объектов в `virtual/<индекс>` или `resources/`.
fn copy_assets(index: &AssetIndex, layout: &Layout, dir: &Path) -> std::io::Result<()> {
	for (name, asset) in &index.objects {
		let rel = Path::new(name);
		if rel
			.components()
			.any(|c| !matches!(c, std::path::Component::Normal(_)))
		{
			continue;
		}
		let target = dir.join(rel);
		if std::fs::metadata(&target).is_ok_and(|m| m.len() == asset.size) {
			continue;
		}
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::copy(layout.asset_object(&asset.hash), &target)?;
	}
	Ok(())
}

/// Полная установка файлов сборки. Возвращает, что установлено, и выбранную версию загрузчика.
pub async fn install_files(
	l: &Launcher,
	instance: &Instance,
	mode: InstallMode,
	reporter: &Reporter,
	cancel: &CancellationToken,
) -> Result<(Installed, Option<String>), AppError> {
	let refetch = mode == InstallMode::Reinstall;
	let verify = if mode == InstallMode::Install {
		Verify::Size
	} else {
		Verify::Hash
	};

	reporter.stage(InstallStage::Metadata, 0.0, 3.0);
	let resolved = l
		.meta
		.resolve(
			&instance.game_version,
			instance.loader.kind,
			instance.loader.version.as_deref(),
			refetch,
			cancel,
		)
		.await?;
	let info = &resolved.info;
	let major = info.java_major();
	let component = info.java_version.as_ref().and_then(|j| j.component.clone());

	let java_progress = reporter.stage(InstallStage::Java, 3.0, 15.0);
	let java = resolve_java(
		l,
		instance.java_path.as_deref(),
		major,
		component.as_deref(),
		java_progress,
		cancel,
	)
	.await?;
	let platform = Platform::with_arch(&java.arch);

	let has_processors = info.processors.as_ref().is_some_and(|p| !p.is_empty());
	let download_end = if has_processors { 85.0 } else { 98.0 };
	let progress = reporter.stage(InstallStage::Downloading, 15.0, download_end);
	let game_dir = l.layout.game_dir(&instance.id);
	game_files(
		l,
		&resolved,
		&game_dir,
		platform,
		verify,
		progress.clone(),
		cancel,
	)
	.await?;
	progress.flush();

	if has_processors {
		let progress = reporter.stage(InstallStage::Processors, 85.0, 98.0);
		super::processors::run(l, &resolved, &java, mode, progress, cancel).await?;
	}

	reporter.stage(InstallStage::Finishing, 99.0, 100.0);
	tokio::fs::create_dir_all(&game_dir)
		.await
		.map_err(|e| AppError::storage(format!("{}: {e}", game_dir.display())))?;
	Ok((
		Installed {
			version_id: resolved.version_id.clone(),
			java_major: major,
		},
		resolved.loader_version.clone(),
	))
}

/// Занимает сборку и ставит задачу в список загрузок.
pub async fn begin(l: &Launcher, id: &str, mode: InstallMode) -> Result<Started, AppError> {
	let previous = l.instances.get(id).await?.install_state;
	let _content = l.content_locks.lock(id).await;
	if l.downloads.has_active_content(id) {
		return Err(AppError::coded(
			crate::error::ErrorCode::InstanceBusy,
			id.to_owned(),
		));
	}
	let cancel = l.instances.begin_task(id)?;
	let task_id = l.downloads.begin(
		Target::Instance {
			id: id.to_owned(),
			mode,
		},
		cancel.clone(),
	);
	Ok(Started {
		id: id.to_owned(),
		previous,
		mode,
		cancel,
		task_id,
	})
}

pub struct Started {
	pub id: String,
	previous: InstallState,
	mode: InstallMode,
	cancel: CancellationToken,
	pub task_id: String,
}

/// Установка целиком; освобождает сборку, занятую в [`begin`].
pub async fn run(l: Arc<Launcher>, started: Started, events: Arc<dyn InstallEvents>) {
	let Started {
		id,
		previous,
		mode,
		cancel,
		task_id,
	} = started;
	events.changed(l.instances.list().await);
	let reporter = Reporter::new(id.clone(), events.clone()).tracked(Tracked {
		downloads: l.downloads.clone(),
		task_id: task_id.clone(),
	});

	let result = async {
		let _queue = match l.install_lock.try_lock() {
			Ok(guard) => guard,
			Err(_) => {
				reporter.stage(InstallStage::Queued, 0.0, 0.0);
				tokio::select! {
					_ = cancel.cancelled() => return Err(AppError::new(ErrorCode::Cancelled)),
					g = l.install_lock.lock() => g,
				}
			}
		};
		let instance = l.instances.get(&id).await?;
		log::info!(
			"установка {id}: Minecraft {} {} {:?}, режим {mode:?}",
			instance.game_version,
			instance.loader.kind.as_str(),
			instance.loader.version
		);
		install_files(&l, &instance, mode, &reporter, &cancel).await
	}
	.await;

	let (state, finished) = match result {
		Ok((installed, loader_version)) => {
			let saved = l
				.instances
				.modify(&id, |inst| {
					inst.install_state = InstallState::Installed;
					inst.installed = Some(installed);
					if inst.loader.version.is_none() {
						inst.loader.version = loader_version;
					}
					Ok(())
				})
				.await;
			match saved {
				Ok(_) => (None, (InstallResult::Installed, None)),
				Err(err) => (
					Some(InstallState::Failed),
					(InstallResult::Failed, Some(err)),
				),
			}
		}
		Err(err) if err.code == ErrorCode::Cancelled => {
			let back = if previous == InstallState::Installing {
				InstallState::NotInstalled
			} else {
				previous
			};
			(Some(back), (InstallResult::Cancelled, None))
		}
		Err(err) => (
			Some(InstallState::Failed),
			(InstallResult::Failed, Some(err)),
		),
	};
	if let Some(state) = state {
		let _ = l
			.instances
			.modify(&id, |inst| {
				inst.install_state = state;
				Ok(())
			})
			.await;
	}
	match &finished.1 {
		Some(err) => log::warn!(
			"установка {id}: {:?} {}",
			err.code,
			err.details.as_deref().unwrap_or("")
		),
		None => log::info!("установка {id}: {:?}", finished.0),
	}
	let download_result = match (&finished.0, &finished.1) {
		(InstallResult::Installed, _) => Ok(()),
		(InstallResult::Cancelled, _) => Err(AppError::new(ErrorCode::Cancelled)),
		(InstallResult::Failed, err) => Err(err
			.clone()
			.unwrap_or_else(|| AppError::unknown("установка не удалась"))),
	};
	l.downloads.finish(&task_id, download_result);
	l.instances.end_task(&id);
	events.changed(l.instances.list().await);
	events.finished(InstallFinished {
		instance_id: id,
		result: finished.0,
		error: finished.1,
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	const WIN64: Platform = Platform {
		os: "windows",
		arch: "x64",
	};

	fn info(libraries: serde_json::Value) -> VersionInfo {
		serde_json::from_value(json!({
			"id": "x",
			"assetIndex": { "id": "5", "sha1": "a", "size": 1, "url": "https://x/5.json" },
			"assets": "5",
			"libraries": libraries,
			"mainClass": "M",
			"type": "release"
		}))
		.unwrap()
	}

	#[test]
	fn builds_library_jobs() {
		let layout = Layout::new(PathBuf::from("root"));
		let info = info(json!([
			{ "name": "a:b:1", "downloads": { "artifact": { "path": "a/b/1/b-1.jar", "sha1": "s", "size": 5, "url": "https://libraries.minecraft.net/a/b/1/b-1.jar" } } },
			{ "name": "osx:only:1", "rules": [{ "action": "allow", "os": { "name": "osx" } }], "downloads": { "artifact": { "path": "p", "sha1": "s", "size": 1, "url": "u" } } },
			{ "name": "net.fabricmc:fabric-loader:0.16.14", "url": "https://launcher-meta.modrinth.com/maven/" },
			{ "name": "com.mojang:x:1" },
			{ "name": "com.modrinth.daedalus:forge-installer-extracts:1.20.1-47.4.0:server@lzma", "url": "https://launcher-meta.modrinth.com/maven/", "include_in_classpath": false },
			{ "name": "com.modrinth.daedalus:forge-installer-extracts:1.20.1-47.4.0:client@lzma", "url": "https://launcher-meta.modrinth.com/maven/", "include_in_classpath": false },
			{ "name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4", "natives": { "windows": "natives-windows-${arch}" },
			  "extract": { "exclude": ["META-INF/"] },
			  "downloads": { "classifiers": { "natives-windows-64": { "path": "n/w64.jar", "sha1": "n", "size": 7, "url": "https://libraries.minecraft.net/n/w64.jar" } } } },
			{ "name": "skip:me:1", "downloadable": false }
		]));
		let (jobs, natives) = library_jobs(&info, &layout, WIN64).unwrap();
		let urls: Vec<&str> = jobs.iter().map(|j| j.url.as_str()).collect();
		assert_eq!(
			urls,
			vec![
				"https://libraries.minecraft.net/a/b/1/b-1.jar",
				"https://launcher-meta.modrinth.com/maven/net/fabricmc/fabric-loader/0.16.14/fabric-loader-0.16.14.jar",
				"https://libraries.minecraft.net/com/mojang/x/1/x-1.jar",
				"https://launcher-meta.modrinth.com/maven/com/modrinth/daedalus/forge-installer-extracts/1.20.1-47.4.0/forge-installer-extracts-1.20.1-47.4.0-client.lzma",
				"https://libraries.minecraft.net/n/w64.jar",
			]
		);
		assert_eq!(jobs[0].sha1.as_deref(), Some("s"));
		assert!(jobs[1].sha1.is_none());
		assert_eq!(natives.len(), 1);
		assert_eq!(natives[0].path, layout.libraries_dir().join("n/w64.jar"));
		assert_eq!(natives[0].exclude, vec!["META-INF/"]);
	}

	#[test]
	fn builds_asset_jobs() {
		let layout = Layout::new(PathBuf::from("root"));
		let index: AssetIndex = serde_json::from_value(json!({
			"objects": { "a/b.ogg": { "hash": "50a59a4f56e4046701b758ddbb1c1587efa4cadf", "size": 10 } }
		}))
		.unwrap();
		let jobs = asset_jobs(&index, &layout).unwrap();
		assert_eq!(
			jobs[0].url,
			"https://resources.download.minecraft.net/50/50a59a4f56e4046701b758ddbb1c1587efa4cadf"
		);
		assert!(
			jobs[0]
				.path
				.ends_with("objects/50/50a59a4f56e4046701b758ddbb1c1587efa4cadf")
		);
		assert!(!index.is_virtual);
	}

	#[test]
	fn rejects_unsafe_metadata() {
		let layout = Layout::new(PathBuf::from("root"));
		for hash in [
			"../../../../evil/50a59a4f56e4046701b758ddbb1c1587efa4",
			"50a59a4f56e4046701b758ddbb1c1587efa4cad",
			"",
		] {
			let index: AssetIndex = serde_json::from_value(json!({
				"objects": { "a": { "hash": hash, "size": 1 } }
			}))
			.unwrap();
			assert_eq!(
				asset_jobs(&index, &layout).unwrap_err().code,
				ErrorCode::DownloadFailed
			);
		}
		for path in ["../../evil.jar", "/abs.jar"] {
			let bad = info(json!([
				{ "name": "a:b:1", "downloads": { "artifact": { "path": path, "sha1": "s", "size": 5, "url": "https://x/b.jar" } } }
			]));
			assert_eq!(
				library_jobs(&bad, &layout, WIN64).unwrap_err().code,
				ErrorCode::DownloadFailed
			);
		}
		let bad_native = info(json!([
			{ "name": "n:n:1", "natives": { "windows": "natives-windows" },
			  "downloads": { "classifiers": { "natives-windows": { "path": "../n.jar", "sha1": "n", "size": 7, "url": "https://x/n.jar" } } } }
		]));
		assert!(library_jobs(&bad_native, &layout, WIN64).is_err());

		let mut bad_index = info(json!([]));
		bad_index.asset_index.id = "../../x".into();
		assert_eq!(
			check_version_info(&bad_index).unwrap_err().code,
			ErrorCode::DownloadFailed
		);
		let logging: VersionInfo = serde_json::from_value(json!({
			"id": "x",
			"assetIndex": { "id": "5", "sha1": "a", "size": 1, "url": "https://x/5.json" },
			"assets": "5",
			"libraries": [],
			"mainClass": "M",
			"type": "release",
			"logging": { "client": { "argument": "-D=${path}", "type": "log4j2-xml",
				"file": { "id": "..\\..\\evil.xml", "sha1": "a", "size": 1, "url": "https://x/l.xml" } } }
		}))
		.unwrap();
		assert!(check_version_info(&logging).is_err());
		assert!(check_version_info(&info(json!([]))).is_ok());
	}

	#[test]
	fn extracts_natives_without_meta_inf() {
		use std::io::Write;
		let dir = tempfile::tempdir().unwrap();
		let jar = dir.path().join("n.jar");
		{
			let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
			let opts = zip::write::SimpleFileOptions::default();
			zip.start_file("lwjgl64.dll", opts).unwrap();
			zip.write_all(b"dll").unwrap();
			zip.start_file("META-INF/MANIFEST.MF", opts).unwrap();
			zip.write_all(b"m").unwrap();
			zip.start_file("skip/me.txt", opts).unwrap();
			zip.write_all(b"s").unwrap();
			zip.finish().unwrap();
		}
		let out = dir.path().join("out");
		extract_natives(&jar, &out, &["skip/".into()]).unwrap();
		assert_eq!(std::fs::read(out.join("lwjgl64.dll")).unwrap(), b"dll");
		assert!(!out.join("META-INF").exists());
		assert!(!out.join("skip").exists());
		// Повторная распаковка поверх — без ошибок.
		extract_natives(&jar, &out, &[]).unwrap();
	}
}
