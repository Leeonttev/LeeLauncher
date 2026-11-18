//! Настоящая установка из сети: `cargo test live_ -- --ignored --nocapture --test-threads 1`,
//! папка — `LEELAUNCHER_LIVE_DIR`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use super::install::{
	self, InstallEvents, InstallFinished, InstallMode, InstallProgress, InstallResult,
};
use super::{InstallState, Instance, Loader};
use crate::download::{self, Progress, Verify};
use crate::launcher::Launcher;
use crate::meta::LoaderKind;
use crate::meta::model::AssetIndex;
use crate::meta::rules::Platform;
use crate::paths::Layout;

#[derive(Default)]
struct Recorder {
	finished: Mutex<Vec<InstallFinished>>,
	last_percent: Mutex<f32>,
}

impl InstallEvents for Recorder {
	fn progress(&self, p: InstallProgress) {
		let mut last = self.last_percent.lock().unwrap();
		if p.percent - *last >= 5.0 || p.percent >= 99.0 {
			println!(
				"  {:?} {:.0}% файлы {}/{} байты {}/{}",
				p.stage, p.percent, p.files_done, p.files_total, p.bytes_done, p.bytes_total
			);
			*last = p.percent;
		}
	}

	fn changed(&self, _: Vec<Instance>) {}

	fn finished(&self, f: InstallFinished) {
		self.finished.lock().unwrap().push(f);
	}
}

pub(crate) fn root() -> (PathBuf, Option<tempfile::TempDir>) {
	match std::env::var_os("LEELAUNCHER_LIVE_DIR") {
		Some(dir) => (PathBuf::from(dir), None),
		None => {
			let dir = tempfile::tempdir().unwrap();
			(dir.path().to_path_buf(), Some(dir))
		}
	}
}

pub(crate) async fn install(
	l: &Arc<Launcher>,
	name: &str,
	game: &str,
	kind: LoaderKind,
	version: Option<&str>,
) -> Instance {
	let instance = l
		.instances
		.create(
			name,
			game,
			Loader {
				kind,
				version: version.map(Into::into),
			},
			None,
			None,
		)
		.await
		.unwrap();
	let recorder = Arc::new(Recorder::default());
	let started = std::time::Instant::now();
	println!("== {name}: Minecraft {game} {}", kind.as_str());
	let task = install::begin(l, &instance.id, InstallMode::Install)
		.await
		.unwrap();
	install::run(l.clone(), task, recorder.clone()).await;
	let finished = recorder.finished.lock().unwrap().pop().unwrap();
	assert_eq!(
		finished.result,
		InstallResult::Installed,
		"{:?}",
		finished.error
	);
	let instance = l.instances.get(&instance.id).await.unwrap();
	assert_eq!(instance.install_state, InstallState::Installed);
	let task = l
		.downloads
		.list()
		.into_iter()
		.find(|t| t.instance_id.as_deref() == Some(instance.id.as_str()))
		.unwrap();
	assert_eq!(task.state, crate::downloads::DownloadState::Finished);
	println!(
		"== {name}: готово за {:?}, {:?}",
		started.elapsed(),
		instance.installed
	);
	instance
}

/// Все файлы версии на месте и совпадают по sha1.
async fn verify(l: &Launcher, instance: &Instance) {
	let installed = instance.installed.clone().unwrap();
	let bytes = std::fs::read(l.layout.version_json(&installed.version_id)).unwrap();
	let info: crate::meta::model::VersionInfo = serde_json::from_slice(&bytes).unwrap();
	let (jobs, natives) = install::library_jobs(&info, &l.layout, Platform::current()).unwrap();
	let mut hashed = 0;
	for job in &jobs {
		assert!(job.path.is_file(), "нет {}", job.path.display());
		if job.sha1.is_some() {
			assert!(
				download::is_valid(job, Verify::Hash).await.unwrap(),
				"sha1 {}",
				job.path.display()
			);
			hashed += 1;
		}
	}
	for jar in &natives {
		assert!(jar.path.is_file());
	}
	let client = l.layout.version_jar(&instance.game_version);
	let expected = &info.downloads["client"].sha1;
	assert_eq!(&download::file_sha1(client).await.unwrap(), expected);

	let index: AssetIndex =
		serde_json::from_slice(&std::fs::read(l.layout.asset_index(&info.asset_index.id)).unwrap())
			.unwrap();
	for asset in index.objects.values() {
		let meta = std::fs::metadata(l.layout.asset_object(&asset.hash)).unwrap();
		assert_eq!(meta.len(), asset.size);
	}
	println!(
		"   проверено: библиотек {} (sha1 у {hashed}), natives {}, ассетов {}",
		jobs.len(),
		natives.len(),
		index.objects.len()
	);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "качает ~1 ГБ из сети"]
async fn live_install_vanilla_fabric_forge() {
	let (root, _guard) = root();
	println!("папка: {}", root.display());
	let l = Launcher::new(Layout::new(root)).unwrap();

	let vanilla = install(&l, "Vanilla", "1.20.1", LoaderKind::Vanilla, None).await;
	verify(&l, &vanilla).await;
	assert_eq!(vanilla.installed.as_ref().unwrap().version_id, "1.20.1");
	assert_eq!(vanilla.installed.as_ref().unwrap().java_major, 17);

	let fabric = install(&l, "Fabric", "1.20.1", LoaderKind::Fabric, None).await;
	verify(&l, &fabric).await;
	assert!(fabric.loader.version.is_some());

	let forge = install(&l, "Forge", "1.20.1", LoaderKind::Forge, Some("47.4.0")).await;
	verify(&l, &forge).await;
	let patched = l
		.layout
		.libraries_dir()
		.join("net/minecraftforge/forge/1.20.1-47.4.0/forge-1.20.1-47.4.0-client.jar");
	assert!(
		patched.is_file(),
		"процессоры не создали {}",
		patched.display()
	);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "качает ~1 ГБ из сети"]
async fn live_install_neoforge_and_old() {
	let (root, _guard) = root();
	let l = Launcher::new(Layout::new(root)).unwrap();
	let neo = install(&l, "NeoForge", "1.21.1", LoaderKind::Neoforge, None).await;
	verify(&l, &neo).await;
	let quilt = install(&l, "Quilt", "1.20.1", LoaderKind::Quilt, None).await;
	verify(&l, &quilt).await;
	// Старые natives (распаковка) и Java 8.
	let old = install(&l, "Old", "1.12.2", LoaderKind::Vanilla, None).await;
	verify(&l, &old).await;
	let natives = l.layout.natives_dir("1.12.2");
	assert!(
		std::fs::read_dir(&natives).unwrap().count() > 0,
		"natives не распакованы"
	);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "качает Java из сети"]
async fn live_java_runtime() {
	let (root, _guard) = root();
	let l = Launcher::new(Layout::new(root)).unwrap();
	let java = crate::java::runtime::install(
		&l.meta,
		&l.dl,
		&l.layout.java_dir(),
		Some("java-runtime-gamma"),
		17,
		Progress::silent(),
		&CancellationToken::new(),
		&|_| false,
	)
	.await
	.unwrap();
	println!("{java:?}");
	assert_eq!(java.major, 17);
	assert!(java.managed);
	let found = crate::java::detect_all(l.layout.java_dir()).await;
	assert!(found.iter().any(|j| j.path == java.path && j.managed));
}
