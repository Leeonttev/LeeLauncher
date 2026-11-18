import {
	ArrowUpDown,
	Braces,
	CircleAlert,
	Compass,
	FolderOpen,
	Globe,
	type LucideIcon,
	OctagonX,
	Package,
	Palette,
	Puzzle,
	RotateCcw,
	Search,
	Sparkles,
	WifiOff,
	X,
} from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CatalogFilters } from "@/components/catalog/catalog-filters";
import { InstanceContextPicker } from "@/components/catalog/instance-context-picker";
import { ProjectRow, ProjectRowSkeleton } from "@/components/catalog/project-row";
import { SlowNotice } from "@/components/catalog/slow-notice";
import { CenteredState } from "@/components/centered-state";
import { ScrollContainerContext, useScrollContainer } from "@/components/layout/scroll-container";
import { PageTabs } from "@/components/page-tabs";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { TabsContent } from "@/components/ui/tabs";
import { overlayOpen, typingTarget } from "@/hooks/use-create-shortcut";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import type { ContentSource, Instance, ProjectHit, SearchParams } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import {
	applyCatalogQuery,
	catalogTypesOf,
	contextInstance,
	effectiveSort,
	getCatalog,
	hasMoreResults,
	hitKey,
	isUnreachable,
	loadMoreCatalog,
	MAX_OFFSET,
	otherSource,
	PAGE_SIZE,
	resetCatalogFilters,
	saveCatalogScroll,
	searchCatalog,
	searchParams,
	setCatalogContext,
	setCatalogQuery,
	setCatalogSort,
	setCatalogSource,
	setCatalogType,
	setCompatibleOnly,
	sortsOf,
	SOURCE_NAMES,
	useCatalog,
	useSources,
	viewInfo,
	type CatalogState,
	type CatalogType,
} from "@/lib/catalog";
import { errorText } from "@/lib/errors";
import { formatList, formatNumber } from "@/lib/format";
import { useInstances } from "@/lib/instances";
import { pickModpackFile, useModpackFileShortcut } from "@/lib/modpacks";
import { cn } from "@/lib/utils";

const TYPE_ICONS: Record<CatalogType, LucideIcon> = {
	mod: Puzzle,
	resourcepack: Palette,
	shader: Sparkles,
	datapack: Braces,
	world: Globe,
	modpack: Package,
};
/** С этого числа строк список виртуальный. */
const VIRTUAL_FROM = 200;
const SEARCH_DELAY = 300;
const easeOut = [0.22, 1, 0.36, 1] as const;

