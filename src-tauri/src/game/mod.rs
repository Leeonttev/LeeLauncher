//! Запуск игры: подготовка (Java, память, classpath, аргументы), процесс, лог, время игры.

pub mod args;
pub mod commands;
#[cfg(test)]
mod live_tests;
pub mod logs;
mod options;
mod running;
mod tree;

use std::collections::HashMap;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use self::args::LaunchContext;
use self::logs::{Buffer, LogLine, Parser, Stream};
use crate::accounts::{AccountKind, LaunchCredentials};
use crate::error::{AppError, ErrorCode};
use crate::instances::{GameWindow, InstallState, Instance, now_iso};
use crate::java::{self, JavaInstall};
use crate::launcher::Launcher;
use crate::meta::model::{AssetIndex, VersionInfo};
use crate::meta::rules::Platform;
use crate::paths::Layout;
use crate::settings::Memory;
use crate::sysmem;

pub const EVENT_PROCESS: &str = "instance-process";
pub const EVENT_LOG: &str = "instance-log";
/// Свой лог вывода игры в `<папка игры>/logs/` (у игры там свой `latest.log`).
pub const OUTPUT_LOG: &str = "launcher-output.log";
const OUTPUT_LOG_PREVIOUS: &str = "launcher-output.prev.log";
/// Как часто отправлять накопленные строки лога в интерфейс.
const LOG_FLUSH: Duration = Duration::from_millis(100);
/// Сколько ждать конца вывода после выхода процесса (вывод могут держать его дочерние процессы).
const OUTPUT_DRAIN: Duration = Duration::from_secs(5);
/// Длиннее этого командная строка Windows (32 767 символов) уже не помещается: аргументы JVM
/// уходят в файл `@args` (Java 9+).
const COMMAND_LINE_LIMIT: usize = 30_000;
const ARGFILE: &str = "launch-args.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
	/// Идёт подготовка: аккаунт, Java, файлы.
	Starting,
	/// Процесс игры запущен.
	Running,
	Exited,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEvent {
	pub instance_id: String,
	pub state: ProcessState,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pid: Option<u32>,
	/// С какого аккаунта игра. Нет — аккаунт ещё не выбран (первое `starting`) или запуск сорвался раньше.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub account: Option<GameAccount>,
	/// Код выхода (`exited`). Нет — процесс не запустился или завершён без кода.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub exit_code: Option<i32>,
	/// `exited`: вылет — код не 0 и игру не останавливали из лаунчера.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub crashed: Option<bool>,
	/// `exited`: игру остановили из лаунчера (`instance_kill`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub killed: Option<bool>,
	/// `exited`: сколько шла игра, секунд.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub duration_secs: Option<u64>,
	/// `exited` без запуска процесса: почему не запустилась (та же ошибка, что вернула команда).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<AppError>,
}

impl ProcessEvent {
	fn new(instance_id: &str, state: ProcessState) -> Self {
		Self {
			instance_id: instance_id.to_owned(),
			state,
			pid: None,
			account: None,
			exit_code: None,
			crashed: None,
			killed: None,
			duration_secs: None,
			error: None,
		}
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogBatch {
	pub instance_id: String,
	pub lines: Vec<LogLine>,
}

/// Аккаунт, с которым запущена игра. Снимок на момент запуска: переименование, удаление
/// или смена активного аккаунта его не меняют.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAccount {
	pub id: String,
	/// UUID игрока, с которым запущена игра (по нему голова и у удалённого аккаунта).
	pub uuid: String,
	pub username: String,
	pub kind: AccountKind,
}

impl GameAccount {
	fn of(creds: &LaunchCredentials) -> Self {
		Self {
			id: creds.account_id.clone(),
			uuid: creds.uuid.clone(),
			username: creds.username.clone(),
			kind: creds.kind,
		}
	}
}

/// Запущенная игра для `instances_running`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningGame {
	pub instance_id: String,
	pub state: ProcessState,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pid: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub account: Option<GameAccount>,
	/// ISO 8601: начало запуска.
	pub started_at: String,
	/// Игру запустил прошлый экземпляр лаунчера: лога нет, за процессом следим по PID.
	#[serde(skip_serializing_if = "std::ops::Not::not")]
	pub recovered: bool,
	/// Путь к Java игры: пока она идёт, эту Java не обновляем.
	#[serde(skip)]
	pub java: Option<String>,
}

/// Куда запуск сообщает о себе (в приложении — события Tauri, в тестах — печать).
pub trait GameEvents: Send + Sync {
	fn process(&self, event: ProcessEvent);
	fn log(&self, batch: LogBatch);
	fn changed(&self, instances: Vec<Instance>);
}

/// Запущенные игры и их логи; лежит в [`Launcher`].
pub struct Games {
	running: Mutex<HashMap<String, RunningGame>>,
	/// Лог последнего запуска каждой сборки (живёт до перезапуска лаунчера).
	logs: Mutex<HashMap<String, Arc<Mutex<Buffer>>>>,
	/// `running.json`: чтобы найти игры после перезапуска лаунчера.
	file: running::RunningFile,
}

impl Default for Games {
	fn default() -> Self {
		Self::new(None)
	}
}

