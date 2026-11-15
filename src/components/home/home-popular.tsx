import { ArrowRight } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ProjectCard, ProjectCardSkeleton } from "@/components/catalog/project-card";
import { PixelGrid } from "@/components/pixel-grid";
import { Button } from "@/components/ui/button";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { catalogApi, type ContentType, type Instance, type ProjectHit, type SearchParams } from "@/lib/api";
import { DEFAULT_SOURCE, instanceLoaders, setCatalogContext, setCatalogType } from "@/lib/catalog";
import { findProjectItem, useInstanceContent } from "@/lib/content";
import { navigate } from "@/lib/router";

const MAX_COLUMNS = 4;
const MIN_COLUMN = 240;
const GAP = 16;
const HOUR = 60 * 60 * 1000;
const MONTH = 30 * 24 * HOUR;

const cache = new Map<string, { at: number; hits: ProjectHit[] }>();

type PopularState = { key: string; hits: ProjectHit[] | null; at: number; failed: boolean };

function fromCache(key: string): PopularState {
	const cached = cache.get(key);
	return { key, hits: cached?.hits ?? null, at: cached?.at ?? 0, failed: false };
}

function paramsFor(hero: Instance | null): { type: ContentType; params: SearchParams } {
	if (!hero)
		return { type: "mod", params: { source: DEFAULT_SOURCE, type: "mod", sort: "downloads", limit: 30 } };
	const type: ContentType = hero.loader.kind === "vanilla" ? "resourcepack" : "mod";
	const loaders = instanceLoaders(type, hero);
	return {
		type,
		params: {
			source: DEFAULT_SOURCE,
			type,
			gameVersions: [hero.gameVersion],
			loaders: loaders.length ? loaders : undefined,
			sort: "downloads",
			limit: 30,
			instanceId: hero.id,
		},
	};
}

export function HomePopular({ hero }: { hero: Instance | null }) {
	const { t } = useTranslation();
	const { type, params } = paramsFor(hero);
	const key = JSON.stringify(params);
	const [state, setState] = useState<PopularState>(() => fromCache(key));
	const content = useInstanceContent(hero?.id ?? null);
	const [columns, setColumns] = useState(MAX_COLUMNS);
	const observerRef = useRef<ResizeObserver | null>(null);

	// Сменилась сборка баннера — начинаем заново прямо в рендере.
	if (state.key !== key) setState(fromCache(key));

	useEffect(() => {
		const cached = cache.get(key);
		if (cached && Date.now() - cached.at < HOUR) return;
		let alive = true;
		catalogApi
			.search(JSON.parse(key) as SearchParams)
			.then((result) => {
				const at = Date.now();
				if (!result.offline) cache.set(key, { at, hits: result.hits });
				if (alive) setState({ key, hits: result.hits, at, failed: false });
			})
			.catch((err: unknown) => {
				console.error("catalog_search (Главная)", err);
				if (alive) setState((prev) => (prev.hits ? prev : { key, hits: null, at: 0, failed: true }));
			});
		return () => {
			alive = false;
		};
	}, [key]);

	const measure = useCallback((node: HTMLDivElement | null) => {
		observerRef.current?.disconnect();
		observerRef.current = null;
		if (!node) return;
		const observer = new ResizeObserver(([entry]) => {
			const width = entry.contentRect.width;
			setColumns(Math.max(1, Math.min(MAX_COLUMNS, Math.floor((width + GAP) / (MIN_COLUMN + GAP)))));
		});
		observer.observe(node);
		observerRef.current = observer;
	}, []);

	const loading = state.hits === null && !state.failed;
	const showSkeleton = useDelayedFlag(loading, 150);
	if (state.failed) return null;

	const fits = (hit: ProjectHit) =>
		(!hit.install || hit.install.kind === "none") && !findProjectItem(content, hit.id);
	const candidates = (state.hits ?? []).filter(fits);
	const fresh = candidates.filter((h) => h.updatedAt > 0 && state.at - h.updatedAt < MONTH);
	const rest = candidates.filter((h) => !fresh.includes(h));
	const hits = [...fresh, ...rest].slice(0, columns);
	if (!loading && hits.length === 0) return null;

	const title = !hero
		? t("home.popular.titleAll")
		: type === "resourcepack"
			? t("home.popular.titleResourcepacks", { name: hero.name })
			: t("home.popular.title", { name: hero.name });

	return (
		<section aria-labelledby="home-popular">
			<div className="flex items-center justify-between gap-4">
				<h2
					id="home-popular"
					className="min-w-0 truncate text-xl/[26px] font-semibold tracking-[-0.015em]"
				>
					{title}
				</h2>
				<Button
					variant="ghost"
					size="sm"
					className="shrink-0 font-semibold text-primary-text"
					onClick={() => {
						if (hero) setCatalogContext(hero.id);
						setCatalogType(type);
						navigate("/catalog");
					}}
				>
					{t("home.popular.open")}
					<ArrowRight />
				</Button>
			</div>
			<div ref={measure} className="mt-4">
				{loading ? (
					showSkeleton ? (
						<PixelGrid aria-busy columns={columns}>
							{Array.from({ length: columns }, (_, i) => (
								<ProjectCardSkeleton key={i} />
							))}
						</PixelGrid>
					) : (
						<div className="h-[196px]" />
					)
				) : (
					<PixelGrid columns={columns}>
						{hits.map((hit) => (
							<ProjectCard key={hit.id} hit={hit} type={type} instance={hero} />
						))}
					</PixelGrid>
				)}
			</div>
		</section>
	);
}
