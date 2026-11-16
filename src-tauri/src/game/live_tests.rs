//! Настоящий запуск игры во временной папке (окно игры ненадолго появится):
//! `cargo test live_launch -- --ignored --nocapture --test-threads 1`, папка — `LEELAUNCHER_LIVE_DIR`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::logs::LogLine;
use super::{GameEvents, LogBatch, ProcessEvent, ProcessState};
use crate::accounts::LaunchCredentials;
use crate::instances::Instance;
use crate::instances::live_tests::{install, root};
use crate::launcher::Launcher;
use crate::meta::LoaderKind;
use crate::paths::Layout;
use crate::settings::Memory;

#[derive(Default)]
struct Recorder {
	process: Mutex<Vec<ProcessEvent>>,
	lines: Mutex<Vec<LogLine>>,
	batches: Mutex<usize>,
}

impl GameEvents for Recorder {
	fn process(&self, event: ProcessEvent) {
		println!(
			"  процесс: {:?} pid {:?} код {:?} вылет {:?} остановлена {:?} {:?} с",
			event.state,
			event.pid,
			event.exit_code,
			event.crashed,
			event.killed,
			event.duration_secs
		);
		self.process.lock().unwrap().push(event);
	}

	fn log(&self, batch: LogBatch) {
		*self.batches.lock().unwrap() += 1;
		self.lines.lock().unwrap().extend(batch.lines);
	}

	fn changed(&self, _: Vec<Instance>) {}
}

impl Recorder {
	fn has_line(&self, needle: &str) -> bool {
		self.lines
			.lock()
			.unwrap()
			.iter()
			.any(|l| l.text.contains(needle))
	}

	fn exited(&self) -> Option<ProcessEvent> {
		self.process
			.lock()
			.unwrap()
			.iter()
			.find(|e| e.state == ProcessState::Exited)
			.cloned()
	}
}

fn offline() -> LaunchCredentials {
	LaunchCredentials {
		account_id: "test".into(),
		kind: crate::accounts::AccountKind::Offline,
		username: "LeeTest".into(),
		// md5("OfflinePlayer:LeeTest") с битами версии 3 — значение не важно для запуска.
		uuid: "6e2b4e43-7a0b-3d4f-9a52-2f1f7b0c1d2e".into(),
		access_token: "0".into(),
		user_type: "legacy",
	}
}

async fn run_game(l: &Arc<Launcher>, instance: &Instance, needles: &[&str]) {
	l.instances
		.modify(&instance.id, |i| {
			i.memory = Some(Memory {
				min_mb: 512,
				max_mb: 2048,
			});
			Ok(())
		})
		.await
		.unwrap();
	let recorder = Arc::new(Recorder::default());
	let started = Instant::now();
	let game = super::launch(
		l.clone(),
		instance.id.clone(),
		None,
		recorder.clone(),
		|| async { Ok(offline()) },
	)
	.await
	.unwrap();
	println!("== {}: процесс {:?}", instance.name, game.pid);
	assert_eq!(
		super::launch(
			l.clone(),
			instance.id.clone(),
			None,
			recorder.clone(),
			|| async { Ok(offline()) }
		)
		.await
		.unwrap_err()
		.code,
		crate::error::ErrorCode::AlreadyRunning
	);

	// Ждём строк загрузки (до 90 с), игра всё это время должна жить.
	let deadline = Instant::now() + Duration::from_secs(90);
	while !needles.iter().all(|n| recorder.has_line(n)) {
		assert!(
			recorder.exited().is_none(),
			"игра закрылась: {:?}\n{}",
			recorder.exited(),
			tail(&recorder)
		);
		assert!(
			Instant::now() < deadline,
			"нет строк {needles:?}\n{}",
			tail(&recorder)
		);
		tokio::time::sleep(Duration::from_millis(250)).await;
	}
	println!(
		"== {}: строки загрузки через {:?}",
		instance.name,
		started.elapsed()
	);
	// Ещё немного поработать (окно игры уже открыто).
	tokio::time::sleep(Duration::from_secs(10)).await;
	assert!(recorder.exited().is_none(), "{}", tail(&recorder));

	assert!(l.instances.kill(&instance.id));
	let deadline = Instant::now() + Duration::from_secs(20);
	let exited = loop {
		if let Some(e) = recorder.exited() {
			break e;
		}
		assert!(Instant::now() < deadline, "игра не остановилась");
		tokio::time::sleep(Duration::from_millis(100)).await;
	};
	assert_eq!(exited.killed, Some(true));
	assert_eq!(exited.crashed, Some(false));

	let lines = recorder.lines.lock().unwrap().len();
	let batches = *recorder.batches.lock().unwrap();
	let buffered = l.games.log(&instance.id).len();
	println!(
		"== {}: строк {lines} в {batches} пачках, в буфере {buffered}, работала {:?} с",
		instance.name, exited.duration_secs
	);
	assert_eq!(lines, buffered);
	assert!(batches < lines);
	for line in recorder.lines.lock().unwrap().iter().take(8) {
		println!("   {}", line.format());
	}

	let after = l.instances.get(&instance.id).await.unwrap();
	assert!(after.playtime >= 10);
	assert!(after.last_played_at.is_some());
	let output = l
		.layout
		.game_dir(&instance.id)
		.join("logs")
		.join(super::OUTPUT_LOG);
	let text = std::fs::read_to_string(&output).unwrap();
	assert!(text.contains(needles[0]));
	// Повторный запуск после выхода разрешён (сразу же убиваем).
	assert!(l.games.running().is_empty());
}

fn tail(recorder: &Recorder) -> String {
	let lines = recorder.lines.lock().unwrap();
	let start = lines.len().saturating_sub(40);
	lines[start..]
		.iter()
		.map(LogLine::format)
		.collect::<Vec<_>>()
		.join("\n")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "качает ~1 ГБ и открывает окно игры"]
async fn live_launch_vanilla_and_fabric() {
	let (root, _guard) = root();
	println!("папка: {}", root.display());
	let l = Launcher::new(Layout::new(root)).unwrap();

	let vanilla = install(&l, "Vanilla", "1.20.1", LoaderKind::Vanilla, None).await;
	run_game(&l, &vanilla, &["Setting user: LeeTest", "LWJGL"]).await;

	let fabric = install(&l, "Fabric", "1.20.1", LoaderKind::Fabric, None).await;
	run_game(&l, &fabric, &["Loading Minecraft", "Setting user: LeeTest"]).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "качает ~1 ГБ и открывает окно игры"]
async fn live_launch_forge() {
	let (root, _guard) = root();
	let l = Launcher::new(Layout::new(root)).unwrap();
	let forge = install(&l, "Forge", "1.20.1", LoaderKind::Forge, Some("47.4.0")).await;
	run_game(&l, &forge, &["Setting user: LeeTest", "Forge mod loading"]).await;
}