impl Games {
	pub fn new(file: Option<PathBuf>) -> Self {
		Self {
			running: Mutex::default(),
			logs: Mutex::default(),
			file: running::RunningFile::new(file),
		}
	}

	/// Идёт игра на Java из этой папки.
	pub fn java_in_use(&self, dir: &Path) -> bool {
		let dir = dir.to_string_lossy().to_lowercase();
		let dir = dir.trim_end_matches(['\\', '/']);
		self.running
			.lock()
			.map(|r| {
				r.values().any(|g| {
					g.java.as_deref().is_some_and(|java| {
						java.to_lowercase()
							.strip_prefix(dir)
							.is_some_and(|rest| rest.starts_with(['\\', '/']))
					})
				})
			})
			.unwrap_or(false)
	}

	pub fn running(&self) -> Vec<RunningGame> {
		self.running
			.lock()
			.map(|r| r.values().cloned().collect())
			.unwrap_or_default()
	}

	fn set_running(&self, game: RunningGame) {
		if let Ok(mut running) = self.running.lock() {
			running.insert(game.instance_id.clone(), game);
		}
	}

	fn clear_running(&self, id: &str) {
		if let Ok(mut running) = self.running.lock() {
			running.remove(id);
		}
	}

	/// Новый пустой буфер лога для запуска.
	fn new_log(&self, id: &str) -> Arc<Mutex<Buffer>> {
		let buffer = Arc::new(Mutex::new(Buffer::default()));
		if let Ok(mut logs) = self.logs.lock() {
			logs.insert(id.to_owned(), buffer.clone());
		}
		buffer
	}

	pub fn log(&self, id: &str) -> Vec<LogLine> {
		let buffer = self.logs.lock().ok().and_then(|l| l.get(id).cloned());
		buffer
			.and_then(|b| b.lock().ok().map(|b| b.lines()))
			.unwrap_or_default()
	}

	pub fn forget(&self, id: &str) {
		if let Ok(mut logs) = self.logs.lock() {
			logs.remove(id);
		}
	}
}

/// Память для игры: своя у сборки → по умолчанию из настроек → автоматически
/// (512 МБ – рекомендуемый максимум для этого компьютера).
pub fn memory(own: Option<Memory>, default: Option<Memory>, total_mb: u64) -> Memory {
	own.or(default).unwrap_or_else(|| {
		let max_mb = sysmem::recommended_max_mb(total_mb);
		Memory {
			min_mb: 512.min(max_mb),
			max_mb,
		}
	})
}

fn missing(details: impl Into<String>) -> AppError {
	AppError::coded(ErrorCode::FilesMissing, details)
}

fn launch_failed(details: impl Into<String>) -> AppError {
	AppError::coded(ErrorCode::LaunchFailed, details)
}

/// Готовая команда запуска.
pub struct Plan {
	pub java: JavaInstall,
	pub args: args::Args,
	/// Все аргументы для лога (токен скрыт).
	pub masked: Vec<String>,
	pub game_dir: PathBuf,
}

/// Файлы и пути для запуска — всё, что читается с диска.
struct DiskState {
	info: VersionInfo,
	classpath: Vec<PathBuf>,
	game_assets: PathBuf,
	log_config: Option<PathBuf>,
}

fn read_disk_state(
	layout: &Layout,
	instance: &Instance,
	version_id: &str,
	platform: Platform,
) -> Result<DiskState, AppError> {
	crate::meta::check_id("версия", version_id)?;
	crate::meta::check_id("версия игры", &instance.game_version)?;
	let json_path = layout.version_json(version_id);
	let bytes =
		std::fs::read(&json_path).map_err(|e| missing(format!("{}: {e}", json_path.display())))?;
	let info: VersionInfo = serde_json::from_slice(&bytes)
		.map_err(|e| missing(format!("{}: {e}", json_path.display())))?;
	crate::meta::check_version_info(&info)?;

	let client = layout.version_jar(&instance.game_version);
	let classpath = args::classpath(&info, &layout.libraries_dir(), &client, platform)?;
	if let Some(absent) = classpath.iter().find(|p| !p.is_file()) {
		return Err(missing(format!("нет файла {}", absent.display())));
	}

	let index_path = layout.asset_index(&info.asset_index.id);
	let index: AssetIndex = std::fs::read(&index_path)
		.ok()
		.and_then(|b| serde_json::from_slice(&b).ok())
		.ok_or_else(|| missing(format!("индекс ассетов {}", index_path.display())))?;
	let game_dir = layout.game_dir(&instance.id);
	let game_assets = if index.map_to_resources {
		game_dir.join("resources")
	} else if index.is_virtual {
		layout.virtual_assets_dir(&info.asset_index.id)
	} else {
		layout.assets_dir()
	};

	let log_config = info
		.logging
		.as_ref()
		.and_then(|l| l.get("client"))
		.map(|c| layout.log_config(&c.file.id))
		.filter(|p| {
			let ok = p.is_file();
			if !ok {
				log::warn!("нет конфига логов {}, запускаем без него", p.display());
			}
			ok
		});

	for dir in [&game_dir, &layout.natives_dir(version_id)] {
		std::fs::create_dir_all(dir)
			.map_err(|e| AppError::storage(format!("{}: {e}", dir.display())))?;
	}
	// «Во весь экран» или свой размер окна — через `options.txt`; без настройки файл не трогаем.
	if let Some(window) = instance.resolution {
		let value = if window.is_fullscreen() {
			"true"
		} else {
			"false"
		};
		if let Err(err) = options::set(&game_dir, &[("fullscreen", value)]) {
			log::warn!("не удалось записать {}: {err}", options::FILE_NAME);
		}
	}
	Ok(DiskState {
		info,
		classpath,
		game_assets,
		log_config,
	})
}

