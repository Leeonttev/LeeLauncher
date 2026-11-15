import type { Instance, LoaderKind } from "@/lib/api";

export const LOADER_NAMES: Record<LoaderKind, string> = {
	vanilla: "Vanilla",
	fabric: "Fabric",
	forge: "Forge",
	neoforge: "NeoForge",
	quilt: "Quilt",
};

export function versionLabel(instance: Instance, withLoaderVersion = false): string {
	const loader = LOADER_NAMES[instance.loader.kind];
	const loaderVersion =
		withLoaderVersion && instance.loader.kind !== "vanilla" && instance.loader.version
			? ` ${instance.loader.version}`
			: "";
	return `${instance.gameVersion} · ${loader}${loaderVersion}`;
}

function parseRelease(id: string): number[] | null {
	const match = /^1\.(\d+)(?:\.(\d+))?$/.exec(id);
	if (!match) return null;
	return [Number(match[1]), Number(match[2] ?? 0)];
}

/** Снапшоты и непонятные id — `null`: точный номер знает ядро после установки. */
export function javaMajorFor(gameVersion: string): number | null {
	const release = parseRelease(gameVersion);
	if (!release) return null;
	const [minor, patch] = release;
	if (minor <= 16) return 8;
	if (minor < 20 || (minor === 20 && patch <= 4)) return 17;
	return 21;
}

export function instanceJavaMajor(instance: Instance): number | null {
	return instance.installed?.javaMajor ?? javaMajorFor(instance.gameVersion);
}

/** И старые `1.21.11`, и новые `26.1.2` (первая часть — год). */
function releaseParts(id: string): number[] | null {
	const match = /^(\d+)\.(\d+)(?:\.(\d+))?$/.exec(id);
	if (!match) return null;
	return [Number(match[1]), Number(match[2]), Number(match[3] ?? 0)];
}

export function compareGameVersions(a: string, b: string): number {
	const ra = releaseParts(a);
	const rb = releaseParts(b);
	if (ra && rb) return rb[0] - ra[0] || rb[1] - ra[1] || rb[2] - ra[2];
	if (ra) return -1;
	if (rb) return 1;
	return b.localeCompare(a);
}
