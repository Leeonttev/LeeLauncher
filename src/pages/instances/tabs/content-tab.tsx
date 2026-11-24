import {
	ArrowRight,
	ArrowUpDown,
	ArrowUpRight,
	Braces,
	CircleAlert,
	Compass,
	Ellipsis,
	ExternalLink,
	FolderOpen,
	Globe,
	Info,
	LoaderCircle,
	OctagonX,
	Palette,
	Plus,
	Puzzle,
	RefreshCw,
	RotateCcw,
	Sparkles,
	Trash2,
	WifiOff,
	type LucideIcon,
} from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { AnimatePresence, motion } from "motion/react";
import { createElement, type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { CenteredState } from "@/components/centered-state";
import { useScrollContainer } from "@/components/layout/scroll-container";
import { ManualStrip } from "@/components/manual/manual-strip";
import { ModpackStrip } from "@/components/modpacks/modpack-strip";
import { SearchField } from "@/components/search-field";
import { Segments } from "@/components/segments";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { usePendingAction } from "@/hooks/use-pending-action";
import {
	CONTENT_TYPES,
	external,
	type ContentItem,
	type ContentSource,
	type ContentType,
	type FailedSource,
	type Instance,
	type InstanceSubfolder,
} from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import {
	isSourceEnabled,
	openProject,
	setCatalogContext,
	setCatalogType,
	SOURCE_NAMES,
	useSources,
} from "@/lib/catalog";
import {
	checkContentUpdates,
	loadContent,
	shortVersion,
	removeContent,
	toggleContent,
	useInstanceContent,
	useUpdatesCheck,
	type UpdatesCheck,
} from "@/lib/content";
import {
	contentBusy,
	failuresText,
	sourceTotal,
	updateAllContent,
	updateAllKey,
	updateProjectItem,
	useInstallUi,
} from "@/lib/content-install";
import { useDownloads } from "@/lib/downloads";
import { errorText } from "@/lib/errors";
import { formatAgo, formatBytes, formatNumber } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { setContentFilter, useInstancePageState, type ContentFilter } from "@/lib/instance-page";
import { openInstanceFolder, statusOf, useInstances } from "@/lib/instances";
import { openManualDialog, useManualPending } from "@/lib/manual";
import { navigate } from "@/lib/router";
import { cn } from "@/lib/utils";
import { ariaDeleteKey, isDeleteKey } from "@/lib/platform";

const TYPE_ICONS: Record<ContentType, LucideIcon> = {
	mod: Puzzle,
	resourcepack: Palette,
	shader: Sparkles,
	datapack: Braces,
	world: Globe,
};

const VIRTUAL_FROM = 200;
/** Строка 56 + линия снизу. */
const ROW_HEIGHT = 57;
const HEADER_HEIGHT = 40;
/** Колонки не уже 480, между ними 32, не больше 3. */
const MIN_COLUMN = 480;
const COLUMN_GAP = 32;
const MAX_COLUMNS = 3;

type Lock = "running" | "installing" | null;

type Group = { type: ContentType; items: ContentItem[] };

type Entry =
	| { kind: "header"; type: ContentType; count: number; first: boolean }
	| { kind: "row"; type: ContentType; items: ContentItem[] };

type Summary = { count: number; size: number; disabled: number };

function itemName(item: ContentItem) {
	return item.project?.title ?? item.file;
}

function itemKey(item: ContentItem) {
	return `${item.type}:${item.world ?? ""}:${item.file}`;
}

function itemLocation(item: ContentItem): { sub: InstanceSubfolder; file: string } {
	const sub: InstanceSubfolder =
		item.type === "mod"
			? "mods"
			: item.type === "resourcepack"
				? "resourcepacks"
				: item.type === "shader"
					? "shaderpacks"
					: item.world
						? `saves/${item.world}/datapacks`
						: "datapacks";
	return { sub, file: item.enabled ? item.file : `${item.file}.disabled` };
}

function showInFolder(instanceId: string, item: ContentItem) {
	const { sub, file } = itemLocation(item);
	void openInstanceFolder(instanceId, sub, file);
}

function openSite(url: string) {
	external.open(url).catch((err: unknown) => {
		console.error("open-url", err);
		toast.error(i18n.t("common.openLinkFailed"));
	});
}

/** У файлов из остальных источников источник пишется в строке. */
function majoritySource(items: ContentItem[]): ContentSource | null {
	const counts = new Map<ContentSource, number>();
	for (const item of items)
		if (item.project) counts.set(item.project.source, (counts.get(item.project.source) ?? 0) + 1);
	let best: ContentSource | null = null;
	for (const [source, count] of counts) if (!best || count > (counts.get(best) ?? 0)) best = source;
	return best;
}

export function ContentTab({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const content = useInstanceContent(instance.id);
	const check = useUpdatesCheck(instance.id);
	const page = useInstancePageState();
	const store = useInstances();
	const ui = useInstallUi();
	const downloads = useDownloads();
	const status = statusOf(instance, store);
	const lock: Lock =
		status === "running" || status === "launching"
			? "running"
			: status === "queued" || status === "installing"
				? "installing"
				: null;
	const { reduceMotion } = useAppearance();
	const [query, setQuery] = useState("");
	const [removing, setRemoving] = useState<ContentItem | null>(null);
	const loading = !content || content.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);
	const sources = useSources().items;
	const manual = useManualPending(instance.id);

	useEffect(() => {
		void checkContentUpdates(instance.id);
	}, [instance.id]);

	const items = content?.status === "ready" ? content.items : [];
	const counts = new Map<ContentType, number>();
	for (const item of items) counts.set(item.type, (counts.get(item.type) ?? 0) + 1);
	const types = CONTENT_TYPES.filter((type) => counts.get(type));
	const stored = page.filters[instance.id] ?? "all";
	const filter: ContentFilter = stored !== "all" && types.some((type) => type === stored) ? stored : "all";
	const updates = items.filter((item) => item.update).length;
	const majority = majoritySource(items);
	// Источник выключен: файлы на месте, но обновления не проверяются.
	const disabledSource = items.find(
		(item) => item.project && !isSourceEnabled(item.project.source, sources),
	)?.project?.source;

	const add = () => {
		setCatalogType(filter === "all" ? "mod" : filter);
		setCatalogContext(instance.id, { flash: true });
		navigate("/catalog");
	};

	let body: ReactNode;
	if (content?.status === "error") {
		body = (
			<CenteredState
				icon={OctagonX}
				tone="destructive"
				heading="h2"
				title={t("content.error.title")}
				description={content.error ? errorText(content.error) : t("errors.fallback")}
				action={
					<Button variant="outline" onClick={() => void loadContent(instance.id)}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				}
			/>
		);
	} else if (loading) {
		body = showSkeleton ? <ContentSkeleton /> : null;
	} else if (!items.length) {
		body = (
			<CenteredState
				icon={Puzzle}
				heading="h2"
				title={t("content.empty.title")}
				description={t("content.empty.description")}
				action={
					<Button variant="outline" onClick={add}>
						<Compass />
						{t("content.empty.action")}
					</Button>
				}
			/>
		);
	} else {
		const needle = query.trim().toLocaleLowerCase();
		const matches = items.filter((item) => {
			if (filter !== "all" && item.type !== filter) return false;
			if (!needle) return true;
			return [item.project?.title, item.project?.author, item.file].some((text) =>
				text?.toLocaleLowerCase().includes(needle),
			);
		});
		const collator = new Intl.Collator(i18n.language, { sensitivity: "base", numeric: true });
		matches.sort((a, b) => collator.compare(itemName(a), itemName(b)));
		const groups: Group[] = CONTENT_TYPES.map((type) => ({
			type,
			items: matches.filter((item) => item.type === type),
		})).filter((group) => group.items.length);
		const summary: Summary = {
			count: items.length,
			size: items.reduce((sum, item) => sum + item.size, 0),
			disabled: items.filter((item) => !item.enabled).length,
		};

		const renderRow = (item: ContentItem) => (
			<ContentRow
				instance={instance}
				item={item}
				lock={lock}
				busy={item.project ? contentBusy(ui, downloads, instance.id, item.project.id) : false}
				showSource={Boolean(item.project && majority && item.project.source !== majority)}
				sourceEnabled={!item.project || isSourceEnabled(item.project.source, sources)}
				onRemove={setRemoving}
			/>
		);

		body = (
			<>
				{disabledSource ? (
					<p className="mt-3 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
						<Info className="shrink-0" />
						{t("content.sourceDisabled", { source: SOURCE_NAMES[disabledSource] })}
					</p>
				) : (
					<UpdatesNotice
						instanceId={instance.id}
						check={check}
						offline={Boolean(content?.offline)}
						contentFailed={content?.failedSources ?? []}
						items={items}
					/>
				)}
				{groups.length === 0 ? (
					<p className="py-10 text-center text-[13px]/[18px] text-muted-foreground">
						{t("content.notFound")}
					</p>
				) : (
					<ContentList groups={groups} summary={summary} renderRow={renderRow} />
				)}
			</>
		);
	}

	return (
		<div className="@container flex flex-1 flex-col">
			<AnimatePresence initial={false}>
				{instance.modpack && (
					<motion.div
						key="modpack"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: reduceMotion ? 0 : 0.12 }}
					>
						<ModpackStrip instance={instance} link={instance.modpack} />
					</motion.div>
				)}
				{manual.length > 0 && (
					<motion.div
						key="manual"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: reduceMotion ? 0 : 0.12 }}
					>
						<ManualStrip
							count={manual.length}
							onOpen={(target) => void openManualDialog(instance.id, undefined, target)}
						/>
					</motion.div>
				)}
			</AnimatePresence>
			{content?.status === "ready" && items.length > 0 && (
				<div className="flex flex-wrap items-center gap-3">
					<SearchField
						value={query}
						onChange={setQuery}
						placeholder={t("content.search")}
						className="w-65"
					/>
					{types.length > 1 && (
						<Segments
							label={t("content.typeLabel")}
							value={filter}
							onChange={(value) => setContentFilter(instance.id, value as ContentFilter)}
							layoutId="content-type"
							options={[
								{
									value: "all",
									content: <SegmentLabel label={t("content.all")} count={items.length} />,
								},
								...types.map((type) => ({
									value: type,
									content: (
										<SegmentLabel
											label={t(`catalog.type.${type}`)}
											count={counts.get(type) ?? 0}
										/>
									),
								})),
							]}
						/>
					)}
					<div className="ml-auto flex items-center gap-2">
						<UpdateAllButton
							instanceId={instance.id}
							count={updates}
							checking={check?.status === "checking"}
							checkedAt={check?.checkedAt ?? null}
							pending={Boolean(ui.pending[updateAllKey(instance.id)])}
							lock={lock}
						/>
						<Button variant="outline" onClick={add}>
							<Plus />
							{t("content.add")}
						</Button>
					</div>
				</div>
			)}
			{body}
			<RemoveDialog instanceId={instance.id} item={removing} onClose={() => setRemoving(null)} />
		</div>
	);
}

