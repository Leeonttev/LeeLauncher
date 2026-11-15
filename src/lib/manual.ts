import { useEffect, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { contentApi, manualApi, type ManualFile } from "@/lib/api";
import { isSourceEnabled } from "@/lib/catalog";
import { loadContent } from "@/lib/content";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";

/** Файлы «только с сайта»: ждущие файлы сборок и сеанс ядра со слежением за «Загрузками». */
export type ManualDialog = {
	open: boolean;
	session: number;
	instanceId: string | null;
	returnFocus: HTMLElement | null;
	status: "loading" | "ready" | "error";
	files: ManualFile[];
	path: string | null;
	watching: boolean;
	error: AppError | null;
	/** Для «Повторить». */
	extra: ManualFile[] | undefined;
};

type State = {
	dialog: ManualDialog;
	pending: Record<string, ManualFile[]>;
};

const closedDialog: ManualDialog = {
	open: false,
	session: 0,
	instanceId: null,
	returnFocus: null,
	status: "loading",
	files: [],
	path: null,
	watching: false,
	error: null,
	extra: undefined,
};

let state: State = { dialog: closedDialog, pending: {} };
const listeners = new Set<() => void>();
/** Сеансы прошлой страницы некому закрыть; `begin` ждёт этого. */
let sessionsReset: Promise<void> = Promise.resolve();

function set(next: Partial<State>) {
	state = { ...state, ...next };
	for (const listener of listeners) listener();
}

function setDialog(patch: Partial<ManualDialog>) {
	set({ dialog: { ...state.dialog, ...patch } });
}

export function isWaiting(file: ManualFile) {
	return file.state !== "added";
}

const requests = new Map<string, number>();
let requestSeq = 0;

export async function loadManual(id: string) {
	const seq = ++requestSeq;
	requests.set(id, seq);
	try {
		const files = await manualApi.list(id);
		if (requests.get(id) !== seq) return;
		set({ pending: { ...state.pending, [id]: files.filter(isWaiting) } });
	} catch (err) {
		// Список ждущих — подсказка, а не основное содержимое: ошибку не показываем.
		console.error("instance_manual", err);
	}
}

export function useManualPending(id: string): ManualFile[] {
	const pending = useSyncExternalStore(subscribe, () => state.pending[id]);
	useEffect(() => {
		if (!(id in state.pending)) void loadManual(id);
	}, [id]);
	return (pending ?? []).filter((file) => isSourceEnabled(file.source));
}

/* Диалог «Скачайте вручную» */

/** `files` — одиночный мод, в ждущие не пишется. */
export async function openManualDialog(
	instanceId: string,
	files?: ManualFile[],
	returnFocus?: HTMLElement | null,
) {
	if (files?.length && !files.some((file) => isSourceEnabled(file.source))) return;
	const previous = state.dialog;
	if (previous.open && previous.instanceId && previous.instanceId !== instanceId)
		void endSession(previous.instanceId);
	const session = previous.session + 1;
	set({
		dialog: {
			...closedDialog,
			open: true,
			session,
			instanceId,
			returnFocus: returnFocus ?? (document.activeElement as HTMLElement | null),
			extra: files,
		},
	});
	try {
		await sessionsReset;
		const result = await manualApi.begin(instanceId, files);
		if (state.dialog.session !== session) return;
		if (!state.dialog.open) {
			void endSession(instanceId);
			return;
		}
		setDialog({
			status: "ready",
			files: result.files,
			path: result.path,
			watching: result.watching,
			error: null,
		});
	} catch (err) {
		if (state.dialog.session !== session) return;
		console.error("instance_manual_begin", err);
		setDialog({ status: "error", error: toAppError(err) });
	}
}

export function retryManualDialog() {
	const { instanceId, extra, returnFocus } = state.dialog;
	if (instanceId) void openManualDialog(instanceId, extra, returnFocus);
}

async function endSession(instanceId: string) {
	try {
		await manualApi.end(instanceId);
	} catch (err) {
		console.error("instance_manual_end", err);
	}
	void loadManual(instanceId);
	void loadContent(instanceId);
}

/** Недобавленное остаётся в ждущих сборки. */
export function closeManualDialog() {
	const { instanceId, open } = state.dialog;
	if (!open) return;
	setDialog({ open: false });
	if (instanceId) void endSession(instanceId);
}

export function restoreManualFocus(event: Event) {
	const target = state.dialog.returnFocus;
	if (!target?.isConnected) return;
	event.preventDefault();
	target.focus();
}

export function manualDialogTarget(): string | null {
	const dialog = state.dialog;
	return dialog.open && dialog.status === "ready" ? dialog.instanceId : null;
}

function baseName(path: string) {
	return path.split(/[\\/]/).pop() ?? path;
}

function rejectedToast(rejected: string[]) {
	if (rejected.length)
		toast.error(i18n.t("drop.unsupported", { names: rejected.map(baseName).join(", ") }));
}

export async function addManualFiles(paths: string[]) {
	const id = manualDialogTarget();
	if (!id || !paths.length) return;
	const session = state.dialog.session;
	try {
		const result = await manualApi.add(id, paths);
		if (state.dialog.session === session) setDialog({ files: result.files });
		rejectedToast(result.rejected);
	} catch (err) {
		toast.error(errorText(toAppError(err)));
	}
}

let picking = false;

export async function pickManualFiles() {
	const id = manualDialogTarget();
	if (!id || picking) return;
	picking = true;
	const session = state.dialog.session;
	try {
		const result = await manualApi.pick(id, i18n.language === "en" ? "en" : "ru");
		if (!result) return;
		if (state.dialog.session === session) setDialog({ files: result.files });
		rejectedToast(result.rejected);
	} catch (err) {
		toast.error(errorText(toAppError(err)));
	} finally {
		picking = false;
	}
}

const unlisteners: Array<() => void> = [];
let started = false;

export function initManual() {
	if (started) return;
	started = true;
	sessionsReset = manualApi.endAll().catch((err: unknown) => console.error("instance_manual_end_all", err));
	void manualApi
		.onChanged(({ id, files }) => {
			const dialog = state.dialog;
			if (dialog.open && dialog.instanceId === id && dialog.status === "ready") setDialog({ files });
			else set({ pending: { ...state.pending, [id]: files.filter(isWaiting) } });
		})
		.then((unlisten) => unlisteners.push(unlisten))
		.catch((err: unknown) => console.error("Не удалось подписаться на instance-manual-changed", err));
	// Файл положили в сборку сами — ядро убирает его из ждущих при чтении контента.
	void contentApi
		.onContentChanged((id) => {
			if (id in state.pending && !(state.dialog.open && state.dialog.instanceId === id))
				void loadManual(id);
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

export function useManualDialog(): ManualDialog {
	return useSyncExternalStore(subscribe, () => state.dialog);
}

import.meta.hot?.dispose(() => {
	for (const unlisten of unlisteners) unlisten();
});
