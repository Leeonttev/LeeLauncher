use std::fmt;

use crate::error::{AppError, ErrorCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
	DeviceToken,
	SisuAuthenticate,
	LoginWindow,
	OAuthToken,
	OAuthRefresh,
	SisuAuthorize,
	Xsts,
	MinecraftToken,
	Entitlements,
	Profile,
}

impl fmt::Display for Step {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::DeviceToken => "device token",
			Self::SisuAuthenticate => "sisu authenticate",
			Self::LoginWindow => "окно входа",
			Self::OAuthToken => "oauth token",
			Self::OAuthRefresh => "oauth refresh",
			Self::SisuAuthorize => "sisu authorize",
			Self::Xsts => "xsts",
			Self::MinecraftToken => "minecraft token",
			Self::Entitlements => "entitlements",
			Self::Profile => "minecraft profile",
		})
	}
}

/// Ошибка шага входа. Сохраняются только тела ответов с ошибкой: в успешных токены.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
	#[error("{step}: сеть: {message}")]
	Network { step: Step, message: String },
	#[error("{step}: HTTP {status}: {body}")]
	Response {
		step: Step,
		status: u16,
		body: String,
		/// Насколько часы компьютера расходятся с сервером (по заголовку `Date`), в секундах.
		clock_skew: Option<i64>,
	},
	#[error("{step}: {message}")]
	Unexpected { step: Step, message: String },
	#[error("отменено")]
	Cancelled,
	/// Окно входа ждало пользователя дольше 15 минут.
	#[error("время входа истекло")]
	Timeout,
}

/// Расхождение часов, после которого считаем, что время на компьютере сбито.
const MAX_CLOCK_SKEW_SECS: i64 = 5 * 60;
const MAX_BODY_IN_DETAILS: usize = 600;

impl AuthError {
	pub fn unexpected(step: Step, message: impl Into<String>) -> Self {
		Self::Unexpected {
			step,
			message: message.into(),
		}
	}

	pub fn from_reqwest(step: Step, err: reqwest::Error) -> Self {
		// Без URL: в query бывают одноразовые коды.
		Self::Network {
			step,
			message: err.without_url().to_string(),
		}
	}

	/// Refresh token отозван или истёк: без повторного входа не обойтись.
	pub fn is_invalid_grant(&self) -> bool {
		matches!(
			self,
			Self::Response { step: Step::OAuthRefresh, status: 400 | 401, body, .. }
				if serde_json::from_str::<serde_json::Value>(body)
					.ok()
					.and_then(|v| v.get("error")?.as_str().map(|e| e == "invalid_grant"))
					.unwrap_or(false)
		)
	}

	pub fn is_network(&self) -> bool {
		matches!(self, Self::Network { .. })
	}

	pub fn code(&self) -> ErrorCode {
		match self {
			Self::Cancelled => ErrorCode::Cancelled,
			Self::Timeout => ErrorCode::LoginTimeout,
			Self::Network { .. } => ErrorCode::Network,
			Self::Unexpected { .. } => ErrorCode::Unknown,
			Self::Response {
				step,
				status,
				body,
				clock_skew,
			} => classify_response(*step, *status, body, *clock_skew),
		}
	}

	fn details(&self) -> Option<String> {
		match self {
			Self::Cancelled | Self::Timeout => None,
			Self::Response {
				step, status, body, ..
			} => {
				let mut body = body.trim().to_owned();
				if body.len() > MAX_BODY_IN_DETAILS {
					let cut = body.floor_char_boundary(MAX_BODY_IN_DETAILS);
					body.truncate(cut);
					body.push('…');
				}
				Some(format!("{step}: HTTP {status} {body}"))
			}
			other => Some(other.to_string()),
		}
	}
}

fn xerr(body: &str) -> Option<u64> {
	serde_json::from_str::<serde_json::Value>(body)
		.ok()?
		.get("XErr")?
		.as_u64()
}

fn classify_response(step: Step, status: u16, body: &str, clock_skew: Option<i64>) -> ErrorCode {
	// Коды XErr: https://minecraft.wiki/w/Microsoft_authentication
	match xerr(body) {
		// Нет профиля Xbox; 2148916234 — не приняты условия Xbox, лечится там же, на xbox.com.
		Some(2148916233 | 2148916234) => return ErrorCode::NoXboxAccount,
		// Ребёнок не в семейной группе; 2148916229 — ограничение родительского контроля.
		Some(2148916238 | 2148916229) => return ErrorCode::ChildAccount,
		// Xbox Live недоступен в стране; 2148916236/7 — требуется подтверждение возраста (Южная Корея).
		Some(2148916235..=2148916237) => return ErrorCode::RegionBlocked,
		_ => {}
	}
	if status == 429 {
		return ErrorCode::RateLimited;
	}
	let signed = matches!(
		step,
		Step::DeviceToken | Step::SisuAuthenticate | Step::SisuAuthorize | Step::Xsts
	);
	if signed && clock_skew.is_some_and(|s| s.abs() > MAX_CLOCK_SKEW_SECS) {
		return ErrorCode::ClockOutOfSync;
	}
	// В подписи есть время: отказ на первом подписанном шаге почти всегда из-за часов.
	if matches!(step, Step::DeviceToken | Step::SisuAuthenticate) && (400..500).contains(&status) {
		return ErrorCode::ClockOutOfSync;
	}
	if status >= 500 {
		return ErrorCode::ServiceUnavailable;
	}
	if step == Step::Profile && status == 404 {
		return ErrorCode::NoMinecraft;
	}
	ErrorCode::Unknown
}

