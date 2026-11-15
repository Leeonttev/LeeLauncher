import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import {
	catalogApi,
	contentApi,
	downloadsApi,
	type AlsoOn,
	type ContentInstallState,
	type ContentItem,
	type ContentSource,
	type ContentType,
	type Download,
	type Instance,
	type InstallRequest,
	type InstallResult,
	type ProjectHit,
	type ResolveView,
} from "@/lib/api";
import { fileRef, findProjectItem, loadContent, type InstanceContent } from "@/lib/content";
import { addDownload, getDownloads, isDone, subscribeDownloads, type DownloadsStore } from "@/lib/downloads";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";
import { getInstances, statusOf, type InstancesStore } from "@/lib/instances";
import type { InstanceWorlds } from "@/lib/worlds";
import { LOADER_NAMES } from "@/lib/loaders";
import { loadManual, openManualDialog } from "@/lib/manual";

export type ProjectRef = {
	source: ContentSource;
	id: string;
	title: string;
	type: ContentType;
	iconUrl: string | null;
	gameVersions?: string[];
	loaders?: string[];
	siteOnly?: boolean;
	/** Стоит двойник — кнопка показывает «Установлен». */
	alsoOn?: AlsoOn[];
};

export function projectRefOf(
	hit: ProjectHit,
	type: ContentType,
): ProjectRef & { install?: ContentInstallState } {
	return {
		source: hit.source,
		id: hit.id,
		title: hit.title,
		type,
		iconUrl: hit.iconUrl,
		gameVersions: hit.gameVersions,
		loaders: hit.loaders,
		siteOnly: hit.siteOnly,
		install: hit.install,
		alsoOn: hit.alsoOn,
	};
}

export type InstallTarget = {
	instanceId: string;
	project: ProjectRef;
	/** Нет — лучшая подходящая. */
	versionId?: string;
	world?: string;
};

export type DepsDialog = {
	open: boolean;
	session: number;
	target: InstallTarget | null;
	view: ResolveView | null;
	returnFocus: HTMLElement | null;
};

type UiState = {
	/** Ключ `сборка:проект`. */
	pending: Record<string, true>;
	/** Ошибка до появления задачи загрузок. */
	errors: Record<string, AppError>;
	dialog: DepsDialog;
};

let ui: UiState = {
	pending: {},
	errors: {},
	dialog: { open: false, session: 0, target: null, view: null, returnFocus: null },
};
const listeners = new Set<() => void>();

function set(next: Partial<UiState>) {
	ui = { ...ui, ...next };
	for (const listener of listeners) listener();
}

function keyOf(instanceId: string, projectId: string) {
	return `${instanceId}:${projectId}`;
}

function without<T>(record: Record<string, T>, key: string): Record<string, T> {
	if (!(key in record)) return record;
	const next = { ...record };
	delete next[key];
	return next;
}

function requestOf(target: InstallTarget, optional?: string[]): InstallRequest {
	return {
		source: target.project.source,
		projectId: target.project.id,
		versionId: target.versionId,
		type: target.project.type,
		world: target.world,
		optional: optional?.length ? optional : undefined,
	};
}

/* Задачи загрузок, запущенные отсюда: по их завершению — тосты. */
const tracked = new Map<string, { instanceId: string; title: string }>();
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
			onTaskDone(download, info);
		}
	});
}

function gameRunning(instanceId: string) {
	const store = getInstances();
	const instance = store.instances.find((i) => i.id === instanceId);
	return instance ? statusOf(instance, store) === "running" : false;
}

function onTaskDone(download: Download, info: { instanceId: string; title: string }) {
	void loadContent(info.instanceId);
	if (download.state === "failed") {
		toast.error(errorText(download.error ?? toAppError(null)), {
			action: {
				label: i18n.t("catalog.install.retry"),
				onClick: () => void retryTask(download.id, info),
			},
		});
		return;
	}
	if (gameRunning(info.instanceId)) toast(i18n.t("catalog.install.restartHint", { title: info.title }));
}

