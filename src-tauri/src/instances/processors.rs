//! Процессоры установщика Forge/NeoForge: java-программы, которые собирают пропатченный клиент.
//! Описание и файлы — из launcher-meta.modrinth.com (содержимое установщика уже разложено по maven).

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::install::InstallMode;
use crate::download::{self, Job, Progress, Verify};
use crate::error::{AppError, ErrorCode};
use crate::java::JavaInstall;
use crate::launcher::Launcher;
use crate::meta::Resolved;
use crate::meta::model::{Processor, SidedData};
use crate::meta::rules::maven_path;

/// Отметка в папке версии: процессоры отработали.
const DONE_MARKER: &str = ".processors-done";
/// Сколько последних символов вывода процессора попадает в `details` ошибки.
const OUTPUT_TAIL: usize = 3000;

fn failed(details: impl Into<String>) -> AppError {
	AppError::coded(ErrorCode::ProcessorFailed, details)
}

fn library_path(libs: &Path, coords: &str) -> Result<PathBuf, AppError> {
	maven_path(coords)
		.and_then(|rel| crate::fs_util::safe_join(libs, &rel))
		.ok_or_else(|| failed(format!("неверный артефакт {coords}")))
}

/// Значение из `data`: `[артефакт]` → путь в libraries, `'текст'` → текст, иначе как есть.
fn resolve_value(value: &str, libs: &Path) -> Result<String, AppError> {
	if let Some(coords) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
		return Ok(library_path(libs, coords)?.to_string_lossy().into_owned());
	}
	if let Some(text) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
		return Ok(text.to_owned());
	}
	Ok(value.to_owned())
}

/// Аргумент процессора: `[артефакт]` → путь, `{КЛЮЧ}` внутри строки → значение из `data`.
fn resolve_arg(arg: &str, libs: &Path, data: &HashMap<String, String>) -> Result<String, AppError> {
	if let Some(coords) = arg.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
		return Ok(library_path(libs, coords)?.to_string_lossy().into_owned());
	}
	let mut out = arg.to_owned();
	for (key, value) in data {
		out = out.replace(&format!("{{{key}}}"), value);
	}
	Ok(out)
}

pub fn build_data(
	raw: Option<&HashMap<String, SidedData>>,
	libs: &Path,
	extra: &[(&str, String)],
) -> Result<HashMap<String, String>, AppError> {
	let mut data = HashMap::new();
	for (key, sided) in raw.into_iter().flatten() {
		data.insert(key.clone(), resolve_value(&sided.client, libs)?);
	}
	for (key, value) in extra {
		data.insert((*key).to_owned(), value.clone());
	}
	Ok(data)
}

fn is_client(p: &Processor) -> bool {
	p.sides
		.as_ref()
		.is_none_or(|s| s.iter().any(|side| side == "client"))
}

/// Значение аргумента после флага (`--output <путь>`).
fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
	args.iter()
		.position(|a| a == flag)
		.and_then(|i| args.get(i + 1))
		.map(String::as_str)
}

fn main_class(jar: &Path) -> Result<String, AppError> {
	let file = std::fs::File::open(jar).map_err(|e| failed(format!("{}: {e}", jar.display())))?;
	let mut archive =
		zip::ZipArchive::new(file).map_err(|e| failed(format!("{}: {e}", jar.display())))?;
	let manifest = archive
		.by_name("META-INF/MANIFEST.MF")
		.map_err(|e| failed(format!("{}: {e}", jar.display())))?;
	for line in BufReader::new(manifest).lines() {
		let line = line.map_err(|e| failed(e.to_string()))?;
		if let Some(class) = line.strip_prefix("Main-Class:") {
			return Ok(class.trim().to_owned());
		}
	}
	Err(failed(format!("{}: нет Main-Class", jar.display())))
}

fn tail(text: &str) -> &str {
	let start = text.len().saturating_sub(OUTPUT_TAIL);
	let start = (start..text.len())
		.find(|&i| text.is_char_boundary(i))
		.unwrap_or(text.len());
	&text[start..]
}

