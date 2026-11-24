import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import { getAccounts, onAccountLogin, reloadAccounts } from "@/lib/accounts";
import {
	skinsApi,
	type Account,
	type Cape,
	type SkinChanges,
	type SkinEntry,
	type SkinModel,
	type SkinProfile,
	type SkinRejected,
	type SkinsAdded,
	type WornSkin,
} from "@/lib/api";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { formatDate, formatDuration } from "@/lib/format";
import { i18n } from "@/lib/i18n";

const NO_ACCOUNT = "";
/** Ядро пропускает смену скина не чаще раза в 10 с. */
const WEAR_PAUSE_SECONDS = 10;
const CELEBRATION_MS = 1200;
const WORN_HINT_MS = 5000;
/** Сколько новые карточки помнят, что появились от действия пользователя. */
const FRESH_MS = 600;
/** Потом голова берётся с mc-heads.net; поздний ответ всё равно применится. */
const WORN_WAIT_MS = 5000;
const MODEL_SAVE_DELAY = 300;
export const SKIN_NAME_MAX = 48;
export const SEARCH_FROM = 12;

/** Только в памяти. `skinId: null` — текущий скин; поля `model`/`capeId` нет — как в профиле, `capeId: null` — без плаща. */
export type SkinDraft = { skinId: string | null; model?: SkinModel; capeId?: string | null };

export type ProfileState = {
	/** При повторных запросах прежний профиль остаётся на экране. */
	status: "loading" | "ready" | "error";
	profile: SkinProfile | null;
	error: AppError | null;
};

export type WornView = {
	/** Пока `loading`, голова — скелетон, запроса к mc-heads.net нет. */
	status: "loading" | "ready" | "error";
	map: Record<string, WornSkin>;
};

export type SkinDialogKind = "rename" | "remove";

export type SkinFilter = "all" | "uploaded" | "history";

export type ReplaceRequest = { account: Account; changes: SkinChanges; returnFocus: HTMLElement | null };

export type SkinDialogs =
	| { kind: null; session: number }
	| {
			kind: SkinDialogKind;
			session: number;
			skin: SkinEntry;
			returnFocus: HTMLElement | null;
	  }
	| { kind: "takeOff"; session: number; accountId: string; returnFocus: HTMLElement | null }
	| ({ kind: "replace"; session: number } & ReplaceRequest)
	| { kind: "details"; session: number; title: string; details: string };

export type SkinsStore = {
	status: "idle" | "loading" | "ready" | "error";
	error: AppError | null;
	items: SkinEntry[];
	worn: WornView;
	/** Запрашиваются при входе на страницу, не при запуске приложения. */
	profiles: Record<string, ProfileState>;
	/** Новый профиль черновик не сбрасывает. */
	drafts: Record<string, SkinDraft>;
	capeTurn: number;
	query: string;
	filter: SkinFilter;
	walking: boolean;
	/** Ответ относится к отправленному, а не к текущему черновику. */
	applying: Record<string, true>;
	/** Запрос изменения один, поэтому снятие блокирует и «Надеть». */
	resetting: Record<string, true>;
	waits: Record<string, number>;
	/** Mojang отверг и свежий токен: флага `needsRelogin` в ядре в этом случае нет. */
	relogin: Record<string, true>;
	celebration: { seq: number; accountId: string } | null;
	/** Аккаунт, на который только что надели скин: под кнопкой подсказка про игру. */
	wornHint: string | null;
	addedSeq: number;
	/** Добавлены пользователем только что: только они появляются с анимацией. */
	fresh: string[];
	dialogs: SkinDialogs;
};

let state: SkinsStore = {
	status: "idle",
	error: null,
	items: [],
	worn: { status: "loading", map: {} },
	profiles: {},
	drafts: {},
	capeTurn: 0,
	query: "",
	filter: "all",
	walking: false,
	applying: {},
	resetting: {},
	waits: {},
	relogin: {},
	celebration: null,
	wornHint: null,
	addedSeq: 0,
	fresh: [],
	dialogs: { kind: null, session: 0 },
};
const listeners = new Set<() => void>();

