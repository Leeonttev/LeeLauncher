//! Экспорт в `.mrpack`: известное Modrinth — ссылками, остальное — в `overrides/`.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::launchers::{self, Category};
use super::mrpack::{self, Env, IndexFile};
use crate::content::installed;
use crate::content::{ContentVersion, Side, SourceKind};
use crate::download::Progress;
use crate::error::{AppError, ErrorCode};
use crate::instances::Instance;
use crate::launcher::Launcher;
use crate::meta::LoaderKind;

/// Опознание файлов в каталоге — не дольше этого.
const IDENTIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
	pub category: Category,
	pub count: usize,
	pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFile {
	pub file: String,
	pub size: u64,
}

/// Что войдёт в `.mrpack` (`instance_export_preview`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview {
	/// Только непустые категории.
	pub categories: Vec<CategoryInfo>,
	/// Модов ссылками на Modrinth.
	pub mods_linked: usize,
	/// Моды, которых нет на Modrinth: войдут файлами.
	pub mods_files: Vec<ModFile>,
	/// Выключенные моды (не войдут).
	pub disabled: usize,
	/// Каталог не ответил: все моды войдут файлами.
	pub offline: bool,
}

pub fn sha512_file(path: &Path) -> std::io::Result<String> {
	use sha2::Digest;
	let mut file = std::fs::File::open(path)?;
	let mut hasher = sha2::Sha512::new();
	let mut buf = vec![0u8; 256 * 1024];
	loop {
		let n = file.read(&mut buf)?;
		if n == 0 {
			break;
		}
		hasher.update(&buf[..n]);
	}
	Ok(crate::download::to_hex(&hasher.finalize()))
}

fn dependency_key(kind: LoaderKind) -> Option<&'static str> {
	match kind {
		LoaderKind::Vanilla => None,
		LoaderKind::Fabric => Some("fabric-loader"),
		LoaderKind::Quilt => Some("quilt-loader"),
		LoaderKind::Forge => Some("forge"),
		LoaderKind::Neoforge => Some("neoforge"),
	}
}

fn side_name(side: Side) -> &'static str {
	match side {
		Side::Optional => "optional",
		Side::Unsupported => "unsupported",
		Side::Required | Side::Unknown => "required",
	}
}

/// Включённые файлы контента: ключ, sha1, размер.
async fn content_files(
	l: &Launcher,
	instance: &Instance,
) -> Result<(Vec<(String, String, u64)>, usize), AppError> {
	let _guard = l.content_locks.lock(&instance.id).await;
	let path = installed::index_path(&l.layout.instance_dir(&instance.id));
	let dir = l.layout.game_dir(&instance.id);
	tokio::task::spawn_blocking(move || {
		let mut index = installed::load_index(&path)?;
		let (items, changed) = installed::sync(&dir, &mut index);
		if changed {
			installed::save_index(&path, &index)?;
		}
		let disabled = items
			.iter()
			.filter(|i| !i.enabled && i.key.starts_with("mods/"))
			.count();
		let files = items
			.into_iter()
			.filter(|i| i.enabled && i.world.is_none())
			.map(|i| (i.key, i.sha1, i.size))
			.collect();
		Ok((files, disabled))
	})
	.await
	.map_err(|e| AppError::unknown(e.to_string()))?
}

/// sha1 → версия каталога. Без сети — `None`.
async fn identify(l: &Launcher, sha1s: Vec<String>) -> Option<HashMap<String, ContentVersion>> {
	if sha1s.is_empty() {
		return Some(HashMap::new());
	}
	let catalog = l.sources.get(SourceKind::Modrinth);
	match tokio::time::timeout(
		IDENTIFY_TIMEOUT,
		catalog.identify(&crate::content::FileKey::of_sha1s(&sha1s)),
	)
	.await
	{
		Ok(Ok(found)) => Some(found),
		Ok(Err(err)) => {
			log::warn!("экспорт: файлы не опознаны: {:?}", err.code);
			None
		}
		Err(_) => None,
	}
}

