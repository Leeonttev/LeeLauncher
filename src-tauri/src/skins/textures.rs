//! Текстуры Mojang: только `textures.minecraft.net`, только `https`, без токена и перенаправлений.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use tokio::task::JoinSet;

use super::png;
use super::profile::ProfileStore;
use crate::fs_util;

pub const TEXTURES_BASE: &str = "https://textures.minecraft.net";
const TEXTURES_HOST: &str = "textures.minecraft.net";
const MAX_BYTES: usize = 1024 * 1024;
const CONCURRENT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
	Skin,
	Cape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
	pub key: String,
	pub kind: TextureKind,
}

/// Ключ становится именем файла.
pub fn safe_key(key: &str) -> bool {
	fs_util::is_safe_id(key) && fs_util::is_safe_file_name(&format!("{key}.png"))
}

/// Ключ из адреса текстуры. Адрес из ответа дальше не используется: запрашивается всегда
/// `https://textures.minecraft.net/texture/<ключ>`. Чужой адрес — `None`.
pub fn key_from_url(text: &str) -> Option<String> {
	let url = url::Url::parse(text).ok()?;
	let scheme_ok = matches!(url.scheme(), "https" | "http");
	if !scheme_ok
		|| url.host_str() != Some(TEXTURES_HOST)
		|| url.port().is_some()
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.query().is_some()
		|| url.fragment().is_some()
	{
		return None;
	}
	let mut segments = url.path_segments()?;
	if segments.next() != Some("texture") {
		return None;
	}
	let key = segments.next()?;
	(segments.next().is_none() && safe_key(key)).then(|| key.to_owned())
}

pub fn build_client() -> reqwest::Result<Client> {
	Client::builder()
		.user_agent(crate::http::USER_AGENT)
		.connect_timeout(Duration::from_secs(10))
		.timeout(Duration::from_secs(15))
		.redirect(reqwest::redirect::Policy::none())
		.build()
}

pub struct Textures {
	http: Client,
	base: String,
}

impl Textures {
	pub fn new(http: Client, base: impl Into<String>) -> Self {
		Self {
			http,
			base: base.into(),
		}
	}

	/// Докачивает недостающее. Не скачалось — текстура будет `null`.
	pub async fn ensure(&self, store: &Arc<ProfileStore>, downloads: Vec<Download>) {
		let missing: Vec<Download> = downloads
			.into_iter()
			.filter(|item| !store.has_texture(&item.key))
			.collect();
		for part in missing.chunks(CONCURRENT) {
			let mut tasks = JoinSet::new();
			for item in part.iter().cloned() {
				let http = self.http.clone();
				let url = format!("{}/texture/{}", self.base, item.key);
				let store = store.clone();
				tasks.spawn(async move { fetch_and_store(http, url, item, store).await });
			}
			while tasks.join_next().await.is_some() {}
		}
	}
}

async fn fetch_and_store(http: Client, url: String, item: Download, store: Arc<ProfileStore>) {
	let Some(bytes) = fetch(&http, &url).await else {
		log::warn!("текстура {} не получена", item.key);
		return;
	};
	let saved = tokio::task::spawn_blocking(move || {
		let clean = match item.kind {
			TextureKind::Skin => png::parse(&bytes).map(|skin| skin.png),
			TextureKind::Cape => png::parse_cape(&bytes),
		};
		match clean {
			Ok(png) => store.write_texture(&item.key, &png),
			Err(rejection) => {
				log::warn!("текстура {} отвергнута: {rejection:?}", item.key);
				Ok(())
			}
		}
	})
	.await;
	match saved {
		Ok(Ok(())) => {}
		Ok(Err(err)) => log::warn!("текстура не записана: {err}"),
		Err(err) => log::warn!("текстура не записана: {err}"),
	}
}

/// Только 200 и не больше 1 МБ по реально прочитанным байтам.
async fn fetch(http: &Client, url: &str) -> Option<Vec<u8>> {
	let mut response = http.get(url).send().await.ok()?;
	if response.status() != reqwest::StatusCode::OK {
		return None;
	}
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await.ok()? {
		bytes.extend_from_slice(&chunk);
		if bytes.len() > MAX_BYTES {
			return None;
		}
	}
	Some(bytes)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn accepts_only_textures_host() {
		let key = "abc123";
		for good in [
			"https://textures.minecraft.net/texture/abc123",
			"http://textures.minecraft.net/texture/abc123",
			"https://TEXTURES.minecraft.net/texture/abc123",
		] {
			assert_eq!(key_from_url(good).as_deref(), Some(key), "{good}");
		}
		for bad in [
			"https://textures.minecraft.net.evil.com/texture/abc123",
			"https://user@textures.minecraft.net/texture/abc123",
			"https://user:pass@textures.minecraft.net/texture/abc123",
			"https://textures.minecraft.net:8443/texture/abc123",
			"http://textures.minecraft.net:443/texture/abc123",
			"https://evil.com/texture/abc123",
			"https://evil.com/textures.minecraft.net/texture/abc123",
			"ftp://textures.minecraft.net/texture/abc123",
			"file:///textures.minecraft.net/texture/abc123",
			"https://textures.minecraft.net/skin/abc123",
			"https://textures.minecraft.net/texture/abc123/more",
			"https://textures.minecraft.net/texture/",
			"https://textures.minecraft.net/texture/..%2Fevil",
			"https://textures.minecraft.net/texture/../abc123",
			"https://textures.minecraft.net/texture/a..b",
			"https://textures.minecraft.net/texture/con",
			"https://textures.minecraft.net/texture/abc123?x=1",
			"https://textures.minecraft.net/texture/abc123#x",
			"не адрес",
			"",
		] {
			assert_eq!(key_from_url(bad), None, "{bad}");
		}
	}

	#[test]
	fn requests_only_https() {
		assert!(TEXTURES_BASE.starts_with("https://"));
		assert!(TEXTURES_BASE.ends_with(TEXTURES_HOST));
	}
}