/// Подготовка запуска: Java, память, файлы, аргументы. Сеть не нужна.
pub async fn prepare(
	l: &Launcher,
	instance: &Instance,
	credentials: &LaunchCredentials,
	world: Option<&str>,
) -> Result<Plan, AppError> {
	let installed = match (&instance.installed, instance.install_state) {
		(Some(installed), InstallState::Installed) => installed.clone(),
		_ => {
			return Err(AppError::coded(
				ErrorCode::NotInstalled,
				instance.id.clone(),
			));
		}
	};
	let major = installed.java_major;
	let settings = l.settings.get();
	let configured = settings.java_paths.get(&major.to_string()).cloned();
	let java = java::select(
		instance.java_path.as_deref(),
		configured.as_deref(),
		major,
		&l.layout.java_dir(),
		&l.java_found,
	)
	.await?
	.ok_or_else(|| AppError::coded(ErrorCode::JavaNotFound, format!("Java {major}")))?;
	let platform = Platform::with_arch(&java.arch);

	let disk = {
		let layout = l.layout.clone();
		let instance = instance.clone();
		let version_id = installed.version_id.clone();
		tokio::task::spawn_blocking(move || {
			read_disk_state(&layout, &instance, &version_id, platform)
		})
		.await
		.map_err(|e| AppError::unknown(e.to_string()))??
	};

	let memory = memory(
		instance.memory,
		settings.default_memory,
		sysmem::system_memory().total_mb,
	);
	let user_args = instance
		.jvm_args
		.as_deref()
		.map(args::split_user_args)
		.unwrap_or_default();
	let layout = &l.layout;
	let game_dir = layout.game_dir(&instance.id);
	let client = layout.version_jar(&instance.game_version);
	let natives = layout.natives_dir(&installed.version_id);
	let assets_root = layout.assets_dir();
	let libraries = layout.libraries_dir();
	let ctx = LaunchContext {
		info: &disk.info,
		platform,
		credentials,
		version_name: &installed.version_id,
		client_jar: &client,
		classpath: &disk.classpath,
		game_dir: &game_dir,
		assets_root: &assets_root,
		game_assets: &disk.game_assets,
		libraries_dir: &libraries,
		natives_dir: &natives,
		log_config: disk.log_config.as_deref(),
		memory,
		resolution: instance.resolution.and_then(GameWindow::size),
		user_jvm_args: &user_args,
		security_jvm_args: java::mitigation_args(java.major, settings.java4shell_mitigation),
		quick_play_world: world,
	};
	let args = args::build(&ctx);
	let masked = args::masked(&args.all(), &credentials.access_token);
	Ok(Plan {
		java,
		args,
		masked,
		game_dir,
	})
}

/// Длина командной строки Windows: аргументы через пробел, с кавычками.
fn command_line_len(program: &str, args: &[String]) -> usize {
	program.len() + 3 + args.iter().map(|a| a.len() + 3).sum::<usize>()
}

/// Запускает процесс игры без окна консоли; рабочая папка — папка игры сборки.
fn spawn(plan: &Plan, instance_dir: &Path) -> Result<Child, AppError> {
	let mut jvm = plan.args.jvm.clone();
	let all = plan.args.all();
	if command_line_len(&plan.java.path, &all) > COMMAND_LINE_LIMIT && plan.java.major >= 9 {
		// В файле только аргументы JVM: токена там нет.
		let path = instance_dir.join(ARGFILE);
		std::fs::write(&path, args::argfile_text(&jvm))
			.map_err(|e| AppError::storage(format!("{}: {e}", path.display())))?;
		log::info!(
			"командная строка длинная, аргументы JVM — в {}",
			path.display()
		);
		jvm = vec![format!("@{}", path.display())];
	}
	let mut cmd = Command::new(&plan.java.path);
	cmd.args(&jvm)
		.arg(&plan.args.main_class)
		.args(&plan.args.game)
		.current_dir(&plan.game_dir)
		// Настройки Java задаются в сборке; эти переменные перекрыли бы их.
		.env_remove("_JAVA_OPTIONS")
		.env_remove("JAVA_TOOL_OPTIONS")
		.env_remove("JDK_JAVA_OPTIONS")
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		// Игра живёт дольше лаунчера: при его закрытии процесс не трогаем.
		.kill_on_drop(false);
	#[cfg(windows)]
	{
		const CREATE_NO_WINDOW: u32 = 0x0800_0000;
		cmd.creation_flags(CREATE_NO_WINDOW);
	}
	cmd.spawn()
		.map_err(|e| launch_failed(format!("{}: {e}", plan.java.path)))
}

