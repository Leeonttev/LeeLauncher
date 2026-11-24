//! Правила (`rules`) библиотек и аргументов, пути maven-артефактов.

use std::collections::HashMap;

use super::model::{Library, LibraryDownload, Rule, RuleAction};

/// Платформа, для которой выбираются библиотеки и аргументы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
	/// `windows`, `osx`, `linux` — как в манифестах Mojang.
	pub os: &'static str,
	/// Разрядность Java: `x64`, `x86`, `arm64`.
	pub arch: &'static str,
}

impl Platform {
	pub const fn current_os() -> &'static str {
		if cfg!(windows) {
			"windows"
		} else if cfg!(target_os = "macos") {
			"osx"
		} else {
			"linux"
		}
	}

	pub fn current() -> Self {
		Self {
			os: Self::current_os(),
			arch: normalize_arch(std::env::consts::ARCH),
		}
	}

	pub fn with_arch(arch: &str) -> Self {
		Self {
			os: Self::current_os(),
			arch: normalize_arch(arch),
		}
	}

	/// Подстановка `${arch}` в классификаторе старых natives: `32` или `64`.
	pub fn arch_bits(&self) -> &'static str {
		if self.arch == "x86" { "32" } else { "64" }
	}

	fn matches_os_name(&self, name: &str) -> bool {
		match name {
			"windows-arm64" => self.os == "windows" && self.arch == "arm64",
			"osx-arm64" => self.os == "osx" && self.arch == "arm64",
			"linux-arm64" => self.os == "linux" && self.arch == "arm64",
			"linux-arm32" => self.os == "linux" && self.arch == "arm32",
			other => self.os == other,
		}
	}

	fn matches_arch(&self, arch: &str) -> bool {
		normalize_arch(arch) == self.arch
	}
}

/// Названия разрядности из `os.arch` Java и манифестов → `x64`, `x86`, `arm64`, `arm32`.
pub fn normalize_arch(arch: &str) -> &'static str {
	match arch.to_ascii_lowercase().as_str() {
		"x86" | "i386" | "i486" | "i586" | "i686" => "x86",
		"aarch64" | "arm64" => "arm64",
		"arm" | "arm32" | "aarch32" => "arm32",
		_ => "x64",
	}
}

/// Функции лаунчера для правил аргументов (`features`).
#[derive(Debug, Clone, Default)]
pub struct Features {
	pub flags: HashMap<String, bool>,
}

impl Features {
	pub fn get(&self, name: &str) -> bool {
		self.flags.get(name).copied().unwrap_or(false)
	}
}

fn rule_matches(rule: &Rule, platform: Platform, features: &Features) -> bool {
	if let Some(os) = &rule.os {
		if let Some(name) = &os.name
			&& !platform.matches_os_name(name)
		{
			return false;
		}
		if let Some(arch) = &os.arch
			&& !platform.matches_arch(arch)
		{
			return false;
		}
		// `os.version` — регулярное выражение по версии ОС; лаунчеры (Prism, Modrinth) его не учитывают.
	}
	if let Some(wanted) = &rule.features
		&& wanted
			.iter()
			.any(|(name, value)| features.get(name) != *value)
	{
		return false;
	}
	true
}

/// Алгоритм официального лаунчера: без правил — разрешено; иначе последнее подходящее правило решает.
pub fn rules_allow(rules: &[Rule], platform: Platform, features: &Features) -> bool {
	if rules.is_empty() {
		return true;
	}
	let mut allowed = false;
	for rule in rules {
		if rule_matches(rule, platform, features) {
			allowed = rule.action == RuleAction::Allow;
		}
	}
	allowed
}

pub fn library_allowed(lib: &Library, platform: Platform) -> bool {
	lib.rules
		.as_deref()
		.is_none_or(|rules| rules_allow(rules, platform, &Features::default()))
}

/// Файл natives старого формата для этой платформы (классификатор и загрузка).
pub fn native_download(lib: &Library, platform: Platform) -> Option<(String, &LibraryDownload)> {
	let natives = lib.natives.as_ref()?;
	let key = natives
		.get(&format!("{}-{}", platform.os, platform.arch))
		.or_else(|| natives.get(platform.os))?;
	let classifier = key.replace("${arch}", platform.arch_bits());
	let download = lib
		.downloads
		.as_ref()?
		.classifiers
		.as_ref()?
		.get(&classifier)?;
	Some((classifier, download))
}

