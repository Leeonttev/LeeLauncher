import { appApi } from "@/lib/api";
import { i18n, LANGUAGES, type Language } from "@/lib/i18n";

let started = false;

function send(lang: string) {
	const language: Language = LANGUAGES.includes(lang as Language) ? (lang as Language) : "en";
	appApi.setLang(language).catch((err: unknown) => console.warn("app_set_lang", err));
}

/** Ядро показывает свои тексты вне окна (статус Discord, меню трея) — сообщаем ему язык интерфейса. */
export function initCoreLanguage() {
	if (started) return;
	started = true;
	send(i18n.language);
	i18n.on("languageChanged", send);
}
