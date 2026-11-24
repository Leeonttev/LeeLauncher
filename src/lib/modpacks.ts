import { useEffect, useSyncExternalStore } from "react";
import { toast } from "sonner";
import {
	downloadsApi,
	modpackApi,
	type ContentSource,
	type ContentVersion,
	type Download,
	type Instance,
	type InstanceCover,
	type ModpackPreview,
	type ModpackUpdateCheck,
} from "@/lib/api";
import { overlayOpen, typingTarget } from "@/hooks/use-create-shortcut";
import { isSourceEnabled, LOADER_FILTER_NAMES, searchModpacks } from "@/lib/catalog";
import { addDownload, getDownloads, isDone, subscribeDownloads, type DownloadsStore } from "@/lib/downloads";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";
import { getInstances, statusOf, upsertInstance, type InstancesStore } from "@/lib/instances";
import { compareGameVersions, LOADER_NAMES } from "@/lib/loaders";
import { loadManual, openManualDialog } from "@/lib/manual";
import { instancePath, navigate } from "@/lib/router";
import { readStorage, writeStorage } from "@/lib/storage";
import { isPrimaryMod } from "@/lib/platform";

export type ModpackRef = { source: ContentSource; id: string; title: string; iconUrl: string | null };

/** Из файла модпак уже прочитан: ошибки показаны тостом до диалога. */
export type InstallSource =
	| { kind: "catalog"; project: ModpackRef; versionId?: string }
	| { kind: "file"; path: string; preview: ModpackPreview };

/** `pick` — с выбором версии. */
export type UpdateSource =
	{ kind: "version"; versionId: string | null; pick: boolean } | { kind: "file"; preview: ModpackPreview };

type DialogBase = { open: boolean; session: number; returnFocus: HTMLElement | null };

export type ModpackDialogs = {
	install: DialogBase & { source: InstallSource | null };
	update: DialogBase & { instanceId: string | null; source: UpdateSource | null };
	unlink: DialogBase & { instanceId: string | null };
	failed: DialogBase & { downloadId: string | null };
};

export type UpdateCheckEntry = {
	status: "checking" | "ready" | "error";
	result: ModpackUpdateCheck | null;
	error: AppError | null;
	checkedAt: number | null;
};

type State = {
	dialogs: ModpackDialogs;
	checks: Record<string, UpdateCheckEntry>;
	readingFile: boolean;
};

const closedBase: DialogBase = { open: false, session: 0, returnFocus: null };

let state: State = {
	dialogs: {
		install: { ...closedBase, source: null },
		update: { ...closedBase, instanceId: null, source: null },
		unlink: { ...closedBase, instanceId: null },
		failed: { ...closedBase, downloadId: null },
	},
	checks: {},
	readingFile: false,
};
const listeners = new Set<() => void>();

function set(next: Partial<State>) {
	state = { ...state, ...next };
	for (const listener of listeners) listener();
}

function setDialog<K extends keyof ModpackDialogs>(key: K, value: ModpackDialogs[K]) {
	set({ dialogs: { ...state.dialogs, [key]: value } });
}

function focusTarget(returnFocus?: HTMLElement | null) {
	return returnFocus ?? (document.activeElement as HTMLElement | null);
}

function lang(): "ru" | "en" {
	return i18n.language === "en" ? "en" : "ru";
}

export function restoreModpackFocus(key: keyof ModpackDialogs, event: Event) {
	const target = state.dialogs[key].returnFocus;
	if (!target?.isConnected) return;
	event.preventDefault();
	target.focus();
}

export function closeModpackDialog(key: keyof ModpackDialogs) {
	setDialog(key, { ...state.dialogs[key], open: false });
}

/* Связь сборки с модпаком */

/** У файла — `null`: обновлений из каталога нет. */
export function catalogLink(instance: Instance): { source: ContentSource; projectId: string } | null {
	const link = instance.modpack;
	if (!link || link.source === "file" || !link.projectId) return null;
	return { source: link.source, projectId: link.projectId };
}

export function instancesOfModpack(instances: Instance[], projectId: string): Instance[] {
	return instances.filter((i) => i.modpack?.projectId === projectId && i.modpack.source !== "file");
}

export function activeModpackTask(downloads: DownloadsStore, instanceId: string): Download | undefined {
	return downloads.items.find(
		(d) =>
			d.kind === "modpack" &&
			d.instanceId === instanceId &&
			(d.state === "queued" || d.state === "running"),
	);
}

