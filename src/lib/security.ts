import { useSyncExternalStore } from "react";
import { toast } from "sonner";
import { appApi, external } from "@/lib/api";
import { i18n } from "@/lib/i18n";

export const FONT_VULN_URL = "https://wurst.wiki/minecraft2shell";

export function openFontVulnInfo() {
	external.open(FONT_VULN_URL).catch((err: unknown) => {
		console.error("open-url", err);
		toast.error(i18n.t("common.openLinkFailed"));
	});
}

/* Предупреждение на странице сборки скрывается только до перезапуска. */

let dismissed: ReadonlySet<string> = new Set();
const listeners = new Set<() => void>();

function subscribeDismissed(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useFontVulnDismissed(id: string): boolean {
	return useSyncExternalStore(subscribeDismissed, () => dismissed.has(id));
}

export function dismissFontVuln(id: string) {
	if (dismissed.has(id)) return;
	dismissed = new Set(dismissed).add(id);
	for (const listener of listeners) listener();
}

let started = false;
const notified = new Set<string>();

export function initServerPacksNotice() {
	if (started) return;
	started = true;
	appApi
		.onServerPacksGuarded(({ id, servers }) => {
			if (servers <= 0 || notified.has(id)) return;
			notified.add(id);
			toast(i18n.t("security.serverPacks.toast", { count: servers }), {
				duration: 10000,
				action: { label: i18n.t("common.details"), onClick: openFontVulnInfo },
			});
		})
		.catch((err: unknown) => console.warn("server-packs-guarded", err));
}
