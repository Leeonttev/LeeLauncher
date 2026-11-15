import { useEffect } from "react";
import type { GameVersion, Instance } from "@/lib/api";
import { revalidateVersions, useGameVersions } from "@/lib/versions";

export type HomeVersion = { version: GameVersion; kind: "release" | "snapshot" };

/** `hidden` — ошибка без кэша или версий нет. */
export type HomeVersionsState =
	| { status: "loading"; items: [] }
	| { status: "ready"; items: HomeVersion[] }
	| { status: "hidden"; items: [] };

function latestVersions(
	versions: GameVersion[],
	latest: { release: string; snapshot: string },
): HomeVersion[] {
	const release = versions.find((v) => v.id === latest.release);
	const snapshot = versions.find((v) => v.id === latest.snapshot);
	const items: HomeVersion[] = [];
	if (release) items.push({ version: release, kind: "release" });
	// После выхода релиза Mojang ставит его же в `latest.snapshot` — тогда строка одна.
	const snapshotNewer =
		snapshot &&
		latest.snapshot !== latest.release &&
		(!release || Date.parse(snapshot.releaseTime) > Date.parse(release.releaseTime));
	if (snapshot && snapshotNewer) items.push({ version: snapshot, kind: "snapshot" });
	return items;
}

export function useHomeVersions(): HomeVersionsState {
	// Ошибки забываются, ответ из старого кэша ядра обновляется.
	useEffect(() => revalidateVersions(), []);
	const resource = useGameVersions();
	if (!resource || resource.status === "loading") return { status: "loading", items: [] };
	if (resource.status === "error") return { status: "hidden", items: [] };
	// Ответ без списка или без `latest` прячет блок, а не роняет Главную.
	const { versions, latest } = resource.value ?? {};
	if (!Array.isArray(versions) || !latest) return { status: "hidden", items: [] };
	const items = latestVersions(versions, latest);
	return items.length ? { status: "ready", items } : { status: "hidden", items: [] };
}

export function hasPlayed(instances: Instance[]) {
	return instances.some((i) => i.playtime > 0 || i.lastPlayedAt);
}
