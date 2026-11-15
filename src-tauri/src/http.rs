use std::time::Duration;

pub const USER_AGENT: &str = concat!("LeeLauncher/", env!("CARGO_PKG_VERSION"));

/// Общий HTTP-клиент (rustls).
pub fn build_client() -> reqwest::Result<reqwest::Client> {
	reqwest::Client::builder()
		.user_agent(USER_AGENT)
		.connect_timeout(Duration::from_secs(15))
		.timeout(Duration::from_secs(30))
		.build()
}