async function retryTask(id: string, info: { instanceId: string; title: string }) {
	try {
		const next = await downloadsApi.retry(id);
		addDownload(next);
		tracked.set(next.id, info);
	} catch (err) {
		toast.error(errorText(toAppError(err)));
	}
}

function startTracking(download: Download, instanceId: string, title: string) {
	ensureTracking();
	addDownload(download);
	const info = { instanceId, title };
	// `downloads-changed` мог прийти раньше ответа команды — задача уже завершилась.
	const current = getDownloads().items.find((d) => d.id === download.id);
	if (current && isDone(current)) onTaskDone(current, info);
	else tracked.set(download.id, info);
}

function failed(key: string, err: unknown, retry: () => void) {
	const error = toAppError(err);
	set({ pending: without(ui.pending, key), errors: { ...ui.errors, [key]: error } });
	toast.error(errorText(error), { action: { label: i18n.t("catalog.install.retry"), onClick: retry } });
}

/** `pass` — передать файлы «только с сайта» в сеанс: у `content_install` их ещё нет в ждущих. */
function applyResult(
	result: InstallResult,
	instanceId: string,
	title: string,
	pass: boolean,
	returnFocus?: HTMLElement | null,
) {
	if (result.download) startTracking(result.download, instanceId, title);
	if (result.manual.length)
		void openManualDialog(instanceId, pass ? result.manual : undefined, returnFocus);
	else if (!result.download) void loadContent(instanceId);
	if (!pass && result.manual.length) void loadManual(instanceId);
}

async function runInstall(target: InstallTarget, optional?: string[], returnFocus?: HTMLElement | null) {
	const key = keyOf(target.instanceId, target.project.id);
	set({ pending: { ...ui.pending, [key]: true }, errors: without(ui.errors, key) });
	try {
		const result = await catalogApi.install(target.instanceId, requestOf(target, optional));
		set({ pending: without(ui.pending, key) });
		applyResult(result, target.instanceId, target.project.title, true, returnFocus);
	} catch (err) {
		failed(key, err, () => void runInstall(target, optional));
	}
}

export function manualDependencies(view: ResolveView) {
	return view.manual.filter(
		(file) => !(file.source === view.project.source && file.projectId === view.project.id),
	);
}

/** Окно зависимостей — только если есть что выбрать, конфликты, недостающее или файлы «только с сайта». */
export async function installProject(target: InstallTarget, returnFocus?: HTMLElement | null) {
	const key = keyOf(target.instanceId, target.project.id);
	if (ui.pending[key]) return;
	set({ pending: { ...ui.pending, [key]: true }, errors: without(ui.errors, key) });
	let view: ResolveView;
	try {
		view = await catalogApi.resolve(target.instanceId, requestOf(target));
	} catch (err) {
		failed(key, err, () => void installProject(target));
		return;
	}
	set({ pending: without(ui.pending, key) });
	if (
		view.optional.length ||
		view.conflicts.length ||
		view.missing.length ||
		manualDependencies(view).length
	) {
		set({
			dialog: {
				open: true,
				session: ui.dialog.session + 1,
				target,
				view,
				returnFocus: returnFocus ?? (document.activeElement as HTMLElement | null),
			},
		});
		return;
	}
	await runInstall(target, undefined, returnFocus);
}

export function confirmDeps(optional: string[]) {
	const { target, returnFocus } = ui.dialog;
	set({ dialog: { ...ui.dialog, open: false } });
	if (target) void runInstall(target, optional, returnFocus);
}

export function closeDepsDialog() {
	set({ dialog: { ...ui.dialog, open: false } });
}

