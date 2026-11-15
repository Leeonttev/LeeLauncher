//! Тексты установщика на двух языках.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
	Ru,
	En,
}

impl Lang {
	pub fn code(self) -> &'static str {
		match self {
			Lang::Ru => "ru",
			Lang::En => "en",
		}
	}

	/// Язык системы: ru/be/kk/uk/ky → русский, иначе английский.
	pub fn system() -> Lang {
		let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
		// Основной язык — младшие 10 бит LANGID.
		match id & 0x3ff {
			0x19 | 0x23 | 0x3f | 0x22 | 0x40 => Lang::Ru,
			_ => Lang::En,
		}
	}
}

const TEXTS: &[(&str, &str, &str)] = &[
	("step.welcome", "Приветствие", "Welcome"),
	("step.folder", "Папка и ярлыки", "Folder and shortcuts"),
	("step.checks", "Проверка системы", "System check"),
	("step.webview", "WebView2", "WebView2"),
	("step.install", "Установка", "Install"),
	("step.test", "Пробный запуск", "Test run"),
	("step.done", "Готово", "Done"),
	("step.skipped", "уже есть", "already there"),
	("version", "Версия {v}", "Version {v}"),
	("common.next", "Далее", "Next"),
	("common.back", "Назад", "Back"),
	("common.cancel", "Отмена", "Cancel"),
	("common.close", "Закрыть", "Close"),
	("common.retry", "Повторить", "Try again"),
	("common.details", "Подробнее", "Details"),
	("common.hideDetails", "Скрыть подробности", "Hide details"),
	("common.copy", "Скопировать", "Copy"),
	("common.copied", "Скопировано", "Copied"),
	(
		"welcome.title",
		"Установка LeeLauncher",
		"Install LeeLauncher",
	),
	(
		"welcome.text",
		"Лаунчер Minecraft: сборки, моды, скины. Займёт пару минут, права администратора не нужны",
		"A Minecraft launcher for instances, mods and skins. Takes a couple of minutes, no admin rights needed",
	),
	("welcome.language", "Язык", "Language"),
	("folder.title", "Куда установить", "Where to install"),
	(
		"folder.text",
		"Папка программ вашей учётной записи",
		"The programs folder of your account",
	),
	("folder.change", "Изменить…", "Change…"),
	(
		"folder.space",
		"Нужно {need} · свободно {free} на диске {drive}",
		"Needs {need} · {free} free on {drive}",
	),
	(
		"folder.desktop",
		"Ярлык на рабочем столе",
		"Desktop shortcut",
	),
	(
		"folder.startMenu",
		"Ярлык в меню «Пуск»",
		"Start menu shortcut",
	),
	(
		"folder.errData",
		"Сюда нельзя: здесь хранятся данные лаунчера",
		"Can’t install here: the launcher keeps its data in this folder",
	),
	(
		"folder.errAdmin",
		"Для этой папки нужны права администратора. Выберите другую",
		"This folder needs admin rights. Choose another one",
	),
	(
		"folder.errRoot",
		"Выберите папку, а не весь диск",
		"Choose a folder, not a whole drive",
	),
	("checks.title", "Проверка системы", "System check"),
	(
		"checks.text",
		"Смотрим, всё ли готово для лаунчера",
		"Making sure everything is ready",
	),
	("checks.windows", "Windows", "Windows"),
	(
		"checks.windowsOk",
		"Windows {ver}, 64-бит",
		"Windows {ver}, 64-bit",
	),
	(
		"checks.windowsErr",
		"Нужна 64-битная Windows 10 версии 1809 или новее",
		"Requires 64-bit Windows 10 version 1809 or later",
	),
	("checks.disk", "Место на диске", "Disk space"),
	(
		"checks.diskOk",
		"Свободно {free}, нужно {need}",
		"{free} free, {need} needed",
	),
	(
		"checks.diskErr",
		"Не хватает места: нужно {need}",
		"Not enough space: {need} needed",
	),
	("checks.access", "Доступ к папке", "Folder access"),
	(
		"checks.accessOk",
		"Можно записывать файлы",
		"Files can be written",
	),
	(
		"checks.accessErr",
		"Нет доступа к папке",
		"No access to the folder",
	),
	(
		"checks.otherFolder",
		"Выбрать другую папку",
		"Choose another folder",
	),
	("checks.webview", "WebView2", "WebView2"),
	("checks.webviewOk", "Версия {v}", "Version {v}"),
	(
		"checks.webviewWarn",
		"Не найден. Установим на следующем шаге",
		"Not found. We’ll install it next",
	),
	("checks.closed", "LeeLauncher закрыт", "LeeLauncher closed"),
	("checks.closedOk", "Готово", "Ready"),
	(
		"checks.closedErr",
		"LeeLauncher открыт. Закройте его, чтобы продолжить",
		"LeeLauncher is open. Close it to continue",
	),
	(
		"checks.closeApp",
		"Закрыть LeeLauncher",
		"Close LeeLauncher",
	),
	("checks.net", "Сеть", "Network"),
	(
		"checks.netOk",
		"Есть доступ к Microsoft",
		"Microsoft is reachable",
	),
	(
		"checks.netWarn",
		"Нет интернета. Он нужен, чтобы установить WebView2",
		"No internet. It’s needed to install WebView2",
	),
	("checks.checking", "Проверяем…", "Checking…"),
	("checks.waiting", "Ждём", "Waiting"),
	("webview.title", "Нужен WebView2", "WebView2 is needed"),
	(
		"webview.text",
		"На нём нарисован интерфейс лаунчера. Это бесплатный компонент Microsoft, тот же, что в браузере Edge. В Windows 11 он обычно уже есть, а здесь его нет",
		"The launcher’s interface runs on it. It’s a free Microsoft component, the same one Edge uses. Windows 11 usually has it, but this PC doesn’t",
	),
	(
		"webview.size",
		"Скачаем установщик Microsoft (2 МБ), он докачает ещё около 130 МБ",
		"We’ll download Microsoft’s installer (2 MB), it fetches about 130 MB more",
	),
	(
		"webview.trust",
		"Ставится с серверов Microsoft и обновляется сам",
		"Comes from Microsoft’s servers and updates itself",
	),
	(
		"webview.time",
		"Займёт 1–3 минуты, нужен интернет",
		"Takes 1–3 minutes, needs internet",
	),
	("webview.about", "Что такое WebView2", "What is WebView2"),
	("webview.install", "Установить WebView2", "Install WebView2"),
	("webview.installing", "Устанавливаем…", "Installing…"),
	(
		"webview.runTitle",
		"Устанавливаем WebView2",
		"Installing WebView2",
	),
	(
		"webview.runText",
		"Работает установщик Microsoft. Не выключайте компьютер",
		"Microsoft’s installer is running. Don’t turn off your PC",
	),
	(
		"webview.runLabel",
		"Скачивание и установка",
		"Downloading and installing",
	),
	("webview.elapsed", "прошло {time}", "{time} elapsed"),
	(
		"webview.usually",
		"Обычно 1–3 минуты",
		"Usually 1–3 minutes",
	),
	(
		"webview.waitTitle",
		"Дождитесь WebView2",
		"Wait for WebView2",
	),
	(
		"webview.waitText",
		"Установщик Microsoft уже работает, прервать его нельзя. После него окно можно закрыть",
		"Microsoft’s installer is already running and can’t be stopped. You can close the window after it finishes",
	),
	("webview.waitOk", "Понятно", "Got it"),
	(
		"webview.errTitle",
		"WebView2 не установился",
		"WebView2 didn’t install",
	),
	(
		"webview.errText",
		"Установщик Microsoft не смог скачать файлы. Проверьте интернет и попробуйте ещё раз или скачайте WebView2 с сайта Microsoft",
		"Microsoft’s installer couldn’t download its files. Check your connection and try again, or get WebView2 from Microsoft’s site",
	),
	("webview.site", "Сайт Microsoft", "Microsoft website"),
	(
		"webview.done",
		"WebView2 установлен · {v}",
		"WebView2 installed · {v}",
	),
	("install.title", "Устанавливаем", "Installing"),
	(
		"install.text",
		"Копируем файлы лаунчера",
		"Copying the launcher files",
	),
	("install.files", "Файлы программы", "Program files"),
	("install.shortcuts", "Ярлыки", "Shortcuts"),
	(
		"install.registry",
		"Запись в «Установленных приложениях»",
		"Entry in Installed apps",
	),
	("test.title", "Пробный запуск", "Test run"),
	(
		"test.text",
		"Открываем лаунчер незаметно и проверяем окно. Ваши данные не трогаем",
		"Opening the launcher in the background to check it works. Your data stays untouched",
	),
	("test.starts", "Программа запускается", "The app starts"),
	("test.startsOk", "Ответила за {t} с", "Responded in {t} s"),
	("test.webview", "WebView2 работает", "WebView2 works"),
	("test.render", "Интерфейс рисуется", "The interface renders"),
	(
		"done.title",
		"LeeLauncher установлен",
		"LeeLauncher is installed",
	),
	(
		"done.updated",
		"LeeLauncher обновлён до {v}",
		"LeeLauncher updated to {v}",
	),
	(
		"done.shortcuts",
		"Ярлыки на рабочем столе и в меню «Пуск»",
		"Shortcuts are on the desktop and in the Start menu",
	),
	(
		"done.desktop",
		"Ярлык на рабочем столе",
		"The shortcut is on the desktop",
	),
	(
		"done.startMenu",
		"Ярлык в меню «Пуск»",
		"The shortcut is in the Start menu",
	),
	(
		"done.folder",
		"Найти его можно в папке {path}",
		"You’ll find it in {path}",
	),
	("done.launch", "Запустить LeeLauncher", "Launch LeeLauncher"),
	("err.copyTitle", "Не удалось установить", "Couldn’t install"),
	(
		"err.copyText",
		"Файлы не записались. Возможно, папку занял антивирус или другая программа",
		"Files weren’t written. An antivirus or another app may be using the folder",
	),
	("err.spaceTitle", "Не хватает места", "Not enough space"),
	(
		"err.spaceText",
		"На диске {drive} закончилось место. Освободите {need} и попробуйте ещё раз",
		"{drive} ran out of space. Free up {need} and try again",
	),
	(
		"err.testTitle",
		"Лаунчер не запустился",
		"The launcher didn’t start",
	),
	(
		"err.testText",
		"Файлы установлены, но пробный запуск не прошёл. Попробуйте ещё раз. Если не поможет, скопируйте подробности и отправьте автору",
		"Files are installed but the test run failed. Try again. If it keeps failing, copy the details and send them to the author",
	),
	("err.logs", "Папка логов", "Logs folder"),
	("err.finishAnyway", "Всё равно завершить", "Finish anyway"),
	(
		"err.wvTitle",
		"WebView2 не отвечает",
		"WebView2 isn’t responding",
	),
	(
		"err.wvText",
		"WebView2 установлен, но не запускается. Переустановите его",
		"WebView2 is installed but won’t start. Reinstall it",
	),
	(
		"err.wvReinstall",
		"Переустановить WebView2",
		"Reinstall WebView2",
	),
	(
		"err.noPayload",
		"В этом установщике нет лаунчера",
		"This installer has no launcher inside",
	),
	(
		"cancel.title",
		"Прервать установку?",
		"Stop the installation?",
	),
	(
		"cancel.text",
		"Скопированные файлы удалим. Установку можно будет начать заново",
		"Copied files will be removed. You can start again later",
	),
	("cancel.stop", "Прервать", "Stop"),
	("cancel.keep", "Продолжить установку", "Keep installing"),
	(
		"existing.title",
		"LeeLauncher уже установлен",
		"LeeLauncher is already installed",
	),
	(
		"existing.text",
		"Версия {old}. Сборки, миры и аккаунты не тронем",
		"Version {old}. Your instances, worlds and accounts stay as they are",
	),
	("existing.update", "Обновить до {new}", "Update to {new}"),
	(
		"existing.updateText",
		"Поставим новую версию поверх старой",
		"Installs the new version over the old one",
	),
	("existing.updateAction", "Обновить", "Update"),
	("existing.reinstall", "Переустановить", "Reinstall"),
	(
		"existing.reinstallText",
		"Заменим файлы программы, если что-то сломалось",
		"Replaces the program files in case something broke",
	),
	(
		"existing.downgrade",
		"Установить {new} вместо {old}",
		"Install {new} instead of {old}",
	),
	(
		"existing.downgradeText",
		"Более старая версия. Новые сборки могут не открыться",
		"An older version. Newer instances may not open",
	),
	("existing.downgradeAction", "Установить", "Install"),
	("existing.remove", "Удалить", "Uninstall"),
	(
		"existing.removeText",
		"Программу удалим, данные по вашему выбору",
		"Removes the app, your data only if you choose",
	),
	(
		"uninstall.title",
		"Удалить LeeLauncher?",
		"Uninstall LeeLauncher?",
	),
	(
		"uninstall.text",
		"Уберём программу и ярлыки",
		"Removes the app and its shortcuts",
	),
	(
		"uninstall.data",
		"Удалить и мои данные",
		"Also delete my data",
	),
	(
		"uninstall.dataSize",
		"Удалить и мои данные · {size}",
		"Also delete my data · {size}",
	),
	(
		"uninstall.dataHint",
		"Сборки, миры, скриншоты, скины, вход в аккаунты",
		"Instances, worlds, screenshots, skins, account sign-ins",
	),
	(
		"uninstall.warn",
		"Миры и сборки удалятся навсегда, вернуть их не получится",
		"Worlds and instances will be gone for good",
	),
	("uninstall.action", "Удалить", "Uninstall"),
	("uninstall.running", "Удаляем", "Uninstalling"),
	(
		"uninstall.done",
		"LeeLauncher удалён",
		"LeeLauncher is uninstalled",
	),
	(
		"uninstall.doneData",
		"Ваши данные остались в {path}",
		"Your data is still in {path}",
	),
	(
		"uninstall.doneAll",
		"Программа и данные удалены",
		"The app and its data are removed",
	),
	("uninstall.openFolder", "Открыть папку", "Open folder"),
	(
		"uninstall.errTitle",
		"Не удалось удалить",
		"Couldn’t uninstall",
	),
	(
		"uninstall.errText",
		"Часть файлов занята другой программой. Закройте её и попробуйте ещё раз",
		"Some files are in use by another app. Close it and try again",
	),
	(
		"silent.title",
		"Обновляем LeeLauncher до {v}",
		"Updating LeeLauncher to {v}",
	),
	(
		"silent.text",
		"Лаунчер откроется сам",
		"The launcher will open by itself",
	),
	(
		"silent.progress",
		"Заменяем файлы · {percent}%",
		"Replacing files · {percent}%",
	),
	("silent.errTitle", "Не удалось обновить", "Couldn’t update"),
	(
		"silent.errText",
		"Прежняя версия на месте. Попробуйте ещё раз",
		"The previous version is still here. Try again",
	),
	("silent.openPrevious", "Открыть прежнюю", "Open previous"),
	("window.minimize", "Свернуть", "Minimize"),
	("window.close", "Закрыть", "Close"),
	("size.kb", "{n} КБ", "{n} KB"),
	("size.mb", "{n} МБ", "{n} MB"),
	("size.gb", "{n} ГБ", "{n} GB"),
];