/// Свой лог вывода: прошлый переименовывается в `launcher-output.prev.log`.
fn open_output_log(game_dir: &Path) -> Option<std::io::BufWriter<std::fs::File>> {
	let dir = game_dir.join("logs");
	let path = dir.join(OUTPUT_LOG);
	let result = std::fs::create_dir_all(&dir).and_then(|_| {
		if path.is_file() {
			let _ = std::fs::rename(&path, dir.join(OUTPUT_LOG_PREVIOUS));
		}
		std::fs::File::create(&path)
	});
	match result {
		Ok(file) => Some(std::io::BufWriter::new(file)),
		Err(err) => {
			log::warn!("не удалось открыть {}: {err}", path.display());
			None
		}
	}
}

/// Возвращается, как только процесс запущен; дальше — события. Сборка помечается запущенной
/// до `credentials`, чтобы двойной клик не запустил игру дважды.
pub async fn launch<F, Fut>(
	l: Arc<Launcher>,
	id: String,
	world: Option<String>,
	events: Arc<dyn GameEvents>,
	credentials: F,
) -> Result<RunningGame, AppError>
where
	F: FnOnce() -> Fut,
	Fut: Future<Output = Result<LaunchCredentials, AppError>>,
{
	let instance = l.instances.get(&id).await?;
	let kill = l.instances.begin_run(&id)?;
	if instance.install_state != InstallState::Installed || instance.installed.is_none() {
		l.instances.end_run(&id);
		return Err(AppError::coded(ErrorCode::NotInstalled, id));
	}
	let started_at = now_iso();
	l.games.set_running(RunningGame {
		instance_id: id.clone(),
		state: ProcessState::Starting,
		pid: None,
		account: None,
		started_at: started_at.clone(),
		recovered: false,
		java: None,
	});
	events.process(ProcessEvent::new(&id, ProcessState::Starting));

	let mut account = None;
	let result = async {
		let creds = credentials().await?;
		// Аккаунт известен — второе `starting`, уже с ним.
		let who = GameAccount::of(&creds);
		account = Some(who.clone());
		l.games.set_running(RunningGame {
			instance_id: id.clone(),
			state: ProcessState::Starting,
			pid: None,
			account: Some(who.clone()),
			started_at: started_at.clone(),
			recovered: false,
			java: None,
		});
		let mut event = ProcessEvent::new(&id, ProcessState::Starting);
		event.account = Some(who);
		events.process(event);
		let plan = prepare(&l, &instance, &creds, world.as_deref()).await?;
		// Java выбрана: с этой минуты её не обновляем (`java_in_use`), хотя процесса ещё нет.
		l.games.set_running(RunningGame {
			instance_id: id.clone(),
			state: ProcessState::Starting,
			pid: None,
			account: account.clone(),
			started_at: started_at.clone(),
			recovered: false,
			java: Some(plan.java.path.clone()),
		});
		log::info!(
			"запуск {id}: Minecraft {} {} {:?}, Java {} ({}), аккаунт {} ({})",
			instance.game_version,
			instance.loader.kind.as_str(),
			instance.loader.version,
			plan.java.version,
			plan.java.path,
			creds.username,
			creds.user_type,
		);
		log::debug!("запуск {id}: аргументы {:?}", plan.masked);
		// Остановили, пока шла подготовка, — процесс не запускаем.
		if kill.is_cancelled() {
			return Err(AppError {
				message: "Запуск отменён".to_owned(),
				..AppError::new(ErrorCode::Cancelled)
			});
		}
		let child = spawn(&plan, &l.layout.instance_dir(&id))?;
		Ok::<_, AppError>((plan, child))
	}
	.await;

	let (plan, child) = match result {
		Ok(ok) => ok,
		Err(err) => {
			log::warn!(
				"запуск {id}: {:?} {}",
				err.code,
				err.details.as_deref().unwrap_or("")
			);
			l.instances.end_run(&id);
			l.games.clear_running(&id);
			let mut event = ProcessEvent::new(&id, ProcessState::Exited);
			event.account = account;
			event.error = Some(err.clone());
			events.process(event);
			return Err(err);
		}
	};
	let started = Instant::now();
	let pid = child.id();
	log::info!("запуск {id}: процесс {pid:?}");

	let buffer = l.games.new_log(&id);
	let output = open_output_log(&plan.game_dir);
	if let Err(err) = l
		.instances
		.modify(&id, |i| {
			i.last_played_at = Some(now_iso());
			Ok(())
		})
		.await
	{
		log::warn!("запуск {id}: не удалось записать время запуска: {err}");
	}
	events.changed(l.instances.list().await);

	let game = RunningGame {
		instance_id: id.clone(),
		state: ProcessState::Running,
		pid,
		account: account.clone(),
		started_at: started_at.clone(),
		recovered: false,
		java: Some(plan.java.path.clone()),
	};
	l.games.set_running(game.clone());
	if let Some(pid) = pid
		&& let Some(created) = tree::created(pid)
	{
		l.games.file.add(running::Record {
			instance_id: id.clone(),
			pid,
			created,
			started_at,
			java: Some(plan.java.path.clone()),
			account: account.clone(),
		});
	}
	let mut event = ProcessEvent::new(&id, ProcessState::Running);
	event.pid = pid;
	event.account = account.clone();
	events.process(event);

	tokio::spawn(watch(Watch {
		l,
		id,
		child,
		kill,
		events,
		buffer,
		output,
		started,
		account,
	}));
	Ok(game)
}