/** Пока идёт проверка, прошлый результат не показывается: рядом уже «Проверяем…». */
function UpdatesNotice({
	instanceId,
	check,
	offline,
	contentFailed,
	items,
}: {
	instanceId: string;
	check: UpdatesCheck | null;
	offline: boolean;
	contentFailed: FailedSource[];
	items: ContentItem[];
}) {
	const { t } = useTranslation();
	if (check?.status === "checking") return null;
	// Свежая проверка обновлений важнее того, что ядро видело при чтении списка.
	const failed = check?.status === "done" ? check.failedSources : check ? [] : contentFailed;
	const error = check?.status === "error" ? check.error : null;
	const timeout = error?.code === "source_unavailable" && error.details === "timeout";
	const network = error?.code === "network";
	// Удачная проверка значит, что сеть есть: прежний признак `offline` уже не важен.
	const noNetwork = network || (!error && offline && check?.status !== "done");
	let text: string | null = null;
	let Icon = WifiOff;
	if (error && !network) {
		Icon = CircleAlert;
		text = timeout
			? t("content.sourcesTimeout")
			: t("content.updatesFailed", { error: errorText(error) });
	} else if (failed.length) {
		text = failuresText(failed, (source) => sourceTotal(items, source, "check"), "check");
	} else if (noNetwork) text = t("content.offline");
	if (!text) return null;
	return (
		<div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 text-[13px]/[18px] text-muted-foreground">
			<span className="flex min-w-0 items-center gap-1.5">
				<Icon className="shrink-0" />
				<span className="min-w-0">{text}</span>
			</span>
			<Button variant="outline" size="sm" onClick={() => void checkContentUpdates(instanceId, true)}>
				<RotateCcw />
				{t("common.retry")}
			</Button>
		</div>
	);
}

