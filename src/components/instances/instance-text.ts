import type { Instance } from "@/lib/api";
import { formatDuration, formatWhenCapitalized } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { useNow, type InstancesStore } from "@/lib/instances";

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
