import {
	ChevronDown,
	Download,
	FolderPlus,
	LayoutGrid,
	List,
	OctagonX,
	Package,
	RotateCcw,
	Search,
	X,
	type LucideIcon,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { createElement, useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CenteredState } from "@/components/centered-state";
import { InstanceCardSkeleton } from "@/components/instances/instance-card";
import { GRID_CLASS, InstanceGrid } from "@/components/instances/instance-grid";
import { InstanceRow } from "@/components/instances/instance-row";
import { CreateInstanceButton, InstancesEmpty } from "@/components/instances/instances-empty";
import { GroupHeader } from "@/components/instances/group-header";
import { PixelGrid } from "@/components/pixel-grid";
import { Segments } from "@/components/segments";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuShortcut,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { overlayOpen, typingTarget, useCreateShortcut } from "@/hooks/use-create-shortcut";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { usePendingAction } from "@/hooks/use-pending-action";
import { LOADER_KINDS, type Instance, type LoaderKind } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import {
	getGroupsUi,
	groupKey,
	listGroups,
	openGroupDialog,
	pruneCollapsed,
	setGroupCollapsed,
	setGroupsCollapsed,
	setGroupVisibilityProbe,
	useGroupsUi,
} from "@/lib/groups";
import { errorText } from "@/lib/errors";
import { compareRecent, reloadInstances, useInstances } from "@/lib/instances";
import { compareGameVersions, LOADER_NAMES } from "@/lib/loaders";
import { pickModpackFile, useModpackFileShortcut } from "@/lib/modpacks";
import { readStorage, writeStorage } from "@/lib/storage";

const SORTS = ["recent", "name", "version", "created"] as const;
type Sort = (typeof SORTS)[number];
const SORT_KEY = "instances-sort";

const VIEWS = ["grid", "list"] as const;
type View = (typeof VIEWS)[number];
const VIEW_KEY = "instances-view";
const VIEW_ICONS: Record<View, LucideIcon> = { grid: LayoutGrid, list: List };

type Filter = LoaderKind | "all";

function loadSort(): Sort {
	const value = readStorage(SORT_KEY);
	return SORTS.includes(value as Sort) ? (value as Sort) : "recent";
}

function loadView(): View {
	return readStorage(VIEW_KEY) === "list" ? "list" : "grid";
}

function sortInstances(instances: Instance[], sort: Sort, lang: string): Instance[] {
	const list = [...instances];
	switch (sort) {
		case "recent":
			return list.sort(compareRecent);
		case "name":
			return list.sort((a, b) =>
				a.name.localeCompare(b.name, lang, { numeric: true, sensitivity: "base" }),
			);
		case "version":
			return list.sort(
				(a, b) => compareGameVersions(a.gameVersion, b.gameVersion) || compareRecent(a, b),
			);
		case "created":
			return list.sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt));
	}
}

function matches(instance: Instance, query: string): boolean {
	const haystack = `${instance.name} ${instance.gameVersion} ${LOADER_NAMES[instance.loader.kind]}`;
	return haystack.toLocaleLowerCase().includes(query.toLocaleLowerCase());
}