pub fn t(lang: Lang, key: &str) -> &'static str {
	match TEXTS.iter().find(|(k, _, _)| *k == key) {
		Some((_, ru, en)) => match lang {
			Lang::Ru => ru,
			Lang::En => en,
		},
		None => {
			debug_assert!(false, "нет текста {key}");
			""
		}
	}
}

/// Текст с подстановками `{имя}`.
pub fn tf(lang: Lang, key: &str, args: &[(&str, &str)]) -> String {
	let mut s = t(lang, key).to_owned();
	for (name, value) in args {
		s = s.replace(&format!("{{{name}}}"), value);
	}
	s
}

/// Размер для людей: «82 МБ», «1,4 ГБ».
pub fn size(lang: Lang, bytes: u64) -> String {
	const KB: f64 = 1024.0;
	let b = bytes as f64;
	let (key, value) = if b >= KB * KB * KB {
		("size.gb", b / (KB * KB * KB))
	} else if b >= KB * KB {
		("size.mb", b / (KB * KB))
	} else {
		("size.kb", (b / KB).max(1.0))
	};
	let mut n = if value >= 10.0 {
		format!("{value:.0}")
	} else {
		format!("{value:.1}")
	};
	if n.ends_with(".0") {
		n.truncate(n.len() - 2);
	}
	if lang == Lang::Ru {
		n = n.replace('.', ",");
	}
	tf(lang, key, &[("n", &n)])
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_key_has_both_languages() {
		for (key, ru, en) in TEXTS {
			assert!(!ru.is_empty() && !en.is_empty(), "{key}");
		}
		let mut keys: Vec<_> = TEXTS.iter().map(|(k, _, _)| *k).collect();
		keys.sort();
		keys.dedup();
		assert_eq!(keys.len(), TEXTS.len(), "повтор ключа");
	}

	#[test]
	fn formats_sizes() {
		assert_eq!(size(Lang::Ru, 1536 * 1024), "1,5 МБ");
		assert_eq!(size(Lang::En, 200 * 1024 * 1024), "200 MB");
		assert_eq!(size(Lang::Ru, 3 * 1024 * 1024 * 1024), "3 ГБ");
		assert_eq!(size(Lang::En, 10), "1 KB");
		assert_eq!(tf(Lang::Ru, "version", &[("v", "0.2.0")]), "Версия 0.2.0");
	}
}
