import { INSTANCE_COVERS, type Instance, type InstanceCover } from "@/lib/api";

export { INSTANCE_COVERS, type InstanceCover };

/** `card` — карточка и мелкие места, `wide` — баннер Главной. */
export type CoverVariant = "card" | "wide";

/** Размер в пикселях арта (1×). Единственное место, где он задан. */
export const COVER_SIZE: Record<CoverVariant, { width: number; height: number }> = {
	card: { width: 100, height: 40 },
	wide: { width: 640, height: 60 },
};

type Layers = { back: string; front: string };

const files = import.meta.glob<string>("../assets/pixel/cover-*.png", {
	eager: true,
	import: "default",
});

function file(name: string): string {
	const url = files[`../assets/pixel/${name}.png`];
	if (!url) throw new Error(`Missing cover image ${name}`);
	return url;
}

const URLS = Object.fromEntries(
	INSTANCE_COVERS.map((scene) => [
		scene,
		{
			card: { back: file(`cover-${scene}-back`), front: file(`cover-${scene}-front`) },
			wide: { back: file(`cover-${scene}-wide-back`), front: file(`cover-${scene}-wide-front`) },
		},
	]),
) as Record<InstanceCover, Record<CoverVariant, Layers>>;

export function coverUrls(scene: InstanceCover, variant: CoverVariant): Layers {
	return URLS[scene][variant];
}

export function isCover(value: unknown): value is InstanceCover {
	return INSTANCE_COVERS.includes(value as InstanceCover);
}

/** Ядро всегда заполняет поле; запасной вариант — на случай неизвестного имени. */
export function coverOf(instance: Pick<Instance, "cover">): InstanceCover {
	return isCover(instance.cover) ? instance.cover : INSTANCE_COVERS[0];
}

/** Наименее занятая, при равенстве — первая по порядку. */
export function leastUsedCover(instances: Pick<Instance, "cover">[]): InstanceCover {
	const used = new Map<InstanceCover, number>();
	for (const instance of instances) {
		const cover = coverOf(instance);
		used.set(cover, (used.get(cover) ?? 0) + 1);
	}
	let best: InstanceCover = INSTANCE_COVERS[0];
	for (const cover of INSTANCE_COVERS) {
		if ((used.get(cover) ?? 0) < (used.get(best) ?? 0)) best = cover;
	}
	return best;
}