/// Предел строки вывода: без него бинарный мусор без переводов строки съел бы память.
const MAX_LINE_BYTES: usize = 32 * 1024;

/// Строка вывода без перевода строки в конце; обрезанная помечается многоточием.
fn finish_line(buf: &mut Vec<u8>, truncated: bool) -> String {
	while matches!(buf.last(), Some(b'\n' | b'\r')) {
		buf.pop();
	}
	let mut line = String::from_utf8_lossy(buf).into_owned();
	if truncated {
		line.push('…');
	}
	buf.clear();
	line
}

async fn read_stream<R: AsyncRead + Unpin>(
	reader: R,
	stream: Stream,
	tx: mpsc::Sender<(String, Stream)>,
) {
	let mut reader = BufReader::new(reader);
	let mut buf = Vec::new();
	let mut truncated = false;
	loop {
		let chunk = match reader.fill_buf().await {
			Ok(chunk) => chunk,
			Err(err) => {
				log::warn!("чтение вывода игры: {err}");
				break;
			}
		};
		if chunk.is_empty() {
			if !buf.is_empty() || truncated {
				let _ = tx.send((finish_line(&mut buf, truncated), stream)).await;
			}
			break;
		}
		let newline = chunk.iter().position(|&b| b == b'\n');
		let part = &chunk[..newline.unwrap_or(chunk.len())];
		let room = MAX_LINE_BYTES.saturating_sub(buf.len());
		if part.len() > room {
			truncated = true;
		}
		buf.extend_from_slice(&part[..part.len().min(room)]);
		let used = newline.map_or(chunk.len(), |i| i + 1);
		reader.consume(used);
		if newline.is_some() {
			let line = finish_line(&mut buf, truncated);
			truncated = false;
			if tx.send((line, stream)).await.is_err() {
				break;
			}
		}
	}
}

/// Принятые строки: в файл, в буфер, в очередь на отправку.
struct Sink {
	buffer: Arc<Mutex<Buffer>>,
	output: Option<std::io::BufWriter<std::fs::File>>,
	pending: Vec<LogLine>,
}

impl Sink {
	fn accept(&mut self, lines: Vec<LogLine>) {
		if lines.is_empty() {
			return;
		}
		if let Some(file) = self.output.as_mut() {
			let written = lines
				.iter()
				.try_for_each(|line| writeln!(file, "{}", line.format()));
			if let Err(err) = written {
				log::warn!("запись {OUTPUT_LOG}: {err}");
				self.output = None;
			}
		}
		if let Ok(mut buffer) = self.buffer.lock() {
			for line in &lines {
				buffer.push(line.clone());
			}
		}
		self.pending.extend(lines);
	}

	fn flush(&mut self, id: &str, events: &dyn GameEvents) {
		if let Some(file) = self.output.as_mut() {
			let _ = file.flush();
		}
		if !self.pending.is_empty() {
			events.log(LogBatch {
				instance_id: id.to_owned(),
				lines: std::mem::take(&mut self.pending),
			});
		}
	}
}

/// Собирает строки вывода и отправляет их пачками не чаще [`LOG_FLUSH`].
async fn collect(
	id: String,
	mut rx: mpsc::Receiver<(String, Stream)>,
	mut sink: Sink,
	events: Arc<dyn GameEvents>,
) {
	let mut parser = Parser::default();
	let mut tick = tokio::time::interval(LOG_FLUSH);
	tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	loop {
		tokio::select! {
			message = rx.recv() => match message {
				Some((line, stream)) => sink.accept(parser.push(line, stream)),
				None => break,
			},
			_ = tick.tick() => sink.flush(&id, events.as_ref()),
		}
	}
	sink.accept(parser.finish());
	sink.flush(&id, events.as_ref());
}

struct Watch {
	l: Arc<Launcher>,
	id: String,
	child: Child,
	kill: CancellationToken,
	events: Arc<dyn GameEvents>,
	buffer: Arc<Mutex<Buffer>>,
	output: Option<std::io::BufWriter<std::fs::File>>,
	started: Instant,
	account: Option<GameAccount>,
}

/// Следит за процессом: лог, остановка, выход. После выхода — время игры и события.
async fn watch(w: Watch) {
	let Watch {
		l,
		id,
		mut child,
		kill,
		events,
		buffer,
		output,
		started,
		account,
	} = w;
	let (tx, rx) = mpsc::channel(1024);
	if let Some(stdout) = child.stdout.take() {
		tokio::spawn(read_stream(stdout, Stream::Stdout, tx.clone()));
	}
	if let Some(stderr) = child.stderr.take() {
		tokio::spawn(read_stream(stderr, Stream::Stderr, tx.clone()));
	}
	drop(tx);
	let sink = Sink {
		buffer,
		output,
		pending: Vec::new(),
	};
	let mut collector = tokio::spawn(collect(id.clone(), rx, sink, events.clone()));

	let mut killed = false;
	let status = loop {
		tokio::select! {
			status = child.wait() => break status,
			_ = kill.cancelled(), if !killed => {
				killed = true;
				log::info!("игра {id}: остановка");
				// Сначала потомки: запущенный процесс может быть заглушкой, а игра — её дочерним процессом.
				if let Some(pid) = child.id() {
					let stopped = tokio::task::spawn_blocking(move || tree::kill_descendants(pid))
						.await
						.unwrap_or(0);
					if stopped > 0 {
						log::info!("игра {id}: остановлено дочерних процессов: {stopped}");
					}
				}
				if let Err(err) = child.start_kill() {
					log::warn!("игра {id}: не удалось остановить: {err}");
				}
			}
		}
	};
	if tokio::time::timeout(OUTPUT_DRAIN, &mut collector)
		.await
		.is_err()
	{
		log::warn!("игра {id}: вывод не закрылся за {OUTPUT_DRAIN:?}");
	}

	let duration = started.elapsed().as_secs();
	let exit_code = match &status {
		Ok(status) => status.code(),
		Err(err) => {
			log::warn!("игра {id}: не удалось дождаться процесса: {err}");
			None
		}
	};
	finish(
		&l,
		&id,
		events.as_ref(),
		exit_code,
		killed,
		duration,
		account,
	)
	.await;
}