function SegmentLabel({ label, count }: { label: string; count: number }) {
	return (
		<span className="flex items-center gap-1.5">
			{label}
			<span className="text-muted-foreground tabular-nums">{formatNumber(count)}</span>
		</span>
	);
}

function lockText(lock: Lock) {
	return lock === "running" ? i18n.t("content.gameRunning") : i18n.t("catalog.install.waitInstance");
}

/** Обёртка `span`, чтобы подсказка не перезаписала `data-state` у Switch. */
function Hint({ text, children }: { text: string | null; children: ReactNode }) {
	if (!text) return children;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span className="inline-flex">{children}</span>
			</TooltipTrigger>
			<TooltipContent>{text}</TooltipContent>
		</Tooltip>
	);
}

function UpdateAllButton({
	instanceId,
	count,
	checking,
	checkedAt,
	pending,
	lock,
}: {
	instanceId: string;
	count: number;
	checking: boolean;
	checkedAt: number | null;
	pending: boolean;
	lock: Lock;
}) {
	const { t } = useTranslation();
	if (checking) {
		return (
			<Button variant="outline" aria-disabled className="pointer-events-none text-muted-foreground">
				<LoaderCircle className="animate-spin" />
				{t("content.checking")}
			</Button>
		);
	}
	if (!count) return null;
	const hint = lock
		? lockText(lock)
		: checkedAt
			? t("content.checkedAt", { when: formatAgo(checkedAt) })
			: null;
	const disabled = Boolean(lock) || pending;
	return (
		<Hint text={hint}>
			<Button
				variant="outline"
				aria-disabled={disabled || undefined}
				className="aria-disabled:opacity-60"
				onClick={() => {
					if (!disabled) void updateAllContent(instanceId);
				}}
			>
				{pending ? <LoaderCircle className="animate-spin" /> : <RefreshCw />}
				{t("content.updateAll", { count })}
			</Button>
		</Hint>
	);
}