impl From<AuthError> for AppError {
	fn from(err: AuthError) -> Self {
		let app = AppError::new(err.code());
		match err.details() {
			Some(details) => app.with_details(details),
			None => app,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn resp(step: Step, status: u16, body: &str) -> AuthError {
		AuthError::Response {
			step,
			status,
			body: body.into(),
			clock_skew: Some(0),
		}
	}

	fn xsts(xerr: u64) -> AuthError {
		resp(
			Step::Xsts,
			401,
			&format!(
				r#"{{"Identity":"0","XErr":{xerr},"Message":"","Redirect":"https://start.ui.xboxlive.com/CreateAccount"}}"#
			),
		)
	}

	#[test]
	fn maps_xbox_errors() {
		assert_eq!(xsts(2148916233).code(), ErrorCode::NoXboxAccount);
		assert_eq!(xsts(2148916234).code(), ErrorCode::NoXboxAccount);
		assert_eq!(xsts(2148916238).code(), ErrorCode::ChildAccount);
		assert_eq!(xsts(2148916229).code(), ErrorCode::ChildAccount);
		assert_eq!(xsts(2148916235).code(), ErrorCode::RegionBlocked);
		assert_eq!(xsts(2148916236).code(), ErrorCode::RegionBlocked);
		assert_eq!(xsts(2148916237).code(), ErrorCode::RegionBlocked);
		assert_eq!(xsts(2148916227).code(), ErrorCode::Unknown);
		assert_eq!(resp(Step::Xsts, 400, "").code(), ErrorCode::Unknown);
	}

	#[test]
	fn maps_http_errors() {
		assert_eq!(
			resp(Step::MinecraftToken, 429, "").code(),
			ErrorCode::RateLimited
		);
		assert_eq!(
			resp(Step::MinecraftToken, 503, "").code(),
			ErrorCode::ServiceUnavailable
		);
		assert_eq!(
			resp(Step::Profile, 404, r#"{"error":"NOT_FOUND"}"#).code(),
			ErrorCode::NoMinecraft
		);
		assert_eq!(
			resp(Step::SisuAuthenticate, 401, "").code(),
			ErrorCode::ClockOutOfSync
		);
		assert_eq!(
			AuthError::Response {
				step: Step::Xsts,
				status: 401,
				body: String::new(),
				clock_skew: Some(-3600),
			}
			.code(),
			ErrorCode::ClockOutOfSync
		);
		assert_eq!(
			AuthError::Network {
				step: Step::OAuthToken,
				message: "timeout".into()
			}
			.code(),
			ErrorCode::Network
		);
		assert_eq!(AuthError::Cancelled.code(), ErrorCode::Cancelled);
		assert_eq!(AuthError::Timeout.code(), ErrorCode::LoginTimeout);
	}

	#[test]
	fn detects_invalid_grant() {
		let err = resp(
			Step::OAuthRefresh,
			400,
			r#"{"error":"invalid_grant","error_description":"expired"}"#,
		);
		assert!(err.is_invalid_grant());
		assert!(
			!resp(Step::OAuthRefresh, 400, r#"{"error":"invalid_request"}"#).is_invalid_grant()
		);
		assert!(!resp(Step::OAuthToken, 400, r#"{"error":"invalid_grant"}"#).is_invalid_grant());
	}

	#[test]
	fn app_error_has_contract_text_and_details() {
		let app: AppError = xsts(2148916233).into();
		assert_eq!(app.code, ErrorCode::NoXboxAccount);
		assert_eq!(
			app.message,
			"У этой учётной записи Microsoft нет профиля Xbox. Создайте его на xbox.com и попробуйте снова"
		);
		assert!(app.details.unwrap().contains("2148916233"));

		let long = "я".repeat(1000);
		let app: AppError = resp(Step::Xsts, 400, &long).into();
		assert!(app.details.unwrap().chars().count() < 700);

		let app: AppError = AuthError::Cancelled.into();
		assert!(app.details.is_none());
	}
}
