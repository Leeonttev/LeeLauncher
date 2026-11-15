import i18next from "i18next";
import { useSyncExternalStore } from "react";
import { initReactI18next } from "react-i18next";
import { readStorage, writeStorage } from "@/lib/storage";
import { en } from "@/locales/en";
import { ru } from "@/locales/ru";

export const LANGUAGES = ["ru", "en"] as const;
export type Language = (typeof LANGUAGES)[number];

export const LANGUAGE_INFO: Record<Language, { code: string; name: string; description: string }> = {
	ru: { code: "RU", name: "Русский", description: "Интерфейс на русском" },
	en: { code: "EN", name: "English", description: "English interface" },
};

export const resources = {
	ru: { translation: ru },
	en: { translation: en },
} as const;

export type LanguageState = {
	/** `null` — язык ещё не выбран. */
	language: Language | null;
	resolvedLanguage: Language;
};

const KEY = "language";
/** Повторено в `public/theme-init.js`. */
const RUSSIAN_DEFAULT = ["ru", "be", "kk", "ky"];

function isLanguage(value: unknown): value is Language {
	return LANGUAGES.includes(value as Language);
}

export function systemLanguage(): Language {
	try {
		const tag = navigator.languages?.[0] ?? navigator.language ?? "";
		const base = tag.toLowerCase().split("-")[0];
		return RUSSIAN_DEFAULT.includes(base) ? "ru" : "en";
	} catch {
		return "en";
	}
}

function load(): LanguageState {
	const stored = readStorage(KEY);
	const language = isLanguage(stored) ? stored : null;
	return { language, resolvedLanguage: language ?? systemLanguage() };
}

let state = load();
const listeners = new Set<() => void>();

export const i18n = i18next.createInstance();
void i18n.use(initReactI18next).init({
	resources,
	lng: state.resolvedLanguage,
	fallbackLng: "ru",
	supportedLngs: LANGUAGES,
	initAsync: false,
	interpolation: { escapeValue: false, prefix: "{", suffix: "}" },
	returnNull: false,
});

document.documentElement.lang = state.resolvedLanguage;

function update(next: LanguageState) {
	const changed = next.resolvedLanguage !== state.resolvedLanguage;
	state = next;
	document.documentElement.lang = next.resolvedLanguage;
	if (changed) void i18n.changeLanguage(next.resolvedLanguage);
	for (const listener of listeners) listener();
}

export function isLanguageChosen(): boolean {
	return state.language !== null;
}

export function setLanguage(language: Language) {
	if (!writeStorage(KEY, language)) console.warn("Не удалось сохранить язык: localStorage недоступен");
	update({ language, resolvedLanguage: language });
}

/** Без сохранения выбора — для экрана первого запуска. */
export function previewLanguage(language: Language) {
	update({ language: state.language, resolvedLanguage: language });
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useLanguage(): LanguageState {
	return useSyncExternalStore(subscribe, () => state);
}