export async function updateProjectItem(
	instanceId: string,
	item: ContentItem,
	returnFocus?: HTMLElement | null,
) {
	const projectId = item.project?.id;
	if (!projectId) return;
	const key = keyOf(instanceId, projectId);
	set({ pending: { ...ui.pending, [key]: true }, errors: without(ui.errors, key) });
	try {
		const result = await contentApi.update(instanceId, [fileRef(item)]);
		set({ pending: without(ui.pending, key) });
		applyResult(result, instanceId, item.project?.title ?? item.file, false, returnFocus);
	} catch (err) {
		failed(key, err, () => void updateProjectItem(instanceId, item, returnFocus));
	}
}

export function updateAllKey(instanceId: string) {
	return keyOf(instanceId, "*");
}

export async function updateAllContent(instanceId: string, returnFocus?: HTMLElement | null) {
	const key = updateAllKey(instanceId);
	if (ui.pending[key]) return;
	set({ pending: { ...ui.pending, [key]: true }, errors: without(ui.errors, key) });
	try {
		const result = await contentApi.update(instanceId);
		set({ pending: without(ui.pending, key) });
		applyResult(result, instanceId, result.download?.content?.projectTitle ?? "", false, returnFocus);
	} catch (err) {
		failed(key, err, () => void updateAllContent(instanceId, returnFocus));
	}
}

