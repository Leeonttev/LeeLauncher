import { useAccounts } from "@/lib/accounts";
import type { GameAccountInfo, Instance } from "@/lib/api";
import { formatDuration, formatWhenCapitalized } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { gameAccountOf, showGameAccount, useNow, type InstancesStore } from "@/lib/instances";

/** Аккаунт идущей игры, если ник нужно показать; иначе `null`. */
export function useShownGameAccount(instanceId: string, store: InstancesStore): GameAccountInfo | null {
	const accounts = useAccounts();
	const account = gameAccountOf(instanceId, store);
	return showGameAccount(account, accounts) ? account : null;
}

/** То же для уже закрытой игры (аккаунт из события выхода). */
export function useShownAccount(account: GameAccountInfo | null): GameAccountInfo | null {
	const accounts = useAccounts();
	return showGameAccount(account, accounts) ? account : null;
}

export function useSessionSeconds(instance: Instance, store: InstancesStore): number | null {
	const startedAt = store.processes[instance.id]?.startedAt ?? null;
	const now = useNow(startedAt !== null);
	if (startedAt === null) return null;
	return Math.max(0, Math.floor((now - startedAt) / 1000));
}

/** Вызывать вместе с `useTranslation()`, чтобы текст сменился с языком. */
export function playedText(instance: Instance, hero = false): string {
	if (!instance.lastPlayedAt) return i18n.t("time.never");
	const when = formatWhenCapitalized(instance.lastPlayedAt);
	if (instance.playtime < 60) return when;
	const total = formatDuration(instance.playtime);
	return i18n.t(hero ? "home.heroMeta" : "instances.card.lastPlayed", { when, total });
}
