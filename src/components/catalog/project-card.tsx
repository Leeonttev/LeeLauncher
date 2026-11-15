import { Download } from "lucide-react";
import { useTranslation } from "react-i18next";
import { InstallButton } from "@/components/catalog/install-button";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { linkFocusRing, stretchedLink } from "@/components/instances/instance-link";
import { Skeleton } from "@/components/ui/skeleton";
import type { ContentType, Instance, ProjectHit } from "@/lib/api";
import { openProject } from "@/lib/catalog";
import { projectRefOf } from "@/lib/content-install";
import { formatCompact, formatNumber } from "@/lib/format";
import { cn } from "@/lib/utils";

export function ProjectCard({
	hit,
	type,
	instance,
}: {
	hit: ProjectHit;
	type: ContentType;
	instance: Instance | null;
}) {
	const { t } = useTranslation();
	const open = () => openProject(hit, type);
	return (
		<div
			className={cn(
				"@container/card relative flex h-full flex-col rounded-xl border border-border bg-background p-5 hover:border-border-hover hover:bg-accent",
				linkFocusRing,
			)}
		>
			<div className="flex h-12 min-w-0 items-center gap-3">
				<ProjectIcon url={hit.iconUrl} type={hit.type} size={48} />
				<div className="min-w-0 flex-1">
					<button
						type="button"
						data-link
						className={cn("block max-w-full truncate text-sm/5 font-semibold", stretchedLink)}
						onClick={open}
					>
						{hit.title}
					</button>
					{hit.author && (
						<p className="truncate text-[13px]/[18px] text-muted-foreground">{hit.author}</p>
					)}
				</div>
			</div>
			{/* Ровно три строки места, чтобы низ карточек в ряду был на одной линии. */}
			<p className="mt-3 line-clamp-3 h-15 shrink-0 text-sm/5 text-muted-foreground">
				{hit.description}
			</p>
			<div className="mt-auto flex items-center justify-between gap-2 pt-4">
				<span className="flex min-w-0 items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
					<Download />
					<span aria-hidden className="min-w-0 truncate tabular-nums">
						{formatCompact(hit.downloads)}
					</span>
					<span className="sr-only">
						{t("catalog.meta.downloadsAria", { count: formatNumber(hit.downloads) })}
					</span>
				</span>
				<InstallButton
					compact
					project={projectRefOf(hit, type)}
					instance={instance}
					onIncompatibleClick={open}
				/>
			</div>
		</div>
	);
}

export function ProjectCardSkeleton() {
	return (
		<div aria-hidden className="relative rounded-xl border border-border p-5">
			<div className="flex items-center gap-3">
				<Skeleton className="size-12 rounded-lg" />
				<div className="flex-1">
					<Skeleton className="h-4 w-32" />
					<Skeleton className="mt-2 h-3 w-20" />
				</div>
			</div>
			<Skeleton className="mt-4 h-3.5 w-full" />
			<Skeleton className="mt-2 h-3.5 w-[80%]" />
			<div className="mt-6 flex items-center justify-between">
				<Skeleton className="h-3 w-16" />
				<Skeleton className="h-8 w-29 rounded-lg" />
			</div>
		</div>
	);
}