/// После выхода игры: время игры, сборка свободна, события.
async fn finish(
	l: &Launcher,
	id: &str,
	events: &dyn GameEvents,
	exit_code: Option<i32>,
	killed: bool,
	duration: u64,
	account: Option<GameAccount>,
) {
	let crashed = !killed && exit_code != Some(0);
	log::info!(
		"игра {id}: выход с кодом {exit_code:?} через {duration} с, остановлена из лаунчера: {killed}"
	);

	if let Err(err) = l
		.instances
		.modify(id, |i| {
			i.playtime = i.playtime.saturating_add(duration);
			i.last_played_at = Some(now_iso());
			Ok(())
		})
		.await
	{
		log::warn!("игра {id}: не удалось записать время игры: {err}");
	}
	l.games.file.remove(id);
	l.instances.end_run(id);
	l.games.clear_running(id);
	events.changed(l.instances.list().await);
	let mut event = ProcessEvent::new(id, ProcessState::Exited);
	event.exit_code = exit_code;
	event.crashed = Some(crashed);
	event.killed = Some(killed);
	event.duration_secs = Some(duration);
	event.account = account;
	events.process(event);
}

/// Как часто проверять игру, запущенную прошлым экземпляром лаунчера.
const RECOVERED_POLL: Duration = Duration::from_secs(2);

/// Секунды от начала запуска (ISO 8601) до сейчас.
fn seconds_since(started_at: &str) -> u64 {
	chrono::DateTime::parse_from_rfc3339(started_at)
		.map(|t| {
			(chrono::Utc::now() - t.with_timezone(&chrono::Utc))
				.num_seconds()
				.max(0) as u64
		})
		.unwrap_or(0)
}

/// Игры из `running.json`, пережившие прошлый экземпляр лаунчера: снова считаются запущенными,
/// за ними следим по PID. Мёртвые записи убираются. Возвращает число найденных игр.
pub async fn recover(l: &Arc<Launcher>, events: Arc<dyn GameEvents>) -> usize {
	let mut alive: Vec<(running::Record, tree::Watched, CancellationToken)> = Vec::new();
	for record in l.games.file.load() {
		if alive
			.iter()
			.any(|(r, ..)| r.instance_id == record.instance_id)
		{
			continue;
		}
		let Some(watched) = tree::Watched::open(record.pid, record.created) else {
			log::info!(
				"игра {} (процесс {}) уже закрыта",
				record.instance_id,
				record.pid
			);
			continue;
		};
		if l.instances.get(&record.instance_id).await.is_err() {
			continue;
		}
		let Ok(kill) = l.instances.begin_run(&record.instance_id) else {
			continue;
		};
		log::info!(
			"игра {} (процесс {}) идёт с прошлого запуска лаунчера",
			record.instance_id,
			record.pid
		);
		l.games.set_running(RunningGame {
			instance_id: record.instance_id.clone(),
			state: ProcessState::Running,
			pid: Some(record.pid),
			account: record.account.clone(),
			started_at: record.started_at.clone(),
			recovered: true,
			java: record.java.clone(),
		});
		alive.push((record, watched, kill));
	}
	l.games
		.file
		.replace(alive.iter().map(|(r, ..)| r.clone()).collect());
	let count = alive.len();
	for (record, watched, kill) in alive {
		tokio::spawn(watch_recovered(
			l.clone(),
			record,
			Arc::new(watched),
			kill,
			events.clone(),
		));
	}
	count
}