export function CatalogPage() {
	const { t } = useTranslation();
	const state = useCatalog();
	const store = useInstances();
	const sources = useSources();
	const info = viewInfo(state.source, sources);
	const searchRef = useRef<HTMLInputElement>(null);
	// Первый запрос — когда прочитаны и сборки, и возможности источников: выбранный мог оказаться выключен.
	const instancesReady =
		(store.status === "ready" || store.status === "error") && sources.status !== "loading";
	const modpacks = state.type === "modpack";
	const instance = modpacks ? null : contextInstance(store.instances, state.contextId);
	const params = searchParams(state, instance);
	const key = JSON.stringify(params);

	// Иначе ушёл бы запрос без сборки и сразу второй.
	useEffect(() => {
		if (instancesReady) void searchCatalog(JSON.parse(key) as SearchParams);
	}, [key, instancesReady]);

	useEffect(() => {
		const timer = setTimeout(() => applyCatalogQuery(state.query), SEARCH_DELAY);
		return () => clearTimeout(timer);
	}, [state.query]);

	// Возврат со страницы проекта — туда же, где были.
	const listRef = useRef<HTMLDivElement>(null);
	useLayoutEffect(() => {
		const list = listRef.current;
		if (list) list.scrollTop = getCatalog().scrollTop;
	}, []);

	// Новый запрос — к началу списка; вернулись на прежний источник — туда, где были.
	const hitsKey = state.results.hitsKey;
	const firstHitsKey = useRef(hitsKey);
	const lastSource = useRef(state.source);
	useEffect(() => {
		if (state.source !== lastSource.current) {
			lastSource.current = state.source;
			firstHitsKey.current = hitsKey;
			if (listRef.current) listRef.current.scrollTop = getCatalog().scrollTop;
			return;
		}
		if (hitsKey === firstHitsKey.current) return;
		firstHitsKey.current = hitsKey;
		listRef.current?.scrollTo({ top: 0 });
	}, [hitsKey, state.source]);

	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			const ctrlF = event.code === "KeyF" && event.ctrlKey && !event.altKey && !event.shiftKey;
			const slash =
				(event.code === "Slash" || event.code === "NumpadDivide") &&
				!event.ctrlKey &&
				!event.altKey &&
				!event.shiftKey;
			if (!ctrlF && !slash) return;
			if (typingTarget(event.target) || overlayOpen()) return;
			event.preventDefault();
			searchRef.current?.focus();
			searchRef.current?.select();
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);

	useModpackFileShortcut(true);

	const pickInstance = (id: string) => setCatalogContext(id, { persist: true, flash: true });

	return (
		<div className="flex min-h-0 flex-1 flex-col">
			<div className="flex items-start justify-between gap-6">
				<div className="min-w-0">
					<h1 className="text-[28px]/[34px] font-semibold tracking-[-0.025em]">
						{t("catalog.title")}
					</h1>
					<p className="mt-1 text-sm text-muted-foreground">
						{modpacks ? t("catalog.subtitleModpacks") : t("catalog.subtitle")}
					</p>
				</div>
				<div className="grid shrink-0">
					<AnimatePresence mode="popLayout" initial={false}>
						<motion.div
							key={modpacks ? "file" : "context"}
							initial={{ opacity: 0 }}
							animate={{ opacity: 1 }}
							exit={{ opacity: 0 }}
							transition={{ duration: 0.12 }}
							className="flex justify-end"
						>
							{modpacks ? (
								<Button
									variant="outline"
									aria-keyshortcuts="Control+O"
									onClick={(e) => void pickModpackFile(e.currentTarget)}
								>
									<FolderOpen />
									{t("catalog.fromFile")}
								</Button>
							) : (
								<InstanceContextPicker
									instances={store.instances}
									value={contextInstance(store.instances, state.contextId)}
									flash={state.contextFlash}
									onChange={(id) => setCatalogContext(id, { persist: true })}
								/>
							)}
						</motion.div>
					</AnimatePresence>
				</div>
			</div>
			<PageTabs
				className="-mx-8 mt-5 min-h-0 flex-1"
				listClassName="px-8"
				label={t("catalog.typesLabel")}
				layoutId="catalog-type"
				value={state.type}
				onValueChange={setCatalogType}
				tabs={catalogTypesOf(info).map((type) => ({
					value: type,
					label: t(`catalog.type.${type}`),
					icon: TYPE_ICONS[type],
				}))}
			>
				{/* Фильтры и список прокручиваются независимо, холст оболочки — нет. */}
				<TabsContent value={state.type} className="flex min-h-0 flex-1">
					<div className="shrink-0 overflow-y-auto border-r py-6 pr-6 pl-8">
						<CatalogFilters state={state} instance={instance} />
					</div>
					<div className="flex min-w-0 flex-1 flex-col">
						<div className="flex items-center gap-3 px-8 pt-6 pb-1">
							<Input
								ref={searchRef}
								type="search"
								value={state.query}
								placeholder={t(`catalog.search.${state.type}`)}
								aria-label={t(`catalog.search.${state.type}`)}
								aria-keyshortcuts="Control+F /"
								spellCheck={false}
								autoComplete="off"
								className="h-9 min-w-0 flex-1"
								inputClassName="px-2 [&::-webkit-search-cancel-button]:hidden"
								start={
									<Search className="pointer-events-none ml-2.5 text-muted-foreground" />
								}
								end={
									state.query && (
										<Button
											variant="ghost"
											size="icon-xs"
											aria-label={t("common.clear")}
											className="mr-0.5 size-7 text-muted-foreground focus-visible:bg-accent"
											onClick={() => {
												setCatalogQuery("");
												applyCatalogQuery("");
												searchRef.current?.focus();
											}}
										>
											<X />
										</Button>
									)
								}
								onChange={(e) => setCatalogQuery(e.target.value)}
								onKeyDown={(e) => {
									if (e.key === "Enter") applyCatalogQuery();
									if (e.key === "Escape" && state.query) {
										e.preventDefault();
										setCatalogQuery("");
										applyCatalogQuery("");
									}
								}}
							/>
							<Select
								value={effectiveSort(state, info)}
								onValueChange={(value) => setCatalogSort(value as typeof state.sort)}
							>
								<SelectTrigger
									className="h-9 w-50 shrink-0"
									aria-label={t("catalog.sort.label")}
								>
									<ArrowUpDown className="text-muted-foreground" />
									<SelectValue />
								</SelectTrigger>
								<SelectContent position="popper" align="end">
									{sortsOf(info).map((sort) => (
										<SelectItem key={sort} value={sort}>
											{t(`catalog.sort.${sort}`)}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<ScrollContainerContext value={listRef}>
							<div
								ref={listRef}
								className="min-h-0 flex-1 overflow-y-auto px-8 pb-10"
								onScroll={(e) => saveCatalogScroll(e.currentTarget.scrollTop)}
							>
								<Results
									state={state}
									instance={instance}
									params={params}
									onPickInstance={pickInstance}
								/>
							</div>
						</ScrollContainerContext>
					</div>
				</TabsContent>
			</PageTabs>
		</div>
	);
}

function Results({
	state,
	instance,
	params,
	onPickInstance,
}: {
	state: CatalogState;
	instance: Instance | null;
	params: SearchParams;
	onPickInstance: (id: string) => void;
}) {
	const { t } = useTranslation();
	const results = state.results;
	const firstLoad = results.status === "loading" && results.hitsKey === null;
	const showSkeleton = useDelayedFlag(firstLoad || results.status === "idle", 150);
	const items = useSources().items;
	const single = state.source === "all" ? null : state.source;
	const source = single ? SOURCE_NAMES[single] : "";
	// В «Все» другой источник и так в списке.
	const other = single ? otherSource(single, items) : null;
	const otherButton = (label: "catalog.error.openSource" | "catalog.empty.otherSource") =>
		other ? (
			<Button variant="outline" onClick={() => setCatalogSource(other)}>
				{label === "catalog.empty.otherSource" ? <Search /> : <Compass />}
				{t(label, { source: SOURCE_NAMES[other] })}
			</Button>
		) : null;

	if (results.status === "idle" || firstLoad) {
		return (
			<>
				<SlowNotice loading={firstLoad} source={state.source} switchable className="mt-4" />
				{showSkeleton && (
					<div aria-busy className="mt-7">
						{Array.from({ length: 6 }, (_, i) => (
							<ProjectRowSkeleton key={i} />
						))}
					</div>
				)}
			</>
		);
	}

	if (results.status === "error" && results.error) {
		const error = results.error;
		// Один источник: сеть или сбой — скорее всего, сайт не открывается именно отсюда.
		const unreachable = single !== null && isUnreachable(error);
		const offline = !unreachable && error.code === "network";
		const sourceDown = single !== null && error.code === "rate_limited";
		const disabled = single !== null && error.code === "source_disabled";
		const retry = (
			<Button variant="outline" onClick={() => void searchCatalog(params, true)}>
				<RotateCcw />
				{t("common.retry")}
			</Button>
		);
		return (
			<CenteredState
				icon={offline ? WifiOff : OctagonX}
				tone="destructive"
				heading="h2"
				fill={false}
				className="py-16"
				title={
					unreachable
						? t("catalog.error.unreachableTitle", { source })
						: offline
							? t("catalog.error.offlineTitle")
							: disabled
								? t("catalog.error.disabledTitle", { source })
								: sourceDown
									? t("catalog.error.sourceTitle", { source })
									: t("catalog.error.title")
				}
				description={
					unreachable
						? t("catalog.error.unreachableDescription")
						: offline
							? t("catalog.error.offlineDescription")
							: disabled && error.details === "denied"
								? t("catalog.error.deniedDescription", { source })
								: errorText(error)
				}
				action={
					unreachable || sourceDown || disabled ? (
						<div className="flex flex-wrap justify-center gap-2">
							{retry}
							{otherButton("catalog.error.openSource")}
						</div>
					) : (
						retry
					)
				}
			/>
		);
	}

	const refreshing = results.status === "loading";
	const locked = Boolean(instance && state.compatibleOnly);
	const filtered =
		Boolean(state.appliedQuery) ||
		state.filters.categories.length > 0 ||
		Boolean(state.filters.gameVersion) ||
		Boolean(state.filters.loader);

	return (
		<div
			aria-busy={refreshing || undefined}
			className={cn("transition-opacity duration-(--duration-base)", refreshing && "opacity-60")}
		>
			<SlowNotice loading={refreshing} source={state.source} switchable className="mt-3" />
			<p className="mt-3 text-[13px]/[18px] text-muted-foreground">
				{/* В «Все» `total` считает и дубли: только оценка. */}
				{single
					? t("catalog.found", { count: formatNumber(results.total) })
					: t("catalog.foundApprox", { count: formatNumber(results.total) })}
			</p>
			{results.partial.length > 0 && <PartialWarning state={state} params={params} />}
			{results.offline && (
				<p className="mt-1 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
					<WifiOff />
					{t("catalog.offlineCached")}
				</p>
			)}
			{results.hits.length === 0 ? (
				<CenteredState
					icon={Search}
					heading="h2"
					fill={false}
					className="py-16"
					title={t("catalog.empty.title")}
					description={t("catalog.empty.description")}
					action={
						(locked || filtered || other) && (
							<div className="flex flex-wrap justify-center gap-2">
								{locked && !filtered ? (
									<Button variant="outline" onClick={() => setCompatibleOnly(false)}>
										{t("catalog.empty.showAll")}
									</Button>
								) : filtered ? (
									<Button variant="outline" onClick={resetCatalogFilters}>
										{t("catalog.empty.reset")}
									</Button>
								) : null}
								{otherButton("catalog.empty.otherSource")}
							</div>
						)
					}
				/>
			) : (
				<>
					<div id="catalog-results" className="mt-3">
						{results.hits.length > VIRTUAL_FROM ? (
							<VirtualRows
								hits={results.hits}
								type={state.type}
								instance={instance}
								showSource={!single}
								onPickInstance={onPickInstance}
							/>
						) : (
							<PlainRows
								key={results.hitsKey}
								hits={results.hits}
								type={state.type}
								instance={instance}
								showSource={!single}
								onPickInstance={onPickInstance}
							/>
						)}
					</div>
					<ListFooter state={state} params={params} />
				</>
			)}
		</div>
	);
}

type RowsProps = {
	hits: ProjectHit[];
	type: CatalogType;
	instance: Instance | null;
	showSource: boolean;
	onPickInstance: (id: string) => void;
};

function PartialWarning({ state, params }: { state: CatalogState; params: SearchParams }) {
	const { t } = useTranslation();
	const all = useSources().all;
	const failed = state.results.partial.map((p) => p.source);
	const shown = all.sources.filter((id) => !failed.includes(id));
	const names = (ids: ContentSource[]) => formatList(ids.map((id) => SOURCE_NAMES[id]));
	const busy = state.results.status === "loading";
	return (
		<div role="status" className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 text-[13px]/[18px]">
			<span className="flex min-w-0 items-center gap-1.5">
				<CircleAlert className="shrink-0 text-warning" />
				<span className="min-w-0">
					{shown.length
						? t("catalog.partial", { source: names(failed), other: names(shown) })
						: failed.length > 1
							? t("catalog.partialNoneMany", { sources: names(failed) })
							: t("catalog.partialNone", { source: names(failed) })}
				</span>
			</span>
			<Button
				variant="ghost"
				size="sm"
				aria-disabled={busy || undefined}
				onClick={() => {
					if (!busy) void searchCatalog(params, true);
				}}
			>
				<RotateCcw />
				{t("common.retry")}
			</Button>
		</div>
	);
}

function PlainRows({ hits, type, instance, showSource, onPickInstance }: RowsProps) {
	const { reduceMotion } = useAppearance();
	return (
		<div className="flex flex-col">
			{hits.map((hit, index) => {
				const first = index < PAGE_SIZE;
				return (
					<motion.div
						key={hitKey(hit)}
						initial={reduceMotion ? false : first ? { opacity: 0, y: 4 } : { opacity: 0 }}
						animate={{
							opacity: 1,
							y: 0,
							transition: {
								duration: 0.2,
								ease: easeOut,
								delay: first ? Math.min(index, 10) * 0.02 : 0,
							},
						}}
					>
						<ProjectRow
							hit={hit}
							type={type}
							instance={instance}
							showSource={showSource}
							onPickInstance={onPickInstance}
						/>
					</motion.div>
				);
			})}
		</div>
	);
}

function VirtualRows({ hits, type, instance, showSource, onPickInstance }: RowsProps) {
	const scrollRef = useScrollContainer();
	const listRef = useRef<HTMLDivElement>(null);
	const [margin, setMargin] = useState(0);

	useLayoutEffect(() => {
		const main = scrollRef.current;
		const list = listRef.current;
		if (!main || !list) return;
		setMargin(list.getBoundingClientRect().top - main.getBoundingClientRect().top + main.scrollTop);
	}, [scrollRef]);

	// Компилятор React не мемоизирует компонент с виртуализатором — так и нужно.
	// eslint-disable-next-line react-hooks/incompatible-library
	const virtualizer = useVirtualizer({
		count: hits.length,
		getScrollElement: () => scrollRef.current,
		estimateSize: () => 116,
		overscan: 6,
		scrollMargin: margin,
		getItemKey: (index) => hitKey(hits[index]),
	});

	return (
		<div ref={listRef} className="relative" style={{ height: virtualizer.getTotalSize() }}>
			{virtualizer.getVirtualItems().map((item) => (
				<div
					key={item.key}
					ref={virtualizer.measureElement}
					data-index={item.index}
					className="absolute inset-x-0 top-0"
					style={{ transform: `translateY(${item.start - virtualizer.options.scrollMargin}px)` }}
				>
					<ProjectRow
						hit={hits[item.index]}
						type={type}
						instance={instance}
						showSource={showSource}
						onPickInstance={onPickInstance}
					/>
				</div>
			))}
		</div>
	);
}

function ListFooter({ state, params }: { state: CatalogState; params: SearchParams }) {
	const { t } = useTranslation();
	const scrollRef = useScrollContainer();
	const sentinelRef = useRef<HTMLDivElement>(null);
	const results = state.results;
	const count = results.hits.length;
	const hasMore = hasMoreResults(results, state.source);
	const canLoad = hasMore && results.status === "ready" && !results.loadingMore && !results.moreError;
	const paramsKey = JSON.stringify(params);

	useEffect(() => {
		const sentinel = sentinelRef.current;
		const root = scrollRef.current;
		if (!canLoad || !sentinel || !root) return;
		const observer = new IntersectionObserver(
			([entry]) => {
				if (entry.isIntersecting) void loadMoreCatalog(JSON.parse(paramsKey) as SearchParams);
			},
			{ root, rootMargin: "0px 0px 600px 0px" },
		);
		observer.observe(sentinel);
		return () => observer.disconnect();
	}, [canLoad, paramsKey, scrollRef]);

	let footer: ReactNode = null;
	if (results.loadingMore) {
		footer = (
			<div aria-busy>
				<SlowNotice loading source={state.source} className="justify-center py-3" />
				<ProjectRowSkeleton />
				<ProjectRowSkeleton />
			</div>
		);
	} else if (results.moreError) {
		footer = (
			<div className="flex flex-wrap items-center justify-center gap-3 py-4 text-[13px]/[18px] text-destructive">
				<OctagonX />
				<span>
					{t("catalog.loadMoreFailed")} · {errorText(results.moreError)}
				</span>
				<Button variant="outline" size="sm" onClick={() => void loadMoreCatalog(params)}>
					<RotateCcw />
					{t("common.retry")}
				</Button>
			</div>
		);
	} else if (!hasMore) {
		footer = (
			<p className="py-4 text-center text-[13px]/[18px] text-muted-foreground">
				{state.source !== "all" && count >= MAX_OFFSET
					? t("catalog.tooFar")
					: t("catalog.end", { count: formatNumber(count) })}
			</p>
		);
	}

	return (
		<>
			<div ref={sentinelRef} aria-hidden className="h-px" />
			{footer}
		</>
	);
}
