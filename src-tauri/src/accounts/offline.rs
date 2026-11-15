use md5::{Digest, Md5};
use uuid::Uuid;

/// Ник офлайн-аккаунта: 3–16 символов `A–Z a–z 0–9 _`.
pub fn is_valid_username(username: &str) -> bool {
	(3..=16).contains(&username.len())
		&& username
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// UUID игрока в офлайн-режиме, как его считает сама игра:
/// `UUID.nameUUIDFromBytes("OfflinePlayer:" + ник)` — MD5 с битами версии 3.
pub fn offline_uuid(username: &str) -> Uuid {
	let digest: [u8; 16] = Md5::digest(format!("OfflinePlayer:{username}")).into();
	uuid::Builder::from_md5_bytes(digest).into_uuid()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn offline_uuid_matches_game() {
		assert_eq!(
			offline_uuid("Notch").to_string(),
			"b50ad385-829d-3141-a216-7e7d7539ba7f"
		);
		assert_eq!(
			offline_uuid("jeb_").to_string(),
			"a762f560-4fce-3236-812a-b80efff0b62b"
		);
		assert_eq!(offline_uuid("Notch").get_version_num(), 3);
	}

	#[test]
	fn username_rules() {
		for ok in ["abc", "Steve", "jeb_", "A_1", "abcdefghijklmnop"] {
			assert!(is_valid_username(ok), "{ok}");
		}
		for bad in [
			"",
			"ab",
			"abcdefghijklmnopq",
			"with space",
			"дима",
			"name-1",
			"name.1",
			"ник_abc",
			"ab\u{0301}",
		] {
			assert!(!is_valid_username(bad), "{bad}");
		}
	}
}
