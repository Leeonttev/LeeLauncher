import { useEffect, useState, useSyncExternalStore } from "react";
import { toast } from "sonner";
import {
	activeAccount,
	getAccounts,
	openAddAccountDialog,
	openReloginDialog,
	type AccountsStore,
} from "@/lib/accounts";
import {
	instancesApi,
	type Account,
	type GameAccountInfo,
	type InstallFinished,
	type InstallMode,
	type InstallProgress,
	type InstallStage,
	type Instance,
	type InstanceProcessEvent,
	type InstancePatch,
	type InstanceSubfolder,
	type LoaderKind,
	type LogLine,
} from "@/lib/api";
import type { InstanceCover } from "@/lib/covers";
import { getDownloads } from "@/lib/downloads";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { formatDuration } from "@/lib/format";
import { openAppFolder } from "@/lib/game-settings";
import {
	beginGroupRename,
	findGroup,
	flashGroup,
	getGroupsUi,
	getGroupVisibilityProbe,
	groupKey,
	normalizeGroupName,
} from "@/lib/groups";
import { i18n } from "@/lib/i18n";
import { LOADER_NAMES, javaMajorFor } from "@/lib/loaders";
import { getRoute, instancePath, navigate } from "@/lib/router";
import { readStorage, writeStorage } from "@/lib/storage";
import { invalidateGameVersions, revalidateVersions } from "@/lib/versions";

export type InstanceStatus =
	"not_installed" | "queued" | "installing" | "install_failed" | "installed" | "launching" | "running";

export type ProcessInfo = {
	/** `launching` — команда ушла в ядро, `starting` — ядро готовит запуск. */
	phase: "launching" | "starting" | "running";
	startedAt: number | null;
	/** С какого аккаунта идёт игра; до ответа ядра — тот, которым запускали. */
	account: GameAccountInfo | null;
	/** Аккаунт прислало ядро. Старое ядро его не шлёт — тогда ник не показываем и о втором запуске не спрашиваем. */
	coreAccount: boolean;
};

export type InstancesStore = {
	status: "idle" | "loading" | "ready" | "error";
	error: AppError | null;
	instances: Instance[];
	progress: Record<string, InstallProgress>;
	processes: Record<string, ProcessInfo>;
	/** Живут до перезапуска лаунчера. */
	installErrors: Record<string, AppError>;
	/** Ждём `instance-install-finished`. */
	cancelling: Record<string, true>;
	/** Секунды. */
	lastSessions: Record<string, number>;
	lastExits: Record<string, LastExit>;
};

export type LastExit = {
	exitCode: number | null;
	crashed: boolean;
	at: number;
	durationSecs: number;
	account: GameAccountInfo | null;
};

let state: InstancesStore = {
	status: "idle",
	error: null,
	instances: [],
	progress: {},
	processes: {},
	installErrors: {},
	cancelling: {},
	lastSessions: {},
	lastExits: {},
};
const listeners = new Set<() => void>();

function set(next: Partial<InstancesStore>) {
	state = { ...state, ...next };
	for (const listener of listeners) listener();
}

function without<T>(record: Record<string, T>, id: string): Record<string, T> {
	if (!(id in record)) return record;
	const next = { ...record };
	delete next[id];
	return next;
}

function findInstance(id: string): Instance | undefined {
	return state.instances.find((i) => i.id === id);
}

const LOG_TAIL = 40;
const logTails = new Map<string, LogLine[]>();

function applyList(instances: Instance[]) {
	const ids = new Set(instances.map((i) => i.id));
	const progress: Record<string, InstallProgress> = {};
	for (const [id, value] of Object.entries(state.progress)) {
		const instance = instances.find((i) => i.id === id);
		if (instance && (instance.installState === "installing" || state.processes[id])) progress[id] = value;
	}
	const cancelling: Record<string, true> = {};
	for (const id of Object.keys(state.cancelling)) {
		if (instances.find((i) => i.id === id)?.installState === "installing") cancelling[id] = true;
	}
	const installErrors = Object.fromEntries(
		Object.entries(state.installErrors).filter(
			([id]) => ids.has(id) && instances.find((i) => i.id === id)?.installState === "failed",
		),
	);
	set({ status: "ready", error: null, instances, progress, cancelling, installErrors });
}

