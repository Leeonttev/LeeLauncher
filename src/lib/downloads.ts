import { useEffect, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { downloadsApi, type Download } from "@/lib/api";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { isPrimaryMod } from "@/lib/platform";

/** Столько же хранит ядро. */
const DONE_LIMIT = 20;

export type DownloadsStore = {
	status: "idle" | "loading" | "ready" | "error";
	error: AppError | null;
	items: Download[];
	/** Завершённое позже этого времени ещё не видели: точка на кнопке. */
	seenUntil: number;
	open: boolean;
};

let state: DownloadsStore = { status: "idle", error: null, items: [], seenUntil: Date.now(), open: false };
const listeners = new Set<() => void>();

function set(next: Partial<DownloadsStore>) {
	state = { ...state, ...next };
	for (const listener of listeners) listener();
}

function applyList(items: Download[]) {
	set({ status: "ready", error: null, items });
}

/** Устаревшее событие не возвращает завершённую задачу в «Сейчас». */
function applyProgress(download: Download) {
	const index = state.items.findIndex((d) => d.id === download.id);
	if (index < 0) return;
	const current = state.items[index];
	if (current.state === "finished" || current.state === "failed") return;
	const items = [...state.items];
	items[index] = download;
	set({ items });
}

const unlisteners: Array<() => void> = [];
let started = false;

function listen(subscribe: Promise<() => void>, name: string) {
	void subscribe
		.then((unlisten) => unlisteners.push(unlisten))
		.catch((err: unknown) => console.error(`Не удалось подписаться на ${name}`, err));
}

export function initDownloads() {
	if (started) return;
	started = true;
	listen(downloadsApi.onChanged(applyList), "downloads-changed");
	listen(downloadsApi.onProgress(applyProgress), "download-progress");
	void reloadDownloads();
}

export async function reloadDownloads() {
	if (state.status !== "ready") set({ status: "loading", error: null });
	try {
		applyList(await downloadsApi.list());
	} catch (err) {
		console.error("downloads_list", err);
		if (state.status !== "ready") set({ status: "error", error: toAppError(err) });
	}
}

function showError(err: unknown) {
	toast.error(errorText(toAppError(err)));
}

function withoutItem(id: string) {
	set({ items: state.items.filter((d) => d.id !== id) });
}

export async function cancelDownload(id: string) {
	try {
		await downloadsApi.cancel(id);
	} catch (err) {
		showError(err);
	}
}

export async function retryDownload(id: string) {
	try {
		const next = await downloadsApi.retry(id);
		// Ответ — сразу в список, не дожидаясь `downloads-changed`.
		if (!state.items.some((d) => d.id === next.id)) {
			set({ items: [next, ...state.items.filter((d) => d.id !== id)] });
		}
	} catch (err) {
		showError(err);
	}
}

export async function dismissDownload(id: string) {
	withoutItem(id);
	try {
		await downloadsApi.dismiss(id);
	} catch (err) {
		showError(err);
		void reloadDownloads();
	}
}

export async function clearDownloads() {
	set({ items: state.items.filter((d) => !isDone(d)) });
	try {
		await downloadsApi.clear();
	} catch (err) {
		showError(err);
		void reloadDownloads();
	}
}

export function isDone(download: Download) {
	return download.state === "finished" || download.state === "failed";
}

export function downloadSections(items: Download[]) {
	const active = items.filter((d) => d.state === "running");
	const queued = items.filter((d) => d.state === "queued").sort((a, b) => a.queuedAt - b.queuedAt);
	const done = items
		.filter(isDone)
		.sort((a, b) => (b.finishedAt ?? 0) - (a.finishedAt ?? 0))
		.slice(0, DONE_LIMIT);
	return { active, queued, done };
}

/** Байты по задачам с известным размером, иначе средний `percent`. */
export function overallPercent(items: Download[]): number | null {
	const running = items.filter((d) => d.state === "running");
	const sized = running.filter((d) => d.bytesTotal !== null && d.bytesTotal > 0);
	if (sized.length) {
		const done = sized.reduce((sum, d) => sum + Math.min(d.bytesDone, d.bytesTotal ?? 0), 0);
		const total = sized.reduce((sum, d) => sum + (d.bytesTotal ?? 0), 0);
		return Math.min(100, Math.max(0, (done / total) * 100));
	}
	const known = running.filter((d) => d.percent !== null);
	if (!known.length) return null;
	return known.reduce((sum, d) => sum + (d.percent ?? 0), 0) / known.length;
}

export function buttonState(store: DownloadsStore): "active" | "failed" | "new" | "idle" {
	const items = store.items;
	if (items.some((d) => d.state === "running" || d.state === "queued")) return "active";
	const unseen = items.filter((d) => isDone(d) && (d.finishedAt ?? 0) > store.seenUntil);
	if (unseen.some((d) => d.state === "failed")) return "failed";
	if (unseen.length) return "new";
	return "idle";
}

/** И открытие, и закрытие помечают завершённое просмотренным. */
export function setDownloadsOpen(open: boolean) {
	if (open === state.open) return;
	set({ open, seenUntil: Date.now() });
}

export function subscribeDownloads(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

const subscribe = subscribeDownloads;

export function getDownloads(): DownloadsStore {
	return state;
}

export function addDownload(download: Download) {
	if (state.items.some((d) => d.id === download.id)) return;
	set({ items: [download, ...state.items] });
}

export function useDownloads(): DownloadsStore {
	return useSyncExternalStore(subscribe, () => state);
}

function blockingOverlay() {
	return Boolean(
		document.querySelector(
			'[role="dialog"]:not([data-downloads-panel]), [role="alertdialog"], [role="menu"]',
		),
	);
}

/** По `event.code`, как в браузерах. */
export function useDownloadsShortcut() {
	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.code !== "KeyJ" || !isPrimaryMod(event)) return;
			// WebView2 по Ctrl+J открывает свои загрузки — гасим всегда.
			event.preventDefault();
			if (event.repeat || blockingOverlay()) return;
			setDownloadsOpen(!state.open);
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);
}

import.meta.hot?.dispose(() => {
	for (const unlisten of unlisteners) unlisten();
});
