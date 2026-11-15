//! Проверка каталога Modrinth из сети: `cargo test live_content -- --ignored --nocapture`.
//! Работает во временной папке, данные пользователя не трогает.

use std::collections::HashMap;

use tokio_util::sync::CancellationToken;

use super::install::{self, InstallRequest};
use super::installed;
use super::{ProjectType, SearchQuery, VersionFilter};
use crate::download::Progress;
use crate::instances::Loader;
use crate::launcher::Launcher;
use crate::meta::LoaderKind;
use crate::paths::Layout;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "ходит в api.modrinth.com и качает несколько мегабайт"]
async fn live_content_modrinth() {
	let dir = tempfile::tempdir().unwrap();
	let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
	let source = l.sources.get(super::SourceKind::Modrinth);

	let query: SearchQuery = serde_json::from_value(serde_json::json!({
		"type": "mod", "query": "sodium", "gameVersions": ["1.20.1"], "loaders": ["fabric"], "limit": 5
	}))
	.unwrap();
	let page = source.search(&query).await.unwrap();
	println!(
		"поиск: {} всего, первый {:?}",
		page.total,
		page.hits.first().map(|h| &h.title)
	);
	assert!(page.total > 0 && !page.hits.is_empty());
	assert!(page.hits.iter().all(|h| h.updated_at > 0));

	let project = source.project("sodium").await.unwrap();
	println!(
		"проект: {} от {}, версий {}, галерея {}",
		project.hit.title,
		project.hit.author,
		project.versions_count,
		project.gallery.len()
	);
	assert!(!project.hit.author.is_empty() && project.versions_count > 0);

	let (categories, offline) = source.categories().await.unwrap();
	assert!(!offline && categories.iter().any(|c| c.project_type == "shader"));

	let instance = l
		.instances
		.create(
			"Live",
			"1.20.1",
			Loader {
				kind: LoaderKind::Fabric,
				version: None,
			},
			None,
			None,
		)
		.await
		.unwrap();
	let game_dir = l.layout.game_dir(&instance.id);
	let instance_dir = l.layout.instance_dir(&instance.id);

	// Шейдер: план включает Iris.
	let request: InstallRequest =
		serde_json::from_value(serde_json::json!({ "projectId": "complementary-reimagined" }))
			.unwrap();
	let plan = install::resolve(
		source.as_ref(),
		&instance,
		&game_dir,
		&Default::default(),
		&request,
	)
	.await
	.unwrap();
	println!(
		"шейдер: {} + {:?}",
		plan.target.key(),
		plan.required.iter().map(|r| r.key()).collect::<Vec<_>>()
	);
	assert_eq!(plan.target.hit.project_type, ProjectType::Shader);
	assert!(
		plan.required
			.iter()
			.any(|r| r.hit.id == super::modrinth::IRIS_ID)
	);

	// Мод с обязательной зависимостью (Sodium Extra → Sodium) ставится целиком.
	let request: InstallRequest =
		serde_json::from_value(serde_json::json!({ "projectId": "sodium-extra" })).unwrap();
	let mut index = installed::Index::default();
	let plan = install::resolve(source.as_ref(), &instance, &game_dir, &index, &request)
		.await
		.unwrap();
	let items = plan.items();
	println!(
		"modmenu: {:?}",
		items.iter().map(|i| i.key()).collect::<Vec<_>>()
	);
	assert!(items.iter().any(|i| i.hit.slug == "sodium"));
	let staging = install::download(
		&l.dl,
		&instance_dir,
		&items,
		Progress::silent(),
		&CancellationToken::new(),
	)
	.await
	.unwrap();
	install::apply(&game_dir, &staging, &mut index, &items, &plan.conflicts).unwrap();
	let (list, _) = installed::sync(&game_dir, &mut index);
	assert_eq!(list.len(), items.len());

	// Определение по sha1 и проверка обновлений.
	let sha1s: Vec<String> = list.iter().map(|c| c.sha1.clone()).collect();
	let found = source
		.identify(&super::FileKey::of_sha1s(&sha1s))
		.await
		.unwrap();
	assert_eq!(found.len(), sha1s.len());
	let filter = VersionFilter::for_instance(&instance, ProjectType::Mod);
	let latest: HashMap<_, _> = source
		.latest(&super::FileKey::of_sha1s(&sha1s), &filter)
		.await
		.unwrap();
	assert_eq!(latest.len(), sha1s.len());
	let hits = source
		.projects(&[found.values().next().unwrap().project_id.clone()])
		.await
		.unwrap();
	println!("автор по команде: {}", hits[0].author);
	assert!(!hits[0].author.is_empty());
}

