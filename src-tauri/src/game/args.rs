//! Аргументы запуска: classpath, аргументы JVM и игры по описанию версии, подстановки `${…}`.
//! Логика перенесена из Modrinth App (`launcher/args.rs`) и сверена с официальным лаунчером.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::accounts::LaunchCredentials;
use crate::error::{AppError, ErrorCode};
use crate::instances::Resolution;
use crate::meta::model::{Argument, ArgumentValue, VersionInfo, VersionType};
use crate::meta::rules::{Features, Platform, library_allowed, maven_path, rules_allow};
use crate::settings::Memory;

pub const LAUNCHER_NAME: &str = "LeeLauncher";
/// `${clientid}` — как в Modrinth App.
const CLIENT_ID: &str = "c4502edb-87c6-40cb-b595-64a280cf8906";
/// Чем заменяется токен в аргументах, которые пишутся в лог.
pub const MASK: &str = "<скрыто>";

/// Всё, что нужно для сборки аргументов.
#[derive(Debug)]
pub struct LaunchContext<'a> {
	pub info: &'a VersionInfo,
	pub platform: Platform,
	pub credentials: &'a LaunchCredentials,
	/// Id версии (`1.20.1-fabric-0.16.14`) — `${version_name}` в аргументах игры.
	pub version_name: &'a str,
	/// Клиент игры (`meta/versions/<версия игры>/<версия игры>.jar`).
	pub client_jar: &'a Path,
	pub classpath: &'a [PathBuf],
	pub game_dir: &'a Path,
	pub assets_root: &'a Path,
	/// `${game_assets}`: для старых версий — копия ассетов по именам (`virtual/<индекс>` или `resources/`).
	pub game_assets: &'a Path,
	pub libraries_dir: &'a Path,
	pub natives_dir: &'a Path,
	/// Конфиг логов log4j (уже на диске) — подставляется в `logging.client.argument`.
	pub log_config: Option<&'a Path>,
	pub memory: Memory,
	pub resolution: Option<Resolution>,
	pub user_jvm_args: &'a [String],
	/// Сразу зайти в этот мир (папка в `saves/`): `--quickPlaySingleplayer` (1.20+).
	pub quick_play_world: Option<&'a str>,
}

fn classpath_separator(platform: Platform) -> &'static str {
	if platform.os == "windows" { ";" } else { ":" }
}

fn bad_library(name: &str) -> AppError {
	AppError::coded(
		ErrorCode::FilesMissing,
		format!("неверное имя библиотеки: {name}"),
	)
}

/// Classpath: библиотеки по правилам платформы, затем клиент.
pub fn classpath(
	info: &VersionInfo,
	libraries_dir: &Path,
	client_jar: &Path,
	platform: Platform,
) -> Result<Vec<PathBuf>, AppError> {
	let mut out = Vec::new();
	let mut seen = HashSet::new();
	for lib in &info.libraries {
		if !lib.include_in_classpath || !library_allowed(lib, platform) {
			continue;
		}
		let artifact = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref());
		if lib.natives.is_some() && artifact.is_none() {
			continue;
		}
		let rel = match artifact.and_then(|a| a.path.clone()) {
			Some(p) => p,
			None => maven_path(&lib.name).ok_or_else(|| bad_library(&lib.name))?,
		};
		let path = crate::fs_util::safe_join(libraries_dir, &rel)
			.ok_or_else(|| crate::meta::unsafe_meta(format!("библиотека {}: {rel}", lib.name)))?;
		if seen.insert(path.clone()) {
			out.push(path);
		}
	}
	if seen.insert(client_jar.to_path_buf()) {
		out.push(client_jar.to_path_buf());
	}
	Ok(out)
}