export function contentBusy(ui: UiState, downloads: DownloadsStore, instanceId: string, projectId: string) {
	if (ui.pending[keyOf(instanceId, projectId)] || ui.pending[updateAllKey(instanceId)]) return true;
	return downloads.items.some(
		(d) =>
			d.kind === "content" &&
			d.instanceId === instanceId &&
			(d.state === "queued" || d.state === "running") &&
			(d.content?.projectId === projectId || (d.content?.action === "update" && !d.content.projectId)),
	);
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

export function useInstallUi(): UiState {
	return useSyncExternalStore(subscribe, () => ui);
}

/* Состояние кнопки установки */

export type InstallButtonState =
	| { kind: "pick" }
	| { kind: "install" }
	| { kind: "installing"; percent: number | null }
	| { kind: "installed"; item: ContentItem | null }
	| { kind: "update"; item: ContentItem | null }
	/** `null` — нужен загрузчик модов (ванильная сборка). */
	| { kind: "incompatible"; target: string | null }
	| { kind: "error"; error: AppError; retry: () => void }
	| { kind: "waitInstance" }
	| { kind: "siteOnly" };

export type Incompatibility = { kind: "needs_loader" } | { kind: "no_version"; target: string };

/** Quilt читает и моды Fabric. */
function modLoadersOf(instance: Instance): string[] {
	if (instance.loader.kind === "quilt") return ["quilt", "fabric"];
	return [instance.loader.kind];
}

/** Оценка по спискам проекта, без запроса к ядру; точный ответ — `install` в результатах поиска. */
export function incompatibilityOf(
	project: { type: ContentType; gameVersions?: string[]; loaders?: string[] },
	instance: Instance,
): Incompatibility | null {
	const needsLoader = project.type === "mod" || project.type === "shader";
	if (needsLoader && instance.loader.kind === "vanilla") return { kind: "needs_loader" };
	const versions = project.gameVersions ?? [];
	if (versions.length && !versions.includes(instance.gameVersion))
		return { kind: "no_version", target: instance.gameVersion };
	const loaders = project.loaders ?? [];
	if (project.type === "mod" && loaders.length && !modLoadersOf(instance).some((l) => loaders.includes(l)))
		return { kind: "no_version", target: LOADER_NAMES[instance.loader.kind] };
	return null;
}

/** `null` — нужен загрузчик, `undefined` — подходит. */
function incompatibleTarget(
	state: ContentInstallState | undefined,
	instance: Instance,
): string | null | undefined {
	if (state?.kind !== "incompatible") return undefined;
	if (state.reason === "needs_loader") return null;
	if (state.reason === "loader") return LOADER_NAMES[instance.loader.kind];
	return instance.gameVersion;
}

function activeTask(downloads: DownloadsStore, instanceId: string, projectId: string) {
	return downloads.items.find(
		(d) =>
			d.kind === "content" &&
			d.instanceId === instanceId &&
			d.content?.projectId === projectId &&
			(d.state === "queued" || d.state === "running"),
	);
}

function failedTask(downloads: DownloadsStore, instanceId: string, projectId: string) {
	return downloads.items.find(
		(d) =>
			d.kind === "content" &&
			d.instanceId === instanceId &&
			d.content?.projectId === projectId &&
			d.state === "failed",
	);
}

/** С `versionId` «Установлена» — только если стоит именно эта версия. */
export function installButtonState(args: {
	project: ProjectRef & { install?: ContentInstallState };
	instance: Instance | null;
	instances: InstancesStore;
	content: InstanceContent | null;
	worlds?: InstanceWorlds | null;
	downloads: DownloadsStore;
	ui: UiState;
	versionId?: string;
	versionInstalled?: boolean;
}): InstallButtonState {
	const { project, instance, content, downloads } = args;
	if (!instance) return { kind: "pick" };
	const status = statusOf(instance, args.instances);
	if (status === "queued" || status === "installing") return { kind: "waitInstance" };

	const key = keyOf(instance.id, project.id);
	const twins = project.alsoOn ?? [];
	const task = [project.id, ...twins.map((twin) => twin.id)]
		.map((id) => activeTask(downloads, instance.id, id))
		.find(Boolean);
	if (task) return { kind: "installing", percent: task.state === "running" ? task.percent : null };
	if (args.ui.pending[key]) return { kind: "installing", percent: null };

	// Проект мог встать из другого каталога.
	const item =
		findProjectItem(content, project.id) ??
		twins.map((twin) => findProjectItem(content, twin.id)).find(Boolean) ??
		null;
	if (project.type === "world") {
		// Мир ставится копией; «Установлен» — если в сборке есть мир из этого проекта.
		const worlds = args.worlds;
		const placed =
			worlds?.status === "ready"
				? worlds.items.some(
						(w) => w.source?.projectId === project.id && w.source.source === project.source,
					)
				: project.install?.kind === "installed";
		if (placed && !args.versionId) return { kind: "installed", item: null };
	} else if (args.versionId) {
		if (args.versionInstalled || item?.project?.versionId === args.versionId)
			return { kind: "installed", item };
	} else if (item) {
		return item.update ? { kind: "update", item } : { kind: "installed", item };
	} else if (!content || content.status !== "ready") {
		// Список сборки ещё не прочитан — верим поиску.
		const kinds = [project.install?.kind, ...twins.map((twin) => twin.install?.kind)];
		if (kinds.includes("installed")) return { kind: "installed", item: null };
		if (kinds.includes("update")) return { kind: "update", item: null };
	}

	const error = args.ui.errors[key];
	if (error) return { kind: "error", error, retry: () => void installProject(targetOf(args)) };
	const failedDownload = failedTask(downloads, instance.id, project.id);
	if (failedDownload) {
		return {
			kind: "error",
			error: failedDownload.error ?? toAppError(null),
			retry: () => void retryTask(failedDownload.id, { instanceId: instance.id, title: project.title }),
		};
	}

	const fromSearch = args.versionId ? undefined : incompatibleTarget(project.install, instance);
	if (fromSearch !== undefined) return { kind: "incompatible", target: fromSearch };
	if (!project.install) {
		const guess = incompatibilityOf(project, instance);
		if (guess)
			return { kind: "incompatible", target: guess.kind === "needs_loader" ? null : guess.target };
	}
	return project.siteOnly ? { kind: "siteOnly" } : { kind: "install" };
}

function targetOf(args: {
	project: ProjectRef;
	instance: Instance | null;
	versionId?: string;
}): InstallTarget {
	return { instanceId: args.instance?.id ?? "", project: args.project, versionId: args.versionId };
}
