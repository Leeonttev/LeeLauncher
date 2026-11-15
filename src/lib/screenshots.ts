import { useEffect, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { contentApi, type Screenshot } from "@/lib/api";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";

export const SCREENSHOTS_PAGE = 60;
/** Больше за один вызов ядро не отдаёт. */
const MAX_LIMIT = 200;

export type InstanceScreenshots = {
	status: "loading" | "ready" | "error";
	items: Screenshot[];
	total: number;
	error: AppError | null;
	loadingMore: boolean;
	moreError: AppError | null;
};

let cache: Record<string, InstanceScreenshots> = {};
const listeners = new Set<() => void>();
const requests = new Map<string, number>();
let requestSeq = 0;

function put(id: string, value: InstanceScreenshots) {
	cache = { ...cache, [id]: value };
	for (const listener of listeners) listener();
}

/** После изменения папки — столько же, сколько было показано. */
export async function loadScreenshots(id: string) {
	const seq = ++requestSeq;
	requests.set(id, seq);
	const previous = cache[id];
	const limit = Math.min(MAX_LIMIT, Math.max(SCREENSHOTS_PAGE, previous?.items.length ?? 0));
	if (!previous || previous.status === "error")
		put(id, { status: "loading", items: [], total: 0, error: null, loadingMore: false, moreError: null });
	try {
		const result = await contentApi.screenshots(id, 0, limit);
		if (requests.get(id) !== seq) return;
		put(id, {
			status: "ready",
			items: result.items,
			total: result.total,
			error: null,
			loadingMore: false,
			moreError: null,
		});
	} catch (err) {
		if (requests.get(id) !== seq) return;
		console.error("instance_screenshots", err);
		if (previous?.status === "ready") return;
		put(id, {
			status: "error",
			items: [],
			total: 0,
			error: toAppError(err),
			loadingMore: false,
			moreError: null,
		});
	}
}

export async function loadMoreScreenshots(id: string) {
	const entry = cache[id];
	if (!entry || entry.status !== "ready" || entry.loadingMore || entry.items.length >= entry.total) return;
	const seq = requests.get(id);
	put(id, { ...entry, loadingMore: true, moreError: null });
	try {
		const result = await contentApi.screenshots(id, entry.items.length, SCREENSHOTS_PAGE);
		const current = cache[id];
		if (requests.get(id) !== seq || !current) return;
		const known = new Set(current.items.map((s) => s.file));
		put(id, {
			...current,
			items: [...current.items, ...result.items.filter((s) => !known.has(s.file))],
			total: result.total,
			loadingMore: false,
		});
	} catch (err) {
		const current = cache[id];
		if (requests.get(id) !== seq || !current) return;
		put(id, { ...current, loadingMore: false, moreError: toAppError(err) });
	}
}

const unlisteners: Array<() => void> = [];
let started = false;

export function initScreenshots() {
	if (started) return;
	started = true;
	void contentApi
		.onScreenshotsChanged((id) => {
			if (cache[id]) void loadScreenshots(id);
		})
		.then((unlisten) => unlisteners.push(unlisten))
		.catch((err: unknown) =>
			console.error("Не удалось подписаться на instance-screenshots-changed", err),
		);
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

/** Миниатюры готовит ядро, поэтому загружаем только по обращению. */
export function useScreenshots(id: string, load: boolean): InstanceScreenshots | null {
	const all = useSyncExternalStore(subscribe, () => cache);
	useEffect(() => {
		if (!load) return;
		const entry = cache[id];
		if (!entry || entry.status === "error") void loadScreenshots(id);
	}, [id, load]);
	return all[id] ?? null;
}

export async function copyScreenshot(id: string, file: string): Promise<boolean> {
	try {
		await contentApi.copyScreenshot(id, file);
		toast.success(i18n.t("viewer.copied"));
		return true;
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		return false;
	}
}

export async function removeScreenshot(id: string, file: string): Promise<boolean> {
	try {
		await contentApi.removeScreenshot(id, file);
		const entry = cache[id];
		if (entry)
			put(id, {
				...entry,
				items: entry.items.filter((s) => s.file !== file),
				total: Math.max(0, entry.total - 1),
			});
		return true;
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		return false;
	}
}

import.meta.hot?.dispose(() => {
	for (const unlisten of unlisteners) unlisten();
});