/** Целая ширина колонок, как в `PixelGrid`. */
function useColumns(node: HTMLElement | null) {
	const [layout, setLayout] = useState<{ count: number; width: number } | null>(null);
	useEffect(() => {
		if (!node) return;
		const observer = new ResizeObserver(([entry]) => {
			const total = entry.contentRect.width;
			const count = Math.min(
				MAX_COLUMNS,
				Math.max(1, Math.floor((total + COLUMN_GAP) / (MIN_COLUMN + COLUMN_GAP))),
			);
			const width = Math.max(0, Math.floor((total - COLUMN_GAP * (count - 1)) / count));
			setLayout((prev) => (prev?.count === count && prev.width === width ? prev : { count, width }));
		});
		observer.observe(node);
		return () => observer.disconnect();
	}, [node]);
	return layout;
}

function GroupHeader({
	type,
	count,
	summary,
}: {
	type: ContentType;
	count: number;
	summary: Summary | null;
}) {
	const { t } = useTranslation();
	return (
		<div className="flex h-10 items-end gap-2 border-b pb-2">
			<h3 className="text-sm/5 font-semibold">{t(`catalog.type.${type}`)}</h3>
			<span className="text-[13px]/[18px] text-muted-foreground tabular-nums">
				{formatNumber(count)}
			</span>
			{summary && (
				<span className="ml-auto truncate text-[13px]/[18px] text-muted-foreground tabular-nums">
					{t("content.summary", { count: summary.count, size: formatBytes(summary.size) })}
					{summary.disabled > 0 &&
						` · ${t("content.summaryDisabled", { count: summary.disabled })}`}
				</span>
			)}
		</div>
	);
}

