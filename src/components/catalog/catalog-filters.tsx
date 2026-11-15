import { RotateCcw } from "lucide-react";
import { useEffect, useId, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { GameVersionPicker } from "@/components/catalog/game-version-picker";
import { SourceSwitcher } from "@/components/catalog/source-switcher";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { CategoryTag, ContentSource, Instance } from "@/lib/api";
import {
	categoryLabel,
	effectiveFilters,
	LOADER_FILTER_NAMES,
	loadCategories,
	loaderOptions,
	setCatalogFilters,
	setCompatibleOnly,
	useCategories,
	useSources,
	viewInfo,
	type CatalogState,
} from "@/lib/catalog";
import { errorText } from "@/lib/errors";
import { cn } from "@/lib/utils";

const ANY = "__any__";
/** Сколько категорий видно до «Показать все». */
const VISIBLE_CATEGORIES = 8;

export function CatalogFilters({ state, instance }: { state: CatalogState; instance: Instance | null }) {
	const { t } = useTranslation();
	const ids = useId();
	const info = viewInfo(state.source, useSources());
	const { locked, gameVersions, loaders } = effectiveFilters(state, instance);
	const options = loaderOptions(state.type, info);
	const compatibleId = `${ids}-compatible`;

	const lockHint = (node: ReactNode) =>
		locked ? (
			<Tooltip>
				<TooltipTrigger asChild>
					<div className="opacity-60">{node}</div>
				</TooltipTrigger>
				<TooltipContent className="max-w-60">{t("catalog.filter.lockedHint")}</TooltipContent>
			</Tooltip>
		) : (
			node
		);

	return (
		<aside aria-label={t("catalog.filter.label")} className="flex w-58 shrink-0 flex-col gap-5">
			<div className="border-b pb-5">
				<p id={`${ids}-source`} className="mb-2 text-[13px]/[18px] font-semibold">
					{t("catalog.source.label")}
				</p>
				<SourceSwitcher value={state.source} labelledBy={`${ids}-source`} />
			</div>
			{instance && (
				<div className="flex items-start gap-3">
					<label htmlFor={compatibleId} className="min-w-0 flex-1">
						<span className="block text-[13px]/[18px] font-semibold">
							{t("catalog.filter.compatible")}
						</span>
						<span className="block truncate text-[13px]/[18px] text-muted-foreground">
							{t("catalog.filter.compatibleHint", { name: instance.name })}
						</span>
					</label>
					<Switch
						id={compatibleId}
						checked={state.compatibleOnly}
						onCheckedChange={setCompatibleOnly}
						className="mt-0.5"
					/>
				</div>
			)}
			<div>
				<p id={`${ids}-version`} className="mb-2 text-[13px]/[18px] font-semibold">
					{t("catalog.filter.gameVersion")}
				</p>
				{lockHint(
					<GameVersionPicker
						labelledBy={`${ids}-version`}
						value={locked ? (gameVersions[0] ?? null) : state.filters.gameVersion}
						onChange={(gameVersion) => setCatalogFilters({ gameVersion })}
						disabled={locked}
					/>,
				)}
			</div>
			{options && (
				<div>
					<p id={`${ids}-loader`} className="mb-2 text-[13px]/[18px] font-semibold">
						{t("catalog.filter.loader")}
					</p>
					{lockHint(
						<Select
							value={(locked ? loaders[0] : state.filters.loader) ?? ANY}
							onValueChange={(value) =>
								setCatalogFilters({ loader: value === ANY ? null : value })
							}
							disabled={locked}
						>
							<SelectTrigger aria-labelledby={`${ids}-loader`} className="h-9 w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent position="popper">
								<SelectItem value={ANY}>{t("catalog.filter.anyLoader")}</SelectItem>
								{options.map((loader) => (
									<SelectItem key={loader} value={loader}>
										{LOADER_FILTER_NAMES[loader] ?? loader}
									</SelectItem>
								))}
							</SelectContent>
						</Select>,
					)}
				</div>
			)}
			{state.source === "all" ? (
				<div>
					<p className="mb-1 text-[13px]/[18px] font-semibold">{t("catalog.filter.categories")}</p>
					<p className="text-[13px]/[18px] text-muted-foreground">
						{t("catalog.filter.categoriesAll")}
					</p>
				</div>
			) : (
				<Categories state={state} source={state.source} maxCategories={info.maxCategories} />
			)}
		</aside>
	);
}

function orderCategories(items: CategoryTag[]) {
	const rank = (tag: CategoryTag) => (tag.header === "categories" ? 0 : 1);
	return [...items].sort((a, b) => rank(a) - rank(b));
}

type CategoryRow = { tag: CategoryTag; child: boolean };

function categoryRows(items: CategoryTag[]): CategoryRow[] {
	const slugs = new Set(items.map((tag) => tag.slug));
	const roots = orderCategories(items.filter((tag) => !tag.parent || !slugs.has(tag.parent)));
	const rows: CategoryRow[] = [];
	for (const root of roots) {
		rows.push({ tag: root, child: false });
		for (const child of items.filter((tag) => tag.parent === root.slug))
			rows.push({ tag: child, child: true });
	}
	return rows;
}

function Categories({
	state,
	source,
	maxCategories,
}: {
	state: CatalogState;
	source: ContentSource;
	maxCategories: number | null;
}) {
	const { t } = useTranslation();
	const ids = useId();
	const entry = useCategories(state.type, source);
	const [expanded, setExpanded] = useState(false);
	const selected = state.filters.categories;
	const full = maxCategories !== null && selected.length >= maxCategories;

	useEffect(() => {
		loadCategories(state.type, source);
	}, [state.type, source]);

	let body: ReactNode;
	if (!entry || entry.status === "loading") {
		body = (
			// Столько же строк, сколько видно в готовом списке: появление категорий не сдвигает колонку.
			<div aria-busy className="flex flex-col">
				{Array.from({ length: VISIBLE_CATEGORIES }, (_, i) => (
					<div key={i} className="flex h-8 items-center gap-2.5">
						<Skeleton className="size-4 rounded-md" />
						<Skeleton className="h-3.5 w-28" />
					</div>
				))}
			</div>
		);
	} else if (entry.status === "error") {
		body = (
			<div className="flex flex-col items-start gap-2 text-[13px]/[18px] text-muted-foreground">
				<p>
					{t("catalog.filter.categoriesFailed")} · {errorText(entry.error)}
				</p>
				<Button variant="outline" size="sm" onClick={() => loadCategories(state.type, source, true)}>
					<RotateCcw />
					{t("common.retry")}
				</Button>
			</div>
		);
	} else {
		const rows = categoryRows(entry.items);
		const visible = expanded
			? rows
			: rows.filter((row, i) => i < VISIBLE_CATEGORIES || selected.includes(row.tag.slug));
		body = (
			<>
				<ul className="flex flex-col">
					{visible.map(({ tag, child }) => {
						const id = `${ids}-${tag.slug}`;
						const checked = selected.includes(tag.slug);
						// У источника бывает предел выбранных категорий: остальные флажки неактивны.
						const blocked = full && !checked;
						const row = (
							<label
								htmlFor={id}
								className={cn(
									"flex h-8 cursor-pointer items-center gap-2.5 text-sm",
									child && "pl-6",
									blocked && "cursor-default",
								)}
							>
								<Checkbox
									id={id}
									checked={checked}
									aria-disabled={blocked || undefined}
									className="aria-disabled:opacity-50"
									onCheckedChange={(value) => {
										if (value === true && blocked) return;
										setCatalogFilters({
											categories:
												value === true
													? [...selected, tag.slug]
													: selected.filter((slug) => slug !== tag.slug),
										});
									}}
								/>
								<span className={cn("min-w-0 flex-1 truncate", blocked && "opacity-60")}>
									{categoryLabel(tag.slug, source, state.type)}
								</span>
							</label>
						);
						return (
							<li key={tag.slug}>
								{blocked ? (
									<Tooltip>
										<TooltipTrigger asChild>{row}</TooltipTrigger>
										<TooltipContent>
											{t("catalog.filter.categoriesLimit", {
												count: maxCategories ?? 0,
											})}
										</TooltipContent>
									</Tooltip>
								) : (
									row
								)}
							</li>
						);
					})}
				</ul>
				{rows.length > VISIBLE_CATEGORIES && (
					<button
						type="button"
						className="relative mt-1 -ml-1 self-start rounded-md border border-transparent px-1 text-[13px]/[18px] font-semibold text-primary-text hover:underline"
						onClick={() => setExpanded((v) => !v)}
					>
						{expanded
							? t("catalog.filter.showLess")
							: t("catalog.filter.showAll", { count: rows.length })}
					</button>
				)}
			</>
		);
	}

	return (
		<div className="flex flex-col">
			<div className="mb-1 flex h-6 items-center justify-between gap-2">
				<p className="text-[13px]/[18px] font-semibold">{t("catalog.filter.categories")}</p>
				{selected.length > 0 && (
					<button
						type="button"
						className="relative rounded-md border border-transparent px-1 text-[13px]/[18px] font-semibold text-primary-text hover:underline"
						onClick={() => setCatalogFilters({ categories: [] })}
					>
						{t("catalog.filter.reset")}
					</button>
				)}
			</div>
			{body}
		</div>
	);
}