function set(next: Partial<SkinsStore>) {
	state = { ...state, ...next };
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useSkins(): SkinsStore {
	return useSyncExternalStore(subscribe, () => state);
}

/** Только записи: аватар не должен перерисовываться от каждой примерки. */
export function useWornSkins(): WornView {
	return useSyncExternalStore(subscribe, () => state.worn);
}

let librarySeq = 0;
let wornSeq = 0;
/** Выбор рук, ещё не сохранённый в ядре: прочитанная библиотека его не перетирает. */
const pendingModels = new Map<string, { model: SkinModel; timer: ReturnType<typeof setTimeout> }>();

function withPendingModels(items: SkinEntry[]) {
	return items.map((skin) => {
		const pending = pendingModels.get(skin.id);
		return pending ? { ...skin, model: pending.model } : skin;
	});
}

function isEmptyDraft(draft: SkinDraft) {
	return draft.skinId === null && draft.model === undefined && draft.capeId === undefined;
}

function prunedDrafts(items: SkinEntry[]) {
	const known = new Set(items.map((skin) => skin.id));
	const drafts: Record<string, SkinDraft> = {};
	for (const [key, draft] of Object.entries(state.drafts)) {
		const next =
			draft.skinId !== null && !known.has(draft.skinId)
				? { skinId: null, capeId: draft.capeId }
				: draft;
		if (!isEmptyDraft(next)) drafts[key] = next;
	}
	return drafts;
}

function applyLibrary(items: SkinEntry[]) {
	const merged = withPendingModels(items);
	set({ status: "ready", error: null, items: merged, drafts: prunedDrafts(merged) });
}

export async function loadSkinLibrary() {
	const seq = ++librarySeq;
	if (state.status !== "ready") set({ status: "loading", error: null });
	try {
		const items = await skinsApi.library();
		if (seq === librarySeq) applyLibrary(items);
	} catch (err) {
		if (seq !== librarySeq) return;
		const error = toAppError(err);
		console.error("skins_library", error);
		if (state.status === "ready") toast.error(errorText(error));
		else set({ status: "error", error });
	}
}

async function loadWorn() {
	const seq = ++wornSeq;
	try {
		const map = await skinsApi.worn();
		if (seq === wornSeq) set({ worn: { status: "ready", map } });
	} catch (err) {
		if (seq !== wornSeq) return;
		console.error("skins_worn", err);
		if (state.worn.status === "loading") set({ worn: { status: "error", map: {} } });
	}
}

const unlisteners: Array<() => void> = [];
let started = false;

function listen(subscription: Promise<() => void>, name: string) {
	void subscription
		.then((unlisten) => unlisteners.push(unlisten))
		.catch((err: unknown) => console.error(`Не удалось подписаться на ${name}`, err));
}

/** Не зависит от кода страницы: голова в полосе заголовка нужна сразу. */
export function initSkins() {
	if (started) return;
	started = true;
	listen(
		skinsApi.onChanged(() => {
			void loadWorn();
			if (state.status === "ready" || state.status === "loading") void loadSkinLibrary();
			// Пока идёт изменение, профиль придёт в его ответе.
			if (pageAccount && !state.applying[pageAccount] && !state.resetting[pageAccount])
				void loadProfile(pageAccount);
		}),
		"skins-changed",
	);
	// Страница видит снятый флаг и перечитывает профиль сама.
	unlisteners.push(
		onAccountLogin((accountId) => {
			if (!state.relogin[accountId]) return;
			const relogin = { ...state.relogin };
			delete relogin[accountId];
			set({ relogin });
		}),
	);
	setTimeout(() => {
		if (state.worn.status !== "loading") return;
		console.warn("skins_worn не ответил: голова — из прежней цепочки");
		set({ worn: { status: "error", map: {} } });
	}, WORN_WAIT_MS);
	void loadWorn();
}

/** Файл скина мог пропасть, а события об этом нет. */
export function refreshSkins() {
	void loadWorn();
	void loadSkinLibrary();
}

/** По `skins-changed` перечитывается только он. */
let pageAccount: string | null = null;
const profileSeqs = new Map<string, number>();

function nextProfileSeq(accountId: string) {
	const seq = (profileSeqs.get(accountId) ?? 0) + 1;
	profileSeqs.set(accountId, seq);
	return seq;
}

export function setSkinsPageAccount(accountId: string | null) {
	pageAccount = accountId;
}

function applyProfile(accountId: string, profile: SkinProfile) {
	nextProfileSeq(accountId);
	const map = { ...state.worn.map };
	if (profile.skin?.texture) map[accountId] = profile.skin;
	else delete map[accountId];
	set({
		profiles: { ...state.profiles, [accountId]: { status: "ready", profile, error: profile.error } },
		worn: { status: "ready", map },
	});
}

function noteProfileError(accountId: string, error: AppError | null) {
	if (!error) return;
	const pause = Number.parseInt(error.details ?? "", 10);
	if ((error.code === "skin_too_often" || error.code === "rate_limited") && pause > 0)
		startWait(accountId, pause);
	if (error.code === "needs_relogin") set({ relogin: { ...state.relogin, [accountId]: true } });
}

export async function loadProfile(accountId: string, refresh = false) {
	// До нового входа не спрашиваем: при 401 подряд окно частоты ядра не действует.
	const account = getAccounts().accounts.find((a) => a.id === accountId);
	if (state.relogin[accountId] || account?.needsRelogin) return;
	const seq = nextProfileSeq(accountId);
	const known = state.profiles[accountId];
	if (!known?.profile)
		set({
			profiles: { ...state.profiles, [accountId]: { status: "loading", profile: null, error: null } },
		});
	try {
		const profile = await skinsApi.profile(accountId, refresh);
		if (profileSeqs.get(accountId) !== seq) return;
		applyProfile(accountId, profile);
		noteProfileError(accountId, profile.error);
	} catch (err) {
		if (profileSeqs.get(accountId) !== seq) return;
		const error = toAppError(err);
		console.error("skin_profile", error);
		// Показанный профиль остаётся, подпись скажет, что он не обновился.
		const shown = state.profiles[accountId]?.profile ?? null;
		set({
			profiles: {
				...state.profiles,
				[accountId]: { status: shown ? "ready" : "error", profile: shown, error },
			},
		});
		noteProfileError(accountId, error);
	}
}

/** Пока профиля нет — сохранённый. */
export function currentSkin(store: SkinsStore, accountId: string | null): WornSkin | null {
	if (!accountId) return null;
	const profile = store.profiles[accountId]?.profile;
	return profile ? profile.skin : (store.worn.map[accountId] ?? null);
}

export type StageView = {
	current: WornSkin | null;
	entry: SkinEntry | undefined;
	picked: boolean;
	/** `null` — манекен. */
	texture: string | null;
	model: SkinModel;
	capes: Cape[];
	capeId: string | null;
	capeTexture: string | null;
	/** Только изменившееся: это и уходит в `skin_apply`. */
	changes: SkinChanges;
	trying: boolean;
};

export function stageView(store: SkinsStore, accountId: string | null): StageView {
	const draft: SkinDraft = store.drafts[accountId ?? NO_ACCOUNT] ?? { skinId: null };
	const current = currentSkin(store, accountId);
	const profile = accountId ? (store.profiles[accountId]?.profile ?? null) : null;
	const picked = draft.skinId !== null && draft.skinId !== current?.skinId;
	const entryId = picked ? draft.skinId : current?.skinId;
	const entry = store.items.find((skin) => skin.id === entryId);
	const texture = picked ? (entry?.texture ?? null) : (current?.texture ?? null);
	// Обычный скин не загружается как свой: черновик с руками мог появиться до прихода профиля.
	const draftModel = !picked && current?.isDefault ? undefined : draft.model;
	const model = draftModel ?? (picked ? entry?.model : current?.model) ?? "classic";
	const capes = profile?.capes ?? [];
	const activeCape = profile?.activeCapeId ?? null;
	const capeId = draft.capeId === undefined ? activeCape : draft.capeId;
	const changes: SkinChanges = {};
	if (picked && draft.skinId) {
		changes.skinId = draft.skinId;
		changes.model = model;
	} else if (current && model !== current.model) changes.model = model;
	if (profile && capeId !== activeCape) changes.capeId = capeId;
	return {
		current,
		entry,
		picked,
		texture,
		model,
		capes,
		capeId,
		capeTexture: capes.find((cape) => cape.id === capeId)?.texture ?? null,
		changes,
		trying: Object.keys(changes).length > 0,
	};
}

function activeKey() {
	return getAccounts().activeId ?? NO_ACCOUNT;
}

function activeId() {
	return getAccounts().activeId;
}

function setDraft(key: string, draft: SkinDraft | null) {
	const drafts = { ...state.drafts };
	if (draft && !isEmptyDraft(draft)) drafts[key] = draft;
	else delete drafts[key];
	set({ drafts });
}

/** Руки — последний выбор за скином; если у надетого они отличаются, это примерка. */
export function tryOnSkin(skin: SkinEntry) {
	const key = activeKey();
	const current = currentSkin(state, activeId());
	const draft = state.drafts[key];
	if (current?.skinId === skin.id)
		setDraft(key, {
			skinId: null,
			model: skin.model === current.model ? undefined : skin.model,
			capeId: draft?.capeId,
		});
	else setDraft(key, { skinId: skin.id, capeId: draft?.capeId });
}

function saveModel(id: string, model: SkinModel) {
	const pending = pendingModels.get(id);
	if (pending) clearTimeout(pending.timer);
	const timer = setTimeout(() => {
		pendingModels.delete(id);
		skinsApi.update(id, { model }).catch((err: unknown) => {
			toast.error(errorText(toAppError(err)));
			void loadSkinLibrary();
		});
	}, MODEL_SAVE_DELAY);
	pendingModels.set(id, { model, timer });
	set({ items: state.items.map((skin) => (skin.id === id ? { ...skin, model } : skin)) });
}

export function setDraftModel(model: SkinModel) {
	const key = activeKey();
	const view = stageView(state, activeId());
	if (!view.texture || view.model === model || (!view.picked && view.current?.isDefault)) return;
	const draft = state.drafts[key] ?? { skinId: null };
	if (view.entry) saveModel(view.entry.id, model);
	if (view.picked) {
		setDraft(key, { ...draft, model: undefined });
		return;
	}
	setDraft(key, { ...draft, model: model === view.current?.model ? undefined : model });
}

export function setDraftCape(capeId: string | null) {
	const key = activeKey();
	const id = activeId();
	const active = (id ? state.profiles[id]?.profile?.activeCapeId : null) ?? null;
	const draft = state.drafts[key] ?? { skinId: null };
	setDraft(key, { ...draft, capeId: capeId === active ? undefined : capeId });
	if (capeId !== null) set({ capeTurn: state.capeTurn + 1 });
}

export function revertDraft() {
	const key = activeKey();
	const current = currentSkin(state, activeId());
	const entry = state.items.find((skin) => skin.id === current?.skinId);
	if (current && entry && entry.model !== current.model) saveModel(entry.id, current.model);
	setDraft(key, null);
}

export function setSkinsQuery(query: string) {
	set({ query });
}

export function setSkinsFilter(filter: SkinFilter) {
	set({ filter });
}

export function isHistorySkin(skin: SkinEntry) {
	return skin.origin === "account";
}

/** В «Истории» — скины активного аккаунта; без записи `seen` скин считается общим. */
export function inAccountHistory(skin: SkinEntry, accountId: string | null) {
	if (!isHistorySkin(skin)) return false;
	if (!accountId || !skin.seen?.length) return true;
	return skin.seen.some((seen) => seen.accountId === accountId);
}

function seenOf(skin: SkinEntry, accountId: string | null) {
	return skin.seen?.find((seen) => seen.accountId === accountId) ?? skin.seen?.[0];
}

/** Когда скин впервые был на аккаунте; у загруженного — когда добавлен. */
export function skinFirstSeen(skin: SkinEntry, accountId: string | null) {
	return seenOf(skin, accountId)?.firstSeenAt ?? skin.addedAt;
}

/** Последний раз видели или добавили: по нему новые — сверху. */
export function skinRecency(skin: SkinEntry) {
	let latest = Date.parse(skin.addedAt) || 0;
	for (const seen of skin.seen ?? []) latest = Math.max(latest, Date.parse(seen.lastSeenAt) || 0);
	return latest;
}

/** У скина из истории имени может не быть. */
export function skinDisplayName(skin: SkinEntry, accountId: string | null = null) {
	const name = skin.name.trim();
	if (name) return name;
	return i18n.t("skins.card.historyName", { date: formatDate(skinFirstSeen(skin, accountId)) });
}

export function setSkinWalking(walking: boolean) {
	set({ walking });
}

function baseName(path: string) {
	return path.split(/[\\/]/).pop() ?? path;
}

function rejectedText(file: SkinRejected) {
	const name = baseName(file.path);
	switch (file.reason) {
		case "not_png":
			return i18n.t("skins.file.notPng", { name });
		case "bad_size":
			return i18n.t("skins.file.badSize", { name, w: file.width ?? "?", h: file.height ?? "?" });
		case "too_large":
			return i18n.t("skins.file.tooLarge", { name });
		case "unreadable":
			return i18n.t("skins.file.unreadable", { name });
	}
}

const REJECTED_SHOWN = 3;
let freshTimer: ReturnType<typeof setTimeout> | null = null;

function applyAdded(result: SkinsAdded) {
	const { added, duplicates, rejected } = result;
	if (added.length) {
		const ids = new Set(added.map((skin) => skin.id));
		const items = [...[...added].reverse(), ...state.items.filter((skin) => !ids.has(skin.id))];
		set({ items: withPendingModels(items), fresh: [...ids] });
		if (freshTimer !== null) clearTimeout(freshTimer);
		freshTimer = setTimeout(() => {
			freshTimer = null;
			set({ fresh: [] });
		}, FRESH_MS);
		toast.success(i18n.t("skins.toast.added", { count: added.length }));
	}
	// Такого PNG нет в списке: его файл пропадал и теперь восстановлен.
	const known = new Set(state.items.map((skin) => skin.id));
	const restored = duplicates.filter((skin) => !known.has(skin.id));
	for (const skin of duplicates)
		toast(
			i18n.t(known.has(skin.id) ? "skins.toast.duplicate" : "skins.toast.restored", {
				name: skin.name,
			}),
		);
	if (restored.length) {
		set({ items: withPendingModels([...restored, ...state.items]) });
		refreshSkins();
	}
	const next = added.at(-1) ?? duplicates.at(-1);
	if (next) {
		tryOnSkin(state.items.find((skin) => skin.id === next.id) ?? next);
		// Новый скин мог бы оказаться скрыт поиском или фильтром «История».
		set({
			addedSeq: state.addedSeq + 1,
			query: "",
			filter: state.filter === "history" ? "all" : state.filter,
		});
	}
	if (rejected.length) {
		const lines = rejected.slice(0, REJECTED_SHOWN).map(rejectedText);
		if (rejected.length > REJECTED_SHOWN)
			lines.push(i18n.t("skins.toast.andMore", { n: rejected.length - REJECTED_SHOWN }));
		toast.error(i18n.t("skins.toast.badFile"), { description: lines.join("; ") });
	}
}

const adding = new Set<string>();

export async function addSkinFiles(paths: string[]) {
	const key = [...paths].sort().join("|");
	if (adding.has(key)) return;
	adding.add(key);
	try {
		applyAdded(await skinsApi.addFiles(paths));
	} catch (err) {
		toast.error(errorText(toAppError(err)));
	} finally {
		adding.delete(key);
	}
}

let picking = false;

export async function pickSkinFiles() {
	if (picking) return;
	picking = true;
	try {
		const result = await skinsApi.pickFiles(i18n.language === "en" ? "en" : "ru");
		if (result) applyAdded(result);
	} catch (err) {
		toast.error(errorText(toAppError(err)));
	} finally {
		picking = false;
	}
}

const waitUntil = new Map<string, number>();
let waitTimer: ReturnType<typeof setInterval> | null = null;

function refreshWaits() {
	const now = Date.now();
	const waits: Record<string, number> = {};
	for (const [id, until] of waitUntil) {
		if (until <= now) waitUntil.delete(id);
		else waits[id] = Math.ceil((until - now) / 1000);
	}
	set({ waits });
	if (!waitUntil.size && waitTimer !== null) {
		clearInterval(waitTimer);
		waitTimer = null;
	}
}

/** Округление вверх, чтобы не обещать раньше срока. */
export function waitTimeText(seconds: number) {
	if (seconds < 60) return i18n.t("skins.action.seconds", { n: seconds });
	return formatDuration(Math.ceil(seconds / 60) * 60);
}

function startWait(accountId: string, seconds: number) {
	waitUntil.set(accountId, Date.now() + seconds * 1000);
	refreshWaits();
	waitTimer ??= setInterval(refreshWaits, 1000);
}

let celebrationSeq = 0;
let celebrationTimer: ReturnType<typeof setTimeout> | null = null;

let wornHintTimer: ReturnType<typeof setTimeout> | null = null;

function celebrate(accountId: string) {
	if (celebrationTimer !== null) clearTimeout(celebrationTimer);
	if (wornHintTimer !== null) clearTimeout(wornHintTimer);
	set({ celebration: { seq: ++celebrationSeq, accountId }, wornHint: accountId });
	celebrationTimer = setTimeout(() => {
		celebrationTimer = null;
		set({ celebration: null });
	}, CELEBRATION_MS);
	wornHintTimer = setTimeout(() => {
		wornHintTimer = null;
		set({ wornHint: null });
	}, WORN_HINT_MS);
}

export function skinChangeBusy(): boolean {
	return Object.keys(state.applying).length > 0 || Object.keys(state.resetting).length > 0;
}

function setApplying(accountId: string, busy: boolean) {
	const applying = { ...state.applying };
	if (busy) applying[accountId] = true;
	else delete applying[accountId];
	set({ applying });
}

function setResetting(accountId: string, busy: boolean) {
	const resetting = { ...state.resetting };
	if (busy) resetting[accountId] = true;
	else delete resetting[accountId];
	set({ resetting });
}

function showDetails(title: string, details: string) {
	set({ dialogs: { kind: "details", session: state.dialogs.session + 1, title, details } });
}

function showChangeError(accountId: string, err: unknown, title: string, retry: () => void) {
	const error = toAppError(err);
	const pause = Number.parseInt(error.details ?? "", 10);
	if (error.code === "skin_too_often") {
		startWait(accountId, pause > 0 ? pause : WEAR_PAUSE_SECONDS);
		return;
	}
	if (error.code === "rate_limited" && pause > 0) {
		startWait(accountId, pause);
		toast.error(title, {
			description: i18n.t("skins.toast.rateLimited", { time: waitTimeText(pause) }),
			duration: 8000,
		});
		return;
	}
	if (error.code === "needs_relogin") {
		set({ relogin: { ...state.relogin, [accountId]: true } });
		void reloadAccounts();
		toast.error(errorText(error));
		return;
	}
	const details = error.details
		? {
				label: i18n.t("common.details"),
				onClick: () => showDetails(title, error.details as string),
			}
		: undefined;
	if (error.code === "skin_rejected") {
		toast.error(errorText(error), { action: details });
		return;
	}
	// 429 — пауза ядра, остальное от повтора не изменится. Часы можно поправить, поэтому `clock_out_of_sync` не здесь.
	const final = [
		"rate_limited",
		"no_minecraft",
		"no_xbox_account",
		"child_account",
		"region_blocked",
		"not_found",
		"skin_not_found",
		"storage",
	].includes(error.code);
	const again = final ? undefined : { label: i18n.t("common.retry"), onClick: retry };
	// Две кнопки сжимают текст тоста в столбик.
	const preferDetails = ["unknown", "storage", "skin_not_found"].includes(error.code);
	toast.error(title, {
		description: errorText(error),
		action: (preferDetails ? details : undefined) ?? again,
		duration: 8000,
	});
}

/** Уходит отправленное, а то, что успели примерить за это время, остаётся. */
function draftAfterApply(accountId: string, sent: SkinChanges, capeFailed: boolean) {
	const draft = state.drafts[accountId];
	if (!draft) return;
	const next = { ...draft };
	if (sent.skinId !== undefined && draft.skinId === sent.skinId) {
		next.skinId = null;
		next.model = undefined;
	} else if (sent.skinId === undefined && sent.model !== undefined && draft.model === sent.model)
		next.model = undefined;
	if (sent.capeId !== undefined && !capeFailed && draft.capeId === sent.capeId) next.capeId = undefined;
	setDraft(accountId, next);
}

function openReplaceDialog(account: Account, changes: SkinChanges, returnFocus: HTMLElement | null) {
	set({
		dialogs: { kind: "replace", session: state.dialogs.session + 1, account, changes, returnFocus },
	});
}

async function apply(account: Account, sent: SkinChanges, allowLosePrevious = false) {
	if (state.applying[account.id] || state.resetting[account.id]) return;
	setApplying(account.id, true);
	const skinSent = sent.skinId !== undefined || sent.model !== undefined;
	try {
		// Чтение, начатое раньше, устарело: профиль придёт в ответе.
		nextProfileSeq(account.id);
		const { profile, capeError, savedPrevious } = await skinsApi.apply(
			account.id,
			sent,
			allowLosePrevious,
		);
		applyProfile(account.id, profile);
		draftAfterApply(account.id, sent, Boolean(capeError));
		if (savedPrevious && !state.items.some((skin) => skin.id === savedPrevious.id))
			set({ items: withPendingModels([savedPrevious, ...state.items]) });
		startWait(account.id, WEAR_PAUSE_SECONDS);
		celebrate(account.id);
		const nick = account.username;
		const notes = [
			i18n.t("skins.toast.wornHint"),
			...(savedPrevious ? [i18n.t("skins.toast.savedPrevious")] : []),
		].join(". ");
		if (capeError) {
			const pause = Number.parseInt(capeError.details ?? "", 10);
			const limited = capeError.code === "rate_limited" || capeError.code === "skin_too_often";
			if (limited && pause > 0) startWait(account.id, pause);
			toast.warning(i18n.t("skins.toast.partial"), {
				description: errorText(capeError),
				action: limited
					? undefined
					: {
							label: i18n.t("common.retry"),
							onClick: () => void apply(account, { capeId: sent.capeId }),
						},
				duration: 8000,
			});
			if (savedPrevious) toast(i18n.t("skins.toast.savedPrevious"));
		} else if (skinSent) toast.success(i18n.t("skins.toast.worn", { nick }), { description: notes });
		else
			toast.success(
				i18n.t(sent.capeId ? "skins.toast.capeWorn" : "skins.toast.capeRemoved", { nick }),
				{ description: i18n.t("skins.toast.wornHint") },
			);
	} catch (err) {
		// На сервере ничего не менялось, поэтому паузы нет.
		if (toAppError(err).code === "skin_previous_unknown")
			openReplaceDialog(account, sent, document.querySelector<HTMLElement>("[data-skin-main]"));
		else
			showChangeError(
				account.id,
				err,
				i18n.t(skinSent ? "skins.toast.wearFailed" : "skins.toast.capeFailed"),
				() => void apply(account, sent, allowLosePrevious),
			);
		// Чтение, начатое до отправки, отменено: без него экран остался бы в «Загружаем ваш скин…».
		if (state.profiles[account.id]?.status === "loading") void loadProfile(account.id);
	} finally {
		setApplying(account.id, false);
	}
}

export function wearDraft(account: Account, returnFocus: HTMLElement | null) {
	const { changes, trying } = stageView(state, account.id);
	if (!trying) return;
	if (changes.skinId !== undefined && losesCurrentSkin(state, account.id)) {
		openReplaceDialog(account, changes, returnFocus);
		return;
	}
	void apply(account, changes);
}

/** Профиль не загрузился или текстуры нет — прежний скин положить в библиотеку не из чего. */
function losesCurrentSkin(store: SkinsStore, accountId: string) {
	const entry = store.profiles[accountId];
	if (!entry?.profile) return true;
	const { skin } = entry.profile;
	return Boolean(skin && !skin.texture && !skin.isDefault && !skin.skinId);
}

/** Двойной щелчок и Ctrl/⌘+Enter по карточке: примерить и сразу надеть. */
export function wearSkin(account: Account, skin: SkinEntry, returnFocus: HTMLElement | null) {
	tryOnSkin(skin);
	wearDraft(account, returnFocus);
}

export function confirmReplaceSkin({ account, changes }: ReplaceRequest) {
	void apply(account, changes, true);
}

export async function takeOffSkin(account: Account) {
	if (state.applying[account.id] || state.resetting[account.id]) return;
	setResetting(account.id, true);
	try {
		nextProfileSeq(account.id);
		applyProfile(account.id, await skinsApi.reset(account.id));
		startWait(account.id, WEAR_PAUSE_SECONDS);
		toast.success(i18n.t("skins.toast.takenOff", { nick: account.username }));
	} catch (err) {
		showChangeError(account.id, err, i18n.t("skins.toast.takeOffFailed"), () => {
			void takeOffSkin(account);
		});
		// Чтение, начатое до отправки, отменено: без него «Надеть» осталась бы неактивной.
		if (state.profiles[account.id]?.status === "loading") void loadProfile(account.id);
	} finally {
		setResetting(account.id, false);
	}
}

function patchCurrentSkinId(match: (skin: WornSkin, accountId: string) => boolean, skinId: string | null) {
	const profiles = { ...state.profiles };
	for (const [accountId, entry] of Object.entries(profiles)) {
		const skin = entry.profile?.skin;
		if (entry.profile && skin && match(skin, accountId))
			profiles[accountId] = { ...entry, profile: { ...entry.profile, skin: { ...skin, skinId } } };
	}
	const map = { ...state.worn.map };
	for (const [accountId, skin] of Object.entries(map))
		if (match(skin, accountId)) map[accountId] = { ...skin, skinId };
	set({ profiles, worn: { ...state.worn, map } });
}

export async function renameSkin(id: string, name: string) {
	const updated = await skinsApi.update(id, { name });
	set({ items: state.items.map((skin) => (skin.id === id ? { ...skin, name: updated.name } : skin)) });
}

export async function removeSkin(id: string) {
	await skinsApi.remove(id);
	const items = state.items.filter((skin) => skin.id !== id);
	set({ items, drafts: prunedDrafts(items) });
	// На аккаунте скин остаётся и виден из кэша, но в библиотеке его больше нет.
	patchCurrentSkinId((skin) => skin.skinId === id, null);
}

export async function revealSkin(id: string) {
	try {
		await skinsApi.reveal(id);
	} catch (err) {
		toast.error(errorText(toAppError(err)));
	}
}

export function openSkinDialog(kind: SkinDialogKind, skin: SkinEntry, returnFocus: HTMLElement | null) {
	set({ dialogs: { kind, session: state.dialogs.session + 1, skin, returnFocus } });
}

export function openTakeOffDialog(accountId: string, returnFocus: HTMLElement | null) {
	set({ dialogs: { kind: "takeOff", session: state.dialogs.session + 1, accountId, returnFocus } });
}

export function closeSkinDialog() {
	set({ dialogs: { kind: null, session: state.dialogs.session } });
}

import.meta.hot?.dispose(() => {
	for (const unlisten of unlisteners) unlisten();
	if (waitTimer !== null) clearInterval(waitTimer);
	if (wornHintTimer !== null) clearTimeout(wornHintTimer);
	if (freshTimer !== null) clearTimeout(freshTimer);
});
