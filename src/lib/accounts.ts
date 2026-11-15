import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import { accountsApi, type Account, type AccountsState } from "@/lib/api";
import { errorText, toAppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";

export type AccountsStore = AccountsState & {
	/** `error` показываем как «нет аккаунтов». */
	status: "idle" | "loading" | "ready" | "error";
};

let state: AccountsStore = { status: "idle", accounts: [], activeId: null };
const listeners = new Set<() => void>();

function emit() {
	for (const listener of listeners) listener();
}

/** Аккаунтам, у которых `needsRelogin` стал `true`, показываем тост. */
function apply(next: AccountsState) {
	const previous = state.status === "ready" ? new Map(state.accounts.map((a) => [a.id, a])) : null;
	state = { status: "ready", accounts: next.accounts, activeId: next.activeId };
	emit();
	if (!previous) return;
	for (const account of next.accounts) {
		const before = previous.get(account.id);
		if (account.needsRelogin && before && !before.needsRelogin) notifyNeedsRelogin(account);
	}
}

function notifyNeedsRelogin(account: Account) {
	toast.warning(i18n.t("accounts.toast.needsRelogin", { username: account.username }), {
		action: {
			label: i18n.t("accounts.toast.needsReloginAction"),
			onClick: () => openReloginDialog(account),
		},
		duration: 8000,
	});
}

let unlistenChanged: (() => void) | null = null;
let started = false;

export function initAccounts() {
	if (started) return;
	started = true;
	state = { ...state, status: "loading" };
	emit();
	void accountsApi
		.onChanged(apply)
		.then((unlisten) => {
			unlistenChanged = unlisten;
		})
		.catch((err: unknown) => console.error("Не удалось подписаться на accounts-changed", err));
	void reloadAccounts(true);
}

export async function reloadAccounts(initial = false) {
	try {
		apply(await accountsApi.list());
	} catch (err) {
		console.error("accounts_list", err);
		if (initial || state.status !== "ready") {
			state = { ...state, status: "error" };
			emit();
			toast.error(i18n.t("accounts.toast.loadFailed"));
		}
	}
}

export async function setActiveAccount(id: string) {
	try {
		apply(await accountsApi.setActive(id));
	} catch (err) {
		toast.error(errorText(toAppError(err)));
		void reloadAccounts();
	}
}

export function applyAccountsState(next: AccountsState) {
	apply(next);
}

const loginListeners = new Set<(accountId: string) => void>();

/** Успешный вход любым путём: диалог, тост, меню, запуск игры. */
export function onAccountLogin(listener: (accountId: string) => void) {
	loginListeners.add(listener);
	return () => loginListeners.delete(listener);
}

/** Ответ команды — сразу в стор, не дожидаясь `accounts-changed`. */
export function applyAddedAccount(account: Account) {
	const exists = state.accounts.some((a) => a.id === account.id);
	apply({
		accounts: exists
			? state.accounts.map((a) => (a.id === account.id ? account : a))
			: [...state.accounts, account],
		activeId: account.id,
	});
	for (const listener of loginListeners) listener(account.id);
}

export function getAccounts(): AccountsStore {
	return state;
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useAccounts(): AccountsStore {
	return useSyncExternalStore(subscribe, () => state);
}

export function activeAccount(store: AccountsState): Account | null {
	return store.accounts.find((a) => a.id === store.activeId) ?? null;
}

/* Диалоги аккаунтов открываются из меню, с Главной и из тоста, поэтому состояние общее. */

export type AddDialogState = {
	open: boolean;
	/** `relogin` и `microsoft` пропускают выбор способа входа. */
	mode: "add" | "relogin" | "microsoft";
	account: Account | null;
	/** Меняется при каждом открытии: содержимое диалога начинается заново. */
	session: number;
};

export type RemoveDialogState = {
	open: boolean;
	account: Account | null;
	session: number;
};

export type AccountDialogs = {
	add: AddDialogState;
	remove: RemoveDialogState;
	returnFocus: HTMLElement | null;
};

let dialogs: AccountDialogs = {
	add: { open: false, mode: "add", account: null, session: 0 },
	remove: { open: false, account: null, session: 0 },
	returnFocus: null,
};
const dialogListeners = new Set<() => void>();

function setDialogs(next: AccountDialogs) {
	dialogs = next;
	for (const listener of dialogListeners) listener();
}

export const ACCOUNT_SWITCHER_ATTR = "data-account-switcher";

function focusTarget(from?: HTMLElement | null): HTMLElement | null {
	return from ?? document.querySelector<HTMLElement>(`[${ACCOUNT_SWITCHER_ATTR}]`);
}

export function openAddAccountDialog(returnFocus?: HTMLElement | null) {
	setDialogs({
		...dialogs,
		add: { open: true, mode: "add", account: null, session: dialogs.add.session + 1 },
		returnFocus: focusTarget(returnFocus),
	});
}

export function openMicrosoftLoginDialog(returnFocus?: HTMLElement | null) {
	setDialogs({
		...dialogs,
		add: { open: true, mode: "microsoft", account: null, session: dialogs.add.session + 1 },
		returnFocus: focusTarget(returnFocus),
	});
}

export function openReloginDialog(account: Account, returnFocus?: HTMLElement | null) {
	setDialogs({
		...dialogs,
		add: { open: true, mode: "relogin", account, session: dialogs.add.session + 1 },
		returnFocus: focusTarget(returnFocus),
	});
}

export function closeAddAccountDialog() {
	setDialogs({ ...dialogs, add: { ...dialogs.add, open: false } });
}

export function openRemoveAccountDialog(account: Account, returnFocus?: HTMLElement | null) {
	setDialogs({
		...dialogs,
		remove: { open: true, account, session: dialogs.remove.session + 1 },
		returnFocus: focusTarget(returnFocus),
	});
}

export function closeRemoveAccountDialog() {
	setDialogs({ ...dialogs, remove: { ...dialogs.remove, open: false } });
}

export function restoreDialogFocus(event: Event) {
	const target = dialogs.returnFocus;
	if (!target?.isConnected) return;
	event.preventDefault();
	target.focus();
}

function subscribeDialogs(listener: () => void) {
	dialogListeners.add(listener);
	return () => dialogListeners.delete(listener);
}

export function getAccountDialogs(): AccountDialogs {
	return dialogs;
}

export function useAccountDialogs(): AccountDialogs {
	return useSyncExternalStore(subscribeDialogs, () => dialogs);
}

import.meta.hot?.dispose(() => {
	unlistenChanged?.();
});
