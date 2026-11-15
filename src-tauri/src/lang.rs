use std::sync::atomic::{AtomicU8, Ordering};

use serde::Deserialize;

/// Язык интерфейса для текстов ядра вне окна (Discord, трей). 0 — ещё не известен.
static UI: AtomicU8 = AtomicU8::new(0);

/// Язык нужен ядру только там, где текст показывает оно само (окно входа, системные диалоги).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
	#[default]
	Ru,
	En,
}

impl Lang {
	/// Код рынка Microsoft для параметра `mkt` страницы входа.
	pub fn microsoft_market(self) -> &'static str {
		match self {
			Self::Ru => "ru-RU",
			Self::En => "en-US",
		}
	}
}

/// Язык интерфейса: что прислало окно, иначе выбранный в установщике, иначе русский.
pub fn ui() -> Lang {
	match UI.load(Ordering::Relaxed) {
		1 => Lang::Ru,
		2 => Lang::En,
		_ => match crate::launch::app_launch_lang().as_deref() {
			Some("en") => Lang::En,
			_ => Lang::Ru,
		},
	}
}

pub fn set_ui(lang: Lang) {
	let value = match lang {
		Lang::Ru => 1,
		Lang::En => 2,
	};
	UI.store(value, Ordering::Relaxed);
}

/// Неизвестный язык — английский.
impl<'de> Deserialize<'de> for Lang {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		Ok(match value.as_str() {
			"ru" => Self::Ru,
			_ => Self::En,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn deserialize() {
		let parse = |s: &str| serde_json::from_str::<Lang>(s).unwrap();
		assert_eq!(parse("\"ru\""), Lang::Ru);
		assert_eq!(parse("\"en\""), Lang::En);
		assert_eq!(parse("\"de\""), Lang::En);
	}
}