async fn watch_recovered(
	l: Arc<Launcher>,
	record: running::Record,
	watched: Arc<tree::Watched>,
	kill: CancellationToken,
	events: Arc<dyn GameEvents>,
) {
	let id = record.instance_id;
	let mut killed = false;
	let exit_code = loop {
		if let Some(code) = watched.exit_code() {
			break code;
		}
		tokio::select! {
			_ = tokio::time::sleep(RECOVERED_POLL) => {}
			_ = kill.cancelled(), if !killed => {
				killed = true;
				log::info!("игра {id}: остановка (процесс {})", record.pid);
				let w = watched.clone();
				let _ = tokio::task::spawn_blocking(move || w.kill()).await;
			}
		}
	};
	let duration = seconds_since(&record.started_at);
	finish(
		&l,
		&id,
		events.as_ref(),
		exit_code,
		killed,
		duration,
		record.account,
	)
	.await;
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn limits_long_lines() {
		let mut input = b"first\r\n".to_vec();
		input.extend(std::iter::repeat_n(b'x', MAX_LINE_BYTES * 3));
		input.extend_from_slice(b"\nlast");
		let (tx, mut rx) = mpsc::channel(16);
		read_stream(&input[..], Stream::Stdout, tx).await;
		let mut lines = Vec::new();
		while let Some((line, _)) = rx.recv().await {
			lines.push(line);
		}
		assert_eq!(lines.len(), 3);
		assert_eq!(lines[0], "first");
		assert_eq!(lines[1].len(), MAX_LINE_BYTES + '…'.len_utf8());
		assert!(lines[1].ends_with('…'));
		assert_eq!(lines[2], "last");
	}

	#[test]
	fn picks_memory() {
		let own = Memory {
			min_mb: 1024,
			max_mb: 6144,
		};
		let default = Memory {
			min_mb: 512,
			max_mb: 3072,
		};
		assert_eq!(memory(Some(own), Some(default), 16384), own);
		assert_eq!(memory(None, Some(default), 16384), default);
		assert_eq!(
			memory(None, None, 16384),
			Memory {
				min_mb: 512,
				max_mb: 4096
			}
		);
		assert_eq!(
			memory(None, None, 0),
			Memory {
				min_mb: 512,
				max_mb: 2048
			}
		);
	}

	#[test]
	fn process_event_shape() {
		let mut event = ProcessEvent::new("abc", ProcessState::Exited);
		event.exit_code = Some(-1);
		event.crashed = Some(true);
		event.duration_secs = Some(5);
		let json = serde_json::to_value(&event).unwrap();
		assert_eq!(json["instanceId"], "abc");
		assert_eq!(json["state"], "exited");
		assert_eq!(json["exitCode"], -1);
		assert_eq!(json["crashed"], true);
		assert_eq!(json["durationSecs"], 5);
		assert!(json.get("pid").is_none());
		assert!(json.get("error").is_none());
		assert!(json.get("account").is_none());

		event.account = Some(GameAccount {
			id: "acc".into(),
			uuid: "b50ad385-829d-3141-a216-7e7d7539ba7f".into(),
			username: "Steve".into(),
			kind: AccountKind::Offline,
		});
		let json = serde_json::to_value(&event).unwrap();
		assert_eq!(
			json["account"],
			serde_json::json!({
				"id": "acc",
				"uuid": "b50ad385-829d-3141-a216-7e7d7539ba7f",
				"username": "Steve",
				"kind": "offline"
			})
		);
	}

	#[test]
	fn command_line_length() {
		let args = vec!["a".repeat(10), "b b".into()];
		assert_eq!(command_line_len("java", &args), 7 + 13 + 6);
	}

	#[derive(Default)]
	struct Recorder(Mutex<Vec<ProcessEvent>>);

	impl GameEvents for Recorder {
		fn process(&self, event: ProcessEvent) {
			self.0.lock().unwrap().push(event);
		}
		fn log(&self, _: LogBatch) {}
		fn changed(&self, _: Vec<Instance>) {}
	}

	fn steve() -> LaunchCredentials {
		LaunchCredentials {
			account_id: "acc".into(),
			kind: AccountKind::Offline,
			username: "Steve".into(),
			uuid: "b50ad385-829d-3141-a216-7e7d7539ba7f".into(),
			access_token: "0".into(),
			user_type: "legacy",
		}
	}

	/// Сборка, которая считается установленной, но с Java по несуществующему пути:
	/// подготовка гарантированно падает и ничего не запускает.
	async fn broken_instance(l: &Launcher, root: &Path) -> String {
		let loader = crate::instances::Loader {
			kind: crate::meta::LoaderKind::Vanilla,
			version: None,
		};
		let instance = l
			.instances
			.create("test", "1.20.1", loader, None, None)
			.await
			.unwrap();
		let java = root.join("no-java").join("javaw.exe");
		l.instances
			.modify(&instance.id, |i| {
				i.install_state = InstallState::Installed;
				i.installed = Some(crate::instances::Installed {
					version_id: "1.20.1".into(),
					java_major: 17,
				});
				i.java_path = Some(java.to_string_lossy().into_owned());
				Ok(())
			})
			.await
			.unwrap();
		instance.id
	}

	#[tokio::test]
	async fn prepare_failure_reports_account() {
		let dir = tempfile::tempdir().unwrap();
		let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
		let id = broken_instance(&l, dir.path()).await;
		let recorder = Arc::new(Recorder::default());

		let err = launch(l.clone(), id.clone(), None, recorder.clone(), || async {
			Ok(steve())
		})
		.await
		.unwrap_err();

		let who = GameAccount::of(&steve());
		let events = recorder.0.lock().unwrap();
		let states: Vec<_> = events.iter().map(|e| e.state).collect();
		assert_eq!(
			states,
			[
				ProcessState::Starting,
				ProcessState::Starting,
				ProcessState::Exited
			]
		);
		assert_eq!(events[0].account, None);
		assert_eq!(events[1].account.as_ref(), Some(&who));
		assert_eq!(events[2].account.as_ref(), Some(&who));
		assert_eq!(events[2].error.as_ref().map(|e| e.code), Some(err.code));
		assert_ne!(err.code, ErrorCode::NoAccount);
		// Сборка свободна, в запущенных её нет.
		assert!(!l.instances.is_running(&id));
		assert!(l.games.running().is_empty());
	}

	#[tokio::test]
	async fn account_failure_reports_no_account() {
		let dir = tempfile::tempdir().unwrap();
		let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
		let id = broken_instance(&l, dir.path()).await;

		for code in [ErrorCode::NoAccount, ErrorCode::NeedsRelogin] {
			let recorder = Arc::new(Recorder::default());
			let err = launch(
				l.clone(),
				id.clone(),
				None,
				recorder.clone(),
				|| async move { Err(AppError::new(code)) },
			)
			.await
			.unwrap_err();
			assert_eq!(err.code, code);

			let events = recorder.0.lock().unwrap();
			let states: Vec<_> = events.iter().map(|e| e.state).collect();
			assert_eq!(states, [ProcessState::Starting, ProcessState::Exited]);
			assert_eq!(events[0].account, None);
			assert_eq!(events[1].account, None);
			assert_eq!(events[1].error.as_ref().map(|e| e.code), Some(code));
			assert!(!l.instances.is_running(&id));
		}
	}

	/// Игра, пережившая лаунчер: находится по `running.json`, останавливается деревом,
	/// после выхода пишется время игры и убирается запись.
	#[cfg(windows)]
	#[tokio::test(flavor = "multi_thread")]
	async fn recovers_running_game() {
		use std::process::Command as StdCommand;

		let dir = tempfile::tempdir().unwrap();
		let l = Launcher::new(Layout::new(dir.path().to_path_buf())).unwrap();
		let loader = crate::instances::Loader {
			kind: crate::meta::LoaderKind::Vanilla,
			version: None,
		};
		let id = l
			.instances
			.create("test", "1.20.1", loader.clone(), None, None)
			.await
			.unwrap()
			.id;
		let gone = l
			.instances
			.create("gone", "1.20.1", loader, None, None)
			.await
			.unwrap()
			.id;

		let mut child = StdCommand::new("cmd")
			.args(["/d", "/c", "ping -n 60 127.0.0.1"])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.unwrap();
		let pid = child.id();
		let created = tree::created(pid).unwrap();
		let started_at = (chrono::Utc::now() - chrono::Duration::seconds(90))
			.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
		let record = |instance_id: &str, created: u64| running::Record {
			instance_id: instance_id.into(),
			pid,
			created,
			started_at: started_at.clone(),
			java: Some(r"C:\Data\java\java-runtime-delta\bin\javaw.exe".into()),
			account: Some(GameAccount::of(&steve())),
		};
		// Вторая запись — тот же PID с другим временем создания: это уже не наша игра.
		running::RunningFile::new(Some(l.layout.running_file()))
			.replace(vec![record(&id, created), record(&gone, created + 1)]);

		let recorder = Arc::new(Recorder::default());
		assert_eq!(recover(&l, recorder.clone()).await, 1);
		let games = l.games.running();
		assert_eq!(games.len(), 1);
		assert!(games[0].recovered);
		assert_eq!(games[0].pid, Some(pid));
		assert_eq!(games[0].account.as_ref().unwrap().username, "Steve");
		let json = serde_json::to_value(&games[0]).unwrap();
		assert_eq!(json["recovered"], true);
		assert!(json.get("java").is_none());
		assert!(l.instances.is_running(&id));
		assert!(!l.instances.is_running(&gone));
		assert_eq!(
			l.instances.begin_run(&id).unwrap_err().code,
			ErrorCode::AlreadyRunning
		);
		assert!(
			l.games
				.java_in_use(Path::new(r"c:\data\java\java-runtime-delta"))
		);
		assert!(!l.games.java_in_use(Path::new(r"C:\Data\java\java-runtime")));
		assert_eq!(l.games.file.load().len(), 1);

		assert!(l.instances.kill(&id));
		let deadline = Instant::now() + Duration::from_secs(15);
		while l.instances.is_running(&id) && Instant::now() < deadline {
			tokio::time::sleep(Duration::from_millis(100)).await;
		}
		assert!(!l.instances.is_running(&id));
		assert!(l.games.running().is_empty());
		assert!(!l.layout.running_file().exists());
		let _ = child.wait();

		{
			let events = recorder.0.lock().unwrap();
			let exited = events.last().unwrap();
			assert_eq!(exited.state, ProcessState::Exited);
			assert_eq!(exited.killed, Some(true));
			assert_eq!(exited.crashed, Some(false));
			assert!(exited.duration_secs.unwrap() >= 90);
			assert_eq!(exited.account.as_ref().unwrap().username, "Steve");
		}
		let instance = l.instances.get(&id).await.unwrap();
		assert!(instance.playtime >= 90);
		assert!(instance.last_played_at.is_some());
	}

	#[test]
	fn seconds_since_start() {
		let t = (chrono::Utc::now() - chrono::Duration::seconds(30))
			.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
		assert!((29..=31).contains(&seconds_since(&t)));
		assert_eq!(seconds_since("not a date"), 0);
		let future = (chrono::Utc::now() + chrono::Duration::seconds(30))
			.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
		assert_eq!(seconds_since(&future), 0);
	}
}
