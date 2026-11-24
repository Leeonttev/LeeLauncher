//! Java: поиск установленных, проверка версии и разрядности, выбор под версию игры, скачивание.

mod exe;
pub mod runtime;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::error::{AppError, ErrorCode};
use crate::meta::rules::normalize_arch;

/// Файл запуска игры: на Windows `javaw.exe` (без окна консоли).
pub const JAVA_BIN: &str = if cfg!(windows) { "javaw.exe" } else { "java" };
/// Консольный файл: для процессоров Forge и проверки версии.
pub const JAVA_CONSOLE_BIN: &str = if cfg!(windows) { "java.exe" } else { "java" };

/// Обход Java4Shell (CVE-2026-114938): выключает интринсик Base64 HotSpot, в котором переполнение.
const JAVA4SHELL_ARGS: [&str; 2] = ["-XX:+UnlockDiagnosticVMOptions", "-XX:-UseBASE64Intrinsics"];

/// Аргументы JVM против Java4Shell. Только Java 17+: уязвимость там, а Java 8 с незнакомым
/// флагом `-XX` не запускается.
pub fn mitigation_args(major: u32, enabled: bool) -> &'static [&'static str] {
	if enabled && major >= 17 {
		&JAVA4SHELL_ARGS
	} else {
		&[]
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaInstall {
	/// Полный путь к `javaw.exe` (на других ОС — `java`).
	pub path: String,
	/// Полная версия: `17.0.15`, `1.8.0_51`.
	pub version: String,
	pub major: u32,
	/// `x64`, `x86`, `arm64`.
	pub arch: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub vendor: Option<String>,
	/// Скачана лаунчером (лежит в `%APPDATA%\LeeLauncher\java`).
	pub managed: bool,
	/// Только у скачанных нами: есть новая версия этого компонента Java (`java_update`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub update_available: Option<String>,
}

impl JavaInstall {
	/// Консольный `java.exe` рядом с `javaw.exe`.
	pub fn console_path(&self) -> PathBuf {
		let path = Path::new(&self.path);
		path.with_file_name(JAVA_CONSOLE_BIN)
	}
}

/// Мажорная версия из строки версии Java: `1.8.0_51` → 8, `17.0.15` → 17, `8u51` → 8, `25-ea` → 25.
pub fn parse_major(version: &str) -> Option<u32> {
	let first: String = version.chars().take_while(|c| c.is_ascii_digit()).collect();
	let first: u32 = first.parse().ok()?;
	if first == 1 {
		let rest = version.strip_prefix("1.")?;
		let second: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
		return second.parse().ok();
	}
	Some(first)
}

fn version_key(version: &str) -> Vec<u64> {
	version
		.split(|c: char| !c.is_ascii_digit())
		.filter(|s| !s.is_empty())
		.filter_map(|s| s.parse().ok())
		.collect()
}

/// Подходящая Java для игры: та же мажорная версия и нужная разрядность (`required`); лучше родная
/// разрядность ОС, скачанная лаунчером и новее.
pub fn pick(
	installs: &[JavaInstall],
	major: u32,
	os_arch: &str,
	required: Option<&str>,
) -> Option<JavaInstall> {
	let mut list: Vec<&JavaInstall> = installs
		.iter()
		.filter(|j| j.major == major && required.is_none_or(|a| j.arch == a))
		.collect();
	list.sort_by(|a, b| {
		(b.arch == os_arch)
			.cmp(&(a.arch == os_arch))
			.then(b.managed.cmp(&a.managed))
			.then_with(|| version_key(&b.version).cmp(&version_key(&a.version)))
	});
	list.first().map(|j| (*j).clone())
}

struct Probe {
	version: String,
	arch: String,
	vendor: Option<String>,
	/// `java.home` из вывода `java -XshowSettings:properties` (у файла `release` его нет).
	home: Option<String>,
}

/// Файл `release` в папке Java: `JAVA_VERSION="17.0.15"`, `OS_ARCH="amd64"`, `IMPLEMENTOR="..."`.
/// `OS_ARCH` бывает не у всех: у рантаймов Mojang (gamma, delta, epsilon) его нет.
struct Release {
	version: String,
	arch: Option<String>,
	vendor: Option<String>,
}

fn parse_release(text: &str) -> Option<Release> {
	let mut version = None;
	let mut arch = None;
	let mut vendor = None;
	for line in text.lines() {
		let Some((key, value)) = line.split_once('=') else {
			continue;
		};
		let value = value.trim().trim_matches('"').to_owned();
		match key.trim() {
			"JAVA_VERSION" if !value.is_empty() => version = Some(value),
			"OS_ARCH" if !value.is_empty() => arch = Some(normalize_arch(&value).to_owned()),
			"IMPLEMENTOR" => vendor = Some(value),
			_ => {}
		}
	}
	Some(Release {
		version: version?,
		arch,
		vendor,
	})
}

/// Вывод `java -XshowSettings:properties -version` (в stderr): `java.version = 17.0.15`, `os.arch = amd64`.
fn parse_properties(text: &str) -> Option<Probe> {
	let mut version = None;
	let mut arch = None;
	let mut vendor = None;
	let mut home = None;
	for line in text.lines() {
		let Some((key, value)) = line.split_once('=') else {
			continue;
		};
		let value = value.trim().to_owned();
		match key.trim() {
			"java.version" => version = Some(value),
			"os.arch" => arch = Some(value),
			"java.vendor" => vendor = Some(value),
			"java.home" => home = Some(value),
			_ => {}
		}
	}
	Some(Probe {
		version: version?,
		arch: normalize_arch(&arch?).to_owned(),
		vendor,
		home,
	})
}

/// Убирает префикс `\\?\` у путей после `canonicalize` на Windows.
fn simplify(path: PathBuf) -> PathBuf {
	let text = path.to_string_lossy();
	if let Some(rest) = text.strip_prefix(r"\\?\")
		&& rest.as_bytes().get(1) == Some(&b':')
	{
		return PathBuf::from(rest);
	}
	path
}

/// Папка `bin` по пути, который дал пользователь или нашёл поиск: файл java, папка `bin`, папка Java
/// или пакет macOS (`*.jdk/Contents/Home`, `jre.bundle/Contents/Home` у рантайма Mojang).
fn bin_dir(path: &Path) -> Option<PathBuf> {
	if path.is_file() {
		let name = path.file_name()?.to_string_lossy();
		if name != JAVA_BIN
			&& name != JAVA_CONSOLE_BIN
			&& let Some(bundle) = bundle_of(path)
		{
			return bin_dir(&bundle);
		}
		return path.parent().map(Path::to_path_buf);
	}
	let homes = [
		path.to_path_buf(),
		path.join("Contents").join("Home"),
		path.join("jre.bundle").join("Contents").join("Home"),
	];
	if let Some(home) = homes
		.iter()
		.find(|h| h.join("bin").join(JAVA_BIN).is_file())
	{
		return Some(home.join("bin"));
	}
	if path.join(JAVA_BIN).is_file() {
		return Some(path.to_path_buf());
	}
	bundle_of(path).and_then(|bundle| bin_dir(&bundle))
}

/// Пакет macOS (`*.jdk`, `*.jre`, `*.bundle`), внутри которого лежит путь.
fn bundle_of(path: &Path) -> Option<PathBuf> {
	path.ancestors()
		.skip(1)
		.find(|p| {
			p.extension()
				.is_some_and(|e| matches!(e.to_string_lossy().as_ref(), "jdk" | "jre" | "bundle"))
		})
		.map(Path::to_path_buf)
}

/// Заголовок системного окна выбора Java (`java_pick_file`).
pub fn pick_title(lang: crate::lang::Lang) -> &'static str {
	match (lang, cfg!(windows)) {
		(crate::lang::Lang::Ru, true) => "Выберите java.exe или javaw.exe",
		(crate::lang::Lang::En, true) => "Select java.exe or javaw.exe",
		(crate::lang::Lang::Ru, false) => "Выберите Java",
		(crate::lang::Lang::En, false) => "Select Java",
	}
}

/// С какой папки открывать окно выбора Java: обычное место установки, если оно есть.
pub fn pick_start_dir() -> Option<PathBuf> {
	if cfg!(target_os = "macos") {
		let dir = PathBuf::from(MAC_JVM_DIR);
		return dir.is_dir().then_some(dir);
	}
	let program_files = std::env::var_os("ProgramFiles")?;
	["Java", "Eclipse Adoptium", "Zulu", "Microsoft"]
		.iter()
		.map(|name| PathBuf::from(&program_files).join(name))
		.find(|dir| dir.is_dir())
}

const MAC_JVM_DIR: &str = "/Library/Java/JavaVirtualMachines";

/// Папка игры официального лаунчера: на macOS `~/Library/Application Support/minecraft`,
/// на Linux `~/.minecraft`, на Windows `%APPDATA%\.minecraft`.
pub fn official_minecraft_dir() -> Option<PathBuf> {
	if cfg!(target_os = "macos") {
		dirs::data_dir().map(|d| d.join("minecraft"))
	} else if cfg!(windows) {
		dirs::data_dir().map(|d| d.join(".minecraft"))
	} else {
		dirs::home_dir().map(|d| d.join(".minecraft"))
	}
}

/// `/usr/bin/java` на macOS — заглушка: без JDK она показывает системное окно «установите Java».
fn is_apple_stub(bin: &Path) -> bool {
	cfg!(target_os = "macos") && bin == Path::new("/usr/bin")
}

/// x64-Java на Mac с Apple Silicon работает только через Rosetta; её нет — `java_invalid` с
/// `details: "rosetta"`.
pub fn ensure_runnable_arch(arch: &str) -> Result<(), AppError> {
	if cfg!(target_os = "macos")
		&& arch == "x64"
		&& crate::meta::rules::Platform::current().arch == "arm64"
		&& !Path::new("/Library/Apple/usr/share/rosetta/rosetta").exists()
	{
		log::warn!("x64-Java без Rosetta не запустится");
		return Err(AppError::coded(ErrorCode::JavaInvalid, "rosetta"));
	}
	Ok(())
}

/// Проверяет Java по пути. Для заглушек вроде Oracle `javapath` отдаёт настоящую Java из `java.home`.
pub fn check_blocking(path: &Path, managed_root: &Path) -> Result<JavaInstall, AppError> {
	let java = check_at(path, managed_root)?;
	if let Some(home) = java.1 {
		let real = simplify(std::fs::canonicalize(&home).unwrap_or(home));
		let current = Path::new(&java.0.path).parent().and_then(Path::parent);
		if current != Some(real.as_path())
			&& let Ok((resolved, _)) = check_at(&real, managed_root)
			&& resolved.major == java.0.major
		{
			log::debug!("java {}: настоящая Java — {}", java.0.path, resolved.path);
			return Ok(resolved);
		}
	}
	Ok(java.0)
}

/// Проверка без замены заглушки; второе — `java.home`, если версию узнавали запуском.
fn check_at(path: &Path, managed_root: &Path) -> Result<(JavaInstall, Option<PathBuf>), AppError> {
	let invalid =
		|why: &str| AppError::coded(ErrorCode::JavaInvalid, format!("{}: {why}", path.display()));
	let bin = bin_dir(path).ok_or_else(|| invalid("нет java"))?;
	let bin = simplify(std::fs::canonicalize(&bin).map_err(|e| invalid(&e.to_string()))?);
	if is_apple_stub(&bin) {
		return Err(invalid("заглушка macOS, выберите JDK"));
	}
	let exe = bin.join(JAVA_BIN);
	if !exe.is_file() {
		return Err(invalid("нет java"));
	}
	let home = bin
		.parent()
		.map(Path::to_path_buf)
		.unwrap_or_else(|| bin.clone());
	let release = std::fs::read_to_string(home.join("release"))
		.ok()
		.and_then(|t| parse_release(&t));
	let probe = match release {
		Some(release) => {
			// Без `OS_ARCH`: отметка нашей Java → заголовок файла → запуск. Обычно JVM не запускается.
			let arch = release
				.arch
				.or_else(|| managed_arch(&exe, managed_root))
				.or_else(|| exe::arch_of(&exe).map(str::to_owned));
			match arch {
				Some(arch) => Some(Probe {
					version: release.version,
					arch,
					vendor: release.vendor,
					home: None,
				}),
				None => run_probe(&bin.join(JAVA_CONSOLE_BIN)),
			}
		}
		None => run_probe(&bin.join(JAVA_CONSOLE_BIN)),
	}
	.ok_or_else(|| invalid("не удалось узнать версию"))?;
	let major = parse_major(&probe.version).ok_or_else(|| invalid("неизвестная версия"))?;
	let managed_root = std::fs::canonicalize(managed_root).map(simplify).ok();
	let java_home = probe.home.map(PathBuf::from);
	Ok((
		JavaInstall {
			path: exe.to_string_lossy().into_owned(),
			version: probe.version,
			major,
			arch: probe.arch,
			vendor: probe.vendor,
			managed: managed_root.is_some_and(|root| exe.starts_with(root)),
			update_available: None,
		},
		java_home,
	))
}

/// Разрядность из отметки скачанной нами Java; отметка без `arch` — родная для ОС.
fn managed_arch(exe: &Path, managed_root: &Path) -> Option<String> {
	let home = runtime::home_of(&exe.to_string_lossy(), managed_root)?;
	runtime::marker(&home).map(|m| m.arch().to_owned())
}

#[cfg(test)]
thread_local! {
	static PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn run_probe(java: &Path) -> Option<Probe> {
	#[cfg(test)]
	PROBES.with(|n| n.set(n.get() + 1));
	let mut cmd = std::process::Command::new(java);
	cmd.args(["-XshowSettings:properties", "-version"])
		.env_remove("_JAVA_OPTIONS")
		.env_remove("JAVA_TOOL_OPTIONS");
	no_window(&mut cmd);
	let text = stderr_with_timeout(cmd, PROBE_TIMEOUT)?;
	parse_properties(&text)
}

/// Сколько ждём `java -version`: зависшая «java» (не та программа) не должна держать поиск.
/// Первый запуск свежей Java на Windows с Defender (проверка каждой DLL) бывает долгим.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Больше этого из stderr не храним (остальное читается и выбрасывается, чтобы процесс не встал).
const PROBE_OUTPUT_LIMIT: usize = 1024 * 1024;

/// stderr процесса. Читаем, пока он работает: иначе он встанет, заполнив буфер канала (~4 КБ).
fn stderr_with_timeout(mut cmd: std::process::Command, timeout: Duration) -> Option<String> {
	use std::io::Read;
	use std::process::Stdio;

	cmd.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::piped());
	let mut child = cmd.spawn().ok()?;
	let Some(mut stderr) = child.stderr.take() else {
		let _ = child.kill();
		let _ = child.wait();
		return None;
	};
	let (tx, rx) = std::sync::mpsc::channel();
	std::thread::spawn(move || {
		let mut kept = Vec::new();
		let mut chunk = [0u8; 8192];
		loop {
			match stderr.read(&mut chunk) {
				Ok(0) | Err(_) => break,
				Ok(n) => {
					let room = PROBE_OUTPUT_LIMIT.saturating_sub(kept.len());
					kept.extend_from_slice(&chunk[..n.min(room)]);
				}
			}
		}
		let _ = tx.send(kept);
	});
	let deadline = std::time::Instant::now() + timeout;
	let exited = loop {
		match child.try_wait() {
			Ok(Some(_)) => break true,
			Ok(None) if std::time::Instant::now() < deadline => {
				std::thread::sleep(Duration::from_millis(20));
			}
			_ => break false,
		}
	};
	if !exited {
		let _ = child.kill();
		let _ = child.wait();
		return None;
	}
	// Канал stderr может держать открытым унаследовавший его внук: ждём конца вывода недолго.
	let bytes = rx.recv_timeout(Duration::from_secs(2)).ok()?;
	Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Не показывать окно консоли у дочернего процесса на Windows.
pub fn no_window(cmd: &mut std::process::Command) {
	#[cfg(windows)]
	{
		use std::os::windows::process::CommandExt;
		const CREATE_NO_WINDOW: u32 = 0x0800_0000;
		cmd.creation_flags(CREATE_NO_WINDOW);
	}
	#[cfg(not(windows))]
	let _ = cmd;
}

pub async fn check(path: PathBuf, managed_root: PathBuf) -> Result<JavaInstall, AppError> {
	tokio::task::spawn_blocking(move || check_blocking(&path, &managed_root))
		.await
		.map_err(|e| AppError::unknown(e.to_string()))?
}

/// Java без скачивания: своя у сборки (как есть) → из настроек → лучшая из найденных.
/// `arch` — разрядность, без которой игра не запустится (natives только x64), её требуют
/// настройки и найденные; своя Java сборки берётся как есть.
pub async fn select(
	own: Option<&str>,
	configured: Option<&str>,
	major: u32,
	arch: Option<&str>,
	managed_root: &Path,
	cache: &JavaCache,
) -> Result<Option<JavaInstall>, AppError> {
	if let Some(path) = own {
		let found = check(PathBuf::from(path), managed_root.to_path_buf()).await?;
		if found.major != major {
			log::warn!(
				"своя Java сборки {} версии {}, игре нужна {major}",
				found.path,
				found.major
			);
		}
		if arch.is_some_and(|a| a != found.arch) {
			log::warn!(
				"своя Java сборки {} для {}, игре нужна {arch:?}",
				found.path,
				found.arch
			);
		}
		return Ok(Some(found));
	}
	if let Some(path) = configured {
		match check(PathBuf::from(path), managed_root.to_path_buf()).await {
			Ok(found) if found.major == major && arch.is_none_or(|a| a == found.arch) => {
				return Ok(Some(found));
			}
			Ok(found) if found.major == major => log::warn!(
				"Java из настроек для {major} — {}, игре нужна {arch:?}: {}",
				found.arch,
				found.path
			),
			Ok(found) => log::warn!(
				"Java из настроек для {major} оказалась {}: {}",
				found.major,
				found.path
			),
			Err(err) => log::warn!("Java из настроек для {major} не работает: {err}"),
		}
	}
	let host = crate::meta::rules::Platform::current().arch;
	let cached = cache.get(managed_root).await;
	if let Some(found) = pick(&cached, major, host, arch)
		&& Path::new(&found.path).is_file()
	{
		return Ok(Some(found));
	}
	// Подходящей нет или её удалили: список мог устареть, ищем заново.
	let fresh = cache.refresh(managed_root).await;
	Ok(pick(&fresh, major, host, arch))
}

/// Кэш найденных Java: поиск запускает каждую, это небыстро.
#[derive(Default)]
pub struct JavaCache(tokio::sync::Mutex<Option<Vec<JavaInstall>>>);

impl JavaCache {
	/// Список из кэша; если его нет — поиск.
	pub async fn get(&self, managed_root: &Path) -> Vec<JavaInstall> {
		let mut slot = self.0.lock().await;
		if let Some(list) = slot.as_ref() {
			return list.clone();
		}
		let list = detect_all(managed_root.to_path_buf()).await;
		*slot = Some(list.clone());
		list
	}

	/// Поиск заново, результат — в кэш.
	pub async fn refresh(&self, managed_root: &Path) -> Vec<JavaInstall> {
		let mut slot = self.0.lock().await;
		let list = detect_all(managed_root.to_path_buf()).await;
		*slot = Some(list.clone());
		list
	}

	pub async fn invalidate(&self) {
		*self.0.lock().await = None;
	}
}

/// Все Java на компьютере: PATH, JAVA_HOME, реестр, стандартные папки, `~/.jdks`,
/// Java официального лаунчера и скачанные нами. Без повторов.
pub async fn detect_all(managed_root: PathBuf) -> Vec<JavaInstall> {
	tokio::task::spawn_blocking(move || {
		let candidates = candidates(&managed_root);
		let mut seen = HashSet::new();
		let mut handles = Vec::new();
		for path in candidates {
			let root = managed_root.clone();
			handles.push(std::thread::spawn(move || {
				check_blocking(&path, &root).ok()
			}));
		}
		let mut found = Vec::new();
		for handle in handles {
			if let Ok(Some(java)) = handle.join()
				&& seen.insert(java.path.to_lowercase())
			{
				found.push(java);
			}
		}
		found.sort_by(|a, b| {
			b.major
				.cmp(&a.major)
				.then_with(|| version_key(&b.version).cmp(&version_key(&a.version)))
		});
		found
	})
	.await
	.unwrap_or_default()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
	std::fs::read_dir(dir)
		.map(|it| {
			it.flatten()
				.map(|e| e.path())
				.filter(|p| p.is_dir())
				.collect()
		})
		.unwrap_or_default()
}

fn candidates(managed_root: &Path) -> Vec<PathBuf> {
	let mut out: Vec<PathBuf> = Vec::new();
	if let Some(path) = std::env::var_os("PATH") {
		out.extend(
			std::env::split_paths(&path)
				.filter(|p| !is_apple_stub(p) && p.join(JAVA_BIN).is_file()),
		);
	}
	if let Some(home) = std::env::var_os("JAVA_HOME") {
		out.push(PathBuf::from(home));
	}
	out.extend(
		subdirs(managed_root)
			.into_iter()
			.filter(|d| runtime::is_complete(d) && !runtime::is_hidden(d)),
	);
	if let Some(home) = dirs::home_dir() {
		out.extend(subdirs(&home.join(".jdks")));
	}
	// Java официального лаунчера: runtime/<компонент>/<платформа>/<компонент>.
	if let Some(game) = official_minecraft_dir() {
		for component in subdirs(&game.join("runtime")) {
			for platform in subdirs(&component) {
				out.extend(subdirs(&platform));
			}
		}
	}
	out.extend(platform_candidates());
	let mut seen = HashSet::new();
	out.retain(|p| seen.insert(p.clone()));
	out
}

#[cfg(windows)]
fn platform_candidates() -> Vec<PathBuf> {
	use winreg::RegKey;
	use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};

	let mut out = Vec::new();
	let vendors = [
		"Java",
		"Eclipse Adoptium",
		"Eclipse Foundation",
		"AdoptOpenJDK",
		"Zulu",
		"Microsoft",
		"BellSoft",
		"Amazon Corretto",
		"Semeru",
		"ojdkbuild",
	];
	for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
		if let Some(base) = std::env::var_os(var) {
			for vendor in vendors {
				out.extend(subdirs(&PathBuf::from(&base).join(vendor)));
			}
		}
	}
	let keys = [
		r"SOFTWARE\JavaSoft\Java Runtime Environment",
		r"SOFTWARE\JavaSoft\Java Development Kit",
		r"SOFTWARE\JavaSoft\JRE",
		r"SOFTWARE\JavaSoft\JDK",
		r"SOFTWARE\Eclipse Foundation\JDK",
		r"SOFTWARE\Eclipse Adoptium\JRE",
		r"SOFTWARE\Eclipse Adoptium\JDK",
		r"SOFTWARE\Azul Systems\Zulu",
		r"SOFTWARE\Microsoft\JDK",
		r"SOFTWARE\BellSoft\Liberica",
	];
	let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
	for key in keys {
		for view in [KEY_WOW64_32KEY, KEY_WOW64_64KEY] {
			let Ok(root) = hklm.open_subkey_with_flags(key, KEY_READ | view) else {
				continue;
			};
			for name in root.enum_keys().flatten() {
				let Ok(sub) = root.open_subkey(&name) else {
					continue;
				};
				for value in ["JavaHome", "InstallationPath"] {
					if let Ok(path) = sub.get_value::<String, _>(value) {
						out.push(PathBuf::from(path));
					}
				}
				// Adoptium: <версия>\hotspot\MSI\Path.
				if let Ok(msi) = sub.open_subkey(r"hotspot\MSI")
					&& let Ok(path) = msi.get_value::<String, _>("Path")
				{
					out.push(PathBuf::from(path));
				}
			}
		}
	}
	out
}

#[cfg(target_os = "linux")]
fn platform_candidates() -> Vec<PathBuf> {
	let mut out = Vec::new();
	for base in [
		"/usr/lib/jvm",
		"/usr/lib64/jvm",
		"/usr/java",
		"/opt/jdk",
		"/opt/jdks",
	] {
		out.extend(subdirs(Path::new(base)));
	}
	out
}

#[cfg(target_os = "macos")]
fn platform_candidates() -> Vec<PathBuf> {
	let mut out = subdirs(Path::new(MAC_JVM_DIR));
	if let Some(home) = dirs::home_dir() {
		out.extend(subdirs(&home.join("Library/Java/JavaVirtualMachines")));
		out.extend(subdirs(&home.join(".sdkman/candidates/java")));
	}
	for brew in ["/opt/homebrew/opt", "/usr/local/opt"] {
		out.extend(
			subdirs(Path::new(brew))
				.into_iter()
				.filter(|p| {
					p.file_name()
						.is_some_and(|n| n.to_string_lossy().starts_with("openjdk"))
				})
				.map(|p| p.join("libexec/openjdk.jdk")),
		);
	}
	out
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn platform_candidates() -> Vec<PathBuf> {
	Vec::new()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn java(version: &str, arch: &str, managed: bool) -> JavaInstall {
		JavaInstall {
			path: format!("{version}-{arch}-{managed}"),
			version: version.into(),
			major: parse_major(version).unwrap(),
			arch: arch.into(),
			vendor: None,
			managed,
			update_available: None,
		}
	}

	#[test]
	fn java4shell_only_for_17_plus() {
		for major in [8, 11, 16] {
			assert!(mitigation_args(major, true).is_empty(), "Java {major}");
		}
		for major in [17, 21, 25] {
			assert_eq!(mitigation_args(major, true), JAVA4SHELL_ARGS);
		}
		assert!(mitigation_args(21, false).is_empty());
	}

	#[test]
	fn majors() {
		assert_eq!(parse_major("1.8.0_51"), Some(8));
		assert_eq!(parse_major("17.0.15"), Some(17));
		assert_eq!(parse_major("21"), Some(21));
		assert_eq!(parse_major("25-ea"), Some(25));
		assert_eq!(parse_major("8u51-cacert462b08"), Some(8));
		assert_eq!(parse_major("16.0.1.9.1"), Some(16));
		assert_eq!(parse_major("abc"), None);
	}

	#[test]
	fn release_file() {
		let probe = parse_release(
			"IMPLEMENTOR=\"Microsoft\"\nJAVA_VERSION=\"17.0.15\"\nOS_ARCH=\"amd64\"\nOS_NAME=\"Windows\"",
		)
		.unwrap();
		assert_eq!(probe.version, "17.0.15");
		assert_eq!(probe.arch.as_deref(), Some("x64"));
		assert_eq!(probe.vendor.as_deref(), Some("Microsoft"));
		// Рантаймы Mojang (gamma, delta, epsilon): только версия и модули, без `OS_ARCH`.
		let mojang = parse_release(
			"JAVA_VERSION=\"25.0.1\"\nMODULES=\"java.base java.compiler jdk.zipfs\"\n",
		)
		.unwrap();
		assert_eq!(mojang.version, "25.0.1");
		assert_eq!(mojang.arch, None);
		let short = parse_release("JAVA_VERSION=\"25\"").unwrap();
		assert_eq!(parse_major(&short.version), Some(25));
		assert!(parse_release("OS_ARCH=\"amd64\"").is_none());
		assert!(parse_release("JAVA_VERSION=\"\"").is_none());
	}

	fn probes() -> usize {
		PROBES.with(|n| n.get())
	}

	/// Java без `OS_ARCH` в `release`: `<root>/<name>/bin/<java>` с заданным содержимым файла.
	fn bare_java(root: &Path, name: &str, version: &str, exe: &[u8]) -> PathBuf {
		let home = root.join(name);
		std::fs::create_dir_all(home.join("bin")).unwrap();
		std::fs::write(home.join("bin").join(JAVA_BIN), exe).unwrap();
		std::fs::write(
			home.join("release"),
			format!("JAVA_VERSION=\"{version}\"\nMODULES=\"java.base\"\n"),
		)
		.unwrap();
		home
	}

	#[test]
	fn arch_from_marker_without_running_java() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("java");
		let arm = exe::tests::macho_header(0x0100_000c);
		let home = bare_java(&root, "java-runtime-epsilon", "25.0.1", &arm);
		std::fs::write(
			home.join(".leelauncher-runtime.json"),
			r#"{"component":"java-runtime-epsilon","version":"25.0.1","arch":"x86"}"#,
		)
		.unwrap();
		let before = probes();
		let java = check_blocking(&home, &root).unwrap();
		assert_eq!((java.major, java.arch.as_str()), (25, "x86"));
		assert!(java.managed);
		// Старая отметка без `arch` — родная разрядность.
		std::fs::write(
			home.join(".leelauncher-runtime.json"),
			r#"{"component":"java-runtime-epsilon","version":"25.0.1"}"#,
		)
		.unwrap();
		let java = check_blocking(&home, &root).unwrap();
		assert_eq!(java.arch, crate::meta::rules::Platform::current().arch);
		assert_eq!(probes(), before);
	}

	#[test]
	fn arch_from_exe_header_without_running_java() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("managed");
		let before = probes();
		let cases = [
			("pe-x64", exe::tests::pe_header(0x8664), "x64"),
			("pe-arm64", exe::tests::pe_header(0xaa64), "arm64"),
			("macho-x64", exe::tests::macho_header(0x0100_0007), "x64"),
		];
		for (name, header, arch) in cases {
			let home = bare_java(dir.path(), name, "21.0.7", &header);
			let java = check_blocking(&home, &root).unwrap();
			assert_eq!((java.major, java.arch.as_str()), (21, arch), "{name}");
			assert!(!java.managed);
		}
		assert_eq!(probes(), before);
		// Ни отметки, ни понятного заголовка — запасной запуск (здесь не запустится).
		let home = bare_java(dir.path(), "garbage", "21.0.7", b"not java");
		let err = check_blocking(&home, &root).unwrap_err();
		assert_eq!(err.code, ErrorCode::JavaInvalid);
		let details = err.details.unwrap_or_default();
		assert!(
			details.contains("garbage") && details.contains("не удалось узнать версию"),
			"{details}"
		);
		assert_eq!(probes(), before + 1);
	}

	/// Скачанная лаунчером Java (`LEELAUNCHER_JAVA_DIR`, папка `java` данных) проверяется без запуска.
	#[test]
	fn checks_downloaded_runtimes() {
		let Some(root) = std::env::var_os("LEELAUNCHER_JAVA_DIR").map(PathBuf::from) else {
			return;
		};
		let before = probes();
		for dir in std::fs::read_dir(&root).unwrap().flatten() {
			let dir = dir.path();
			if !runtime::is_complete(&dir) {
				continue;
			}
			let java = check_blocking(&dir, &root).unwrap();
			let marker = runtime::marker(&dir).unwrap();
			println!(
				"{}: {} {} ({})",
				dir.display(),
				java.version,
				java.arch,
				marker.arch()
			);
			assert!(java.managed);
			assert!(parse_major(&marker.version).is_some());
			assert_eq!(java.arch, marker.arch());
			assert_eq!(java.arch, exe::arch_of(Path::new(&java.path)).unwrap());
		}
		assert_eq!(probes(), before);
	}

	#[test]
	fn properties_output() {
		let text = "Property settings:\n    java.vendor = Eclipse Adoptium\n    java.version = 1.8.0_402\n    os.arch = x86\n    java.home = C:\\Program Files\\Java\\jdk-25\n\nopenjdk version \"1.8.0_402\"";
		let probe = parse_properties(text).unwrap();
		assert_eq!(probe.version, "1.8.0_402");
		assert_eq!(probe.arch, "x86");
		assert_eq!(probe.home.as_deref(), Some(r"C:\Program Files\Java\jdk-25"));
	}

	/// Заглушка Oracle `javapath` (если она есть на этом компьютере) заменяется настоящей Java.
	#[test]
	fn resolves_oracle_javapath() {
		let Some(common) = std::env::var_os("CommonProgramFiles") else {
			return;
		};
		let shim = PathBuf::from(common)
			.join("Oracle")
			.join("Java")
			.join("javapath")
			.join(JAVA_BIN);
		if !shim.is_file() {
			return;
		}
		let dir = tempfile::tempdir().unwrap();
		let java = check_blocking(&shim, dir.path()).unwrap();
		assert!(
			!java.path.to_lowercase().contains("javapath"),
			"{}",
			java.path
		);
		let home = Path::new(&java.path).parent().unwrap().parent().unwrap();
		assert!(home.join("release").is_file() || home.join("lib").is_dir());
	}

	#[test]
	fn picks_java() {
		let list = vec![
			java("17.0.2", "x64", false),
			java("17.0.15", "x86", false),
			java("17.0.8", "x64", true),
			java("21.0.7", "x64", false),
			java("1.8.0_51", "x64", false),
		];
		assert_eq!(pick(&list, 17, "x64", None).unwrap().version, "17.0.8");
		assert_eq!(pick(&list, 8, "x64", None).unwrap().version, "1.8.0_51");
		assert_eq!(pick(&list, 21, "x64", None).unwrap().version, "21.0.7");
		assert!(pick(&list, 16, "x64", None).is_none());
		// Нужна x86 — родная x64 не подходит.
		assert_eq!(
			pick(&list, 17, "x64", Some("x86")).unwrap().version,
			"17.0.15"
		);
		assert!(pick(&list, 21, "x64", Some("arm64")).is_none());
		let only_system = vec![java("17.0.2", "x64", false), java("17.0.15", "x64", false)];
		assert_eq!(
			pick(&only_system, 17, "x64", None).unwrap().version,
			"17.0.15"
		);
	}

	#[test]
	fn simplifies_verbatim_paths() {
		assert_eq!(
			simplify(PathBuf::from(r"\\?\C:\Java\bin")),
			PathBuf::from(r"C:\Java\bin")
		);
		assert_eq!(
			simplify(PathBuf::from(r"\\?\UNC\server\x")),
			PathBuf::from(r"\\?\UNC\server\x")
		);
	}

	fn fake_java(root: &Path, name: &str, version: &str) -> PathBuf {
		let home = root.join(name);
		std::fs::create_dir_all(home.join("bin")).unwrap();
		std::fs::write(home.join("bin").join(JAVA_BIN), b"").unwrap();
		std::fs::write(
			home.join("release"),
			format!(
				"JAVA_VERSION=\"{version}\"
OS_ARCH=\"amd64\"
"
			),
		)
		.unwrap();
		home
	}

	#[test]
	fn finds_bin_in_mac_bundles() {
		let dir = tempfile::tempdir().unwrap();
		let jdk = dir.path().join("zulu-17.jdk");
		let bin = jdk.join("Contents/Home/bin");
		std::fs::create_dir_all(&bin).unwrap();
		std::fs::write(bin.join(JAVA_BIN), b"").unwrap();
		std::fs::write(jdk.join("Contents/Info.plist"), b"").unwrap();
		assert_eq!(bin_dir(&jdk).as_deref(), Some(bin.as_path()));
		assert_eq!(
			bin_dir(&jdk.join("Contents/Info.plist")).as_deref(),
			Some(bin.as_path())
		);
		assert_eq!(
			bin_dir(&jdk.join("Contents")).as_deref(),
			Some(bin.as_path())
		);
		assert_eq!(bin_dir(&bin.join(JAVA_BIN)).as_deref(), Some(bin.as_path()));

		let runtime = dir.path().join("java-runtime-delta");
		let bin = runtime.join("jre.bundle/Contents/Home/bin");
		std::fs::create_dir_all(&bin).unwrap();
		std::fs::write(bin.join(JAVA_BIN), b"").unwrap();
		assert_eq!(bin_dir(&runtime).as_deref(), Some(bin.as_path()));
		assert!(bin_dir(&dir.path().join("nothing")).is_none());
	}

	#[tokio::test]
	async fn selects_java_in_order() {
		let dir = tempfile::tempdir().unwrap();
		let managed = dir.path().join("managed");
		let own = fake_java(dir.path(), "own", "17.0.1");
		let configured = fake_java(dir.path(), "conf", "99.0.1");
		let own = own.to_string_lossy().into_owned();
		let configured = configured.to_string_lossy().into_owned();
		let cache = JavaCache::default();

		// Своя Java сборки — как есть, даже другой версии.
		let found = select(Some(&own), Some(&configured), 99, None, &managed, &cache)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(found.major, 17);
		// Нерабочая своя — ошибка, а не тихая замена.
		let broken = dir.path().join("nope").to_string_lossy().into_owned();
		let err = select(Some(&broken), None, 17, None, &managed, &cache)
			.await
			.unwrap_err();
		assert_eq!(err.code, ErrorCode::JavaInvalid);
		// Из настроек — только той же версии.
		let found = select(None, Some(&configured), 99, None, &managed, &cache)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(found.version, "99.0.1");
		// Нужна другая разрядность — Java из настроек не годится.
		assert!(
			select(None, Some(&configured), 99, Some("arm64"), &managed, &cache)
				.await
				.unwrap()
				.is_none()
		);
		// Другой версии или нерабочая — пропускается; Java 98 на компьютере нет.
		assert!(
			select(None, Some(&configured), 98, None, &managed, &cache)
				.await
				.unwrap()
				.is_none()
		);
		assert!(
			select(None, Some(&broken), 98, None, &managed, &cache)
				.await
				.unwrap()
				.is_none()
		);
	}

	#[test]
	#[cfg(windows)]
	fn reads_large_stderr_without_hanging() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("out.txt");
		let mut text = format!("    java.library.path = {}\r\n", "C:\\x;".repeat(60_000));
		text.push_str("    java.version = 17.0.1\r\n    os.arch = amd64\r\n");
		std::fs::write(&file, &text).unwrap();
		let mut cmd = std::process::Command::new("cmd");
		cmd.arg("/c").arg("type").arg(&file).arg("1>&2");
		let started = std::time::Instant::now();
		let out = stderr_with_timeout(cmd, Duration::from_secs(8)).unwrap();
		assert!(started.elapsed() < Duration::from_secs(8));
		assert!(out.len() > 300_000);
		let probe = parse_properties(&out).unwrap();
		assert_eq!(probe.version, "17.0.1");
		assert_eq!(probe.arch, "x64");

		let mut slow = std::process::Command::new("ping");
		slow.args(["-n", "5", "127.0.0.1"]);
		let started = std::time::Instant::now();
		assert!(stderr_with_timeout(slow, Duration::from_millis(300)).is_none());
		assert!(started.elapsed() < Duration::from_secs(3));
	}

	#[test]
	#[cfg(windows)]
	fn checks_local_jdk() {
		// JDK 17 из ~/.jdks, если есть на этом компьютере.
		let Some(jdk) = dirs::home_dir().map(|h| h.join(".jdks").join("jdk17.0.21_0")) else {
			return;
		};
		if !jdk.exists() {
			return;
		}
		let java = check_blocking(&jdk, Path::new(r"C:\nowhere")).unwrap();
		assert_eq!(java.major, 17);
		assert!(java.path.ends_with("javaw.exe"));
		assert!(!java.managed);
	}
}