/// Выходные файлы процессора уже на месте и с верными sha1.
async fn outputs_valid(outputs: &[(PathBuf, String)]) -> bool {
	if outputs.is_empty() {
		return false;
	}
	for (path, sha1) in outputs {
		match download::file_sha1(path.clone()).await {
			Ok(actual) if actual.eq_ignore_ascii_case(sha1) => {}
			_ => return false,
		}
	}
	true
}

pub async fn run(
	l: &Launcher,
	resolved: &Resolved,
	java: &JavaInstall,
	mode: InstallMode,
	progress: Arc<Progress>,
	cancel: &CancellationToken,
) -> Result<(), AppError> {
	let info = &resolved.info;
	let processors: Vec<&Processor> = info
		.processors
		.iter()
		.flatten()
		.filter(|p| is_client(p))
		.collect();
	if processors.is_empty() {
		return Ok(());
	}
	let marker = l
		.layout
		.versions_dir()
		.join(&resolved.version_id)
		.join(DONE_MARKER);
	let libs = l.layout.libraries_dir();
	let data = build_data(
		info.data.as_ref(),
		&libs,
		&[
			("SIDE", "client".into()),
			(
				"MINECRAFT_JAR",
				l.layout
					.version_jar(&resolved.game_version)
					.to_string_lossy()
					.into_owned(),
			),
			("MINECRAFT_VERSION", resolved.game_version.clone()),
			("ROOT", l.layout.meta_dir().to_string_lossy().into_owned()),
			("LIBRARY_DIR", libs.to_string_lossy().into_owned()),
			// Установщика у нас нет: его содержимое уже разложено по libraries. Нужен только серверным шагам.
			("INSTALLER", String::new()),
		],
	)?;

	progress.add_total(processors.len() as u64, 0);
	let skip_done = mode == InstallMode::Install && marker.is_file();
	let separator = if cfg!(windows) { ";" } else { ":" };

	for processor in processors {
		if cancel.is_cancelled() {
			return Err(AppError::new(ErrorCode::Cancelled));
		}
		let args = processor
			.args
			.iter()
			.map(|a| resolve_arg(a, &libs, &data))
			.collect::<Result<Vec<_>, _>>()?;
		let outputs = processor
			.outputs
			.iter()
			.flatten()
			.map(|(k, v)| {
				Ok((
					PathBuf::from(resolve_arg(k, &libs, &data)?),
					resolve_value(&resolve_arg(v, &libs, &data)?, &libs)?,
				))
			})
			.collect::<Result<Vec<_>, AppError>>()?;

		// Карты Mojang качаем сами (с проверкой и зеркалами), а не процессором через Java.
		if arg_after(&args, "--task") == Some("DOWNLOAD_MOJMAPS")
			&& let Some(output) = arg_after(&args, "--output")
			&& let Some(mappings) = info.downloads.get("client_mappings")
		{
			let job = Job {
				url: mappings.url.clone(),
				path: PathBuf::from(output),
				sha1: Some(mappings.sha1.clone()),
				size: Some(mappings.size),
			};
			l.dl.file(&job, Verify::Hash, &Progress::silent(), cancel)
				.await?;
			progress.file_done();
			continue;
		}

		if (skip_done && outputs.iter().all(|(p, _)| p.is_file()))
			|| (mode == InstallMode::Install && outputs_valid(&outputs).await)
		{
			progress.file_done();
			continue;
		}

		let jar = library_path(&libs, &processor.jar)?;
		let main = {
			let jar = jar.clone();
			tokio::task::spawn_blocking(move || main_class(&jar))
				.await
				.map_err(|e| AppError::unknown(e.to_string()))??
		};
		let mut classpath = Vec::with_capacity(processor.classpath.len() + 1);
		for coords in processor
			.classpath
			.iter()
			.chain(std::iter::once(&processor.jar))
		{
			classpath.push(library_path(&libs, coords)?.to_string_lossy().into_owned());
		}

		log::info!("процессор {} ({main})", processor.jar);
		let mut cmd = tokio::process::Command::new(java.console_path());
		cmd.args(crate::java::mitigation_args(
			java.major,
			l.settings.get().java4shell_mitigation,
		))
		.arg("-cp")
		.arg(classpath.join(separator))
		.arg(&main)
		.args(&args)
		.current_dir(l.layout.meta_dir())
		.env_remove("_JAVA_OPTIONS")
		.env_remove("JAVA_TOOL_OPTIONS")
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.kill_on_drop(true);
		#[cfg(windows)]
		cmd.creation_flags(0x0800_0000);
		let child = cmd
			.spawn()
			.map_err(|e| failed(format!("{}: {e}", java.console_path().display())))?;
		let output = tokio::select! {
			_ = cancel.cancelled() => return Err(AppError::new(ErrorCode::Cancelled)),
			o = child.wait_with_output() => o.map_err(|e| failed(e.to_string()))?,
		};
		if !output.status.success() {
			let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
			text.push_str(&String::from_utf8_lossy(&output.stderr));
			return Err(failed(format!(
				"{} завершился с кодом {:?}:\n{}",
				processor.jar,
				output.status.code(),
				tail(&text)
			)));
		}
		for (path, sha1) in &outputs {
			let actual = download::file_sha1(path.clone()).await?;
			if !actual.eq_ignore_ascii_case(sha1) {
				return Err(failed(format!(
					"{}: sha1 {actual}, ожидался {sha1}",
					path.display()
				)));
			}
		}
		progress.file_done();
	}

	if let Err(err) = crate::fs_util::write_atomic(&marker, b"ok") {
		log::warn!("не удалось записать {}: {err}", marker.display());
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn resolves_values_and_args() {
		let libs = PathBuf::from("L");
		let raw: HashMap<String, SidedData> = serde_json::from_value(serde_json::json!({
			"MAPPINGS": { "client": "[de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412:mappings@txt]", "server": "x" },
			"MC_SLIM_SHA": { "client": "'de86b035d2da0f78940796bb95c39a932ed84834'", "server": "x" }
		}))
		.unwrap();
		let data = build_data(Some(&raw), &libs, &[("SIDE", "client".into())]).unwrap();
		assert_eq!(
			data["MC_SLIM_SHA"],
			"de86b035d2da0f78940796bb95c39a932ed84834"
		);
		assert_eq!(
			PathBuf::from(&data["MAPPINGS"]),
			libs.join("de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412-mappings.txt")
		);
		assert_eq!(resolve_arg("--side", &libs, &data).unwrap(), "--side");
		assert_eq!(resolve_arg("{SIDE}", &libs, &data).unwrap(), "client");
		assert_eq!(resolve_arg("{SIDE}/x", &libs, &data).unwrap(), "client/x");
		assert_eq!(
			PathBuf::from(
				resolve_arg(
					"[net.minecraftforge:forge:1.20.1-47.4.0:client]",
					&libs,
					&data
				)
				.unwrap()
			),
			libs.join("net/minecraftforge/forge/1.20.1-47.4.0/forge-1.20.1-47.4.0-client.jar")
		);
		let args = vec![
			"--task".to_owned(),
			"DOWNLOAD_MOJMAPS".to_owned(),
			"--output".to_owned(),
			"o".to_owned(),
		];
		assert_eq!(arg_after(&args, "--task"), Some("DOWNLOAD_MOJMAPS"));
		assert_eq!(arg_after(&args, "--output"), Some("o"));
		assert_eq!(arg_after(&args, "--nope"), None);
	}

	#[test]
	fn client_side_filter() {
		let p = |sides: Option<Vec<&str>>| Processor {
			jar: "a:b:1".into(),
			classpath: vec![],
			args: vec![],
			outputs: None,
			sides: sides.map(|s| s.into_iter().map(Into::into).collect()),
		};
		assert!(is_client(&p(None)));
		assert!(is_client(&p(Some(vec!["client"]))));
		assert!(!is_client(&p(Some(vec!["server"]))));
	}

	#[test]
	fn output_tail_is_char_safe() {
		let long = "я".repeat(OUTPUT_TAIL);
		let t = tail(&long);
		assert!(t.len() <= OUTPUT_TAIL);
		assert!(t.chars().all(|c| c == 'я'));
	}
}
