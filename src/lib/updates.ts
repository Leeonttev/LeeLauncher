import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import {
	downloadsApi,
	updatesApi,
	type Download,
	type LastUpdate,
	type UpdateInfo,
	type UpdateProgress,
} from "@/lib/api";
import { getDownloads, subscribeDownloads } from "@/lib/downloads";
import { toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";
import { getInstances, stopInstance, subscribeInstances } from "@/lib/instances";
import { skinChangeBusy } from "@/lib/skins";

const MINUTE = 60_000;
const CHECK_EVERY = 120 * MINUTE;
const CHECK_ON_FOCUS_AFTER = 60 * MINUTE;
const CHECK_MIN_GAP = 10 * MINUTE;
const STARTUP_WAIT = 10_000;
const QUIET_LIMIT = 15 * MINUTE;
const QUIET_POLL = 2000;
/** Скачивание, начатое до перезагрузки окна, считается зависшим, если событий нет столько времени. */
const STALL_LIMIT = 20_000;
/** После стольких неудачных попыток подряд можно продолжить без обновления. */
const SKIP_AFTER = 2;

export const RELEASES_URL = "https://github.com/leeonttev/LeeLauncher/releases/latest";

/** `ready` — пакет скачан, установка ждёт, пока закроются игры и закончатся загрузки. */
export type UpdatePhase = "found" | "downloading" | "ready" | "installing" | "error";

export type UpdatesStore = {
	info: UpdateInfo | null;
	phase: UpdatePhase;
	visible: boolean;
	progress: UpdateProgress | null;
	error: AppError | null;
	failures: number;
	/** До перезапуска или следующей проверки, нашедшей обновление. */
	skipped: boolean;
	lastUpdate: LastUpdate | null;
	notesOpen: boolean;
	/** Ядро ответило `game_running`, а в сторе сборок игры нет (запущена до перезагрузки окна). */
	coreGame: boolean;
};

let state: UpdatesStore = {
	info: null,
	phase: "found",
	visible: false,
	progress: null,
	error: null,
	failures: 0,
	skipped: false,
	lastUpdate: null,
	notesOpen: false,
	coreGame: false,
};
const listeners = new Set<() => void>();

function set(next: Partial<UpdatesStore>) {
	state = { ...state, ...next };
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useUpdates(): UpdatesStore {
	return useSyncExternalStore(subscribe, () => state);
}

let started = false;
let checking = false;
let lastCheckAt = 0;
let quietTimer: ReturnType<typeof setInterval> | null = null;
let pendingSince = 0;
let skippedAt = 0;
let retryTimer: ReturnType<typeof setTimeout> | null = null;
/** Скачивание идёт в ядре, но не по нашему вызову: ответа команды не будет, следим по событиям. */
let adopted = false;
let lastProgressAt = 0;
let stallTimer: ReturnType<typeof setInterval> | null = null;
const cleanups: Array<() => void> = [];

export function initUpdates() {
	if (started) return;
	started = true;

	void updatesApi
		.onProgress(onProgress)
		.then((unlisten) => cleanups.push(unlisten))
		.catch((err: unknown) => console.error("app-update-progress", err));

	const frame = requestAnimationFrame(() => void check(true));
	cleanups.push(() => cancelAnimationFrame(frame));

	const timer = setInterval(() => void check(false), CHECK_EVERY);
	cleanups.push(() => clearInterval(timer));

	const onFocus = () => {
		if (Date.now() - lastCheckAt > CHECK_ON_FOCUS_AFTER) void check(false);
	};
	window.addEventListener("focus", onFocus);
	cleanups.push(() => window.removeEventListener("focus", onFocus));

	cleanups.push(subscribeInstances(continueWhenFree));
	cleanups.push(subscribeDownloads(continueWhenFree));

	void showLastUpdate();
}

function onProgress(progress: UpdateProgress) {
	if (state.phase !== "downloading") return;
	lastProgressAt = Date.now();
	// При повторе после обрыва размер может не прийти — держим прошлый, чтобы полоса не сбрасывалась.
	const total = progress.total ?? state.progress?.total ?? null;
	set({ progress: { ...progress, total } });
	if (adopted && total !== null && progress.downloaded >= total) {
		stopAdopted();
		if (state.info) set({ phase: "ready", info: { ...state.info, downloaded: true } });
		retryLater(1000);
	}
}

function stopAdopted() {
	adopted = false;
	if (stallTimer) clearInterval(stallTimer);
	stallTimer = null;
}

function adoptDownload() {
	stopAdopted();
	adopted = true;
	lastProgressAt = Date.now();
	set({ phase: "downloading", error: null });
	stallTimer = setInterval(() => {
		if (state.phase !== "downloading") {
			stopAdopted();
			return;
		}
		if (Date.now() - lastProgressAt < STALL_LIMIT) return;
		stopAdopted();
		set({
			phase: "error",
			error: { code: "update_stalled", message: "" },
			failures: state.failures + 1,
		});
	}, 2000);
}

function retryLater(ms: number) {
	if (retryTimer) clearTimeout(retryTimer);
	retryTimer = setTimeout(continueWhenFree, ms);
}

function busyWithUpdate() {
	return state.info !== null && !state.skipped && (state.visible || quietTimer !== null);
}

async function check(startup: boolean) {
	if (checking || busyWithUpdate()) return;
	if (!startup && Date.now() - lastCheckAt < CHECK_MIN_GAP) return;
	if (state.skipped && Date.now() - skippedAt < CHECK_EVERY) return;
	checking = true;
	lastCheckAt = Date.now();
	let late = false;
	const wait = startup
		? setTimeout(() => {
				late = true;
			}, STARTUP_WAIT)
		: null;
	try {
		const info = await updatesApi.check();
		if (!info) return;
		set({
			info,
			phase: "found",
			progress: null,
			error: null,
			failures: 0,
			skipped: false,
		});
		if (startup && !late) set({ visible: true });
		else showWhenQuiet();
	} catch (err) {
		const error = toAppError(err);
		if (error.code === "update_unavailable")
			console.info("app_update_check", error.details ?? error.message);
		else console.warn("app_update_check", error);
	} finally {
		if (wait) clearTimeout(wait);
		checking = false;
	}
}

function isQuiet() {
	if (document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"]'))
		return false;
	return !skinChangeBusy();
}

function stopQuietTimer() {
	if (quietTimer) clearInterval(quietTimer);
	quietTimer = null;
}

function showWhenQuiet() {
	stopQuietTimer();
	pendingSince = Date.now();
	const tryShow = () => {
		if (isQuiet() || Date.now() - pendingSince > QUIET_LIMIT) {
			stopQuietTimer();
			set({ visible: true });
		}
	};
	quietTimer = setInterval(tryShow, QUIET_POLL);
	tryShow();
}

export function activeGames(): string[] {
	return Object.keys(getInstances().processes);
}

export function activeTasks(items: Download[] = getDownloads().items): Download[] {
	return items.filter((d) => d.state === "running" || d.state === "queued");
}

function fail(err: unknown, installing = false) {
	const error = toAppError(err);
	if (error.code === "cancelled") {
		set({ phase: "found", progress: null, coreGame: false });
		return;
	}
	if (error.code === "update_in_progress") {
		// Окно перезагрузили, пока ядро качало или ставило.
		if (installing) {
			set({ phase: "ready" });
			retryLater(2000);
		} else adoptDownload();
		return;
	}
	if (error.code === "game_running") {
		// Выход из игры, о которой стор не знает, виден только по ответу ядра — спрашиваем снова.
		set({ phase: "ready", coreGame: activeGames().length === 0 });
		retryLater(5000);
		return;
	}
	set({ phase: "error", error, failures: state.failures + 1, coreGame: false });
}

/** «Обновить» и «Повторить». */
export async function startUpdate() {
	if (!state.info || state.phase === "downloading" || state.phase === "installing") return;
	if (state.info.downloaded) {
		set({ phase: "ready", error: null });
		continueWhenFree();
		return;
	}
	set({ phase: "downloading", progress: null, error: null });
	lastProgressAt = Date.now();
	try {
		await updatesApi.download();
		set({ phase: "ready", info: { ...state.info, downloaded: true } });
		continueWhenFree();
	} catch (err) {
		fail(err);
	}
}

function continueWhenFree() {
	if (state.phase !== "ready" || activeGames().length > 0 || activeTasks().length > 0) return;
	set({ phase: "installing", coreGame: false });
	updatesApi
		.install()
		.then(() => updatesApi.restart())
		.catch((err: unknown) => fail(err, true));
}

export async function cancelTasksAndUpdate() {
	const tasks = activeTasks();
	await Promise.all(
		tasks.map((task) => downloadsApi.cancel(task.id).catch((err: unknown) => console.warn(err))),
	);
}

export async function stopGames() {
	await Promise.all(activeGames().map((id) => stopInstance(id)));
}

export function skipUpdate() {
	skippedAt = Date.now();
	set({ skipped: true, visible: false });
}

export function reopenUpdate() {
	set({ visible: true, skipped: false });
}

async function showLastUpdate() {
	try {
		const last = await updatesApi.lastUpdate();
		if (!last) return;
		set({ lastUpdate: last });
		toast.success(i18n.t("update.toast.title", { version: last.version }), {
			description: i18n.t("update.toast.text"),
			duration: 10_000,
			action: last.notes?.trim()
				? { label: i18n.t("update.toast.notes"), onClick: () => set({ notesOpen: true }) }
				: undefined,
		});
	} catch (err) {
		console.warn("app_last_update", err);
	}
}

export function setNotesOpen(open: boolean) {
	set({ notesOpen: open });
}

/** Если в описании есть блоки `<!-- ru -->` и `<!-- en -->`, берётся блок языка интерфейса. */
export function localizedNotes(notes: string | null, lang: string): string | null {
	if (!notes?.trim()) return null;
	const marker = /<!--\s*(ru|en)\s*-->/gi;
	const parts: Record<string, string> = {};
	const matches = [...notes.matchAll(marker)];
	if (!matches.length) return notes;
	matches.forEach((match, i) => {
		const start = (match.index ?? 0) + match[0].length;
		const end = matches[i + 1]?.index ?? notes.length;
		parts[match[1].toLowerCase()] = notes.slice(start, end).trim();
	});
	return parts[lang] || parts.en || parts.ru || notes;
}

export function canSkip(store: UpdatesStore) {
	return store.phase === "error" && store.failures >= SKIP_AFTER;
}

import.meta.hot?.dispose(() => {
	stopQuietTimer();
	if (retryTimer) clearTimeout(retryTimer);
	stopAdopted();
	for (const cleanup of cleanups) cleanup();
});