function onProgress(next: InstallProgress) {
	const previous = state.progress[next.instanceId];
	// Внутри этапа прогресс только растёт.
	const percent =
		previous && previous.stage === next.stage ? Math.max(previous.percent, next.percent) : next.percent;
	set({ progress: { ...state.progress, [next.instanceId]: { ...next, percent } } });
}

function onFinished(finished: InstallFinished) {
	const id = finished.instanceId;
	const instance = findInstance(id);
	const duringLaunch = Boolean(state.processes[id]);
	if (finished.result === "installed") invalidateGameVersions();
	set({
		progress: without(state.progress, id),
		cancelling: without(state.cancelling, id),
		installErrors:
			finished.result === "failed" && finished.error
				? { ...state.installErrors, [id]: finished.error }
				: without(state.installErrors, id),
	});
	if (!instance || duringLaunch || getRoute() === instancePath(id)) return;
	// У обновления модпака свой тост.
	if (modpackUpdating(id)) return;
	if (finished.result === "installed") {
		toast.success(i18n.t("toasts.installed", { name: instance.name }), {
			action: {
				label: i18n.t("instance.play"),
				onClick: () => void launchInstance(id),
			},
		});
	} else if (finished.result === "failed") {
		toast.error(i18n.t("toasts.installFailed", { name: instance.name }), {
			description: errorText(finished.error ?? toAppError(null)),
			action: { label: i18n.t("common.open"), onClick: () => navigate(instancePath(id)) },
		});
	}
}

function modpackUpdating(id: string) {
	const recent = Date.now() - 10_000;
	return getDownloads().items.some(
		(d) =>
			d.kind === "modpack" &&
			d.mode === "update" &&
			d.instanceId === id &&
			(d.finishedAt === undefined || d.finishedAt > recent),
	);
}

function onProcess(event: InstanceProcessEvent) {
	const id = event.instanceId;
	processSeen.add(id);
	const current = state.processes[id];
	if (event.state === "starting") {
		set({
			processes: {
				...state.processes,
				// Первое `starting` приходит без аккаунта: держим тот, которым запускали.
				[id]: {
					phase: "starting",
					startedAt: current?.startedAt ?? null,
					account: event.account ?? current?.account ?? null,
					coreAccount: Boolean(event.account) || (current?.coreAccount ?? false),
				},
			},
		});
		return;
	}
	if (event.state === "running") {
		set({
			processes: {
				...state.processes,
				// У `running` аккаунт есть всегда; нет — ядро старое, ник не показываем.
				[id]: {
					phase: "running",
					startedAt: current?.startedAt ?? Date.now(),
					account: event.account ?? null,
					coreAccount: Boolean(event.account),
				},
			},
			progress: without(state.progress, id),
		});
		return;
	}
	// Ошибку запуска показывает `launchInstance`: команда вернула ту же.
	const startedAt = current?.startedAt ?? null;
	const account = event.account ?? (current?.coreAccount ? current.account : null);
	const ran = !event.error && (event.exitCode !== undefined || startedAt !== null);
	const seconds = event.durationSecs ?? (startedAt ? Math.floor((Date.now() - startedAt) / 1000) : 0);
	set({
		processes: without(state.processes, id),
		progress: without(state.progress, id),
		lastSessions: ran ? { ...state.lastSessions, [id]: seconds } : state.lastSessions,
		lastExits: ran
			? {
					...state.lastExits,
					[id]: {
						exitCode: event.exitCode ?? null,
						crashed: Boolean(event.crashed),
						at: Date.now(),
						durationSecs: seconds,
						account,
					},
				}
			: state.lastExits,
	});
	if (!ran) return;
	const instanceName = findInstance(id)?.name ?? "";
	const name =
		account && showGameAccount(account, getAccounts())
			? i18n.t("toasts.sessionAs", { name: instanceName, nick: account.username })
			: instanceName;
	if (event.crashed) {
		const exitCode = event.exitCode ?? null;
		toast.error(i18n.t("toasts.crashed"), {
			description: name,
			duration: 10000,
			action: {
				label: i18n.t("common.details"),
				onClick: () => void openCrashDialog(id, exitCode),
			},
		});
	} else if (seconds >= 60) {
		toast(i18n.t("toasts.session", { duration: formatDuration(seconds) }), { description: name });
	}
}