/// Подставляет `${ключ}`; неизвестные ключи остаются как есть.
fn substitute(arg: &str, vars: &HashMap<&str, String>) -> String {
	let mut out = String::with_capacity(arg.len());
	let mut rest = arg;
	while let Some(start) = rest.find("${") {
		out.push_str(&rest[..start]);
		let after = &rest[start + 2..];
		match after.find('}') {
			Some(end) => {
				let key = &after[..end];
				match vars.get(key) {
					Some(value) => out.push_str(value),
					None => {
						out.push_str("${");
						out.push_str(key);
						out.push('}');
					}
				}
				rest = &after[end + 1..];
			}
			None => {
				out.push_str(&rest[start..]);
				rest = "";
			}
		}
	}
	out.push_str(rest);
	out
}

fn path_text(path: &Path) -> String {
	path.to_string_lossy().into_owned()
}

fn version_type(kind: VersionType) -> &'static str {
	match kind {
		VersionType::Release => "release",
		VersionType::Snapshot => "snapshot",
		VersionType::OldBeta => "old_beta",
		VersionType::OldAlpha => "old_alpha",
		VersionType::Other => "release",
	}
}

/// Функции лаунчера для правил аргументов: своё разрешение окна, вход сразу в одиночный мир;
/// демо и прочий быстрый вход — нет.
fn features(ctx: &LaunchContext) -> Features {
	let mut flags = HashMap::new();
	flags.insert("has_custom_resolution".to_owned(), ctx.resolution.is_some());
	flags.insert("is_demo_user".to_owned(), false);
	flags.insert(
		"is_quick_play_singleplayer".to_owned(),
		ctx.quick_play_world.is_some(),
	);
	Features { flags }
}

/// Аргументы описания версии (`arguments.jvm` / `arguments.game`) по правилам. Каждое значение —
/// отдельный аргумент целиком, пробелы внутри не делят его (`-DFabricMcEmu= net.minecraft…`).
fn expand(
	list: &[Argument],
	platform: Platform,
	features: &Features,
	vars: &HashMap<&str, String>,
	out: &mut Vec<String>,
) {
	for arg in list {
		match arg {
			Argument::Plain(value) => out.push(substitute(value, vars)),
			Argument::Ruled { rules, value } => {
				if !rules_allow(rules, platform, features) {
					continue;
				}
				match value {
					ArgumentValue::Single(v) => out.push(substitute(v, vars)),
					ArgumentValue::Many(vs) => out.extend(vs.iter().map(|v| substitute(v, vars))),
				}
			}
		}
	}
}

/// Аргументы после пути к Java.
#[derive(Debug, Clone)]
pub struct Args {
	pub jvm: Vec<String>,
	pub main_class: String,
	pub game: Vec<String>,
}

impl Args {
	pub fn all(&self) -> Vec<String> {
		let mut out = self.jvm.clone();
		out.push(self.main_class.clone());
		out.extend(self.game.iter().cloned());
		out
	}
}

