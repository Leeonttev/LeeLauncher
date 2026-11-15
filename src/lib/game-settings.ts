import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import {
	appApi,
	javaApi,
	settingsApi,
	type AppPaths,
	type FolderKind,
	type JavaInstall,
	type Memory,
	type Settings,
	type SettingsPatch,
	type SystemMemory,
} from "@/lib/api";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";

type Loadable<T> = {
	status: "idle" | "loading" | "ready" | "error";
	value: T | null;
	error: AppError | null;
};

export type GameSettingsStore = {
	settings: Loadable<Settings>;
	memory: Loadable<SystemMemory>;
	java: Loadable<JavaInstall[]>;
	paths: Loadable<AppPaths>;
};

const idle = { status: "idle", value: null, error: null } as const;

let state: GameSettingsStore = { settings: idle, memory: idle, java: idle, paths: idle };
const listeners = new Set<() => void>();

function set<K extends keyof GameSettingsStore>(key: K, value: GameSettingsStore[K]) {
	state = { ...state, [key]: value };
	for (const listener of listeners) listener();
}

async function load<K extends keyof GameSettingsStore>(
	key: K,
	fetch: () => Promise<NonNullable<GameSettingsStore[K]["value"]>>,
	force = false,
) {
	const current = state[key];
	if (!force && (current.status === "loading" || current.status === "ready")) return;
	set(key, { ...current, status: "loading", error: null } as GameSettingsStore[K]);
	try {
		set(key, { status: "ready", value: await fetch(), error: null } as GameSettingsStore[K]);
	} catch (err) {
		console.error(key, err);
		set(key, { status: "error", value: current.value, error: toAppError(err) } as GameSettingsStore[K]);
	}
}

export function initGameSettings() {
	void load("settings", settingsApi.get);
	void load("memory", settingsApi.systemMemory);
	void load("java", javaApi.list);
	void load("paths", appApi.paths);
}

export async function openAppFolder(kind: FolderKind) {
	try {
		await appApi.openFolder(kind);
	} catch (err) {
		console.error("app_open_folder", err);
		toast.error(i18n.t("common.openFolderFailed"));
	}
}

export function rescanJava() {
	return load("java", javaApi.list, true);
}

export function reloadSettings() {
	void load("settings", settingsApi.get, true);
	void load("memory", settingsApi.systemMemory, true);
	if (state.paths.status === "error") void load("paths", appApi.paths, true);
}

export async function updateSettings(patch: SettingsPatch): Promise<boolean> {
	try {
		set("settings", { status: "ready", value: await settingsApi.update(patch), error: null });
		return true;
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		return false;
	}
}

/** Из настроек, иначе рекомендация ядра. */
export function defaultMemory(store: GameSettingsStore): Memory | null {
	const settings = store.settings.value;
	if (settings?.defaultMemory) return settings.defaultMemory;
	const memory = store.memory.value;
	if (!memory) return null;
	return { minMb: 512, maxMb: memory.recommendedMaxMb };
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useGameSettings(): GameSettingsStore {
	return useSyncExternalStore(subscribe, () => state);
}