function loaderName(loader: string) {
	return (LOADER_NAMES as Record<string, string>)[loader] ?? LOADER_FILTER_NAMES[loader] ?? loader;
}

export function versionTarget(version: Pick<ContentVersion, "gameVersions" | "loaders">): string {
	const game = [...version.gameVersions].sort(compareGameVersions)[0];
	const loader = version.loaders.find((l) => l !== "minecraft");
	return [game, loader ? loaderName(loader) : null].filter(Boolean).join(" · ");
}

/** `.zip` — CurseForge, остальное — Modrinth. */
export function fileSource(path: string): ContentSource {
	return path.toLowerCase().endsWith(".zip") ? "curseforge" : "modrinth";
}

export function previewTarget(preview: Pick<ModpackPreview, "gameVersion" | "loader">): string {
	return preview.loader.kind === "vanilla"
		? preview.gameVersion
		: `${preview.gameVersion} · ${LOADER_NAMES[preview.loader.kind]}`;
}

/** Последний релиз; нет релизов — новейшая. */
export function defaultVersion(versions: ContentVersion[]): ContentVersion | null {
	return versions.find((v) => v.channel === "release") ?? versions[0] ?? null;
}

/* Диалог установки */

export function openInstallModpack(source: InstallSource, returnFocus?: HTMLElement | null) {
	const current = state.dialogs.install;
	setDialog("install", {
		open: true,
		session: current.session + 1,
		source,
		returnFocus: focusTarget(returnFocus),
	});
}

/** Zip CurseForge без ключа: предлагаем найти модпак на Modrinth. */
export function showCurseforgeToast(name: string) {
	const trimmed = name.trim();
	toast(i18n.t("modpack.curseforge.title"), {
		description: i18n.t("modpack.curseforge.description"),
		action: trimmed
			? {
					label: i18n.t("modpack.curseforge.find", { name: trimmed }),
					onClick: () => searchModpacks(trimmed),
				}
			: undefined,
	});
}