/// Аргументы JVM и игры по описанию версии.
pub fn build(ctx: &LaunchContext) -> Args {
	let creds = ctx.credentials;
	let uuid = creds.uuid.replace('-', "");
	let resolution = ctx.resolution.unwrap_or(Resolution {
		width: 854,
		height: 480,
	});
	let classpath = ctx
		.classpath
		.iter()
		.map(|p| path_text(p))
		.collect::<Vec<_>>()
		.join(classpath_separator(ctx.platform));
	// Forge ищет клиент по имени `${version_name}.jar`, а клиент у нас общий: подставляем имя его файла.
	let client_stem = ctx
		.client_jar
		.file_stem()
		.map(|s| s.to_string_lossy().into_owned())
		.unwrap_or_default();

	let mut vars: HashMap<&str, String> = HashMap::from([
		("auth_player_name", creds.username.clone()),
		("auth_uuid", uuid.clone()),
		("uuid", uuid),
		("auth_access_token", creds.access_token.clone()),
		("accessToken", creds.access_token.clone()),
		("auth_session", creds.access_token.clone()),
		("auth_xuid", "0".to_owned()),
		("clientid", CLIENT_ID.to_owned()),
		("user_type", creds.user_type.to_owned()),
		("user_properties", "{}".to_owned()),
		("version_type", version_type(ctx.info.kind).to_owned()),
		("assets_index_name", ctx.info.asset_index.id.clone()),
		("assets_root", path_text(ctx.assets_root)),
		("game_assets", path_text(ctx.game_assets)),
		("game_directory", path_text(ctx.game_dir)),
		("resolution_width", resolution.width.to_string()),
		("resolution_height", resolution.height.to_string()),
		("natives_directory", path_text(ctx.natives_dir)),
		("library_directory", path_text(ctx.libraries_dir)),
		(
			"classpath_separator",
			classpath_separator(ctx.platform).to_owned(),
		),
		("classpath", classpath),
		("launcher_name", LAUNCHER_NAME.to_owned()),
		("launcher_version", env!("CARGO_PKG_VERSION").to_owned()),
		("quickPlayPath", String::new()),
		(
			"quickPlaySingleplayer",
			ctx.quick_play_world.unwrap_or_default().to_owned(),
		),
		("quickPlayMultiplayer", String::new()),
		("quickPlayRealms", String::new()),
		("version_name", client_stem),
	]);
	let features = features(ctx);
	let arguments = ctx.info.arguments.as_ref();
	let mut out = Vec::new();

	// JVM.
	let json_jvm = arguments.map(|a| a.jvm.as_slice()).unwrap_or_default();
	let has_classpath = json_jvm.iter().any(|a| match a {
		Argument::Plain(v) => v.contains("${classpath}"),
		Argument::Ruled { value, .. } => match value {
			ArgumentValue::Single(v) => v.contains("${classpath}"),
			ArgumentValue::Many(vs) => vs.iter().any(|v| v.contains("${classpath}")),
		},
	});
	if !has_classpath {
		// Старые версии (до 1.13): аргументов JVM в описании нет — как у официального лаунчера.
		if ctx.platform.os == "windows" {
			out.push(
				"-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump"
					.to_owned(),
			);
		}
		for arg in [
			"-Djava.library.path=${natives_directory}",
			"-Dminecraft.launcher.brand=${launcher_name}",
			"-Dminecraft.launcher.version=${launcher_version}",
			"-cp",
			"${classpath}",
		] {
			out.push(substitute(arg, &vars));
		}
	}
	expand(json_jvm, ctx.platform, &features, &vars, &mut out);
	out.push(format!("-Xms{}M", ctx.memory.min_mb));
	out.push(format!("-Xmx{}M", ctx.memory.max_mb));
	// Иначе Java на русской Windows пишет в cp1251 (8–17 — `sun.*`, 18+ — без префикса).
	for prop in [
		"-Dstdout.encoding=UTF-8",
		"-Dstderr.encoding=UTF-8",
		"-Dsun.stdout.encoding=UTF-8",
		"-Dsun.stderr.encoding=UTF-8",
	] {
		out.push(prop.to_owned());
	}
	if let (Some(path), Some(logging)) = (
		ctx.log_config,
		ctx.info.logging.as_ref().and_then(|l| l.get("client")),
	) {
		out.push(logging.argument.replace("${path}", &path_text(path)));
	}
	out.extend(ctx.user_jvm_args.iter().cloned());
	let jvm = std::mem::take(&mut out);

	// Игра.
	vars.insert("version_name", ctx.version_name.to_owned());
	if let Some(legacy) = &ctx.info.minecraft_arguments {
		// Сначала делим по пробелам, потом подставляем: ник и пути с пробелами остаются целыми.
		out.extend(
			legacy
				.split(' ')
				.filter(|a| !a.is_empty())
				.map(|a| substitute(a, &vars)),
		);
		if let Some(r) = ctx.resolution {
			out.extend([
				"--width".to_owned(),
				r.width.to_string(),
				"--height".to_owned(),
				r.height.to_string(),
			]);
		}
	}
	if let Some(arguments) = arguments {
		expand(&arguments.game, ctx.platform, &features, &vars, &mut out);
	}
	Args {
		jvm,
		main_class: ctx.info.main_class.clone(),
		game: out,
	}
}