/** Порядок — слева направо, потом вниз. */
function ContentList({
	groups,
	summary,
	renderRow,
}: {
	groups: Group[];
	summary: Summary;
	renderRow: (item: ContentItem) => ReactNode;
}) {
	const [node, setNode] = useState<HTMLDivElement | null>(null);
	const layout = useColumns(node);
	const columns = layout?.count ?? 1;
	const template = layout ? `repeat(${columns}, ${layout.width}px)` : "minmax(0, 1fr)";
	const rows = groups.reduce((sum, group) => sum + group.items.length, 0);

	const entries: Entry[] = [];
	groups.forEach((group, index) => {
		entries.push({ kind: "header", type: group.type, count: group.items.length, first: index === 0 });
		for (let i = 0; i < group.items.length; i += columns)
			entries.push({ kind: "row", type: group.type, items: group.items.slice(i, i + columns) });
	});

	const renderEntry = (entry: Entry) =>
		entry.kind === "header" ? (
			<GroupHeader
				type={entry.type}
				count={entry.count}
				summary={entry.first && columns > 1 ? summary : null}
			/>
		) : (
			<div className="grid gap-x-8" style={{ gridTemplateColumns: template }}>
				{entry.items.map((item) => (
					<div key={itemKey(item)} className="min-w-0 border-b">
						{renderRow(item)}
					</div>
				))}
			</div>
		);

	return (
		<div ref={setNode} className="mt-1">
			{rows > VIRTUAL_FROM ? (
				<VirtualEntries entries={entries} render={renderEntry} />
			) : (
				groups.map((group, index) => (
					<section key={group.type} className={cn(index > 0 && "mt-2")}>
						<GroupHeader
							type={group.type}
							count={group.items.length}
							summary={index === 0 && columns > 1 ? summary : null}
						/>
						<div className="grid gap-x-8" style={{ gridTemplateColumns: template }}>
							{group.items.map((item) => (
								<div key={itemKey(item)} className="min-w-0 border-b">
									{renderRow(item)}
								</div>
							))}
						</div>
					</section>
				))
			)}
		</div>
	);
}

