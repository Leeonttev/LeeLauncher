//! Кэш ответов каталога в памяти: свежие ответы отдаются без сети, устаревшие — только если сети нет.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Сколько ответов держать. При переполнении уходят самые старые.
const CAPACITY: usize = 512;
/// Устаревший ответ годится «без сети» не дольше этого.
const STALE_LIMIT: Duration = Duration::from_secs(24 * 60 * 60);

struct Entry {
	at: Instant,
	ttl: Duration,
	body: Arc<Vec<u8>>,
}

#[derive(Default)]
pub struct ResponseCache {
	entries: Mutex<HashMap<String, Entry>>,
}

impl ResponseCache {
	/// Свежий ответ.
	pub fn fresh(&self, key: &str) -> Option<Arc<Vec<u8>>> {
		let entries = self.entries.lock().ok()?;
		let entry = entries.get(key)?;
		(entry.at.elapsed() < entry.ttl).then(|| entry.body.clone())
	}

	/// Любой сохранённый ответ не старше суток (для работы без сети).
	pub fn stale(&self, key: &str) -> Option<Arc<Vec<u8>>> {
		let entries = self.entries.lock().ok()?;
		let entry = entries.get(key)?;
		(entry.at.elapsed() < STALE_LIMIT).then(|| entry.body.clone())
	}

	pub fn put(&self, key: String, ttl: Duration, body: Arc<Vec<u8>>) {
		let Ok(mut entries) = self.entries.lock() else {
			return;
		};
		if entries.len() >= CAPACITY && !entries.contains_key(&key) {
			let oldest = entries
				.iter()
				.min_by_key(|(_, e)| e.at)
				.map(|(k, _)| k.clone());
			if let Some(oldest) = oldest {
				entries.remove(&oldest);
			}
		}
		entries.insert(
			key,
			Entry {
				at: Instant::now(),
				ttl,
				body,
			},
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fresh_stale_and_capacity() {
		let cache = ResponseCache::default();
		cache.put("a".into(), Duration::from_secs(60), Arc::new(b"1".to_vec()));
		cache.put("b".into(), Duration::ZERO, Arc::new(b"2".to_vec()));
		assert_eq!(cache.fresh("a").unwrap().as_slice(), b"1");
		assert!(cache.fresh("b").is_none());
		assert_eq!(cache.stale("b").unwrap().as_slice(), b"2");
		assert!(cache.stale("c").is_none());
		for i in 0..CAPACITY + 10 {
			cache.put(format!("k{i}"), Duration::from_secs(60), Arc::new(vec![]));
		}
		assert_eq!(cache.entries.lock().unwrap().len(), CAPACITY);
	}
}