pub async fn preview(l: &Launcher, instance: &Instance) -> Result<ExportPreview, AppError> {
	let (files, disabled) = content_files(l, instance).await?;
	let mods: Vec<&(String, String, u64)> =
		files.iter().filter(|f| f.0.starts_with("mods/")).collect();
	let found = identify(l, mods.iter().map(|f| f.1.clone()).collect()).await;
	let offline = found.is_none();
	let found = found.unwrap_or_default();
	let linked = |sha1: &str| {
		found
			.get(sha1)
			.and_then(|v| v.files.iter().find(|f| f.sha1.eq_ignore_ascii_case(sha1)))
			.is_some_and(|f| mrpack::is_allowed_url(&f.url))
	};
	let game_dir = l.layout.game_dir(&instance.id);
	let categories = tokio::task::spawn_blocking(move || categories(&game_dir))
		.await
		.map_err(|e| AppError::unknown(e.to_string()))?;
	Ok(ExportPreview {
		categories,
		mods_linked: mods.iter().filter(|f| linked(&f.1)).count(),
		mods_files: mods
			.iter()
			.filter(|f| !linked(&f.1))
			.map(|f| ModFile {
				file: f.0.trim_start_matches("mods/").to_owned(),
				size: f.2,
			})
			.collect(),
		disabled,
		offline,
	})
}

/// Непустые категории папки игры: число элементов и размер.
fn categories(game_dir: &Path) -> Vec<CategoryInfo> {
	let mut out: BTreeMap<Category, CategoryInfo> = BTreeMap::new();
	for (name, dir, category) in launchers::top_entries(game_dir) {
		let path = game_dir.join(&name);
		let (count, bytes) = if dir {
			let count = match category {
				Category::Mods => std::fs::read_dir(&path)
					.map(|d| {
						d.flatten()
							.filter(|e| {
								e.file_name()
									.to_string_lossy()
									.to_ascii_lowercase()
									.ends_with(".jar")
							})
							.count()
					})
					.unwrap_or(0),
				Category::Config => 1,
				_ => std::fs::read_dir(&path)
					.map(|d| d.flatten().count())
					.unwrap_or(0),
			};
			(count, launchers::dir_size(&path))
		} else {
			(1, std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0))
		};
		let entry = out.entry(category).or_insert(CategoryInfo {
			category,
			count: 0,
			bytes: 0,
		});
		entry.count += count;
		entry.bytes += bytes;
	}
	out.into_values().filter(|c| c.count > 0).collect()
}

/// Ссылка в индексе для файла контента.
struct Linked {
	rel: String,
	sha1: String,
	url: String,
	size: u64,
	server: Side,
}