/// Проверка CurseForge из сети — только с ключом (`LEELAUNCHER_CURSEFORGE_KEY` в окружении),
/// иначе пропуск: `cargo test live_curseforge -- --ignored --nocapture --test-threads 1`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "ходит в api.curseforge.com, нужен ключ API"]
async fn live_curseforge_content() {
	if super::curseforge::api_key().is_none() {
		println!("нет ключа CurseForge — пропуск");
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
	assert!(l.sources.enabled(super::SourceKind::Curseforge));
	let source = l.sources.get(super::SourceKind::Curseforge);

	let query: SearchQuery = serde_json::from_value(serde_json::json!({
		"source": "curseforge", "type": "mod", "query": "jei", "gameVersions": ["1.20.1"],
		"loaders": ["forge"], "limit": 5
	}))
	.unwrap();
	let page = source.search(&query).await.unwrap();
	println!(
		"поиск: {} всего, первый {:?}",
		page.total,
		page.hits.first().map(|h| &h.title)
	);
	assert!(!page.hits.is_empty());

	let project = source.project("238222").await.unwrap();
	println!(
		"проект: {} ({}) от {}, файлов {}, галерея {}, сайт {}",
		project.hit.title,
		project.hit.slug,
		project.hit.author,
		project.versions_count,
		project.gallery.len(),
		project.links.site
	);
	assert_eq!(project.hit.slug, "jei");
	let by_slug = source
		.project_by_slug("jei", ProjectType::Mod)
		.await
		.unwrap();
	assert_eq!(by_slug.hit.id, "238222");
	let (categories, _) = source.categories().await.unwrap();
	println!(
		"категорий: {}, с родителем: {}",
		categories.len(),
		categories.iter().filter(|c| c.parent.is_some()).count()
	);
	assert!(categories.iter().any(|c| c.project_type == "mod"));
	assert!(categories.iter().any(|c| c.project_type == "world"));

	// Мод с запретом распространения: ищем среди популярных.
	let popular: SearchQuery = serde_json::from_value(serde_json::json!({
		"source": "curseforge", "type": "mod", "sort": "downloads", "limit": 50
	}))
	.unwrap();
	let hits = source.search(&popular).await.unwrap().hits;
	let blocked: Vec<&str> = hits
		.iter()
		.filter(|h| h.site_only)
		.map(|h| h.title.as_str())
		.collect();
	println!("«только с сайта» среди 50 популярных: {blocked:?}");
	if let Some(h) = hits.iter().find(|h| h.site_only) {
		let versions = source
			.project_versions(&h.id, &VersionFilter::default())
			.await
			.unwrap();
		let v = versions.first().unwrap();
		println!(
			"  {}: siteOnly {}, pageUrl {:?}",
			v.version_number, v.site_only, v.page_url
		);
		assert!(v.site_only && v.page_url.is_some() && v.files[0].url.is_empty());
	}

	// Установка маленького мода во временную сборку и опознание по отпечатку.
	let instance = l
		.instances
		.create(
			"Live CF",
			"1.20.1",
			Loader {
				kind: LoaderKind::Forge,
				version: None,
			},
			None,
			None,
		)
		.await
		.unwrap();
	let game_dir = l.layout.game_dir(&instance.id);
	let instance_dir = l.layout.instance_dir(&instance.id);
	let request: InstallRequest = serde_json::from_value(serde_json::json!({
		"source": "curseforge", "projectId": "238222"
	}))
	.unwrap();
	let mut index = installed::Index::default();
	let plan = install::resolve(source.as_ref(), &instance, &game_dir, &index, &request)
		.await
		.unwrap();
	let items = plan.items();
	println!(
		"план: {:?}, вручную {}",
		items
			.iter()
			.map(|i| (i.key(), i.file.url.clone()))
			.collect::<Vec<_>>(),
		plan.manual().len()
	);
	let staging = install::download(
		&l.dl,
		&instance_dir,
		&items,
		Progress::silent(),
		&CancellationToken::new(),
	)
	.await
	.unwrap();
	install::apply(&game_dir, &staging, &mut index, &items, &plan.conflicts).unwrap();
	let (list, _) = installed::sync(&game_dir, &mut index);
	assert_eq!(list.len(), items.len());
	let jar = game_dir.join(&list[0].key);
	let fp = super::fingerprint::of_file(&jar).unwrap();
	println!(
		"отпечаток {}: {fp}, id мода {:?}",
		list[0].file,
		installed::mod_ids(&jar)
	);
	let keys = vec![super::FileKey {
		sha1: list[0].sha1.clone(),
		fingerprint: Some(fp),
		project_id: Some("238222".into()),
	}];
	let found = source.identify(&keys).await.unwrap();
	assert_eq!(found[&list[0].sha1].project_id, "238222");
	let filter = install::update_filter(&instance, ProjectType::Mod);
	let latest = source.latest(&keys, &filter).await.unwrap();
	println!(
		"последняя: {:?}",
		latest.values().next().map(|v| &v.version_number)
	);
	assert!(!latest.is_empty());
}

/// Поиск «Все» из сети (нужен ключ CurseForge):
/// `cargo test live_curseforge_search_all -- --ignored --nocapture --test-threads 1`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "ходит в api.modrinth.com и api.curseforge.com, нужен ключ API"]
async fn live_curseforge_search_all() {
	if super::curseforge::api_key().is_none() {
		println!("нет ключа CurseForge — пропуск");
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
	for (text, sort) in [
		("sodium", "relevance"),
		("jei", "relevance"),
		("jei", "downloads"),
	] {
		let query: SearchQuery = serde_json::from_value(serde_json::json!({
			"type": "mod", "query": text, "sort": sort, "limit": 10
		}))
		.unwrap();
		let mut cursor: Option<String> = None;
		let mut seen = std::collections::HashSet::new();
		let mut merged = Vec::new();
		for n in 0..3 {
			let sources = l.sources.for_all(ProjectType::Mod);
			assert_eq!(sources.len(), 2);
			let page = super::search_all::search(sources, &query, cursor.as_deref())
				.await
				.unwrap();
			assert!(page.partial.is_empty(), "{:?}", page.partial);
			println!(
				"«{text}» {sort}, порция {n}: {} строк, всего ≈{}",
				page.rows.len(),
				page.total
			);
			for row in &page.rows {
				for h in std::iter::once(&row.hit).chain(&row.twins) {
					assert!(
						seen.insert((h.source, h.id.clone())),
						"повтор {:?} {}",
						h.source,
						h.id
					);
				}
				println!(
					"  {:?} {} «{}» {} {}",
					row.hit.source,
					row.hit.slug,
					row.hit.title,
					row.hit.downloads,
					row.twins
						.iter()
						.map(|t| format!("+ {:?} {} «{}»", t.source, t.slug, t.title))
						.collect::<Vec<_>>()
						.join(" ")
				);
				if !row.twins.is_empty() {
					merged.push(row.hit.slug.clone());
				}
			}
			cursor = page.cursor;
			if cursor.is_none() {
				break;
			}
		}
		println!("склеено: {merged:?}");
		assert!(merged.iter().any(|s| s == text), "{text} не склеился");
	}
}
