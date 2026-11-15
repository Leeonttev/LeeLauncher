//! Форматы манифестов Mojang и метаданных загрузчиков launcher-meta.modrinth.com
//! (по образцу крейта daedalus из Modrinth App).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Заглушка версии игры в метаданных загрузчиков Modrinth.
pub const GAME_VERSION_PLACEHOLDER: &str = "${modrinth.gameVersion}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionType {
	Release,
	Snapshot,
	OldBeta,
	OldAlpha,
	#[serde(other)]
	Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersions {
	pub release: String,
	pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestVersion {
	pub id: String,
	#[serde(rename = "type")]
	pub kind: VersionType,
	pub url: String,
	#[serde(default)]
	pub release_time: String,
	#[serde(default)]
	pub sha1: Option<String>,
}

/// `version_manifest_v2.json` Mojang.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
	pub latest: LatestVersions,
	pub versions: Vec<ManifestVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
	pub id: String,
	pub sha1: String,
	pub size: u64,
	#[serde(default)]
	pub total_size: Option<u64>,
	pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
	pub sha1: String,
	pub size: u64,
	pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDownload {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub path: Option<String>,
	#[serde(default)]
	pub sha1: Option<String>,
	#[serde(default)]
	pub size: Option<u64>,
	#[serde(default)]
	pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryDownloads {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub artifact: Option<LibraryDownload>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub classifiers: Option<HashMap<String, LibraryDownload>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
	Allow,
	Disallow,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OsRule {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub arch: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
	pub action: RuleAction,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub os: Option<OsRule>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryExtract {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub exclude: Vec<String>,
}

fn yes() -> bool {
	true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub downloads: Option<LibraryDownloads>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub url: Option<String>,
	/// Старый формат natives: ОС → классификатор (`natives-windows-${arch}`).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub natives: Option<HashMap<String, String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rules: Option<Vec<Rule>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub extract: Option<LibraryExtract>,
	/// Поле Modrinth: библиотека нужна только процессорам, в classpath игры не идёт.
	#[serde(default = "yes")]
	pub include_in_classpath: bool,
	#[serde(default = "yes")]
	pub downloadable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
	Single(String),
	Many(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Argument {
	Plain(String),
	Ruled {
		#[serde(default)]
		rules: Vec<Rule>,
		value: ArgumentValue,
	},
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Arguments {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub game: Vec<Argument>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersionReq {
	#[serde(default)]
	pub component: Option<String>,
	pub major_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingFile {
	pub id: String,
	pub sha1: String,
	pub size: u64,
	pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
	pub argument: String,
	pub file: LoggingFile,
	#[serde(rename = "type", default)]
	pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidedData {
	pub client: String,
	pub server: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Processor {
	pub jar: String,
	#[serde(default)]
	pub classpath: Vec<String>,
	#[serde(default)]
	pub args: Vec<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub outputs: Option<HashMap<String, String>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub sides: Option<Vec<String>>,
}

/// Описание версии: файл Mojang `<версия>.json` или он же, объединённый с профилем загрузчика.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
	pub id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub arguments: Option<Arguments>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub minecraft_arguments: Option<String>,
	pub asset_index: AssetIndexRef,
	pub assets: String,
	#[serde(default)]
	pub downloads: HashMap<String, Download>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub java_version: Option<JavaVersionReq>,
	#[serde(default)]
	pub libraries: Vec<Library>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub logging: Option<HashMap<String, LoggingConfig>>,
	pub main_class: String,
	#[serde(rename = "type")]
	pub kind: VersionType,
	#[serde(default)]
	pub release_time: String,
	/// Forge/NeoForge: значения для процессоров.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub data: Option<HashMap<String, SidedData>>,
	/// Forge/NeoForge: шаги установки, которые выполняются Java после загрузки файлов.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub processors: Option<Vec<Processor>>,
}

impl VersionInfo {
	/// Мажорная версия Java для игры; старые версии без поля — Java 8.
	pub fn java_major(&self) -> u32 {
		self.java_version.as_ref().map_or(8, |j| j.major_version)
	}
}

/// Профиль загрузчика из launcher-meta.modrinth.com: дополняет описание версии Minecraft.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialVersionInfo {
	pub id: String,
	pub inherits_from: String,
	#[serde(default)]
	pub main_class: Option<String>,
	#[serde(default)]
	pub minecraft_arguments: Option<String>,
	#[serde(default)]
	pub arguments: Option<Arguments>,
	#[serde(default)]
	pub libraries: Vec<Library>,
	#[serde(default)]
	pub data: Option<HashMap<String, SidedData>>,
	#[serde(default)]
	pub processors: Option<Vec<Processor>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderVersion {
	pub id: String,
	pub url: String,
	#[serde(default)]
	pub stable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderGameVersion {
	pub id: String,
	#[serde(default)]
	pub stable: bool,
	#[serde(default)]
	pub version_group: Option<String>,
	#[serde(default)]
	pub loaders: Vec<LoaderVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionGroup {
	pub id: String,
	#[serde(default)]
	pub loaders: Vec<LoaderVersion>,
}

/// `<загрузчик>/v<N>/manifest.json` на launcher-meta.modrinth.com.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderManifest {
	pub game_versions: Vec<LoaderGameVersion>,
	#[serde(default)]
	pub version_groups: Vec<VersionGroup>,
}

impl LoaderManifest {
	/// Версии загрузчика для версии игры, новые первыми. У Fabric и Quilt они в общей записи или в группе.
	pub fn loaders_for(&self, game_version: &str) -> Option<&[LoaderVersion]> {
		let entry = self.game_versions.iter().find(|v| v.id == game_version)?;
		let list = if let Some(group) = &entry.version_group {
			self.version_groups
				.iter()
				.find(|g| &g.id == group)
				.map(|g| g.loaders.as_slice())?
		} else if !entry.loaders.is_empty() {
			entry.loaders.as_slice()
		} else {
			self.game_versions
				.iter()
				.find(|v| v.id == GAME_VERSION_PLACEHOLDER)
				.map(|v| v.loaders.as_slice())?
		};
		(!list.is_empty()).then_some(list)
	}

	/// Версии игры, для которых есть хотя бы одна версия загрузчика.
	pub fn game_versions(&self) -> Vec<String> {
		self.game_versions
			.iter()
			.filter(|v| v.id != GAME_VERSION_PLACEHOLDER)
			.filter(|v| self.loaders_for(&v.id).is_some())
			.map(|v| v.id.clone())
			.collect()
	}
}

/// Объект ассета: хеш и размер.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
	pub hash: String,
	pub size: u64,
}

/// Индекс ассетов (`assets/indexes/<id>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndex {
	pub objects: HashMap<String, Asset>,
	/// Версии до 1.7.2: копия ассетов в `assets/virtual/<id>/` по именам.
	#[serde(default, rename = "virtual")]
	pub is_virtual: bool,
	/// Версии до 1.6: копия ассетов в `<папка игры>/resources/`.
	#[serde(default)]
	pub map_to_resources: bool,
}
