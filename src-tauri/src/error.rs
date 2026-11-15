use serde::Serialize;

/// Код ошибки для интерфейса; текст по умолчанию — [`ErrorCode::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
	InvalidUsername,
	Duplicate,
	NotFound,
	Cancelled,
	LoginTimeout,
	NoXboxAccount,
	ChildAccount,
	RegionBlocked,
	NoMinecraft,
	ClockOutOfSync,
	RateLimited,
	ServiceUnavailable,
	Network,
	NeedsRelogin,
	Storage,
	Unknown,
	InstanceNotFound,
	InvalidName,
	VersionNotFound,
	LoaderNotFound,
	InstanceBusy,
	DownloadFailed,
	JavaNotFound,
	JavaInvalid,
	ProcessorFailed,
	NotInstalled,
	AlreadyRunning,
	LaunchFailed,
	InvalidValue,
	NoAccount,
	FilesMissing,
	ContentNotFound,
	ContentIncompatible,
	AlreadyInstalled,
	FileNotFound,
	FileInUse,
	SourceUnavailable,
	ModpackInvalid,
	CurseforgeUnsupported,
	NotEnoughSpace,
	ImportUnrecognized,
	ModpackNotLinked,
	SourceDisabled,
	DistributionBlocked,
	FingerprintMismatch,
	SkinNotFound,
	SkinRejected,
	SkinTooOften,
	SkinPreviousUnknown,
	UpdateUnavailable,
	UpdateFailed,
	UpdateBadSignature,
	UpdateInProgress,
	GameRunning,
}

impl ErrorCode {
	pub fn message(self) -> &'static str {
		match self {
			Self::InvalidUsername => {
				"Ник должен быть от 3 до 16 символов: латинские буквы, цифры и _"
			}
			Self::Duplicate => "Такой аккаунт уже добавлен",
			Self::NotFound => "Аккаунт не найден",
			Self::Cancelled => "Вход отменён",
			Self::LoginTimeout => "Время входа истекло. Попробуйте ещё раз",
			Self::NoXboxAccount => {
				"У этой учётной записи Microsoft нет профиля Xbox. Создайте его на xbox.com и попробуйте снова"
			}
			Self::ChildAccount => {
				"Детскую учётную запись нужно добавить в семейную группу Microsoft"
			}
			Self::RegionBlocked => "Xbox Live недоступен в вашей стране",
			Self::NoMinecraft => "На этой учётной записи не куплен Minecraft: Java Edition",
			Self::ClockOutOfSync => {
				"Время на компьютере сбито. Включите автоматическую установку времени и попробуйте снова"
			}
			Self::RateLimited => "Слишком много запросов. Подождите несколько минут",
			Self::ServiceUnavailable => "Серверы Minecraft сейчас недоступны. Попробуйте позже",
			Self::Network => "Нет соединения с интернетом",
			Self::NeedsRelogin => "Войдите в аккаунт Microsoft заново",
			Self::Storage => "Не удалось сохранить данные на диск",
			Self::Unknown => "Что-то пошло не так",
			Self::InstanceNotFound => "Сборка не найдена",
			Self::InvalidName => "Название должно быть от 1 до 64 символов",
			Self::VersionNotFound => "Такой версии Minecraft нет",
			Self::LoaderNotFound => "Нет такой версии загрузчика для этой версии Minecraft",
			Self::InstanceBusy => "Сборка сейчас устанавливается или запущена",
			Self::DownloadFailed => {
				"Не удалось скачать файлы игры. Проверьте интернет и попробуйте ещё раз"
			}
			Self::JavaNotFound => "Не удалось найти или скачать подходящую Java",
			Self::JavaInvalid => "По этому пути нет рабочей Java",
			Self::ProcessorFailed => "Не удалось установить загрузчик",
			Self::NotInstalled => "Сборка ещё не установлена",
			Self::AlreadyRunning => "Игра уже запущена",
			Self::LaunchFailed => "Не удалось запустить игру",
			Self::InvalidValue => "Недопустимое значение",
			Self::NoAccount => "Добавьте аккаунт, чтобы играть",
			Self::FilesMissing => "Не хватает файлов игры. Почините сборку",
			Self::ContentNotFound => "Проект не найден. Возможно, его удалили",
			Self::ContentIncompatible => "Нет версии для этой сборки",
			Self::AlreadyInstalled => "Уже установлено",
			Self::FileNotFound => "Файл не найден",
			Self::FileInUse => "Файл занят игрой. Закройте игру и попробуйте снова",
			Self::SourceUnavailable => "Каталог не отвечает. Попробуйте позже",
			Self::ModpackInvalid => "Файл модпака повреждён или в неизвестном формате",
			Self::CurseforgeUnsupported => "Модпаки CurseForge недоступны в этой версии лаунчера",
			Self::NotEnoughSpace => "Не хватает места на диске",
			Self::ImportUnrecognized => "В этой папке не нашли сборок",
			Self::ModpackNotLinked => "Сборка не связана с модпаком из каталога",
			Self::SourceDisabled => "Этот каталог недоступен в этой версии лаунчера",
			Self::FingerprintMismatch => "Это другой файл. Скачайте файл с указанным именем",
			Self::DistributionBlocked => {
				"Автор запретил скачивать этот файл в лаунчерах. Скачайте его на сайте"
			}
			Self::SkinNotFound => "Скин не найден",
			Self::SkinRejected => "Mojang не принял этот скин",
			Self::SkinTooOften => "Скин сейчас нельзя сменить. Подождите немного",
			Self::SkinPreviousUnknown => {
				"Прежний скин не удалось сохранить: после замены он может пропасть"
			}
			Self::UpdateUnavailable => "Не удалось связаться с сервером обновлений",
			Self::UpdateFailed => "Не удалось установить обновление",
			Self::UpdateBadSignature => {
				"Файл обновления не прошёл проверку подписи. Попробуйте позже"
			}
			Self::UpdateInProgress => "Обновление уже скачивается",
			Self::GameRunning => "Закройте игру, чтобы обновить лаунчер",
		}
	}
}

