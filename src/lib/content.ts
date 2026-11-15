import { useEffect, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { contentApi, type ContentItem, type FileRef } from "@/lib/api";
import { errorText, toAppError, type AppError } from "@/lib/errors";

export type InstanceContent = {
	status: "loading" | "ready" | "error";
	items: ContentItem[];
	offline: boolean;
	error: AppError | null;
};

let cache: Record<string, InstanceContent> = {};
const listeners = new Set<() => void>();
const requests = new Map<string, number>();
let requestSeq = 0;

function emit() {
	for (const listener of listeners) listener();
}

function put(id: string, value: InstanceContent) {
	cache = { ...cache, [id]: value };
	emit();
}

/** `retry` — повтор после сбоя, не больше одного. */
export async function loadContent(id: string, retry = true) {
	const seq = ++requestSeq;
	requests.set(id, seq);
	const previous = cache[id];
	if (!previous || previous.status === "error")
		put(id, { status: "loading", items: previous?.items ?? [], offline: false, error: null });
	try {
		const result = await contentApi.list(id);
		if (requests.get(id) !== seq) return;
		put(id, { status: "ready", items: result.items, offline: result.offline, error: null });
	} catch (err) {
		if (requests.get(id) !== seq) return;
		console.error("instance_content", err);
		// Сбой бывает, когда ядро в этот момент само пишет `content.json`: показанный список не прячем и пробуем ещё раз.
		if (previous?.status === "ready") {
			if (retry) setTimeout(() => void loadContent(id, false), 1000);
			return;
		}
		put(id, { status: "error", items: [], offline: false, error: toAppError(err) });
	}
}

/** Ответ команды — сразу в список, затем перечитать. */
export function applyAddedContent(id: string, added: ContentItem[]) {
	const entry = cache[id];
	if (entry?.status === "ready" && added.length) {
		const same = (a: ContentItem, b: ContentItem) =>
			a.file === b.file && a.type === b.type && (a.world ?? null) === (b.world ?? null);
		put(id, {
			...entry,
			items: [...entry.items.filter((item) => !added.some((other) => same(item, other))), ...added],
		});
	}
	void loadContent(id);
}

export function ensureContent(id: string) {
	const entry = cache[id];
	if (!entry || entry.status === "error") void loadContent(id);
}

const unlisteners: Array<() => void> = [];
let started = false;

export function initContent() {
	if (started) return;
	started = true;
	void contentApi
		.onContentChanged((id) => {
			if (cache[id]) void loadContent(id);
		})
		.then((unlisten) => unlisteners.push(unlisten))
		.catch((err: unknown) => console.error("Не удалось подписаться на instance-content-changed", err));
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

/* Проверка обновлений — при открытии вкладки, не чаще раза в 10 минут. */

const UPDATES_INTERVAL = 10 * 60 * 1000;

export type UpdatesCheck = {
	status: "checking" | "done" | "error";
	checkedAt: number | null;
};

let checks: Record<string, UpdatesCheck> = {};

function sameFile(a: ContentItem, b: ContentItem) {
	return a.file === b.file && a.type === b.type && (a.world ?? null) === (b.world ?? null);
}

/** Тихая проверка: ошибку не показывает. */
export async function checkContentUpdates(id: string, force = false) {
	const check = checks[id];
	if (check?.status === "checking") return;
	if (!force && check?.checkedAt && Date.now() - check.checkedAt < UPDATES_INTERVAL) return;
	checks = { ...checks, [id]: { status: "checking", checkedAt: check?.checkedAt ?? null } };
	emit();
	try {
		const items = await contentApi.updates(id, force);
		checks = { ...checks, [id]: { status: "done", checkedAt: Date.now() } };
		const entry = cache[id];
		if (entry?.status === "ready") {
			put(id, {
				...entry,
				items: entry.items.map((item) => {
					const fresh = items.find((other) => sameFile(item, other));
					return fresh ? { ...item, update: fresh.update } : item;
				}),
			});
		} else emit();
	} catch (err) {
		console.error("instance_content_updates", err);
		checks = { ...checks, [id]: { status: "error", checkedAt: check?.checkedAt ?? null } };
		emit();
	}
}

export function useUpdatesCheck(id: string): UpdatesCheck | null {
	return useSyncExternalStore(subscribe, () => checks[id] ?? null);
}

export function useContentCache(): Record<string, InstanceContent> {
	return useSyncExternalStore(subscribe, () => cache);
}

export function useInstanceContent(id: string | null): InstanceContent | null {
	const all = useContentCache();
	useEffect(() => {
		if (id) ensureContent(id);
	}, [id]);
	return id ? (all[id] ?? null) : null;
}

export function findProjectItem(content: InstanceContent | null | undefined, projectId: string) {
	if (!content || content.status !== "ready") return null;
	return content.items.find((item) => item.project?.id === projectId) ?? null;
}

const LOADER_PARTS = new Set(["fabric", "forge", "neoforge", "quilt", "iris", "optifine"]);

/** `mc26.2-0.6.13-fabric` → `0.6.13`, `0.119.2+26.2` → `0.119.2`, `Fabric_26.2_25.2.0` → `25.2.0`. */
export function shortVersion(version: string, gameVersion: string): string {
	const game = gameVersion.toLowerCase();
	const parts = version.split(/[-+_ ]+/).filter((part) => {
		const lower = part.toLowerCase();
		if (!lower || LOADER_PARTS.has(lower) || lower === game || lower === `mc${game}`) return false;
		return !/^mc.*\d/.test(lower);
	});
	if (!parts.length) return version;
	parts[0] = parts[0].replace(/^v(?=\d)/i, "");
	return parts.join("-") || version;
}

export function fileRef(item: ContentItem): FileRef {
	return item.world
		? { file: item.file, type: item.type, world: item.world }
		: { file: item.file, type: item.type };
}

/** Ошибку показывает тостом. */
export async function removeContent(id: string, items: ContentItem[]): Promise<boolean> {
	try {
		await contentApi.remove(id, items.map(fileRef));
		void loadContent(id);
		return true;
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		return false;
	}
}

/** Сразу в списке, при ошибке — назад. */
export async function toggleContent(id: string, item: ContentItem, enabled: boolean): Promise<boolean> {
	const swap = (next: ContentItem) => {
		const entry = cache[id];
		if (entry?.status !== "ready") return;
		put(id, {
			...entry,
			items: entry.items.map((other) =>
				other.file === item.file && other.type === item.type && other.world === item.world
					? next
					: other,
			),
		});
	};
	swap({ ...item, enabled });
	try {
		swap(await contentApi.toggle(id, fileRef(item), enabled));
		return true;
	} catch (err) {
		swap(item);
		toast.error(errorText(toAppError(err)));
		return false;
	}
}

import.meta.hot?.dispose(() => {
	for (const unlisten of unlisteners) unlisten();
});
