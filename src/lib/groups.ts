import { useSyncExternalStore } from "react";
import type { Instance } from "@/lib/api";
import { errorText, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";
import { readStorage, writeStorage } from "@/lib/storage";

/** Группа — подпись `Instance.group`: отдельного списка нет, пустых групп не бывает. */

export const GROUP_NAME_MAX = 32;

/** Как в ядре. */
export function normalizeGroupName(value: string): string {
	return value.trim().replace(/\s+/g, " ");
}

/** В символах, не в единицах UTF-16 — как считает ядро. */
export function groupNameLength(value: string): number {
	return [...value].length;
}

/** `""` — «Без группы». */
export function groupKey(name: string | null): string {
	return name ? normalizeGroupName(name).toLocaleLowerCase() : "";
}

export function listGroups(instances: Instance[], lang: string): string[] {
	const byKey = new Map<string, string>();
	for (const instance of instances) {
		if (!instance.group) continue;
		const key = groupKey(instance.group);
		if (!byKey.has(key)) byKey.set(key, instance.group);
	}
	const collator = new Intl.Collator(lang, { sensitivity: "base", numeric: true });
	return [...byKey.values()].sort(collator.compare);
}

/** Совпала без учёта регистра — берём написание существующей. */
export function findGroup(instances: Instance[], name: string): string | null {
	const key = groupKey(name);
	if (!key) return null;
	return instances.find((i) => i.group && groupKey(i.group) === key)?.group ?? null;
}

export function groupErrorText(error: AppError): string {
	if (error.code === "invalid_name" && error.details === "group") return i18n.t("groups.invalidName");
	return errorText(error);
}

/* Свёрнутые группы: JSON-массив ключей. */

const COLLAPSED_KEY = "instances-collapsed";

function loadCollapsed(): ReadonlySet<string> {
	try {
		const value: unknown = JSON.parse(readStorage(COLLAPSED_KEY) ?? "[]");
		return new Set(Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : []);
	} catch {
		return new Set();
	}
}

export type GroupsUi = {
	collapsed: ReadonlySet<string>;
	/** Подсветка свёрнутой группы, в которую перенесли сборку. */
	flash: string | null;
};

let ui: GroupsUi = { collapsed: loadCollapsed(), flash: null };
const uiListeners = new Set<() => void>();
let flashTimer: number | undefined;

function setUi(next: Partial<GroupsUi>) {
	ui = { ...ui, ...next };
	for (const listener of uiListeners) listener();
}

function saveCollapsed(collapsed: ReadonlySet<string>) {
	setUi({ collapsed });
	writeStorage(COLLAPSED_KEY, collapsed.size ? JSON.stringify([...collapsed]) : null);
}

export function setGroupCollapsed(key: string, collapsed: boolean) {
	if (ui.collapsed.has(key) === collapsed) return;
	const next = new Set(ui.collapsed);
	if (collapsed) next.add(key);
	else next.delete(key);
	saveCollapsed(next);
}

export function setGroupsCollapsed(keys: string[], collapsed: boolean) {
	const next = new Set(ui.collapsed);
	for (const key of keys) {
		if (collapsed) next.add(key);
		else next.delete(key);
	}
	saveCollapsed(next);
}

/** Пока идут переименования, `pruneCollapsed` ничего не чистит. */
let renaming = 0;

/** Свёрнутость переезжает сразу: ядро шлёт `instances-changed` раньше ответа команды. `ok = false` — вернуть как было. */
export function beginGroupRename(from: string, to: string): (ok: boolean) => void {
	const before = ui.collapsed;
	renaming++;
	if (from !== to && before.has(from)) {
		const next = new Set(before);
		next.delete(from);
		next.add(to);
		saveCollapsed(next);
	}
	let done = false;
	return (ok) => {
		if (done) return;
		done = true;
		renaming--;
		if (!ok) saveCollapsed(before);
	};
}

export function pruneCollapsed(existing: ReadonlySet<string>) {
	if (renaming > 0) return;
	const next = [...ui.collapsed].filter((key) => existing.has(key));
	if (next.length !== ui.collapsed.size) saveCollapsed(new Set(next));
}

export function flashGroup(key: string) {
	clearTimeout(flashTimer);
	setUi({ flash: key });
	flashTimer = window.setTimeout(() => setUi({ flash: null }), 600);
}

function subscribeUi(listener: () => void) {
	uiListeners.add(listener);
	return () => uiListeners.delete(listener);
}

export function useGroupsUi(): GroupsUi {
	return useSyncExternalStore(subscribeUi, () => ui);
}

export function getGroupsUi(): GroupsUi {
	return ui;
}

/** Регистрирует страница «Сборки»; `null` — страница не открыта. */
let visibilityProbe: ((instance: Instance) => boolean) | null = null;

export function setGroupVisibilityProbe(probe: ((instance: Instance) => boolean) | null) {
	visibilityProbe = probe;
}

export function getGroupVisibilityProbe() {
	return visibilityProbe;
}

/* Диалоги групп */

export type GroupDialogKind = "new" | "rename" | "delete";

export type GroupDialogs = {
	open: boolean;
	kind: GroupDialogKind | null;
	instanceId: string | null;
	group: string | null;
	/** Меняется при каждом открытии: содержимое диалога начинается заново. */
	session: number;
	returnFocus: HTMLElement | null;
};

let dialogs: GroupDialogs = {
	open: false,
	kind: null,
	instanceId: null,
	group: null,
	session: 0,
	returnFocus: null,
};
const dialogListeners = new Set<() => void>();

function setDialogs(next: GroupDialogs) {
	dialogs = next;
	for (const listener of dialogListeners) listener();
}

export function openGroupDialog(
	kind: GroupDialogKind,
	options: { instanceId?: string; group?: string; returnFocus?: HTMLElement | null },
) {
	setDialogs({
		open: true,
		kind,
		instanceId: options.instanceId ?? null,
		group: options.group ?? null,
		session: dialogs.session + 1,
		returnFocus: options.returnFocus ?? (document.activeElement as HTMLElement | null),
	});
}

export function closeGroupDialog() {
	setDialogs({ ...dialogs, open: false });
}

export function restoreGroupDialogFocus(event: Event) {
	const target = dialogs.returnFocus;
	if (!target?.isConnected) return;
	event.preventDefault();
	target.focus();
}

function subscribeDialogs(listener: () => void) {
	dialogListeners.add(listener);
	return () => dialogListeners.delete(listener);
}

export function useGroupDialogs(): GroupDialogs {
	return useSyncExternalStore(subscribeDialogs, () => dialogs);
}