function onLog(event: { instanceId: string; lines: LogLine[] }) {
	const tail = [...(logTails.get(event.instanceId) ?? []), ...event.lines];
	logTails.set(event.instanceId, tail.slice(-LOG_TAIL));
}

const unlisteners: Array<() => void> = [];
let started = false;
/** По ним уже пришло `instance-process`: ответ `instances_running` их не трогает. */
const processSeen = new Set<string>();

function listen(subscribe: Promise<() => void>, name: string) {
	void subscribe
		.then((unlisten) => unlisteners.push(unlisten))
		.catch((err: unknown) => console.error(`Не удалось подписаться на ${name}`, err));
}

export function initInstances() {
	if (started) return;
	started = true;
	listen(instancesApi.onChanged(applyList), "instances-changed");
	listen(instancesApi.onInstallProgress(onProgress), "instance-install-progress");
	listen(instancesApi.onInstallFinished(onFinished), "instance-install-finished");
	listen(instancesApi.onProcess(onProcess), "instance-process");
	listen(instancesApi.onLog(onLog), "instance-log");
	void reloadInstances();
	void restoreRunning();
}

/** Игры, запущенные до перезагрузки окна: ядро о них знает, интерфейс — нет. */
async function restoreRunning() {
	try {
		const running = await instancesApi.running();
		if (!running.length) return;
		const processes = { ...state.processes };
		for (const game of running) {
			if (processes[game.instanceId] || processSeen.has(game.instanceId)) continue;
			const startedAt = Date.parse(game.startedAt);
			processes[game.instanceId] = {
				phase: game.state,
				startedAt: Number.isNaN(startedAt) ? Date.now() : startedAt,
				account: game.account ?? null,
				coreAccount: Boolean(game.account),
			};
		}
		set({ processes });
	} catch (err) {
		console.error("instances_running", err);
	}
}

export async function reloadInstances() {
	if (state.status !== "ready") set({ status: "loading", error: null });
	try {
		applyList(await instancesApi.list());
	} catch (err) {
		console.error("instances_list", err);
		if (state.status !== "ready") set({ status: "error", error: toAppError(err) });
	}
}

export function upsertInstance(instance: Instance) {
	upsert(instance);
}

function upsert(instance: Instance) {
	upsertMany([instance]);
}

function upsertMany(changed: Instance[]) {
	if (!changed.length) return;
	const byId = new Map(changed.map((i) => [i.id, i]));
	const next = state.instances.map((i) => byId.get(i.id) ?? i);
	for (const instance of changed) {
		if (!state.instances.some((i) => i.id === instance.id)) next.push(instance);
	}
	applyList(next);
}

export function statusOf(instance: Instance, store: InstancesStore = state): InstanceStatus {
	const process = store.processes[instance.id];
	if (process) return process.phase === "running" ? "running" : "launching";
	switch (instance.installState) {
		case "installing":
			return store.progress[instance.id]?.stage === "queued" ? "queued" : "installing";
		case "failed":
			return "install_failed";
		default:
			return instance.installState;
	}
}

export function isBusy(status: InstanceStatus) {
	return status === "queued" || status === "installing" || status === "launching" || status === "running";
}

export function stageText(instance: Instance, store: InstancesStore = state): string {
	const progress = store.progress[instance.id];
	const process = store.processes[instance.id];
	if (process && !progress) {
		return i18n.t(process.phase === "launching" ? "install.stage.verifying" : "install.stage.starting");
	}
	return installStageText(progress?.stage ?? "metadata", instance);
}

export function stageShortText(instance: Instance, store: InstancesStore = state): string {
	const progress = store.progress[instance.id];
	const process = store.processes[instance.id];
	if (process && !progress) {
		return i18n.t(
			process.phase === "launching" ? "install.stageShort.verifying" : "install.stageShort.starting",
		);
	}
	const stage = progress?.stage ?? "metadata";
	return stage === "processors"
		? i18n.t("install.stageShort.processors", { loader: LOADER_NAMES[instance.loader.kind] })
		: i18n.t(`install.stageShort.${stage}`);
}

