//! Модпак из сети: `cargo test live_modpack -- --ignored --nocapture`. Без игры, старая версия
//! и обновление. Убранные файлы уходят в корзину Windows.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::install::{self, InspectSource};
use crate::content::{SourceKind, VersionFilter};
use crate::download::Progress;
use crate::instances::Instance;
use crate::instances::install::{InstallEvents, InstallFinished, InstallProgress, Reporter};
use crate::launcher::Launcher;
use crate::paths::Layout;

struct Quiet;

impl InstallEvents for Quiet {
	fn progress(&self, _: InstallProgress) {}
	fn changed(&self, _: Vec<Instance>) {}
	fn finished(&self, _: InstallFinished) {}
}

/// Fabulously Optimized: модпак Fabric с оптимизацией (несколько десятков модов, ~30 МБ).
const PROJECT: &str = "fabulously-optimized";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "ходит в api.modrinth.com и качает несколько мегабайт"]
async fn live_modpack_install_and_update() {
	let dir = tempfile::tempdir().unwrap();
	let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
	let versions = l
		.sources
		.get(SourceKind::Modrinth)
		.project_versions(PROJECT, &VersionFilter::default())
		.await
		.unwrap();
	assert!(versions.len() >= 2, "у модпака меньше двух версий");
	let (newer, older) = (&versions[0], &versions[1]);
	println!(
		"версии: {} → {}",
		older.version_number, newer.version_number
	);

	let cancel = CancellationToken::new();
	let source = InspectSource::Catalog {
		source: crate::content::SourceKind::Modrinth,
		project_id: PROJECT.into(),
		version_id: Some(older.id.clone()),
	};
	let old = install::prepare(&l, &source, Progress::silent(), &cancel)
		.await
		.unwrap();
	let preview = install::preview(&l, &old, &cancel).await.unwrap();
	println!(
		"{} {}: {} {:?}, модов {}, {} байт, Java {:?}",
		preview.name,
		preview.version_number,
		preview.game_version,
		preview.loader.kind,
		preview.mods,
		preview.download_bytes,
		preview.java_major
	);
	assert!(preview.project.is_some() && preview.mods > 0);

	let instance = l
		.instances
		.create(
			"Live pack",
			&old.requirements.game_version,
			old.requirements.loader.clone(),
			None,
			None,
		)
		.await
		.unwrap();
	let events: Arc<dyn InstallEvents> = Arc::new(Quiet);
	let reporter = Reporter::new(instance.id.clone(), events);
	let instance_dir = l.layout.instance_dir(&instance.id);
	let game_dir = l.layout.game_dir(&instance.id);
	let optional = install::default_optional(&old.index, &super::PackState::default());
	let (state, failed) = install::apply_pack(
		&l,
		&old,
		&optional,
		None,
		false,
		&instance_dir,
		&game_dir,
		&reporter,
		&cancel,
	)
	.await
	.unwrap();
	assert!(failed.is_empty(), "{failed:?}");
	for (path, sha1) in &state.files {
		let file = game_dir.join(path);
		assert_eq!(
			&crate::content::installed::sha1_file(&file).unwrap(),
			sha1,
			"{path}"
		);
	}
	super::save_state(&instance_dir, &state).unwrap();
	let mut instance = l
		.instances
		.modify(&instance.id, |i| {
			i.modpack = Some(old.link.clone());
			Ok(())
		})
		.await
		.unwrap();
	println!(
		"поставлено файлов {}, overrides {}",
		state.files.len(),
		state.overrides.len()
	);
	std::fs::write(game_dir.join("mods/my-own.jar"), b"mine").unwrap();

	let source = InspectSource::Catalog {
		source: crate::content::SourceKind::Modrinth,
		project_id: PROJECT.into(),
		version_id: Some(newer.id.clone()),
	};
	let new = install::prepare(&l, &source, Progress::silent(), &cancel)
		.await
		.unwrap();
	let diff = install::update_preview(&l, &instance, &new, None)
		.await
		.unwrap();
	println!(
		"обновление: +{} ~{} -{}, своих файлов {}",
		diff.added.len(),
		diff.updated.len(),
		diff.removed.len(),
		diff.user_files
	);
	assert_eq!(diff.user_files, 1);
	instance = l.instances.get(&instance.id).await.unwrap();
	let optional = install::default_optional(&new.index, &state);
	let (next, failed) = install::apply_pack(
		&l,
		&new,
		&optional,
		Some(&state),
		false,
		&instance_dir,
		&game_dir,
		&reporter,
		&cancel,
	)
	.await
	.unwrap();
	assert!(failed.is_empty(), "{failed:?}");
	for path in state.files.keys().filter(|p| !next.files.contains_key(*p)) {
		assert!(!game_dir.join(path).exists(), "старый файл остался: {path}");
	}
	for path in next.files.keys() {
		assert!(game_dir.join(path).is_file(), "нет {path}");
	}
	assert!(
		game_dir.join("mods/my-own.jar").is_file(),
		"свой мод удалён"
	);
	let _ = instance;
}

