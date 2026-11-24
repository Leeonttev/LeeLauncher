import { i18n } from "@/lib/i18n";
import { ru } from "@/locales/ru";

export type AppError = {
	code: string;
	/** Запасной текст ядра — для кода, которого нет в переводах. */
	message: string;
	/** Технические подробности для «Подробнее». */
	details?: string;
};

type ErrorCode = Exclude<keyof typeof ru.errors, "fallback">;

export function isAppError(value: unknown): value is AppError {
	return (
		typeof value === "object" &&
		value !== null &&
		typeof (value as AppError).code === "string" &&
		typeof (value as AppError).message === "string"
	);
}

/** Не ошибка ядра — код `internal`. */
export function toAppError(value: unknown): AppError {
	if (isAppError(value)) return value;
	return { code: "internal", message: "", details: value instanceof Error ? value.message : String(value) };
}

function isKnownCode(code: string): code is ErrorCode {
	return code !== "fallback" && Object.hasOwn(ru.errors, code);
}

/** В компоненте вызывать вместе с `useTranslation()`, чтобы текст обновлялся при смене языка. */
export function errorText(error: AppError): string {
	// У «нет версии» ядро уточняет причину в `details`: ванильная сборка или модпак.
	if (error.code === "content_incompatible" && error.details === "needs_loader")
		return i18n.t("errorDetails.needsLoader");
	if (error.code === "content_incompatible" && error.details === "modpack")
		return i18n.t("errorDetails.modpack");
	// Каталог отклонил ключ лаунчера (401/403) — не то же, что «нет ключа в этой сборке».
	if (error.code === "source_disabled" && error.details === "denied")
		return i18n.t("errorDetails.sourceDenied");
	if (error.code === "invalid_name" && error.details === "skin") return i18n.t("errorDetails.skinName");
	if (error.code === "java_invalid" && error.details === "rosetta") return i18n.t("errorDetails.rosetta");
	if (isKnownCode(error.code)) return i18n.t(`errors.${error.code}`);
	return error.message.trim() || i18n.t("errors.fallback");
}