function ContentRow({
	instance,
	item,
	lock,
	busy,
	showSource,
	sourceEnabled,
	onRemove,
}: {
	instance: Instance;
	item: ContentItem;
	lock: Lock;
	busy: boolean;
	showSource: boolean;
	/** Источник выключен — вместо нашей страницы «Открыть на сайте». */
	sourceEnabled: boolean;
	onRemove: (item: ContentItem) => void;
}) {
	const { t } = useTranslation();
	const pending = usePendingAction();
	const project = item.project;
	const name = itemName(item);
	const locked = lock !== null;
	const hint = locked ? lockText(lock) : null;
	const dim = !item.enabled && "opacity-50";
	const size = formatBytes(item.size);
	// В сборке из модпака помечаем файлы, которые добавил пользователь.
	const fromPack = Boolean(instance.modpack);
	const mine = fromPack && !item.fromModpack ? t("modpack.addedByYou") : null;
	const subtitle = project
		? [project.author, showSource ? SOURCE_NAMES[project.source] : null, size, mine]
				.filter(Boolean)
				.join(" · ")
		: item.world
			? [t("content.inWorld", { world: item.world }), size, mine].filter(Boolean).join(" · ")
			: fromPack && item.fromModpack
				? size
				: t("content.manualSize", { size });

	const siteUrl = project && !sourceEnabled ? project.url : null;
	const update = item.update;
	const viaHint =
		update?.via && project
			? t("content.updateViaHint", {
					via: SOURCE_NAMES[update.via],
					source: SOURCE_NAMES[project.source],
				})
			: null;
	const updateText = update
		? update.siteOnly
			? t("content.updateSiteOnly", { version: update.number })
			: t("content.updateTo", { version: update.number })
		: "";
	const updateHint = viaHint ? `${updateText}. ${viaHint}` : updateText;
	const open = () => {
		if (siteUrl) openSite(siteUrl);
		else if (project && sourceEnabled) openProject({ source: project.source, id: project.id }, item.type);
		else showInFolder(instance.id, item);
	};

	return (
		<div
			className="relative -mx-3 flex h-14 items-center gap-3 rounded-lg px-3 transition-colors duration-(--duration-fast) ease-out hover:bg-accent"
			onKeyDown={(event) => {
				if (!isDeleteKey(event) || locked) return;
				if ((event.target as HTMLElement).closest("[role=menu]")) return;
				event.preventDefault();
				onRemove(item);
			}}
		>
			<button
				type="button"
				aria-keyshortcuts={ariaDeleteKey}
				className="relative -my-1 -ml-1 flex min-w-0 flex-1 items-center gap-3 rounded-md border border-transparent py-1 pl-1 text-left"
				onClick={open}
			>
				{project ? (
					<ProjectIcon url={project.iconUrl} type={item.type} size={40} className={cn(dim)} />
				) : (
					<span
						aria-hidden
						className={cn(
							"flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground",
							dim,
						)}
					>
						{createElement(TYPE_ICONS[item.type])}
					</span>
				)}
				<span className={cn("min-w-0 flex-1", dim)}>
					<span className="flex min-w-0 items-baseline gap-2">
						<span className="truncate text-sm/5 font-semibold">{name}</span>
						{project && <VersionText item={item} game={instance.gameVersion} />}
					</span>
					<span className="block truncate text-[13px]/[18px] text-muted-foreground">
						{subtitle}
					</span>
				</span>
			</button>
			<span className="flex w-8 shrink-0">
				{update && project && (
					<Hint text={hint ?? updateHint}>
						<Button
							variant="ghost"
							size="icon-sm"
							aria-label={updateHint}
							aria-disabled={locked || busy || undefined}
							className="text-primary-text aria-disabled:opacity-60"
							onClick={(e) => {
								if (!locked && !busy)
									void updateProjectItem(instance.id, item, e.currentTarget);
							}}
						>
							{busy ? <LoaderCircle className="animate-spin" /> : <RefreshCw />}
						</Button>
					</Hint>
				)}
			</span>
			<span className="flex shrink-0">
				<Hint text={hint ?? (item.enabled ? t("content.disable") : t("content.enable"))}>
					<Switch
						checked={item.enabled}
						aria-label={
							item.enabled
								? t("content.enabledAria", { name })
								: t("content.disabledAria", { name })
						}
						aria-disabled={locked || undefined}
						className="aria-disabled:opacity-50"
						onCheckedChange={(checked) => {
							if (!locked) void toggleContent(instance.id, item, checked);
						}}
					/>
				</Hint>
			</span>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<Button
						variant="ghost"
						size="icon-sm"
						aria-label={t("content.menu.label", { name })}
						className="shrink-0 aria-expanded:bg-accent"
					>
						<Ellipsis />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end" className="w-64" onCloseAutoFocus={pending.onCloseAutoFocus}>
					<DropdownMenuLabel className="font-normal">
						<span className="block truncate text-[13px]/[18px] font-semibold">{item.file}</span>
						<span className="block truncate text-[13px]/[18px] text-muted-foreground">
							{`${project ? SOURCE_NAMES[project.source] : t("content.source.file")} · ${size}`}
						</span>
						{viaHint && (
							<span className="mt-1 block text-[13px]/[18px] whitespace-normal text-muted-foreground">
								{viaHint}
							</span>
						)}
					</DropdownMenuLabel>
					<DropdownMenuSeparator />
					{update?.siteOnly && project && (
						<DropdownMenuItem
							disabled={locked || busy}
							onSelect={() => pending.defer(() => void updateProjectItem(instance.id, item))}
						>
							<ExternalLink />
							{t("content.menu.updateOnSite", { version: update.number })}
						</DropdownMenuItem>
					)}
					{project && siteUrl && (
						<DropdownMenuItem onSelect={() => openSite(siteUrl)}>
							<ExternalLink />
							{t("content.menu.openOnSite")}
						</DropdownMenuItem>
					)}
					{project && sourceEnabled && (
						<DropdownMenuItem onSelect={() => pending.defer(open)}>
							<ArrowUpRight />
							{t("content.menu.open")}
						</DropdownMenuItem>
					)}
					{project && sourceEnabled && (
						<DropdownMenuItem
							onSelect={() =>
								pending.defer(() =>
									openProject({ source: project.source, id: project.id }, item.type, {
										tab: "versions",
									}),
								)
							}
						>
							<ArrowUpDown />
							{t("content.menu.otherVersion")}
						</DropdownMenuItem>
					)}
					<DropdownMenuItem onSelect={() => showInFolder(instance.id, item)}>
						<FolderOpen />
						{t("content.menu.showInFolder")}
					</DropdownMenuItem>
					<DropdownMenuSeparator />
					<DropdownMenuItem
						variant="destructive"
						disabled={locked}
						onSelect={() => pending.defer(() => onRemove(item))}
					>
						<Trash2 />
						{t("content.menu.remove")}
					</DropdownMenuItem>
				</DropdownMenuContent>
			</DropdownMenu>
		</div>
	);
}