function installStageText(stage: InstallStage, instance: Instance): string {
	switch (stage) {
		case "java": {
			const major = instance.installed?.javaMajor ?? javaMajorFor(instance.gameVersion);
			return major
				? i18n.t("install.stage.java", { version: major })
				: i18n.t("install.stage.javaGeneric");
		}
		case "processors":
			return i18n.t("install.stage.loader", { loader: LOADER_NAMES[instance.loader.kind] });
		default:
			return i18n.t(`install.stage.${stage}`);
	}
}

export function progressValue(instance: Instance, store: InstancesStore = state): number | null {
	const progress = store.progress[instance.id];
	if (!progress || progress.stage === "queued") return null;
	return Math.min(1, Math.max(0, progress.percent / 100));
}

/* Действия. Ошибки показываются тостом. */

function showError(err: unknown) {
	toast.error(errorText(toAppError(err)));
}

/* Аккаунт игры. */

export function toGameAccount(account: Account): GameAccountInfo {
	return { id: account.id, uuid: account.uuid, username: account.username, kind: account.kind };
}

/** Ник показываем, когда есть что различать: аккаунтов несколько или игра идёт не с активного. */
export function showGameAccount(account: GameAccountInfo | null, accounts: AccountsStore): boolean {
	// До загрузки списка не знаем, сколько аккаунтов: иначе ник мелькнёт и пропадёт.
	if (!account || accounts.status !== "ready") return false;
	return accounts.accounts.length >= 2 || account.id !== accounts.activeId;
}

export function gameAccountOf(id: string, store: InstancesStore = state): GameAccountInfo | null {
	const process = store.processes[id];
	return process?.coreAccount ? process.account : null;
}

/** Сборки, которые сейчас идут или запускаются с этого аккаунта. */
export function gamesOfAccount(
	accountId: string,
	store: InstancesStore = state,
	exceptId?: string,
): Instance[] {
	return store.instances.filter((i) => i.id !== exceptId && gameAccountOf(i.id, store)?.id === accountId);
}

/** `returnFocus` — куда вернуть фокус после диалога аккаунта. `accountId` — не активный, без смены активного. */
export async function launchInstance(
	id: string,
	returnFocus?: HTMLElement | null,
	options?: { world?: string; accountId?: string; confirmed?: boolean },
) {
	const accounts = getAccounts();
	const account = options?.accountId
		? (accounts.accounts.find((a) => a.id === options.accountId) ?? null)
		: activeAccount(accounts);
	if (!account) {
		openAddAccountDialog(returnFocus);
		return;
	}
	if (account.needsRelogin) {
		openReloginDialog(account, returnFocus);
		return;
	}
	if (state.processes[id]) return;
	if (!options?.confirmed && sameAccountWarning() && gamesOfAccount(account.id, state, id).length) {
		// Аккаунт выбран явно («Играть за…») — он и выделен в окне.
		openLaunchAsDialog(
			id,
			account.id,
			Boolean(options?.accountId),
			options?.world ?? null,
			returnFocus ?? null,
		);
		return;
	}
	logTails.delete(id);
	set({
		processes: {
			...state.processes,
			[id]: {
				phase: "launching",
				startedAt: null,
				account: toGameAccount(account),
				coreAccount: false,
			},
		},
		installErrors: without(state.installErrors, id),
	});
	try {
		await instancesApi.launch(id, { world: options?.world, accountId: options?.accountId });
	} catch (err) {
		set({ processes: without(state.processes, id), progress: without(state.progress, id) });
		const error = toAppError(err);
		if (error.code === "needs_relogin") {
			const current = getAccounts().accounts.find((a) => a.id === account.id);
			if (current) openReloginDialog(current, returnFocus);
			return;
		}
		if (error.code === "no_account") {
			openAddAccountDialog(returnFocus);
			return;
		}
		// Игру остановили, пока шла подготовка.
		if (error.code === "cancelled") return;
		toast.error(errorText(error), launchToastAction(error, id));
	}
}

export function isDiskProblem(error: AppError) {
	return error.code === "storage" || error.code === "disk_full" || error.code === "permission_denied";
}

