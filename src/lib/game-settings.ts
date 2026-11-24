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
import { getDownloads, subscribeDownloads } from "@/lib/downloads";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";

type Loadable<T> = {
	status: "idle" | "loading" | "ready" | "error";
	value: T | null;
	error: AppError | null;
};

type Loadables = {
	settings: Loadable<Settings>;
	memory: Loadable<SystemMemory>;
	java: Loadable<JavaInstall[]>;
	paths: Loadable<AppPaths>;
};

export type GameSettingsStore = Loadables & {
	/** Тихий поиск Java: список на месте, статус не меняется. */
	javaRefreshing: boolean;
};

const idle = { status: "idle", value: null, error: null } as const;

let state: GameSettingsStore = {
	settings: idle,
	memory: idle,
	java: idle,
	paths: idle,
	javaRefreshing: false,
};
const listeners = new Set<() => void>();

function set(patch: Partial<GameSettingsStore>) {
	state = { ...state, ...patch };
	for (const listener of listeners) listener();
}

function setLoadable<K extends keyof Loadables>(key: K, value: Loadables[K]) {
	set({ [key]: value });
}

async function load<K extends keyof Loadables>(
	key: K,
	fetch: () => Promise<NonNullable<Loadables[K]["value"]>>,
	force = false,
) {
	const current = state[key];
	if (!force && (current.status === "loading" || current.status === "ready")) return;
	setLoadable(key, { ...current, status: "loading", error: null } as Loadables[K]);
	try {
		setLoadable(key, { status: "ready", value: await fetch(), error: null } as Loadables[K]);
	} catch (err) {
		console.error(key, err);
		setLoadable(key, { status: "error", value: current.value, error: toAppError(err) } as Loadables[K]);
	}
}

/** Мажорные версии Java со своей строкой в Настройках → «Игра». */
export const JAVA_MAJORS = [25, 21, 17, 8] as const;
export type JavaMajor = (typeof JAVA_MAJORS)[number];

export const SETTINGS_JAVA_ROWS_ID = "settings-java";

export function settingsJavaFieldId(major: JavaMajor) {
	return `settings-java-${major}`;
}

export function initGameSettings() {
	void load("settings", settingsApi.get);
	void load("memory", settingsApi.systemMemory);
	void load("java", listJava);
	void load("paths", appApi.paths);
	watchJavaTasks();
}

export async function openAppFolder(kind: FolderKind) {
	try {
		await appApi.openFolder(kind);
	} catch (err) {
		console.error("app_open_folder", err);
		toast.error(i18n.t("common.openFolderFailed"));
	}
}

let lastJavaScan = 0;
let javaScanSeq = 0;

function listJava() {
	lastJavaScan = Date.now();
	javaScanSeq += 1;
	return javaApi.list();
}

export function rescanJava() {
	return load("java", listJava, true);
}

/** Перечитать список Java, не трогая показанный: не чаще `minAgeMs` с прошлого поиска. */
let refreshAgain = false;

export async function refreshJava(minAgeMs: number) {
	if (state.java.status !== "ready") return;
	if (state.javaRefreshing) {
		// Идущий поиск мог начаться до конца загрузки Java — повторим после него.
		if (minAgeMs === 0) refreshAgain = true;
		return;
	}
	if (Date.now() - lastJavaScan < minAgeMs) return;
	const seq = javaScanSeq + 1;
	set({ javaRefreshing: true });
	try {
		const value = await listJava();
		// Пока шёл тихий поиск, могли нажать «Найти снова» — его ответ главнее.
		if (seq === javaScanSeq && state.java.status === "ready") {
			setLoadable("java", { status: "ready", value, error: null });
		}
	} catch (err) {
		console.warn("java_list", err);
	} finally {
		set({ javaRefreshing: false });
		if (refreshAgain) {
			refreshAgain = false;
			void refreshJava(0);
		}
	}
}

/** Скачивание или обновление Java закончилось (в том числе повтор из «Загрузок») — список устарел. */
let javaTasksWatched = false;
const javaTasksActive = new Set<string>();

function watchJavaTasks() {
	if (javaTasksWatched) return;
	javaTasksWatched = true;
	subscribeDownloads(() => {
		for (const task of getDownloads().items) {
			if (task.kind !== "java") continue;
			if (task.state === "queued" || task.state === "running") javaTasksActive.add(task.id);
			else if (javaTasksActive.delete(task.id) && task.state === "finished") void refreshJava(0);
		}
	});
}

const javaUpdates = new Set<number>();

/** Обновить скачанную лаунчером Java; ход — в «Загрузках». */
export async function updateJava(major: number) {
	if (javaUpdates.has(major)) return;
	javaUpdates.add(major);
	try {
		await javaApi.update(major);
	} catch (err) {
		const error = toAppError(err);
		if (error.code === "cancelled") return;
		if (error.code === "file_in_use") {
			toast.error(
				error.details === "locked"
					? i18n.t("settings.game.java.updateLocked")
					: i18n.t("settings.game.java.updateInUse"),
			);
		} else toast.error(errorText(error));
	} finally {
		javaUpdates.delete(major);
		void refreshJava(0);
	}
}

export function reloadSettings() {
	void load("settings", settingsApi.get, true);
	void load("memory", settingsApi.systemMemory, true);
	if (state.paths.status === "error") void load("paths", appApi.paths, true);
}

export async function updateSettings(patch: SettingsPatch): Promise<boolean> {
	try {
		setLoadable("settings", { status: "ready", value: await settingsApi.update(patch), error: null });
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