/// Разрядность Java, которая нужна версии на этой ОС: на arm64 без arm64-natives (Minecraft до 1.19,
/// LWJGL 2.9/3.2) — `x64` через эмуляцию (Rosetta на macOS). `None` — подойдёт любая, лучше родная.
pub fn required_java_arch(libraries: &[Library], host: Platform) -> Option<&'static str> {
	if host.arch != "arm64" {
		return None;
	}
	let arm = Platform {
		os: host.os,
		arch: "arm64",
	};
	let arm_key = format!("{}-arm64", host.os);
	let mut has_natives = false;
	for lib in libraries.iter().filter(|l| library_allowed(l, arm)) {
		if let Some(natives) = &lib.natives {
			if natives.contains_key(&arm_key) {
				return None;
			}
			has_natives |= natives.contains_key(host.os);
			continue;
		}
		let Some(classifier) = lib.name.split('@').next().and_then(|c| c.split(':').nth(3)) else {
			continue;
		};
		if classifier.starts_with("natives-") {
			if classifier.contains("arm64") || classifier.contains("aarch64") {
				return None;
			}
			has_natives = true;
		}
	}
	has_natives.then_some("x64")
}

/// Путь артефакта maven: `group:name:version[:classifier][@ext]` → `group/path/name/version/name-version[-classifier].ext`.
pub fn maven_path(coords: &str) -> Option<String> {
	let (coords, ext) = coords.split_once('@').unwrap_or((coords, "jar"));
	let mut parts = coords.split(':');
	let group = parts.next().filter(|s| !s.is_empty())?;
	let name = parts.next().filter(|s| !s.is_empty())?;
	let version = parts.next().filter(|s| !s.is_empty())?;
	let classifier = parts.next();
	if parts.next().is_some() {
		return None;
	}
	let group = group.replace('.', "/");
	Some(match classifier {
		Some(c) => format!("{group}/{name}/{version}/{name}-{version}-{c}.{ext}"),
		None => format!("{group}/{name}/{version}/{name}-{version}.{ext}"),
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::meta::model::OsRule;

	fn rule(action: RuleAction, name: Option<&str>, arch: Option<&str>) -> Rule {
		Rule {
			action,
			os: Some(OsRule {
				name: name.map(Into::into),
				arch: arch.map(Into::into),
				version: None,
			}),
			features: None,
		}
	}

	const WIN64: Platform = Platform {
		os: "windows",
		arch: "x64",
	};
	const WIN32: Platform = Platform {
		os: "windows",
		arch: "x86",
	};
	const LINUX: Platform = Platform {
		os: "linux",
		arch: "x64",
	};

	#[test]
	fn os_rules() {
		let none = Features::default();
		assert!(rules_allow(&[], WIN64, &none));
		let only_osx = [rule(RuleAction::Allow, Some("osx"), None)];
		assert!(!rules_allow(&only_osx, WIN64, &none));
		let only_windows = [rule(RuleAction::Allow, Some("windows"), None)];
		assert!(rules_allow(&only_windows, WIN64, &none));
		assert!(!rules_allow(&only_windows, LINUX, &none));
		// Старый LWJGL: разрешено везде, кроме osx.
		let not_osx = [
			rule(RuleAction::Allow, None, None),
			rule(RuleAction::Disallow, Some("osx"), None),
		];
		assert!(rules_allow(&not_osx, WIN64, &none));
		assert!(rules_allow(&not_osx, LINUX, &none));
		let x86 = [rule(RuleAction::Allow, None, Some("x86"))];
		assert!(!rules_allow(&x86, WIN64, &none));
		assert!(rules_allow(&x86, WIN32, &none));
		let arm = [rule(RuleAction::Allow, Some("windows-arm64"), None)];
		assert!(!rules_allow(&arm, WIN64, &none));
	}

	#[test]
	fn feature_rules() {
		let demo = Rule {
			action: RuleAction::Allow,
			os: None,
			features: Some(HashMap::from([("is_demo_user".to_string(), true)])),
		};
		assert!(!rules_allow(
			std::slice::from_ref(&demo),
			WIN64,
			&Features::default()
		));
		let res = Features {
			flags: HashMap::from([("is_demo_user".to_string(), true)]),
		};
		assert!(rules_allow(&[demo], WIN64, &res));
	}

	#[test]
	fn maven_paths() {
		assert_eq!(
			maven_path("org.ow2.asm:asm:9.6").as_deref(),
			Some("org/ow2/asm/asm/9.6/asm-9.6.jar")
		);
		assert_eq!(
			maven_path("org.lwjgl:lwjgl:3.3.1:natives-windows").as_deref(),
			Some("org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar")
		);
		assert_eq!(
			maven_path("de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412@zip").as_deref(),
			Some(
				"de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412.zip"
			)
		);
		assert_eq!(
			maven_path("net.minecraft:client:1.20.1-20230612.114412:mappings@txt").as_deref(),
			Some(
				"net/minecraft/client/1.20.1-20230612.114412/client-1.20.1-20230612.114412-mappings.txt"
			)
		);
		assert_eq!(maven_path("broken"), None);
	}

	#[test]
	fn java_arch_for_old_versions() {
		let libs = |v: serde_json::Value| -> Vec<Library> { serde_json::from_value(v).unwrap() };
		const MAC_ARM: Platform = Platform {
			os: "osx",
			arch: "arm64",
		};
		const MAC_X64: Platform = Platform {
			os: "osx",
			arch: "x64",
		};
		const WIN_ARM: Platform = Platform {
			os: "windows",
			arch: "arm64",
		};
		let osx = serde_json::json!([{ "action": "allow", "os": { "name": "osx" } }]);
		let win = serde_json::json!([{ "action": "allow", "os": { "name": "windows" } }]);
		// 1.16.5: natives старого формата только x64.
		let old = libs(serde_json::json!([
			{ "name": "org.lwjgl:lwjgl:3.2.1", "natives": { "osx": "natives-macos" }, "rules": osx },
			{ "name": "org.lwjgl:lwjgl:3.2.2", "natives": { "windows": "natives-windows" },
			  "rules": [{ "action": "allow" }, { "action": "disallow", "os": { "name": "osx" } }] }
		]));
		assert_eq!(required_java_arch(&old, MAC_ARM), Some("x64"));
		assert_eq!(required_java_arch(&old, WIN_ARM), Some("x64"));
		assert_eq!(required_java_arch(&old, MAC_X64), None);
		// 1.19+: есть natives-macos-arm64 / natives-windows-arm64.
		let new = libs(serde_json::json!([
			{ "name": "org.lwjgl:lwjgl:3.3.3:natives-macos", "rules": osx },
			{ "name": "org.lwjgl:lwjgl:3.3.3:natives-macos-arm64", "rules": osx },
			{ "name": "org.lwjgl:lwjgl:3.3.3:natives-windows", "rules": win },
			{ "name": "org.lwjgl:lwjgl:3.3.3:natives-windows-arm64", "rules": win }
		]));
		assert_eq!(required_java_arch(&new, MAC_ARM), None);
		assert_eq!(required_java_arch(&new, WIN_ARM), None);
		// Только x64-natives нового формата.
		let x64 = libs(serde_json::json!([
			{ "name": "org.lwjgl:lwjgl:3.3.0:natives-macos", "rules": osx }
		]));
		assert_eq!(required_java_arch(&x64, MAC_ARM), Some("x64"));
		// Без natives — любая.
		assert_eq!(
			required_java_arch(&libs(serde_json::json!([{ "name": "a:b:1" }])), MAC_ARM),
			None
		);
	}

	#[test]
	fn legacy_natives() {
		let lib: Library = serde_json::from_value(serde_json::json!({
			"name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4",
			"natives": { "linux": "natives-linux", "windows": "natives-windows-${arch}" },
			"downloads": { "classifiers": {
				"natives-windows-64": { "path": "a", "sha1": "1", "size": 1, "url": "https://x/64" },
				"natives-windows-32": { "path": "b", "sha1": "2", "size": 2, "url": "https://x/32" }
			}}
		}))
		.unwrap();
		let (classifier, dl) = native_download(&lib, WIN64).unwrap();
		assert_eq!(classifier, "natives-windows-64");
		assert_eq!(dl.url, "https://x/64");
		assert_eq!(native_download(&lib, WIN32).unwrap().1.url, "https://x/32");
		assert!(native_download(&lib, LINUX).is_none());
	}
}