function launchToastAction(error: AppError, id: string) {
	if (error.code === "files_missing") {
		return {
			action: { label: i18n.t("instance.repair"), onClick: () => void installInstance(id, "repair") },
		};
	}
	if (error.code === "java_not_found" || error.code === "java_invalid") {
		return { action: { label: i18n.t("install.javaSettings"), onClick: () => focusJavaSetting(id) } };
	}
	if (isDiskProblem(error)) {
		return {
			action: { label: i18n.t("install.openDataFolder"), onClick: () => void openAppFolder("data") },
		};
	}
	return undefined;
}

export const INSTANCE_JAVA_FIELD_ID = "instance-java";

export function focusJavaSetting(id: string) {
	navigate(instancePath(id));
	let tries = 0;
	const tick = () => {
		const field = document.getElementById(INSTANCE_JAVA_FIELD_ID);
		if (field) {
			field.scrollIntoView({ block: "center" });
			field.focus({ preventScroll: true });
		} else if (++tries < 20) setTimeout(tick, 50);
	};
	setTimeout(tick, 0);
}

export async function installInstance(id: string, mode?: InstallMode) {
	set({ installErrors: without(state.installErrors, id) });
	try {
		await instancesApi.install(id, mode);
	} catch (err) {
		showError(err);
	}
}

export async function cancelInstall(id: string) {
	set({ cancelling: { ...state.cancelling, [id]: true } });
	try {
		await instancesApi.cancelInstall(id);
	} catch (err) {
		set({ cancelling: without(state.cancelling, id) });
		showError(err);
	}
}

export async function stopInstance(id: string): Promise<boolean> {
	try {
		await instancesApi.kill(id);
		return true;
	} catch (err) {
		showError(err);
		return false;
	}
}

export async function openInstanceFolder(id: string, sub?: InstanceSubfolder, select?: string) {
	try {
		await instancesApi.openFolder(id, { sub, select });
	} catch (err) {
		console.error("instance_open_folder", err);
		toast.error(i18n.t("common.openFolderFailed"));
	}
}

export async function updateInstance(id: string, patch: InstancePatch): Promise<Instance | null> {
	try {
		const instance = await instancesApi.update(id, patch);
		upsert(instance);
		return instance;
	} catch (err) {
		showError(err);
		return null;
	}
}

/** Ошибку пробрасывает: диалог остаётся открытым. */
export async function createInstance(args: Parameters<typeof instancesApi.create>[0]): Promise<Instance> {
	const instance = await instancesApi.create(args);
	upsert(instance);
	void installInstance(instance.id);
	return instance;
}

/** Если сборку теперь не видно на странице «Сборки» — тост. Ошибку пробрасывает. */
export async function moveInstanceToGroup(id: string, group: string | null) {
	const updated = await instancesApi.update(id, { group });
	upsert(updated);
	const key = groupKey(updated.group);
	const collapsed = getGroupsUi().collapsed.has(key);
	if (collapsed) flashGroup(key);
	const probe = getGroupVisibilityProbe();
	if (probe && probe(updated)) return;
	toast(
		updated.group
			? i18n.t("groups.toast.moved", { name: updated.name, group: updated.group })
			: i18n.t("groups.toast.removed", { name: updated.name }),
	);
}

/** По одной: при ошибке уже перенесённые остаются. */
export async function moveInstancesToGroup(ids: string[], group: string) {
	let target = group;
	try {
		for (const id of ids) {
			const updated = await instancesApi.update(id, { group: target });
			upsert(updated);
			// Ядро могло взять написание существующей группы — остальные идут туда же.
			if (updated.group) target = updated.group;
		}
	} finally {
		const key = groupKey(target);
		if (getGroupsUi().collapsed.has(key)) flashGroup(key);
	}
}

/** При `storage` часть сборок могла измениться — список перечитывается. */
export async function renameGroup(from: string, to: string) {
	const target = findGroup(state.instances, to) ?? normalizeGroupName(to);
	const finish = beginGroupRename(groupKey(from), groupKey(target));
	try {
		upsertMany(await instancesApi.renameGroup(from, to));
		finish(true);
	} catch (err) {
		const error = toAppError(err);
		// Часть сборок могла переехать: свёрнутость оставляем на новом имени.
		finish(error.code === "storage");
		if (error.code === "storage") void reloadInstances();
		throw error;
	}
}