/// Собирает `.mrpack` в `dest` (через `<dest>.tmp`). Прогресс — по файлам архива.
#[allow(clippy::too_many_arguments)]
pub async fn export(
	l: &Launcher,
	instance: &Instance,
	name: &str,
	version: &str,
	include: &[Category],
	dest: &Path,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<(), AppError> {
	let mut dependencies = BTreeMap::new();
	dependencies.insert("minecraft".to_owned(), instance.game_version.clone());
	if let Some(key) = dependency_key(instance.loader.kind) {
		let loader = instance
			.loader
			.version
			.clone()
			.ok_or_else(|| AppError::coded(ErrorCode::NotInstalled, instance.id.clone()))?;
		dependencies.insert(key.to_owned(), loader);
	}
	let (files, _) = content_files(l, instance).await?;
	let files: Vec<(String, String, u64)> = files
		.into_iter()
		.filter(|f| {
			let top = f.0.split('/').next().unwrap_or_default();
			launchers::category_of(top, true).is_some_and(|c| include.contains(&c))
		})
		.collect();
	let found = identify(l, files.iter().map(|f| f.1.clone()).collect())
		.await
		.unwrap_or_default();
	let mut project_ids: Vec<String> = found.values().map(|v| v.project_id.clone()).collect();
	project_ids.sort();
	project_ids.dedup();
	let sides: HashMap<String, Side> = if project_ids.is_empty() {
		HashMap::new()
	} else {
		match l
			.sources
			.get(SourceKind::Modrinth)
			.projects(&project_ids)
			.await
		{
			Ok(hits) => hits.into_iter().map(|h| (h.id, h.server_side)).collect(),
			Err(_) => HashMap::new(),
		}
	};
	let linked: Vec<Linked> = files
		.into_iter()
		.filter_map(|(rel, sha1, size)| {
			let version = found.get(&sha1)?;
			let file = version
				.files
				.iter()
				.find(|f| f.sha1.eq_ignore_ascii_case(&sha1))
				.filter(|f| mrpack::is_allowed_url(&f.url))?;
			Some(Linked {
				rel,
				url: file.url.clone(),
				size: if file.size > 0 { file.size } else { size },
				server: sides
					.get(&version.project_id)
					.copied()
					.unwrap_or(Side::Required),
				sha1,
			})
		})
		.collect();
	let game_dir = l.layout.game_dir(&instance.id);
	let name = name.to_owned();
	let version = version.to_owned();
	let include = include.to_vec();
	let dest = dest.to_path_buf();
	let cancel = cancel.clone();
	tokio::task::spawn_blocking(move || {
		write_pack(
			&game_dir,
			&dest,
			&name,
			&version,
			&include,
			linked,
			dependencies,
			&progress,
			&cancel,
		)
	})
	.await
	.map_err(|e| AppError::unknown(e.to_string()))?
}

#[allow(clippy::too_many_arguments)]
fn write_pack(
	game_dir: &Path,
	dest: &Path,
	name: &str,
	version: &str,
	include: &[Category],
	linked: Vec<Linked>,
	dependencies: BTreeMap<String, String>,
	progress: &Progress,
	cancel: &CancellationToken,
) -> Result<(), AppError> {
	let storage =
		|p: &Path, e: &dyn std::fmt::Display| AppError::storage(format!("{}: {e}", p.display()));
	let mut index_files = Vec::new();
	let mut linked_paths = std::collections::HashSet::new();
	for item in linked {
		let path = game_dir.join(&item.rel);
		let Ok(sha512) = sha512_file(&path) else {
			continue;
		};
		index_files.push(IndexFile {
			path: item.rel.clone(),
			hashes: BTreeMap::from([("sha1".into(), item.sha1), ("sha512".into(), sha512)]),
			env: Some(Env {
				client: Some("required".into()),
				server: Some(side_name(item.server).into()),
			}),
			downloads: vec![item.url],
			file_size: item.size,
		});
		linked_paths.insert(item.rel);
	}
	let index = mrpack::Index {
		format_version: 1,
		game: "minecraft".into(),
		version_id: version.into(),
		name: name.into(),
		summary: None,
		files: index_files,
		dependencies,
	};

	let mut overrides = Vec::new();
	for (top, _, category) in launchers::top_entries(game_dir) {
		if !include.contains(&category) {
			continue;
		}
		let mut stack = vec![top];
		while let Some(rel) = stack.pop() {
			let path = game_dir.join(&rel);
			let Ok(meta) = std::fs::symlink_metadata(&path) else {
				continue;
			};
			if meta.is_dir() {
				for e in std::fs::read_dir(&path)
					.map_err(|e| storage(&path, &e))?
					.flatten()
				{
					stack.push(format!("{rel}/{}", e.file_name().to_string_lossy()));
				}
			} else if meta.is_file()
				&& !linked_paths.contains(&rel)
				&& !rel.ends_with(installed::DISABLED_SUFFIX)
			{
				overrides.push(rel);
			}
		}
	}
	overrides.sort();
	progress.add_total(overrides.len() as u64 + 1, 0);

	let tmp = PathBuf::from(format!("{}.tmp", dest.display()));
	let written = (|| -> Result<(), AppError> {
		let file = std::fs::File::create(&tmp).map_err(|e| storage(&tmp, &e))?;
		let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
		let opts = zip::write::SimpleFileOptions::default()
			.compression_method(zip::CompressionMethod::Deflated)
			.large_file(true);
		zip.start_file(mrpack::INDEX_NAME, opts)
			.map_err(|e| storage(&tmp, &e))?;
		let json = serde_json::to_vec_pretty(&index).map_err(|e| storage(&tmp, &e))?;
		zip.write_all(&json).map_err(|e| storage(&tmp, &e))?;
		progress.file_done();
		for rel in &overrides {
			if cancel.is_cancelled() {
				return Err(AppError::new(ErrorCode::Cancelled));
			}
			let path = game_dir.join(rel);
			let mut src = std::fs::File::open(&path).map_err(|e| storage(&path, &e))?;
			zip.start_file(format!("overrides/{rel}"), opts)
				.map_err(|e| storage(&tmp, &e))?;
			std::io::copy(&mut src, &mut zip).map_err(|e| storage(&path, &e))?;
			progress.file_done();
		}
		let mut out = zip.finish().map_err(|e| storage(&tmp, &e))?;
		out.flush().map_err(|e| storage(&tmp, &e))?;
		Ok(())
	})();
	if let Err(err) = written {
		let _ = std::fs::remove_file(&tmp);
		return Err(err);
	}
	std::fs::rename(&tmp, dest).map_err(|e| {
		let _ = std::fs::remove_file(&tmp);
		storage(dest, &e)
	})?;
	progress.flush();
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn writes_readable_pack() {
		let dir = tempfile::tempdir().unwrap();
		let game = dir.path().join("game");
		for (rel, data) in [
			("mods/linked.jar", "abc"),
			("mods/own.jar", "own"),
			("mods/off.jar.disabled", "off"),
			("config/a.toml", "cfg"),
			("saves/W/level.dat", "w"),
			("logs/latest.log", "l"),
		] {
			let p = game.join(rel);
			std::fs::create_dir_all(p.parent().unwrap()).unwrap();
			std::fs::write(p, data).unwrap();
		}
		let cats = categories(&game);
		assert_eq!(
			cats.iter()
				.map(|c| (c.category, c.count))
				.collect::<Vec<_>>(),
			vec![
				(Category::Mods, 2),
				(Category::Config, 1),
				(Category::Saves, 1)
			]
		);
		let dest = dir.path().join("out.mrpack");
		let progress = Progress::silent();
		write_pack(
			&game,
			&dest,
			"Pack",
			"1.0",
			&[Category::Mods, Category::Config],
			vec![Linked {
				rel: "mods/linked.jar".into(),
				sha1: "a9993e364706816aba3e25717850c26c9cd0d89d".into(),
				url: "https://cdn.modrinth.com/data/p/versions/v/linked.jar".into(),
				size: 3,
				server: Side::Unsupported,
			}],
			BTreeMap::from([
				("minecraft".into(), "1.20.1".into()),
				("fabric-loader".into(), "0.15.11".into()),
			]),
			&progress,
			&CancellationToken::new(),
		)
		.unwrap();
		assert_eq!(progress.snapshot().files_done, 3);
		assert!(!PathBuf::from(format!("{}.tmp", dest.display())).exists());

		let index = mrpack::read_index(&dest).unwrap();
		assert_eq!(index.files.len(), 1);
		assert_eq!(index.files[0].hashes["sha512"].len(), 128);
		assert_eq!(
			index.files[0].env.as_ref().unwrap().server.as_deref(),
			Some("unsupported")
		);
		assert_eq!(
			index.requirements().unwrap().loader.kind,
			LoaderKind::Fabric
		);
		let paths = mrpack::override_paths(&dest, &["overrides/"]);
		assert_eq!(paths, vec!["config/a.toml", "mods/own.jar"]);
	}
}