/// Копия аргументов для лога: токен заменён на [`MASK`].
pub fn masked(args: &[String], token: &str) -> Vec<String> {
	// Офлайн-токен «0» не секрет, а замена такой строки испортила бы весь лог.
	let secret = token.len() >= 8;
	let mut out = Vec::with_capacity(args.len());
	let mut hide_next = false;
	for arg in args {
		if hide_next {
			out.push(MASK.to_owned());
			hide_next = false;
			continue;
		}
		hide_next = matches!(arg.as_str(), "--accessToken" | "--session");
		if secret && arg.contains(token) {
			out.push(arg.replace(token, MASK));
		} else {
			out.push(arg.clone());
		}
	}
	out
}

/// Делит строку пользовательских аргументов JVM: пробелы разделяют, кавычки `"…"` и `'…'` группируют.
pub fn split_user_args(text: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut current = String::new();
	let mut quote: Option<char> = None;
	let mut has_token = false;
	for c in text.chars() {
		match quote {
			Some(q) if c == q => quote = None,
			Some(_) => current.push(c),
			None if c == '"' || c == '\'' => {
				quote = Some(c);
				has_token = true;
			}
			None if c.is_whitespace() => {
				if has_token {
					out.push(std::mem::take(&mut current));
					has_token = false;
				}
			}
			None => {
				current.push(c);
				has_token = true;
			}
		}
	}
	if has_token {
		out.push(current);
	}
	out
}