export async function removeGroup(name: string) {
	try {
		upsertMany(await instancesApi.removeGroup(name));
	} catch (err) {
		const error = toAppError(err);
		if (error.code === "storage") void reloadInstances();
		throw error;
	}
}

export async function duplicateInstance(id: string) {
	const source = findInstance(id);
	if (!source) return;
	const name = uniqueName(i18n.t("instance.copyName", { name: source.name }).slice(0, 64));
	try {
		const copy = await instancesApi.duplicate(id, name);
		upsert(copy);
		toast.success(i18n.t("toasts.duplicated"), {
			description: copy.name,
			action: { label: i18n.t("common.open"), onClick: () => navigate(instancePath(copy.id)) },
		});
	} catch (err) {
		showError(err);
	}
}

export async function removeInstance(id: string): Promise<boolean> {
	const instance = findInstance(id);
	try {
		await instancesApi.remove(id);
		applyList(state.instances.filter((i) => i.id !== id));
		logTails.delete(id);
		if (instance) toast(i18n.t("toasts.deleted", { name: instance.name }));
		return true;
	} catch (err) {
		showError(err);
		return false;
	}
}

export function uniqueName(base: string, instances: Instance[] = state.instances): string {
	const taken = new Set(instances.map((i) => i.name.toLocaleLowerCase()));
	if (!taken.has(base.toLocaleLowerCase())) return base;
	for (let n = 2; ; n++) {
		const candidate = `${base} (${n})`;
		if (!taken.has(candidate.toLocaleLowerCase())) return candidate;
	}
}

export function compareRecent(a: Instance, b: Instance): number {
	const pa = a.lastPlayedAt ? Date.parse(a.lastPlayedAt) : 0;
	const pb = b.lastPlayedAt ? Date.parse(b.lastPlayedAt) : 0;
	if (pa !== pb) return pb - pa;
	return Date.parse(b.createdAt) - Date.parse(a.createdAt);
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export const subscribeInstances = subscribe;

export function useInstances(): InstancesStore {
	return useSyncExternalStore(subscribe, () => state);
}

export function getInstances(): InstancesStore {
	return state;
}

export function useNow(active: boolean, interval = 60_000): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (!active) return;
		const update = () => setNow(Date.now());
		const timer = setInterval(update, interval);
		const frame = requestAnimationFrame(update);
		return () => {
			clearInterval(timer);
			cancelAnimationFrame(frame);
		};
	}, [active, interval]);
	return now;
}

/* Диалоги сборок открываются с разных страниц и из тостов, поэтому общие. */

export type InstanceDialogKind = "create" | "rename" | "reinstall" | "remove" | "stop" | "crash";

export type CreatePreset = {
	gameVersion?: string;
	loader?: LoaderKind;
	/** Нет — обложка по умолчанию. */
	cover?: InstanceCover;
	/** Только на эту сессию диалога. */
	showSnapshots?: boolean;
	group?: string;
	afterCreate?: (instance: Instance) => void;
};

export type InstanceDialogs = {
	open: boolean;
	kind: InstanceDialogKind | null;
	instanceId: string | null;
	/** Меняется при каждом открытии: содержимое диалога начинается заново. */
	session: number;
	crash: { exitCode: number | null; lines: string[]; account: GameAccountInfo | null } | null;
	leaveAfterRemove: boolean;
	returnFocus: HTMLElement | null;
	createPreset: CreatePreset | null;
};

let dialogs: InstanceDialogs = {
	open: false,
	kind: null,
	instanceId: null,
	session: 0,
	crash: null,
	leaveAfterRemove: false,
	returnFocus: null,
	createPreset: null,
};
const dialogListeners = new Set<() => void>();

function setDialogs(next: InstanceDialogs) {
	dialogs = next;
	for (const listener of dialogListeners) listener();
}

