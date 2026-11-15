import { useEffect, useSyncExternalStore } from "react";
import { contentApi, instancesApi, type LogFile, type LogLine } from "@/lib/api";
import { toAppError, type AppError } from "@/lib/errors";

/** Как буфер ядра. */
const MAX_LINES = 10_000;

export type GameLog = {
	status: "loading" | "ready" | "error";
	lines: LogLine[];
	/** `live` — запуск в этой сессии, `file` — `logs/latest.log` прошлого запуска. */
	source: "live" | "file" | null;
	file: LogFile | null;
	error: AppError | null;
};

let cache: Record<string, GameLog> = {};
const listeners = new Set<() => void>();
const requests = new Map<string, number>();
let requestSeq = 0;
/** Строки, пришедшие событием, пока идёт `instance_log`: потом склеиваются с ответом. */
const pending = new Map<string, LogLine[]>();

function put(id: string, value: GameLog) {
	cache = { ...cache, [id]: value };
	for (const listener of listeners) listener();
}

function sameLine(a: LogLine, b: LogLine) {
	return a.text === b.text && a.time === b.time && a.thread === b.thread && a.stream === b.stream;
}

/** Начало событий может уже быть в конце буфера ядра. */
function merge(fetched: LogLine[], live: LogLine[]): LogLine[] {
	let overlap = 0;
	for (let k = Math.min(fetched.length, live.length); k > 0; k--) {
		let same = true;
		for (let i = 0; i < k; i++) {
			if (!sameLine(fetched[fetched.length - k + i], live[i])) {
				same = false;
				break;
			}
		}
		if (same) {
			overlap = k;
			break;
		}
	}
	return [...fetched, ...live.slice(overlap)].slice(-MAX_LINES);
}

function onLog(event: { instanceId: string; lines: LogLine[] }) {
	const id = event.instanceId;
	const collecting = pending.get(id);
	if (collecting) {
		collecting.push(...event.lines);
		return;
	}
	const entry = cache[id];
	if (!entry || entry.status !== "ready") return;
	const base = entry.source === "live" ? entry.lines : [];
	put(id, {
		...entry,
		source: "live",
		file: null,
		lines: [...base, ...event.lines].slice(-MAX_LINES),
	});
}

function onStarting(id: string) {
	requests.set(id, ++requestSeq);
	pending.delete(id);
	put(id, { status: "ready", lines: [], source: "live", file: null, error: null });
}

export async function loadGameLog(id: string) {
	const seq = ++requestSeq;
	requests.set(id, seq);
	const previous = cache[id];
	if (!previous || previous.status === "error")
		put(id, { status: "loading", lines: [], source: null, file: null, error: null });
	pending.set(id, []);
	try {
		const fetched = await instancesApi.log(id);
		if (requests.get(id) !== seq) return;
		const live = pending.get(id) ?? [];
		pending.delete(id);
		if (fetched.length || live.length) {
			put(id, {
				status: "ready",
				lines: merge(fetched, live),
				source: "live",
				file: null,
				error: null,
			});
			return;
		}
		// Игру в этой сессии не запускали — показываем `latest.log` прошлого запуска.
		const files = await contentApi.logsList(id);
		if (requests.get(id) !== seq) return;
		const file = files.find((f) => f.kind === "latest") ?? files.find((f) => f.kind === "launcher");
		if (!file) {
			put(id, { status: "ready", lines: [], source: null, file: null, error: null });
			return;
		}
		const content = await contentApi.logsRead(id, file.name);
		if (requests.get(id) !== seq) return;
		const late = cache[id];
		// Пока читали файл, игра могла запуститься — тогда живой лог важнее.
		if (late?.source === "live" && late.lines.length) return;
		put(id, { status: "ready", lines: content.lines, source: "file", file, error: null });
	} catch (err) {
		if (requests.get(id) !== seq) return;
		pending.delete(id);
		console.error("instance_log", err);
		if (previous?.status === "ready") return;
		put(id, { status: "error", lines: [], source: null, file: null, error: toAppError(err) });
	}
}

const unlisteners: Array<() => void> = [];
let started = false;

export function initGameLog() {
	if (started) return;
	started = true;
	const keep = (promise: Promise<() => void>, name: string) =>
		void promise
			.then((unlisten) => unlisteners.push(unlisten))
			.catch((err: unknown) => console.error(`Не удалось подписаться на ${name}`, err));
	keep(instancesApi.onLog(onLog), "instance-log");
	keep(
		instancesApi.onProcess((event) => {
			if (event.state === "starting") onStarting(event.instanceId);
		}),
		"instance-process",
	);
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

export function useGameLog(id: string, load: boolean): GameLog | null {
	const all = useSyncExternalStore(subscribe, () => cache);
	useEffect(() => {
		if (!load) return;
		const entry = cache[id];
		if (!entry || entry.status === "error") void loadGameLog(id);
	}, [id, load]);
	return all[id] ?? null;
}

import.meta.hot?.dispose(() => {
	for (const unlisten of unlisteners) unlisten();
});