/// Файл аргументов Java (`java @файл`, Java 9+): каждый аргумент в кавычках, `\` и `"` экранированы.
pub fn argfile_text(args: &[String]) -> String {
	let mut out = String::new();
	for arg in args {
		out.push('"');
		for c in arg.chars() {
			if c == '\\' || c == '"' {
				out.push('\\');
			}
			out.push(c);
		}
		out.push_str("\"\n");
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	const WIN64: Platform = Platform {
		os: "windows",
		arch: "x64",
	};

	fn creds(token: &str, user_type: &'static str) -> LaunchCredentials {
		LaunchCredentials {
			account_id: "test".into(),
			kind: crate::accounts::AccountKind::Microsoft,
			username: "Steve Jr".into(),
			uuid: "b50ad385-829d-3141-a216-7e7d7539ba7f".into(),
			access_token: token.into(),
			user_type,
		}
	}

	fn modern() -> VersionInfo {
		serde_json::from_value(json!({
			"id": "1.20.1-forge-47.4.0",
			"arguments": {
				"game": [
					"--username", "${auth_player_name}", "--version", "${version_name}",
					"--gameDir", "${game_directory}", "--assetsDir", "${assets_root}",
					"--assetIndex", "${assets_index_name}", "--uuid", "${auth_uuid}",
					"--accessToken", "${auth_access_token}", "--clientId", "${clientid}",
					"--xuid", "${auth_xuid}", "--userType", "${user_type}", "--versionType", "${version_type}",
					{ "rules": [{ "action": "allow", "features": { "is_demo_user": true } }], "value": "--demo" },
					{ "rules": [{ "action": "allow", "features": { "has_custom_resolution": true } }],
					  "value": ["--width", "${resolution_width}", "--height", "${resolution_height}"] },
					{ "rules": [{ "action": "allow", "features": { "has_quick_plays_support": true } }],
					  "value": ["--quickPlayPath", "${quickPlayPath}"] }
				],
				"jvm": [
					{ "rules": [{ "action": "allow", "os": { "name": "osx" } }], "value": ["-XstartOnFirstThread"] },
					{ "rules": [{ "action": "allow", "os": { "name": "windows" } }],
					  "value": "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump" },
					"-Djava.library.path=${natives_directory}",
					"-Dminecraft.launcher.brand=${launcher_name}",
					"-cp", "${classpath}",
					"-DignoreList=bootstraplauncher,forge-,${version_name}.jar",
					"-DlibraryDirectory=${library_directory}",
					"-DFabricMcEmu= net.minecraft.client.main.Main "
				]
			},
			"assetIndex": { "id": "5", "sha1": "a", "size": 1, "url": "https://x/5.json" },
			"assets": "5",
			"logging": { "client": { "argument": "-Dlog4j.configurationFile=${path}",
				"file": { "id": "client-1.12.xml", "sha1": "s", "size": 1, "url": "u" }, "type": "log4j2-xml" } },
			"libraries": [
				{ "name": "a:b:1", "downloads": { "artifact": { "path": "a/b/1/b-1.jar", "sha1": "s", "size": 5, "url": "u" } } },
				{ "name": "org.ow2.asm:asm:9.3", "include_in_classpath": false },
				{ "name": "osx:only:1", "rules": [{ "action": "allow", "os": { "name": "osx" } }] },
				{ "name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4", "natives": { "windows": "natives-windows" },
				  "downloads": { "classifiers": { "natives-windows": { "path": "n.jar", "url": "u" } } } },
				{ "name": "net.minecraftforge:forge:1.20.1-47.4.0:client", "downloadable": false },
				{ "name": "a:b:1", "downloads": { "artifact": { "path": "a/b/1/b-1.jar", "sha1": "s", "size": 5, "url": "u" } } }
			],
			"mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
			"type": "release"
		}))
		.unwrap()
	}

	fn legacy() -> VersionInfo {
		serde_json::from_value(json!({
			"id": "1.12.2",
			"minecraftArguments": "--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory} --assetsDir ${assets_root} --assetIndex ${assets_index_name} --uuid ${auth_uuid} --accessToken ${auth_access_token} --userType ${user_type} --versionType ${version_type}",
			"assetIndex": { "id": "1.12", "sha1": "a", "size": 1, "url": "https://x/1.12.json" },
			"assets": "1.12",
			"libraries": [],
			"mainClass": "net.minecraft.client.main.Main",
			"type": "release"
		}))
		.unwrap()
	}

	struct Paths {
		client: PathBuf,
		game: PathBuf,
		assets: PathBuf,
		libs: PathBuf,
		natives: PathBuf,
		log: PathBuf,
	}

	fn paths() -> Paths {
		let root = PathBuf::from(r"C:\Data\LeeLauncher");
		Paths {
			client: root.join(r"meta\versions\1.20.1\1.20.1.jar"),
			game: root.join(r"instances\abc\minecraft"),
			assets: root.join(r"meta\assets"),
			libs: root.join(r"meta\libraries"),
			natives: root.join(r"meta\natives\1.20.1-forge-47.4.0"),
			log: root.join(r"meta\assets\log_configs\client-1.12.xml"),
		}
	}

	fn ctx<'a>(
		info: &'a VersionInfo,
		p: &'a Paths,
		creds: &'a LaunchCredentials,
		cp: &'a [PathBuf],
		user: &'a [String],
		resolution: Option<Resolution>,
	) -> LaunchContext<'a> {
		LaunchContext {
			info,
			platform: WIN64,
			credentials: creds,
			version_name: &info.id,
			client_jar: &p.client,
			classpath: cp,
			game_dir: &p.game,
			assets_root: &p.assets,
			game_assets: &p.assets,
			libraries_dir: &p.libs,
			natives_dir: &p.natives,
			log_config: Some(&p.log),
			memory: Memory {
				min_mb: 512,
				max_mb: 4096,
			},
			resolution,
			user_jvm_args: user,
			quick_play_world: None,
		}
	}

	fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
		let i = args.iter().position(|a| a == flag).unwrap();
		&args[i + 1]
	}

	#[test]
	fn builds_classpath() {
		let info = modern();
		let p = paths();
		let cp = classpath(&info, &p.libs, &p.client, WIN64).unwrap();
		assert_eq!(
			cp,
			vec![
				p.libs.join("a/b/1/b-1.jar"),
				p.libs
					.join("net/minecraftforge/forge/1.20.1-47.4.0/forge-1.20.1-47.4.0-client.jar"),
				p.client.clone(),
			]
		);
	}

	#[test]
	fn rejects_unsafe_library_path() {
		let mut info = modern();
		let lib: crate::meta::model::Library = serde_json::from_value(json!({
			"name": "evil:x:1",
			"downloads": { "artifact": { "path": "../../../evil.jar", "url": "https://x/e.jar" } }
		}))
		.unwrap();
		info.libraries.push(lib);
		let p = paths();
		assert_eq!(
			classpath(&info, &p.libs, &p.client, WIN64)
				.unwrap_err()
				.code,
			ErrorCode::DownloadFailed
		);
	}

	#[test]
	fn builds_modern_args() {
		let info = modern();
		let p = paths();
		let creds = creds("eyJhbGciOiJIUzI1NiJ9.secret.token", "msa");
		let cp = classpath(&info, &p.libs, &p.client, WIN64).unwrap();
		let user = split_user_args("-XX:+UseG1GC \"-Dname=a b\"");
		let args = build(&ctx(&info, &p, &creds, &cp, &user, None)).all();

		let main = args
			.iter()
			.position(|a| a == "cpw.mods.bootstraplauncher.BootstrapLauncher")
			.unwrap();
		let (jvm, game) = (&args[..main], &args[main + 1..]);
		assert_eq!(
			jvm[0],
			"-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump"
		);
		assert!(!jvm.iter().any(|a| a == "-XstartOnFirstThread"));
		assert_eq!(
			jvm[1],
			r"-Djava.library.path=C:\Data\LeeLauncher\meta\natives\1.20.1-forge-47.4.0"
		);
		assert_eq!(jvm[2], "-Dminecraft.launcher.brand=LeeLauncher");
		assert_eq!(
			after(jvm, "-cp"),
			format!(
				"{};{};{}",
				cp[0].display(),
				cp[1].display(),
				p.client.display()
			)
		);
		// Клиент исключён из модулей по имени своего файла.
		assert!(jvm.contains(&"-DignoreList=bootstraplauncher,forge-,1.20.1.jar".to_owned()));
		assert!(jvm.contains(&r"-DlibraryDirectory=C:\Data\LeeLauncher\meta\libraries".to_owned()));
		assert!(jvm.contains(&"-DFabricMcEmu= net.minecraft.client.main.Main ".to_owned()));
		assert!(jvm.contains(&"-Xms512M".to_owned()));
		assert!(jvm.contains(&"-Xmx4096M".to_owned()));
		assert!(jvm.contains(
			&r"-Dlog4j.configurationFile=C:\Data\LeeLauncher\meta\assets\log_configs\client-1.12.xml"
				.to_owned()
		));
		// Свои аргументы — последними среди JVM, чтобы перекрывать остальные.
		assert_eq!(&jvm[jvm.len() - 2..], ["-XX:+UseG1GC", "-Dname=a b"]);

		assert_eq!(after(game, "--username"), "Steve Jr");
		assert_eq!(after(game, "--version"), "1.20.1-forge-47.4.0");
		assert_eq!(
			after(game, "--gameDir"),
			r"C:\Data\LeeLauncher\instances\abc\minecraft"
		);
		assert_eq!(after(game, "--assetIndex"), "5");
		assert_eq!(after(game, "--uuid"), "b50ad385829d3141a2167e7d7539ba7f");
		assert_eq!(
			after(game, "--accessToken"),
			"eyJhbGciOiJIUzI1NiJ9.secret.token"
		);
		assert_eq!(after(game, "--userType"), "msa");
		assert_eq!(after(game, "--versionType"), "release");
		assert_eq!(after(game, "--xuid"), "0");
		assert!(
			!game
				.iter()
				.any(|a| a == "--demo" || a == "--width" || a == "--quickPlayPath")
		);
		assert!(!args.iter().any(|a| a.contains("${")));
	}

	#[test]
	fn custom_resolution_feature() {
		let info = modern();
		let p = paths();
		let creds = creds("0", "legacy");
		let args = build(&ctx(
			&info,
			&p,
			&creds,
			&[],
			&[],
			Some(Resolution {
				width: 1280,
				height: 720,
			}),
		))
		.all();
		assert_eq!(after(&args, "--width"), "1280");
		assert_eq!(after(&args, "--height"), "720");
	}

	#[test]
	fn builds_legacy_args() {
		let info = legacy();
		let p = paths();
		let creds = creds("0", "legacy");
		let cp = vec![p.libs.join("x.jar"), p.client.clone()];
		let args = build(&ctx(
			&info,
			&p,
			&creds,
			&cp,
			&[],
			Some(Resolution {
				width: 1024,
				height: 768,
			}),
		))
		.all();
		let main = args
			.iter()
			.position(|a| a == "net.minecraft.client.main.Main")
			.unwrap();
		let (jvm, game) = (&args[..main], &args[main + 1..]);
		assert!(jvm[0].starts_with("-XX:HeapDumpPath="));
		assert!(jvm.contains(
			&r"-Djava.library.path=C:\Data\LeeLauncher\meta\natives\1.20.1-forge-47.4.0".to_owned()
		));
		assert!(jvm.contains(&"-Dminecraft.launcher.brand=LeeLauncher".to_owned()));
		assert_eq!(
			after(jvm, "-cp"),
			format!("{};{}", cp[0].display(), cp[1].display())
		);
		// Ник с пробелом не разваливается на два аргумента.
		assert_eq!(after(game, "--username"), "Steve Jr");
		assert_eq!(after(game, "--version"), "1.12.2");
		assert_eq!(after(game, "--accessToken"), "0");
		assert_eq!(after(game, "--userType"), "legacy");
		assert_eq!(after(game, "--assetIndex"), "1.12");
		assert_eq!(
			&game[game.len() - 4..],
			["--width", "1024", "--height", "768"]
		);
		assert!(!args.iter().any(|a| a.contains("${")));
	}

	#[test]
	fn masks_token() {
		let token = "eyJhbGciOiJIUzI1NiJ9.secret.token";
		let args: Vec<String> = [
			"--accessToken",
			token,
			"--session",
			"x",
			&format!("token:{token}:uuid"),
			"0",
		]
		.iter()
		.map(|s| s.to_string())
		.collect();
		let out = masked(&args, token);
		assert_eq!(out[1], MASK);
		assert_eq!(out[3], MASK);
		assert_eq!(out[4], format!("token:{MASK}:uuid"));
		assert_eq!(out[5], "0");
		assert!(!out.join(" ").contains("secret"));
		// Офлайн-токен «0» не прячем в других аргументах.
		let offline = masked(&["-Xmx4096M".into()], "0");
		assert_eq!(offline[0], "-Xmx4096M");
	}

	#[test]
	fn substitutes() {
		let vars = HashMap::from([("a", "1".to_owned()), ("b", "${a}".to_owned())]);
		assert_eq!(substitute("x${a}y${b}z", &vars), "x1y${a}z");
		assert_eq!(substitute("${unknown}-${a}", &vars), "${unknown}-1");
		assert_eq!(substitute("tail ${a", &vars), "tail ${a");
	}

	#[test]
	fn splits_user_args() {
		assert_eq!(
			split_user_args("  -Xss2M  \"-Dpath=C:\\My Games\" -Da='x y'z "),
			vec!["-Xss2M", "-Dpath=C:\\My Games", "-Da=x yz"]
		);
		assert_eq!(split_user_args("\"\""), vec![""]);
		assert!(split_user_args("   ").is_empty());
	}

	#[test]
	fn argfile_escapes() {
		assert_eq!(
			argfile_text(&[r"-cp".into(), r#"C:\a b\"x""#.into()]),
			"\"-cp\"\n\"C:\\\\a b\\\\\\\"x\\\"\"\n"
		);
	}
}
