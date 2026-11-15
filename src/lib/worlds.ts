import { useEffect, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { contentApi, type World } from "@/lib/api";
import { errorText, toAppError, type AppError } from "@/lib/errors";

export type InstanceWorlds = {
	status: "loading" | "ready" | "error";
	items: World[];
	error: AppError | null;
};

let cache: Record<string, InstanceWorlds> = {};
const listeners = new Set<() => void>();
const requests = new Map<string, number>();
let requestSeq = 0;

function put(id: string, value: InstanceWorlds) {
	cache = { ...cache, [id]: value };
	for (const listener of listeners) listener();
}

export async function loadWorlds(id: string) {
	const seq = ++requestSeq;
	requests.set(id, seq);
	const previous = cache[id];
	if (!previous || previous.status === "error") put(id, { status: "loading", items: [], error: null });
	try {
		const items = await contentApi.worlds(id);
		if (requests.get(id) !== seq) return;
		put(id, { status: "ready", items, error: null });
	} catch (err) {
		if (requests.get(id) !== seq) return;
		console.error("instance_worlds", err);
		if (previous?.status === "ready") return;
		put(id, { status: "error", items: [], error: toAppError(err) });
	}
}

const unlisteners: Array<() => void> = [];
let started = false;

export function initWorlds() {
	if (started) return;
	started = true;
	void contentApi
		.onWorldsChanged((id) => {
			if (cache[id]) void loadWorlds(id);
		})
		.then((unlisten) => unlisteners.push(unlisten))
		.catch((err: unknown) => console.error("Не удалось подписаться на instance-worlds-changed", err));
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

export function useWorlds(id: string): InstanceWorlds | null {
	const all = useSyncExternalStore(subscribe, () => cache);
	useEffect(() => {
		const entry = cache[id];
		if (!entry || entry.status === "error") void loadWorlds(id);
	}, [id]);
	return all[id] ?? null;
}

/** `null` — ничего не загружает. */
export function useInstanceWorlds(id: string | null): InstanceWorlds | null {
	const all = useSyncExternalStore(subscribe, () => cache);
	useEffect(() => {
		if (!id) return;
		const entry = cache[id];
		if (!entry || entry.status === "error") void loadWorlds(id);
	}, [id]);
	return id ? (all[id] ?? null) : null;
}

export async function removeWorld(id: string, folder: string): Promise<boolean> {
	try {
		await contentApi.removeWorld(id, folder);
		const entry = cache[id];
		if (entry) put(id, { ...entry, items: entry.items.filter((w) => w.folder !== folder) });
		void loadWorlds(id);
		return true;
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		return false;
	}
}

import.meta.hot?.dispose(() => {
	for (const unlisten of unlisteners) unlisten();
});