export function inspectRequestId(): string {
	return `inspect-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

/** Ошибку отмены не показываем: ответ самого чтения станет `cancelled`. */
export function cancelInspect(requestId: string) {
	modpackApi.inspectCancel(requestId).catch(() => {});
}

/** `null` — чтение прервали или не удалось (ошибку уже показали). */
async function readModpackFile(path: string, notModpack?: string): Promise<ModpackPreview | null> {
	const requestId = inspectRequestId();
	let cancelled = false;
	set({ readingFile: true });
	const loading = toast.loading(i18n.t("modpack.dialog.reading"), {
		action: {
			label: i18n.t("common.cancel"),
			onClick: () => {
				cancelled = true;
				cancelInspect(requestId);
			},
		},
	});
	try {
		const preview = await modpackApi.inspect({ requestId, path });
		return cancelled ? null : preview;
	} catch (err) {
		const error = toAppError(err);
		if (cancelled || error.code === "cancelled") return null;
		if (notModpack && error.code === "modpack_invalid") toast.error(notModpack);
		else showFileError(error);
		return null;
	} finally {
		toast.dismiss(loading);
		set({ readingFile: false });
	}
}

/** `source_disabled` при чтении zip CurseForge — свой тост, в `details` имя модпака. */
function showFileError(err: unknown) {
	const error = toAppError(err);
	if (error.code === "source_disabled" && error.details !== "denied")
		showCurseforgeToast(error.details === "no_key" ? "" : (error.details ?? ""));
	else toast.error(errorText(error));
}

/** `notModpack` — свой текст, если файл не модпак. */
export async function openModpackFile(path: string, returnFocus?: HTMLElement | null, notModpack?: string) {
	if (state.readingFile) return;
	const focus = focusTarget(returnFocus);
	const preview = await readModpackFile(path, notModpack);
	if (preview) openInstallModpack({ kind: "file", path, preview }, focus);
}

export async function pickModpackFile(returnFocus?: HTMLElement | null) {
	if (state.readingFile) return;
	const focus = focusTarget(returnFocus);
	let path: string | null;
	try {
		path = await modpackApi.pickFile(lang());
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		return;
	}
	if (path) await openModpackFile(path, focus);
}

/** Ошибку пробрасывает. `openAfter` — перейти на страницу новой сборки. */
export async function installModpack(
	args: { token: string; name: string; group: string | null; cover: InstanceCover; optional: string[] },
	openAfter: boolean,
) {
	const { instance, download } = await modpackApi.install(args);
	upsertInstance(instance);
	track(download, { instanceId: instance.id, version: "", name: instance.name, install: true });
	closeModpackDialog("install");
	if (openAfter) navigate(instancePath(instance.id));
}

/* Состояние кнопки модпака в каталоге */

export type ModpackButtonState =
	| { kind: "install" }
	| { kind: "installing"; percent: number | null; name: string }
	| { kind: "installed"; instances: Instance[] }
	| { kind: "error"; error: AppError; retry: () => void };

export function modpackButtonState(
	project: ModpackRef,
	instances: InstancesStore,
	downloads: DownloadsStore,
): ModpackButtonState {
	const linked = instancesOfModpack(instances.instances, project.id);
	for (const instance of linked) {
		const status = statusOf(instance, instances);
		const task = activeModpackTask(downloads, instance.id);
		if (task || status === "queued" || status === "installing") {
			const percent =
				task?.state === "running"
					? task.percent
					: status === "installing"
						? (instances.progress[instance.id]?.percent ?? null)
						: null;
			return { kind: "installing", percent, name: instance.name };
		}
	}
	const failed = linked.find((i) => i.installState === "failed");
	if (failed) {
		const task = downloads.items.find(
			(d) => d.kind === "modpack" && d.instanceId === failed.id && d.state === "failed",
		);
		return {
			kind: "error",
			error: task?.error ?? instances.installErrors[failed.id] ?? toAppError(null),
			retry: task
				? () => void retryModpackTask(task.id)
				: () => openInstallModpack({ kind: "catalog", project }),
		};
	}
	if (linked.length) return { kind: "installed", instances: linked };
	return { kind: "install" };
}

/* Проверка обновлений: при открытии «Контента», тихо, не чаще раза в 10 минут на сборку. */

const CHECK_INTERVAL = 10 * 60 * 1000;

function setCheck(id: string, entry: UpdateCheckEntry) {
	set({ checks: { ...state.checks, [id]: entry } });
}

/** Ответ проверки, которую перебила `force`, не применяется. */
const checkSeq = new Map<string, number>();

export async function checkModpackUpdate(id: string, force = false) {
	const instance = getInstances().instances.find((i) => i.id === id);
	if (!instance || !catalogLink(instance)) return;
	const current = state.checks[id];
	if (current?.status === "checking" && !force) return;
	if (!force && current?.checkedAt && Date.now() - current.checkedAt < CHECK_INTERVAL) return;
	const seq = (checkSeq.get(id) ?? 0) + 1;
	checkSeq.set(id, seq);
	setCheck(id, {
		status: "checking",
		result: current?.result ?? null,
		error: null,
		checkedAt: current?.checkedAt ?? null,
	});
	try {
		const result = await modpackApi.checkUpdate(id, force);
		if (checkSeq.get(id) !== seq) return;
		setCheck(id, { status: "ready", result, error: null, checkedAt: result.checkedAt || Date.now() });
	} catch (err) {
		if (checkSeq.get(id) !== seq) return;
		setCheck(id, { status: "error", result: null, error: toAppError(err), checkedAt: Date.now() });
	}
}

function forgetCheck(id: string) {
	checkSeq.set(id, (checkSeq.get(id) ?? 0) + 1);
	if (!(id in state.checks)) return;
	const checks = { ...state.checks };
	delete checks[id];
	set({ checks });
}

/* Обновление */

export function openUpdateModpack(
	instanceId: string,
	source: UpdateSource,
	returnFocus?: HTMLElement | null,
) {
	const current = state.dialogs.update;
	setDialog("update", {
		open: true,
		session: current.session + 1,
		instanceId,
		source,
		returnFocus: focusTarget(returnFocus),
	});
}

export async function updateModpackFromFile(instanceId: string, returnFocus?: HTMLElement | null) {
	if (state.readingFile) return;
	const focus = focusTarget(returnFocus);
	let path: string | null;
	try {
		path = await modpackApi.pickFile(lang());
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		return;
	}
	if (!path) return;
	const preview = await readModpackFile(path);
	if (preview) openUpdateModpack(instanceId, { kind: "file", preview }, focus);
}

const BACKUP_KEY = "modpack-backup";

export function backupPreference(): boolean {
	return readStorage(BACKUP_KEY) === "true";
}

export function setBackupPreference(value: boolean) {
	writeStorage(BACKUP_KEY, value ? "true" : null);
}

type Tracked = { instanceId: string; version: string; name: string; install?: boolean };

/** По завершении — тост; файлы «только с сайта» — сразу диалог, если окно активно и другого диалога нет. */
const tracked = new Map<string, Tracked>();
let trackingStarted = false;

function ensureTracking() {
	if (trackingStarted) return;
	trackingStarted = true;
	subscribeDownloads(() => {
		if (!tracked.size) return;
		for (const download of getDownloads().items) {
			const info = tracked.get(download.id);
			if (!info || !isDone(download)) continue;
			tracked.delete(download.id);
			onUpdateDone(download, info);
		}
	});
}

function showManualAfter(download: Download, info: Tracked) {
	const manual = (download.warnings?.manual ?? []).filter((file) => isSourceEnabled(file.source));
	if (!manual.length) return false;
	void loadManual(info.instanceId);
	if (document.hasFocus() && !overlayOpen()) {
		void openManualDialog(info.instanceId, undefined, null);
		return true;
	}
	toast.warning(i18n.t("modpack.toast.manual", { name: info.name, count: manual.length }), {
		action: {
			label: i18n.t("downloads.manualAction"),
			onClick: () => void openManualDialog(info.instanceId, undefined, null),
		},
	});
	return true;
}

function onUpdateDone(download: Download, info: Tracked) {
	if (info.install) {
		if (download.state === "finished") showManualAfter(download, info);
		return;
	}
	// Версия модпака могла смениться: проверить заново, мимо кэша.
	void checkModpackUpdate(info.instanceId, true);
	if (download.state === "failed") {
		toast.error(errorText(download.error ?? toAppError(null)), {
			action: {
				label: i18n.t("common.retry"),
				onClick: () => void retryModpackTask(download.id, info),
			},
		});
		return;
	}
	if (!showManualAfter(download, info))
		toast.success(i18n.t("modpack.update.done", { version: info.version }));
}

function track(download: Download, info: Tracked) {
	ensureTracking();
	addDownload(download);
	const current = getDownloads().items.find((d) => d.id === download.id);
	if (current && isDone(current)) onUpdateDone(current, info);
	else tracked.set(download.id, info);
}

/** Ошибку пробрасывает. */
export async function runModpackUpdate(args: {
	id: string;
	token: string;
	backup: boolean;
	version: string;
}) {
	const download = await modpackApi.update({
		id: args.id,
		token: args.token,
		backup: args.backup,
		lang: lang(),
	});
	const name = getInstances().instances.find((i) => i.id === args.id)?.name ?? "";
	track(download, { instanceId: args.id, version: args.version, name });
	closeModpackDialog("update");
}

export async function retryModpackTask(id: string, info?: Tracked) {
	try {
		const next = await downloadsApi.retry(id);
		if (info) track(next, info);
		else addDownload(next);
		return true;
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		return false;
	}
}

/* Отвязка */

export function openUnlinkModpack(instanceId: string, returnFocus?: HTMLElement | null) {
	const current = state.dialogs.unlink;
	setDialog("unlink", {
		open: true,
		session: current.session + 1,
		instanceId,
		returnFocus: focusTarget(returnFocus),
	});
}

/** Ошибку пробрасывает: диалог остаётся. */
export async function unlinkModpack(id: string) {
	const instance = await modpackApi.unlink(id);
	upsertInstance(instance);
	forgetCheck(id);
	closeModpackDialog("unlink");
	toast.success(i18n.t("modpack.unlink.done"));
}

/* Файлы, которые не скачались */

/** После закрытия панели загрузок: поповер и диалог не должны спорить за фокус. */
export function openFailedFiles(downloadId: string, returnFocus?: HTMLElement | null) {
	const current = state.dialogs.failed;
	setDialog("failed", {
		open: true,
		session: current.session + 1,
		downloadId,
		returnFocus: returnFocus ?? null,
	});
}

/** Не при открытом диалоге и не в поле ввода. */
export function useModpackFileShortcut(enabled: boolean) {
	useEffect(() => {
		if (!enabled) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.code !== "KeyO" || !isPrimaryMod(event)) return;
			if (event.repeat || typingTarget(event.target) || overlayOpen()) return;
			event.preventDefault();
			void pickModpackFile();
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [enabled]);
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

export function useModpacks(): State {
	return useSyncExternalStore(subscribe, () => state);
}

export function useModpackDialogs(): ModpackDialogs {
	return useSyncExternalStore(subscribe, () => state.dialogs);
}

export function useModpackCheck(id: string): UpdateCheckEntry | null {
	return useSyncExternalStore(subscribe, () => state.checks[id] ?? null);
}
