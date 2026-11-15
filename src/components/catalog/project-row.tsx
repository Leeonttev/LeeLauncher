import { InstallButton } from "@/components/catalog/install-button";
import { ModpackInstallButton } from "@/components/catalog/modpack-install-button";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { ProjectMeta } from "@/components/catalog/project-meta";
import { linkFocusRing, stretchedLink } from "@/components/instances/instance-link";
import { Skeleton } from "@/components/ui/skeleton";
import type { Instance, ProjectHit } from "@/lib/api";
import { openProject, SOURCE_NAMES, type CatalogType } from "@/lib/catalog";
import { projectRefOf } from "@/lib/content-install";
import { compareGameVersions, LOADER_NAMES } from "@/lib/loaders";
import { cn } from "@/lib/utils";

/** Вся строка — растянутая ссылка на страницу проекта. */
export function ProjectRow({
	hit,
	type,
	instance,
	showSource = false,
	onPickInstance,
}: {
	hit: ProjectHit;
	type: CatalogType;
	instance: Instance | null;
	showSource?: boolean;
	onPickInstance?: (id: string) => void;
}) {
	const open = () => openProject(hit, type, { alsoOn: hit.alsoOn });
	const sources = showSource
		? [hit.source, ...hit.alsoOn.map((twin) => twin.source)].map((id) => SOURCE_NAMES[id]).join(" · ")
		: null;
	// У склеенной строки место в сортировке могли решить загрузки двойника, поэтому большее из чисел.
	const meta = { ...hit, downloads: Math.max(hit.downloads, ...hit.alsoOn.map((twin) => twin.downloads)) };
	return (
		<div
			className={cn(
				"relative -mx-3 flex items-start gap-4 rounded-lg border border-transparent p-3 transition-colors duration-(--duration-base) ease-out hover:bg-accent",
				linkFocusRing,
			)}
		>
			<ProjectIcon url={hit.iconUrl} type={hit.type} size={64} />
			<div className="min-w-0 flex-1">
				<div className="flex min-w-0 items-baseline gap-1.5">
					<button
						type="button"
						data-link
						className={cn("min-w-0 truncate text-sm/5 font-semibold", stretchedLink)}
						onClick={open}
					>
						{hit.title}
					</button>
					{hit.author && (
						<span className="max-w-48 shrink-0 truncate text-[13px]/[18px] text-muted-foreground">
							· {hit.author}
						</span>
					)}
				</div>
				<p className="mt-0.5 line-clamp-2 text-sm/5 text-muted-foreground">{hit.description}</p>
				{type === "modpack" ? (
					<ProjectMeta
						hit={meta}
						type={type}
						categories={2}
						lead={modpackTarget(hit)}
						sources={sources}
						className="mt-2"
					/>
				) : (
					<ProjectMeta hit={meta} type={type} sources={sources} className="mt-2" />
				)}
			</div>
			{type === "modpack" ? (
				<ModpackInstallButton
					className="pt-0.5"
					project={{ source: hit.source, id: hit.id, title: hit.title, iconUrl: hit.iconUrl }}
				/>
			) : (
				<InstallButton
					className="pt-0.5"
					project={projectRefOf(hit, type)}
					instance={instance}
					onPickInstance={onPickInstance}
					onIncompatibleClick={open}
				/>
			)}
		</div>
	);
}

const RELEASE = /^\d+\.\d+(\.\d+)?$/;

/** Поиск не отдаёт новейшую версию модпака, берём новейший релиз игры и загрузчик из списков проекта. */
function modpackTarget(hit: ProjectHit): string | null {
	const releases = hit.gameVersions.filter((v) => RELEASE.test(v));
	const game = [...(releases.length ? releases : hit.gameVersions)].sort(compareGameVersions)[0];
	const loader = hit.loaders.find(
		(l): l is keyof typeof LOADER_NAMES => l in LOADER_NAMES && l !== "vanilla",
	);
	const parts = [game, loader ? LOADER_NAMES[loader] : null].filter(Boolean);
	return parts.length ? parts.join(" · ") : null;
}

export function ProjectRowSkeleton() {
	return (
		<div aria-hidden className="flex items-start gap-4 py-3">
			<Skeleton className="size-16 shrink-0 rounded-lg" />
			<div className="min-w-0 flex-1">
				<Skeleton className="h-4 w-48" />
				<Skeleton className="mt-2 h-3.5 w-[90%]" />
				<Skeleton className="mt-1.5 h-3.5 w-[60%]" />
				<Skeleton className="mt-3 h-3 w-60" />
			</div>
			<Skeleton className="h-8 w-29 shrink-0 rounded-lg" />
		</div>
	);
}