export function InstancesPage() {
	const { t, i18n } = useTranslation();
	const store = useInstances();
	const [query, setQuery] = useState("");
	const [filter, setFilter] = useState<Filter>("all");
	const [sort, setSort] = useState<Sort>(loadSort);
	const [view, setView] = useState<View>(loadView);
	const searchRef = useRef<HTMLInputElement>(null);
	const loading = store.status === "idle" || store.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);
	const count = store.instances.length;
	const toolbar = store.status === "ready" && count > 0;
	const empty = store.status === "ready" && count === 0;

	const loaders = useMemo(() => {
		const present = new Set(store.instances.map((i) => i.loader.kind));
		return LOADER_KINDS.filter((kind) => present.has(kind));
	}, [store.instances]);
	// Загрузчик пропал из списка или остался один — фильтр снова «Все».
	if (filter !== "all" && store.status === "ready" && (loaders.length < 2 || !loaders.includes(filter)))
		setFilter("all");
	const activeFilter: Filter = filter !== "all" && loaders.includes(filter) ? filter : "all";

	useCreateShortcut(store.status === "ready");
	useModpackFileShortcut(store.status === "ready");

	useEffect(() => {
		if (!toolbar) return;
		const onKeyDown = (event: KeyboardEvent) => {
			const ctrlF = event.code === "KeyF" && event.ctrlKey && !event.altKey && !event.shiftKey;
			// По `event.code`: в русской раскладке на этой клавише точка.
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
	}, [toolbar]);

	const trimmedQuery = toolbar ? query.trim() : "";
	const filterActive: Filter = toolbar ? activeFilter : "all";
	const filtering = trimmedQuery !== "" || filterActive !== "all";
	const visible = useMemo(() => {
		const sorted = sortInstances(store.instances, sort, i18n.language);
		return sorted.filter(
			(i) =>
				(filterActive === "all" || i.loader.kind === filterActive) &&
				(!trimmedQuery || matches(i, trimmedQuery)),
		);
	}, [store.instances, sort, trimmedQuery, filterActive, i18n.language]);

	const { collapsed } = useGroupsUi();
	/** Пока групп нет — `null`, страница без заголовков. */
	const sections = useMemo(() => {
		const names = listGroups(store.instances, i18n.language);
		if (!names.length) return null;
		const all: Array<string | null> = [...names];
		if (store.instances.some((i) => !i.group)) all.push(null);
		return all.map((name) => {
			const key = groupKey(name);
			return {
				name,
				key,
				total: store.instances.filter((i) => groupKey(i.group) === key).length,
				items: visible.filter((i) => groupKey(i.group) === key),
			};
		});
	}, [store.instances, visible, i18n.language]);

	useEffect(() => {
		if (store.status !== "ready") return;
		pruneCollapsed(new Set(store.instances.map((i) => groupKey(i.group))));
	}, [store.status, store.instances]);

	// Перенос из меню проверяет, видна ли сборка на странице, иначе показывает тост.
	useEffect(() => {
		setGroupVisibilityProbe((instance) => {
			const passes =
				(filterActive === "all" || instance.loader.kind === filterActive) &&
				(!trimmedQuery || matches(instance, trimmedQuery));
			return passes && (filtering || !getGroupsUi().collapsed.has(groupKey(instance.group)));
		});
		return () => setGroupVisibilityProbe(null);
	}, [filterActive, trimmedQuery, filtering]);

	const changeSort = (value: string) => {
		setSort(value as Sort);
		writeStorage(SORT_KEY, value);
	};

	const changeView = (value: string) => {
		const next: View = value === "list" ? "list" : "grid";
		setView(next);
		writeStorage(VIEW_KEY, next);
	};

	const reset = () => {
		setQuery("");
		setFilter("all");
	};

	let body;
	if (store.status === "error") {
		body = (
			<CenteredState
				icon={OctagonX}
				tone="destructive"
				heading="h2"
				title={t("instances.loadFailed")}
				description={store.error ? errorText(store.error) : t("errors.fallback")}
				action={
					<Button variant="outline" onClick={() => void reloadInstances()}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				}
			/>
		);
	} else if (loading) {
		body = showSkeleton ? <SkeletonGrid /> : null;
	} else if (empty) {
		body = <InstancesEmpty />;
	} else {
		const listView = toolbar && view === "list";
		body = (
			<>
				{toolbar && (
					<div className="mt-6 flex flex-wrap items-center gap-3">
						<Input
							ref={searchRef}
							type="search"
							value={query}
							placeholder={t("instances.search.placeholder")}
							aria-label={t("instances.search.placeholder")}
							aria-keyshortcuts="Control+F /"
							spellCheck={false}
							autoComplete="off"
							className="h-9 w-60"
							inputClassName="px-2 [&::-webkit-search-cancel-button]:hidden"
							start={<Search className="pointer-events-none ml-2.5 text-muted-foreground" />}
							end={
								query && (
									<Button
										variant="ghost"
										size="icon-xs"
										aria-label={t("common.clear")}
										className="mr-0.5 size-7 text-muted-foreground focus-visible:bg-accent"
										onClick={() => {
											setQuery("");
											searchRef.current?.focus();
										}}
									>
										<X />
									</Button>
								)
							}
							onChange={(e) => setQuery(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === "Escape" && query) {
									e.preventDefault();
									setQuery("");
								}
							}}
						/>
						{loaders.length > 1 && (
							<Segments
								label={t("instances.filter.label")}
								value={activeFilter}
								onChange={(value) => setFilter(value as Filter)}
								layoutId="loader-filter"
								options={[
									{ value: "all", content: t("instances.filter.all") },
									...loaders.map((kind) => ({ value: kind, content: LOADER_NAMES[kind] })),
								]}
							/>
						)}
						<div className="ml-auto flex items-center gap-3">
							<Select value={sort} onValueChange={changeSort}>
								<SelectTrigger className="h-9 w-48" aria-label={t("instances.sort.label")}>
									<SelectValue />
								</SelectTrigger>
								<SelectContent position="popper" align="end">
									{SORTS.map((option) => (
										<SelectItem key={option} value={option}>
											{t(`instances.sort.${option}`)}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<Segments
								label={t("instances.view.label")}
								value={view}
								onChange={changeView}
								layoutId="instances-view"
								options={VIEWS.map((option) => {
									return {
										value: option,
										label: t(`instances.view.${option}`),
										content: createElement(VIEW_ICONS[option]),
									};
								})}
							/>
						</div>
					</div>
				)}
				{visible.length === 0 ? (
					<div className="flex flex-col items-center py-16 text-center">
						<p className="text-sm text-muted-foreground">
							{t("instances.search.nothing", { query: query.trim() })}
						</p>
						<Button variant="ghost" className="mt-3" onClick={reset}>
							{t("instances.search.reset")}
						</Button>
					</div>
				) : (
					<AnimatePresence mode="wait" initial={false}>
						<motion.div
							key={listView ? "list" : "grid"}
							initial={{ opacity: 0 }}
							animate={{ opacity: 1 }}
							exit={{ opacity: 0 }}
							transition={{ duration: 0.15 }}
							className={sections ? undefined : toolbar ? "mt-5" : "mt-8"}
						>
							{sections ? (
								<GroupSections
									sections={sections}
									collapsed={collapsed}
									filtering={filtering}
									listView={listView}
								/>
							) : (
								<InstancesList instances={visible} listView={listView} />
							)}
						</motion.div>
					</AnimatePresence>
				)}
			</>
		);
	}

	return (
		<div className="flex flex-1 flex-col">
			<div className="flex items-start gap-6">
				<div className="min-w-0 flex-1">
					<h1 className="text-[28px]/[34px] font-semibold tracking-[-0.025em]">
						{t("instances.title")}
					</h1>
					<p className="mt-1 max-w-prose text-sm text-muted-foreground">
						{t("instances.description")}
					</p>
				</div>
				{!empty && (
					<div className="flex shrink-0 items-center gap-2">
						{store.status === "ready" && (
							<Button
								variant="outline"
								onClick={(e) => openGroupDialog("new", { returnFocus: e.currentTarget })}
							>
								<FolderPlus />
								{t("groups.newFolder")}
							</Button>
						)}
						{store.status === "ready" && <ImportMenu />}
						<CreateInstanceButton disabled={store.status !== "ready"} />
					</div>
				)}
			</div>
			{body}
		</div>
	);
}

/** Действие — после закрытия меню. */
function ImportMenu() {
	const { t } = useTranslation();
	const pending = usePendingAction();
	const triggerRef = useRef<HTMLButtonElement>(null);
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button ref={triggerRef} variant="outline" className="aria-expanded:bg-accent">
					<Download />
					{t("instances.import.label")}
					<ChevronDown className="text-muted-foreground" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-78" onCloseAutoFocus={pending.onCloseAutoFocus}>
				<DropdownMenuItem
					onSelect={() => pending.defer(() => void pickModpackFile(triggerRef.current))}
				>
					<Package />
					{t("instances.import.file")}
					<DropdownMenuShortcut className="text-[13px] tracking-normal">
						{t("instances.import.shortcut")}
					</DropdownMenuShortcut>
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function InstancesList({ instances, listView }: { instances: Instance[]; listView: boolean }) {
	return listView ? (
		<div className="flex flex-col gap-1">
			{instances.map((instance) => (
				<InstanceRow key={instance.id} instance={instance} />
			))}
		</div>
	) : (
		<InstanceGrid instances={instances} />
	);
}

type Section = { name: string | null; key: string; total: number; items: Instance[] };

const easeOut = [0.22, 1, 0.36, 1] as const;

function GroupSections({
	sections,
	collapsed,
	filtering,
	listView,
}: {
	sections: Section[];
	collapsed: ReadonlySet<string>;
	filtering: boolean;
	listView: boolean;
}) {
	const { reduceMotion } = useAppearance();
	const baseId = useId();
	const shown = filtering ? sections.filter((s) => s.items.length > 0) : sections;
	const keys = shown.map((s) => s.key);

	return (
		<div className="mt-6">
			{shown.map((section, index) => {
				const isCollapsed = !filtering && collapsed.has(section.key);
				const previous = shown[index - 1];
				const previousCollapsed = previous ? !filtering && collapsed.has(previous.key) : false;
				const contentId = `${baseId}-group-${index}`;
				return (
					<section key={section.name === null ? "__none__" : `g:${section.key}`}>
						<GroupHeader
							name={section.name}
							shown={section.items.length}
							total={section.total}
							filtered={filtering}
							collapsed={isCollapsed}
							contentId={contentId}
							className={index === 0 ? undefined : previousCollapsed ? "mt-2" : "mt-8"}
							onToggle={(all) => {
								if (all) setGroupsCollapsed(keys, !isCollapsed);
								else setGroupCollapsed(section.key, !isCollapsed);
							}}
						/>
						<AnimatePresence initial={false}>
							{!isCollapsed && (
								<motion.div
									key="content"
									id={contentId}
									initial={reduceMotion ? { opacity: 1 } : { opacity: 0, y: -4 }}
									animate={{
										opacity: 1,
										y: 0,
										transition: { duration: 0.2, ease: easeOut },
									}}
									exit={{
										opacity: 0,
										transition: { duration: reduceMotion ? 0 : 0.12, ease: easeOut },
									}}
									className={listView ? "mt-1" : "mt-3"}
								>
									<InstancesList instances={section.items} listView={listView} />
								</motion.div>
							)}
						</AnimatePresence>
					</section>
				);
			})}
		</div>
	);
}

function SkeletonGrid() {
	return (
		<PixelGrid aria-busy className={`mt-8 ${GRID_CLASS}`}>
			{Array.from({ length: 6 }, (_, i) => (
				<InstanceCardSkeleton key={i} />
			))}
		</PixelGrid>
	);
}
