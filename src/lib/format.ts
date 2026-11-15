import { i18n } from "@/lib/i18n";

const DAY = 24 * 60 * 60 * 1000;

function lang() {
	return i18n.language || "ru";
}

/** От 10 часов — только часы. В компоненте вызывать вместе с `useTranslation()`. */
export function formatDuration(seconds: number): string {
	if (seconds < 60) return i18n.t("time.lessThanMinute");
	const totalMinutes = Math.floor(seconds / 60);
	let hours = Math.floor(totalMinutes / 60);
	let minutes = totalMinutes % 60;
	if (hours >= 10) {
		hours = Math.round(totalMinutes / 60);
		minutes = 0;
	}
	const parts = hours > 0 ? (minutes > 0 ? { hours, minutes } : { hours }) : { minutes };
	if (typeof Intl.DurationFormat === "function") {
		return new Intl.DurationFormat(lang(), { style: "narrow" }).format(parts);
	}
	const unit = (value: number, name: "hour" | "minute") =>
		new Intl.NumberFormat(lang(), { style: "unit", unit: name, unitDisplay: "narrow" }).format(value);
	return [hours > 0 ? unit(hours, "hour") : null, minutes > 0 ? unit(minutes, "minute") : null]
		.filter(Boolean)
		.join(" ");
}

function startOfDay(date: Date) {
	return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/** По дням; старше 30 дней — дата. */
export function formatWhen(iso: string, now = Date.now()): string {
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return "";
	const days = Math.round((startOfDay(new Date(now)) - startOfDay(date)) / DAY);
	if (days >= 0 && days <= 30) {
		return new Intl.RelativeTimeFormat(lang(), { numeric: "auto" }).format(-days, "day");
	}
	const sameYear = date.getFullYear() === new Date(now).getFullYear();
	return new Intl.DateTimeFormat(lang(), {
		day: "numeric",
		month: "short",
		year: sameYear ? undefined : "numeric",
	}).format(date);
}

export function formatWhenCapitalized(iso: string, now?: number): string {
	const text = formatWhen(iso, now);
	return text.charAt(0).toLocaleUpperCase(lang()) + text.slice(1);
}

/** Не текущего года — с годом. */
export function formatDate(iso: string, now = Date.now()): string {
	const date = new Date(iso);
	if (Number.isNaN(date.getTime())) return "";
	const sameYear = date.getFullYear() === new Date(now).getFullYear();
	return new Intl.DateTimeFormat(lang(), {
		day: "numeric",
		month: "long",
		year: sameYear ? undefined : "numeric",
	}).format(date);
}

function unitNumber(value: number, unit: "megabyte" | "gigabyte" | "kilobyte") {
	return new Intl.NumberFormat(lang(), {
		style: "unit",
		unit,
		maximumFractionDigits: value < 10 ? 1 : 0,
	}).format(value);
}

export function formatBytes(bytes: number): string {
	const mb = bytes / (1024 * 1024);
	if (mb >= 1024) return unitNumber(mb / 1024, "gigabyte");
	if (mb >= 1) return unitNumber(mb, "megabyte");
	return unitNumber(bytes / 1024, "kilobyte");
}

export function formatMemory(mb: number): string {
	return unitNumber(mb / 1024, "gigabyte");
}

export function formatGbNumber(mb: number): string {
	return new Intl.NumberFormat(lang(), { maximumFractionDigits: 1, useGrouping: false }).format(mb / 1024);
}

export function formatPercent(value: number): string {
	return new Intl.NumberFormat(lang(), { style: "percent", maximumFractionDigits: 0 }).format(value / 100);
}

export function formatNumber(value: number): string {
	return new Intl.NumberFormat(lang()).format(value);
}

export function formatAgo(ms: number, now = Date.now()): string {
	const seconds = Math.max(0, Math.round((now - ms) / 1000));
	const rtf = new Intl.RelativeTimeFormat(lang(), { numeric: "auto" });
	if (seconds < 60) return rtf.format(0, "second");
	const minutes = Math.floor(seconds / 60);
	if (minutes < 60) return rtf.format(-minutes, "minute");
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return rtf.format(-hours, "hour");
	return formatWhen(new Date(ms).toISOString(), now);
}

export function formatCompact(value: number): string {
	return new Intl.NumberFormat(lang(), { notation: "compact", maximumFractionDigits: 1 }).format(value);
}

export function formatDateTime(ms: number, now = Date.now()): string {
	const date = new Date(ms);
	if (Number.isNaN(date.getTime())) return "";
	const time = new Intl.DateTimeFormat(lang(), { hour: "2-digit", minute: "2-digit" }).format(date);
	return `${formatWhenCapitalized(date.toISOString(), now)}, ${time}`;
}