function VersionText({ item, game }: { item: ContentItem; game: string }) {
	const { t } = useTranslation();
	const project = item.project;
	if (!project) return null;
	const short = shortVersion(project.versionNumber, game);
	const via = item.update?.via ?? null;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span className="flex shrink-0 items-center gap-1 text-[13px]/[18px] font-normal text-muted-foreground">
					<span>{short}</span>
					{item.update && (
						<>
							<ArrowRight />
							<span className="text-primary-text">
								{shortVersion(item.update.number, game)}
							</span>
							{via && <span>{t("content.updateVia", { source: SOURCE_NAMES[via] })}</span>}
						</>
					)}
				</span>
			</TooltipTrigger>
			<TooltipContent className="max-w-80">
				{t("content.versionHint", { version: project.versionNumber, file: item.file })}
				{via && (
					<span className="mt-1 block">
						{t("content.updateViaHint", {
							via: SOURCE_NAMES[via],
							source: SOURCE_NAMES[project.source],
						})}
					</span>
				)}
			</TooltipContent>
		</Tooltip>
	);
}

function VirtualEntries({ entries, render }: { entries: Entry[]; render: (entry: Entry) => ReactNode }) {
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
		count: entries.length,
		getScrollElement: () => scrollRef.current,
		estimateSize: (index) => (entries[index].kind === "header" ? HEADER_HEIGHT : ROW_HEIGHT),
		overscan: 8,
		scrollMargin: margin,
		getItemKey: (index) => {
			const entry = entries[index];
			return entry.kind === "header" ? `h:${entry.type}` : `r:${itemKey(entry.items[0])}`;
		},
	});

	return (
		<div ref={listRef} className="relative" style={{ height: virtualizer.getTotalSize() }}>
			{virtualizer.getVirtualItems().map((row) => (
				<div
					key={row.key}
					className="absolute inset-x-0 top-0"
					style={{
						height: row.size,
						transform: `translateY(${row.start - virtualizer.options.scrollMargin}px)`,
					}}
				>
					{render(entries[row.index])}
				</div>
			))}
		</div>
	);
}

function RemoveDialog({
	instanceId,
	item,
	onClose,
}: {
	instanceId: string;
	item: ContentItem | null;
	onClose: () => void;
}) {
	const { t } = useTranslation();
	const [busy, setBusy] = useState(false);
	// Название держим, пока диалог закрывается.
	const [shown, setShown] = useState<ContentItem | null>(item);
	if (item && item !== shown) setShown(item);
	const name = shown ? itemName(shown) : "";

	const confirm = async () => {
		if (!shown || busy) return;
		setBusy(true);
		const ok = await removeContent(instanceId, [shown]);
		setBusy(false);
		if (ok) onClose();
	};

	return (
		<AlertDialog
			open={item !== null}
			onOpenChange={(open) => {
				if (!open && !busy) onClose();
			}}
		>
			<AlertDialogContent className="max-w-[400px] gap-6 sm:max-w-[400px] data-[size=default]:sm:max-w-[400px]">
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle className="break-words">
						{t("content.remove.title", { name })}
					</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{t("content.remove.description")}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus disabled={busy}>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button variant="destructive" disabled={busy} onClick={() => void confirm()}>
						{busy && <LoaderCircle className="animate-spin" />}
						{t("common.remove")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function ContentSkeleton() {
	return (
		<div aria-busy className="mt-1">
			<div className="flex h-10 items-end border-b pb-2">
				<Skeleton className="h-4 w-28" />
			</div>
			{Array.from({ length: 8 }, (_, i) => (
				<div key={i} className="flex h-[57px] items-center gap-3 border-b">
					<Skeleton className="size-10 rounded-lg" />
					<div className="flex flex-1 flex-col gap-1.5">
						<Skeleton className="h-4 w-48" />
						<Skeleton className="h-3 w-32" />
					</div>
					<Skeleton className="h-5 w-9" />
				</div>
			))}
		</div>
	);
}
