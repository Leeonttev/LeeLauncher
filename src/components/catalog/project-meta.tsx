import { Clock, Download, Heart } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ProjectHit } from "@/lib/api";
import { categoryLabel, useCategoryCache, type CatalogType } from "@/lib/catalog";
import { formatAgo, formatCompact, formatNumber } from "@/lib/format";
import { cn } from "@/lib/utils";

export function CategoryChip({ children, strong }: { children: string; strong?: boolean }) {
	return (
		<span
			className={cn(
				"inline-flex h-[22px] shrink-0 items-center rounded-md bg-muted px-[7px] text-[13px] font-medium",
				strong ? "text-foreground" : "text-muted-foreground",
			)}
		>
			{children}
		</span>
	);
}

export function ProjectMeta({
	hit,
	categories = 3,
	updatedPrefix = false,
	lead,
	sources,
	type,
	className,
}: {
	sources?: string | null;
	lead?: string | null;
	hit: Pick<ProjectHit, "source" | "type" | "downloads" | "follows" | "updatedAt" | "categories">;
	/** Slug категории уникален только внутри типа. */
	type?: CatalogType;
	categories?: number;
	updatedPrefix?: boolean;
	className?: string;
}) {
	const { t } = useTranslation();
	// Не у всех источников названия категорий совпадают со slug: подписи обновятся после загрузки категорий.
	useCategoryCache();
	const shown = hit.categories.slice(0, categories);
	const rest = hit.categories.length - shown.length;
	const when = hit.updatedAt > 0 ? formatAgo(hit.updatedAt) : null;
	return (
		<div
			className={cn(
				"flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5 text-[13px]/[18px] text-muted-foreground",
				className,
			)}
		>
			{sources && <span className="text-foreground">{sources}</span>}
			<span className="flex items-center gap-1.5">
				<Download />
				<span aria-hidden className="tabular-nums">
					{formatCompact(hit.downloads)}
				</span>
				<span className="sr-only">
					{t("catalog.meta.downloadsAria", { count: formatNumber(hit.downloads) })}
				</span>
			</span>
			{hit.follows !== null && (
				<span className="flex items-center gap-1.5">
					<Heart />
					<span aria-hidden className="tabular-nums">
						{formatCompact(hit.follows)}
					</span>
					<span className="sr-only">
						{t("catalog.meta.followsAria", { count: formatNumber(hit.follows) })}
					</span>
				</span>
			)}
			{when && (
				<span className="flex items-center gap-1.5">
					<Clock />
					{updatedPrefix ? t("catalog.meta.updated", { when }) : when}
				</span>
			)}
			{(shown.length > 0 || lead) && (
				<span className="flex flex-wrap items-center gap-1.5">
					{lead && <CategoryChip strong>{lead}</CategoryChip>}
					{shown.map((slug) => (
						<CategoryChip key={slug}>
							{categoryLabel(slug, hit.source, type ?? hit.type)}
						</CategoryChip>
					))}
					{rest > 0 && <CategoryChip>{t("catalog.meta.more", { count: rest })}</CategoryChip>}
				</span>
			)}
		</div>
	);
}
