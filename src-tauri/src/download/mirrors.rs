use serde::{Deserialize, Serialize};

/// Откуда качать файлы игры. Настройка лаунчера `downloadSource`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadSource {
	/// Сначала официальный адрес, при ошибке или слишком медленной скорости — зеркало.
	#[default]
	Auto,
	/// Только официальные адреса.
	Official,
	/// Сначала зеркало, потом официальный адрес.
	Mirror,
}

const BMCLAPI: &str = "https://bmclapi2.bangbang93.com";

/// Официальный префикс адреса → префикс на зеркале BMCLAPI. Путь после префикса сохраняется.
const BMCLAPI_RULES: &[(&str, &str)] = &[
	("https://piston-meta.mojang.com/", "/"),
	("https://launchermeta.mojang.com/", "/"),
	("https://piston-data.mojang.com/", "/"),
	("https://launcher.mojang.com/", "/"),
	("https://resources.download.minecraft.net/", "/assets/"),
	("https://libraries.minecraft.net/", "/maven/"),
	("https://maven.minecraftforge.net/", "/maven/"),
	("https://files.minecraftforge.net/maven/", "/maven/"),
	("https://maven.neoforged.net/releases/", "/maven/"),
	("https://maven.fabricmc.net/", "/maven/"),
	("https://maven.quiltmc.org/repository/release/", "/maven/"),
	// Библиотеки загрузчиков. `com.modrinth.daedalus:*` на BMCLAPI нет.
	("https://launcher-meta.modrinth.com/maven/", "/maven/"),
];

pub fn mirror_url(url: &str) -> Option<String> {
	BMCLAPI_RULES.iter().find_map(|(from, to)| {
		url.strip_prefix(from)
			.map(|rest| format!("{BMCLAPI}{to}{rest}"))
	})
}

/// Адреса для попыток по порядку.
pub fn candidates(url: &str, source: DownloadSource) -> Vec<String> {
	let mirror = mirror_url(url);
	match (source, mirror) {
		(DownloadSource::Official, _) | (_, None) => vec![url.to_owned()],
		(DownloadSource::Auto, Some(m)) => vec![url.to_owned(), m],
		(DownloadSource::Mirror, Some(m)) => vec![m, url.to_owned()],
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn maps_known_hosts() {
		assert_eq!(
			mirror_url("https://resources.download.minecraft.net/ab/abcdef").as_deref(),
			Some("https://bmclapi2.bangbang93.com/assets/ab/abcdef")
		);
		assert_eq!(
			mirror_url("https://libraries.minecraft.net/a/b/1/b-1.jar").as_deref(),
			Some("https://bmclapi2.bangbang93.com/maven/a/b/1/b-1.jar")
		);
		assert_eq!(
			mirror_url("https://piston-data.mojang.com/v1/objects/x/client.jar").as_deref(),
			Some("https://bmclapi2.bangbang93.com/v1/objects/x/client.jar")
		);
		assert_eq!(mirror_url("https://example.com/x"), None);
	}

	#[test]
	fn candidate_order() {
		let url = "https://libraries.minecraft.net/x.jar";
		assert_eq!(candidates(url, DownloadSource::Official), vec![url]);
		let auto = candidates(url, DownloadSource::Auto);
		assert_eq!(auto[0], url);
		assert!(auto[1].starts_with(BMCLAPI));
		let mirror = candidates(url, DownloadSource::Mirror);
		assert!(mirror[0].starts_with(BMCLAPI));
		assert_eq!(mirror[1], url);
		assert_eq!(
			candidates("https://example.com/x", DownloadSource::Mirror).len(),
			1
		);
	}
}