/// Ошибка команды: `{ code, message, details? }`. В `details` секретов не кладём.
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct AppError {
	pub code: ErrorCode,
	pub message: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub details: Option<String>,
}

impl AppError {
	pub fn new(code: ErrorCode) -> Self {
		Self {
			code,
			message: code.message().to_owned(),
			details: None,
		}
	}

	pub fn with_details(mut self, details: impl Into<String>) -> Self {
		self.details = Some(details.into());
		self
	}

	pub fn storage(details: impl Into<String>) -> Self {
		Self::new(ErrorCode::Storage).with_details(details)
	}

	pub fn unknown(details: impl Into<String>) -> Self {
		Self::new(ErrorCode::Unknown).with_details(details)
	}

	pub fn coded(code: ErrorCode, details: impl Into<String>) -> Self {
		Self::new(code).with_details(details)
	}
}

impl From<tauri::Error> for AppError {
	fn from(err: tauri::Error) -> Self {
		Self::unknown(err.to_string())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn serializes_for_frontend() {
		let json = serde_json::to_value(AppError::new(ErrorCode::NoXboxAccount)).unwrap();
		assert_eq!(json["code"], "no_xbox_account");
		assert!(json.get("details").is_none());

		let json =
			serde_json::to_value(AppError::new(ErrorCode::Unknown).with_details("x")).unwrap();
		assert_eq!(json["code"], "unknown");
		assert_eq!(json["message"], "Что-то пошло не так");
		assert_eq!(json["details"], "x");

		let json = serde_json::to_value(AppError::new(ErrorCode::LoginTimeout)).unwrap();
		assert_eq!(json["code"], "login_timeout");
	}
}
