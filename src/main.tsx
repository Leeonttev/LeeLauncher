import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { isLanguageChosen, setLanguage } from "@/lib/i18n";
import { appApi } from "@/lib/api";
import { initCoreLanguage } from "@/lib/core-language";
import { isMac } from "@/lib/platform";
import App from "@/App";
import "@/index.css";

// На Mac полосы прокрутки системные (поверх содержимого), свои — только на других ОС.
if (isMac) document.documentElement.dataset.os = "mac";

/** Язык из установщика ждём недолго: окно всё равно скрыто до первого кадра. */
const LAUNCH_LANG_WAIT = 1500;

async function applyLaunchLanguage() {
	if (isLanguageChosen()) return;
	try {
		const lang = await Promise.race([
			appApi.launchLang(),
			new Promise<null>((resolve) => setTimeout(() => resolve(null), LAUNCH_LANG_WAIT)),
		]);
		if (lang === "ru" || lang === "en") setLanguage(lang);
	} catch (err) {
		console.warn("app_launch_lang", err);
	}
}

// Рисуем после ответа, иначе на мгновение мелькнёт экран выбора языка.
void applyLaunchLanguage().then(() => {
	initCoreLanguage();
	createRoot(document.getElementById("root") as HTMLElement).render(
		<StrictMode>
			<App />
		</StrictMode>,
	);
});