/// Маленький модпак CurseForge (только файлы модпака, без игры) — только с ключом API.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "ходит в api.curseforge.com, нужен ключ API"]
async fn live_curseforge_modpack() {
	if crate::content::curseforge::api_key().is_none() {
		println!("нет ключа CurseForge — пропуск");
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
	let cf = l.sources.get(SourceKind::Curseforge);
	let query: crate::content::SearchQuery = serde_json::from_value(serde_json::json!({
		"source": "curseforge", "type": "modpack", "gameVersions": ["1.20.1"],
		"loaders": ["fabric"], "sort": "downloads", "limit": 30
	}))
	.unwrap();
	let hits = cf.search(&query).await.unwrap().hits;
	// Самый маленький архив среди первых лучших версий.
	let mut best: Option<(String, crate::content::ContentVersion)> = None;
	for h in hits.iter().take(15) {
		let Ok(versions) = cf.project_versions(&h.id, &VersionFilter::default()).await else {
			continue;
		};
		if let Some(v) = crate::content::pick_best(&versions).filter(|v| !v.site_only)
			&& best
				.as_ref()
				.is_none_or(|(_, b)| v.files[0].size < b.files[0].size)
		{
			best = Some((h.title.clone(), v.clone()));
		}
	}
	let (title, version) = best.expect("нет модпака");
	println!(
		"модпак: {title} {} ({} байт)",
		version.version_number, version.files[0].size
	);
	let cancel = CancellationToken::new();
	let source = InspectSource::Catalog {
		source: SourceKind::Curseforge,
		project_id: version.project_id.clone(),
		version_id: Some(version.id.clone()),
	};
	let pack = install::prepare(&l, &source, Progress::silent(), &cancel)
		.await
		.unwrap();
	let preview = install::preview(&l, &pack, &cancel).await.unwrap();
	println!(
		"{}: {} {:?}, модов {}, {} байт, вручную {}, не узнано {}, необязательных {}",
		preview.name,
		preview.game_version,
		preview.loader,
		preview.mods,
		preview.download_bytes,
		preview.manual_count,
		pack.unresolved.len(),
		preview.optional.len()
	);
	assert!(preview.project.is_some());
	let instance = l
		.instances
		.create(
			"Live CF pack",
			&pack.requirements.game_version,
			pack.requirements.loader.clone(),
			None,
			None,
		)
		.await
		.unwrap();
	let events: Arc<dyn InstallEvents> = Arc::new(Quiet);
	let reporter = Reporter::new(instance.id.clone(), events);
	let instance_dir = l.layout.instance_dir(&instance.id);
	let game_dir = l.layout.game_dir(&instance.id);
	let optional = pack.default_optional(&super::PackState::default());
	let (state, failed) = install::apply_pack(
		&l,
		&pack,
		&optional,
		None,
		false,
		&instance_dir,
		&game_dir,
		&reporter,
		&cancel,
	)
	.await
	.unwrap();
	println!(
		"поставлено файлов {}, overrides {}, не скачалось {failed:?}",
		state.files.len(),
		state.overrides.len()
	);
	for (path, sha1) in &state.files {
		let file = game_dir.join(path);
		if file.is_file() {
			assert_eq!(
				&crate::content::installed::sha1_file(&file).unwrap(),
				sha1,
				"{path}"
			);
		}
	}
	assert!(!state.files.is_empty());
}