export function openInstanceDialog(
	kind: Exclude<InstanceDialogKind, "crash">,
	instanceId: string | null,
	options?: {
		returnFocus?: HTMLElement | null;
		leaveAfterRemove?: boolean;
		createPreset?: CreatePreset | null;
	},
) {
	if (kind === "create") revalidateVersions();
	setDialogs({
		open: true,
		kind,
		instanceId,
		session: dialogs.session + 1,
		crash: null,
		leaveAfterRemove: options?.leaveAfterRemove ?? false,
		returnFocus: options?.returnFocus ?? (document.activeElement as HTMLElement | null),
		createPreset: kind === "create" ? (options?.createPreset ?? null) : null,
	});
}

export function openCreateInstanceDialog(returnFocus?: HTMLElement | null, preset?: CreatePreset) {
	openInstanceDialog("create", null, { returnFocus, createPreset: preset ?? null });
}

function logText(line: LogLine) {
	return line.time ? `[${line.time}] ${line.text}` : line.text;
}

export async function openCrashDialog(id: string, exitCode: number | null) {
	let lines = (logTails.get(id) ?? []).map(logText);
	try {
		const log = await instancesApi.log(id);
		if (log.length) lines = log.slice(-LOG_TAIL).map(logText);
	} catch (err) {
		console.error("instance_log", err);
	}
	setDialogs({
		open: true,
		kind: "crash",
		instanceId: id,
		session: dialogs.session + 1,
		crash: { exitCode, lines, account: state.lastExits[id]?.account ?? null },
		leaveAfterRemove: false,
		returnFocus: null,
		createPreset: null,
	});
}

export function closeInstanceDialog() {
	setDialogs({ ...dialogs, open: false });
}

export function restoreInstanceDialogFocus(event: Event) {
	const target = dialogs.returnFocus;
	if (!target?.isConnected) return;
	event.preventDefault();
	target.focus();
}

function subscribeDialogs(listener: () => void) {
	dialogListeners.add(listener);
	return () => dialogListeners.delete(listener);
}

export function useInstanceDialogs(): InstanceDialogs {
	return useSyncExternalStore(subscribeDialogs, () => dialogs);
}

export function getInstanceDialogs(): InstanceDialogs {
	return dialogs;
}

/* Второй запуск с того же аккаунта: спросить, за кого играть. */

const WARN_KEY = "warn-same-account";
let warnSameAccount = readStorage(WARN_KEY) !== "off";
const warnListeners = new Set<() => void>();

function sameAccountWarning() {
	return warnSameAccount;
}

export function setSameAccountWarning(on: boolean) {
	warnSameAccount = on;
	writeStorage(WARN_KEY, on ? null : "off");
	for (const listener of warnListeners) listener();
}

export function useSameAccountWarning(): boolean {
	return useSyncExternalStore(
		(listener) => {
			warnListeners.add(listener);
			return () => warnListeners.delete(listener);
		},
		() => warnSameAccount,
	);
}

export type LaunchAsDialog = {
	open: boolean;
	session: number;
	instanceId: string | null;
	/** Аккаунт, которым собирались запустить. */
	accountId: string | null;
	/** Выбран явно: он выделен по умолчанию, а не первый свободный. */
	explicit: boolean;
	world: string | null;
	returnFocus: HTMLElement | null;
};

let launchAs: LaunchAsDialog = {
	open: false,
	session: 0,
	instanceId: null,
	accountId: null,
	explicit: false,
	world: null,
	returnFocus: null,
};
const launchAsListeners = new Set<() => void>();

function setLaunchAs(next: LaunchAsDialog) {
	launchAs = next;
	for (const listener of launchAsListeners) listener();
}

function openLaunchAsDialog(
	instanceId: string,
	accountId: string,
	explicit: boolean,
	world: string | null,
	returnFocus: HTMLElement | null,
) {
	setLaunchAs({
		open: true,
		session: launchAs.session + 1,
		instanceId,
		accountId,
		explicit,
		world,
		returnFocus: returnFocus ?? (document.activeElement as HTMLElement | null),
	});
}

export function closeLaunchAsDialog() {
	setLaunchAs({ ...launchAs, open: false });
}

export function useLaunchAsDialog(): LaunchAsDialog {
	return useSyncExternalStore(
		(listener) => {
			launchAsListeners.add(listener);
			return () => launchAsListeners.delete(listener);
		},
		() => launchAs,
	);
}

import.meta.hot?.dispose(() => {
	for (const unlisten of unlisteners) unlisten();
});
