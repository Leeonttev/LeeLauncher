import { useSyncExternalStore } from "react";
import {
	catalogApi,
	type AllInfo,
	type AlsoOn,
	type CatalogProjectType,
	type CategoryTag,
	type ContentSource,
	type Instance,
	type PartialFailure,
	type ProjectHit,
	type SearchParams,
	type SearchSort,
	type SourceInfo,
} from "@/lib/api";
import { toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";
import { navigate, projectPath } from "@/lib/router";
import { readStorage, writeStorage } from "@/lib/storage";
import { ru, type Dictionary } from "@/locales/ru";

export const SOURCES: ContentSource[] = ["modrinth", "curseforge"];
export const SOURCE_NAMES: Record<ContentSource, string> = { modrinth: "Modrinth", curseforge: "CurseForge" };
/** Показывается, когда выбранный источник выключен. */
export const DEFAULT_SOURCE: ContentSource = "modrinth";
export type CatalogSource = ContentSource | "all";
export const CATALOG_SOURCES: CatalogSource[] = ["all", ...SOURCES];

export type CatalogType = CatalogProjectType;
export const CATALOG_TYPES: CatalogType[] = ["mod", "resourcepack", "shader", "datapack", "world", "modpack"];

export const SEARCH_SORTS: SearchSort[] = ["relevance", "downloads", "follows", "newest", "updated", "name"];
export const PAGE_SIZE = 20;
/** Дальше источники не отдают. */
export const MAX_OFFSET = 10_000;
/** Сохранённый список источника показывается без запроса, если он не старше этого. */
const STASH_FRESH = 2 * 60 * 1000;

export const LOADER_FILTERS: Partial<Record<CatalogType, string[]>> = {
	mod: ["fabric", "forge", "neoforge", "quilt"],
	shader: ["iris", "optifine", "canvas"],
	modpack: ["fabric", "forge", "neoforge", "quilt"],
};
export const LOADER_FILTER_NAMES: Record<string, string> = {
	fabric: "Fabric",
	forge: "Forge",
	neoforge: "NeoForge",
	quilt: "Quilt",
	iris: "Iris",
	optifine: "OptiFine",
	canvas: "Canvas",
};

const SERVICE_LOADERS = new Set(["minecraft", "datapack"]);

export function visibleLoaders(loaders: string[]) {
	return loaders.filter((loader) => !SERVICE_LOADERS.has(loader));
}

const SORT_KEY = "catalog-sort";
const INSTANCE_KEY = "catalog-instance";
const SOURCE_KEY = "catalog-source";

/** Свой предел на случай, если ядро не ответит: дольше ждать нет смысла. */
export const CATALOG_DEADLINE = 20_000;
/** С этого момента под скелетоном — «отвечает долго». */
export const SLOW_AFTER = 4_000;

/** Ответ ядра или `source_unavailable` с `details: "timeout"` через `ms`. */
export function withDeadline<T>(promise: Promise<T>, ms = CATALOG_DEADLINE): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		const timer = setTimeout(
			() => reject({ code: "source_unavailable", message: "", details: "timeout" } satisfies AppError),
			ms,
		);
		promise.then(
			(value) => {
				clearTimeout(timer);
				resolve(value);
			},
			(err: unknown) => {
				clearTimeout(timer);
				reject(err);
			},
		);
	});
}

/** Не удалось связаться с источником: сеть, сбой источника или истёк предел. */
export function isUnreachable(error: AppError): boolean {
	return error.code === "network" || error.code === "source_unavailable";
}

/* Возможности источников */

/** До ответа ядра: один источник по умолчанию с общими возможностями. */
const FALLBACK_INFO: SourceInfo = {
	id: DEFAULT_SOURCE,
	enabled: true,
	types: ["mod", "resourcepack", "shader", "datapack", "modpack"],
	sorts: ["relevance", "downloads", "follows", "newest", "updated"],
	shaderLoaders: true,
	maxCategories: null,
	hasFollows: true,
};

const FALLBACK_ALL: AllInfo = {
	enabled: false,
	sources: [DEFAULT_SOURCE],
	types: FALLBACK_INFO.types,
	sorts: ["relevance", "downloads", "updated", "newest"],
	categories: false,
};

export type SourcesState = { status: "loading" | "ready" | "error"; items: SourceInfo[]; all: AllInfo };

let sources: SourcesState = { status: "loading", items: [FALLBACK_INFO], all: FALLBACK_ALL };
const sourceListeners = new Set<() => void>();
let sourcesStarted = false;

function setSources(next: SourcesState) {
	sources = next;
	for (const listener of sourceListeners) listener();
	// Выбранный источник выключен — показываем главный, но выбор пользователя не перезаписываем.
	if (!viewInfo(state.source).enabled) switchSource(DEFAULT_SOURCE, false);
}

export function initCatalogSources() {
	if (sourcesStarted) return;
	sourcesStarted = true;
	const all = catalogApi.allInfo().catch((err: unknown) => {
		console.error("catalog_all_info", err);
		return FALLBACK_ALL;
	});
	catalogApi
		.sources()
		.then(async (items) =>
			setSources({ status: "ready", items: items.length ? items : [FALLBACK_INFO], all: await all }),
		)
		.catch((err: unknown) => {
			console.error("catalog_sources", err);
			setSources({ status: "error", items: [FALLBACK_INFO], all: FALLBACK_ALL });
		});
}

export function useSources(): SourcesState {
	return useSyncExternalStore(
		(listener) => {
			sourceListeners.add(listener);
			return () => {
				sourceListeners.delete(listener);
			};
		},
		() => sources,
	);
}

export function sourceInfo(id: ContentSource, list: SourceInfo[] = sources.items): SourceInfo {
	return list.find((s) => s.id === id) ?? { ...FALLBACK_INFO, id, enabled: false };
}

export function isSourceEnabled(id: ContentSource, list: SourceInfo[] = sources.items): boolean {
	return sourceInfo(id, list).enabled;
}

export function sourceList(list: SourceInfo[] = sources.items): SourceInfo[] {
	return SOURCES.map((id) => list.find((s) => s.id === id)).filter((s): s is SourceInfo => Boolean(s));
}

export function otherSource(
	current: ContentSource,
	list: SourceInfo[] = sources.items,
): ContentSource | null {
	return sourceList(list).find((s) => s.id !== current && s.enabled)?.id ?? null;
}

export type ViewInfo = Omit<SourceInfo, "id"> & {
	id: CatalogSource;
	categories: boolean;
};

export function viewInfo(id: CatalogSource, state: SourcesState = sources): ViewInfo {
	if (id !== "all") return { ...sourceInfo(id, state.items), categories: true };
	const parts = state.all.sources.map((source) => sourceInfo(source, state.items));
	return {
		id,
		enabled: state.all.enabled,
		types: state.all.types,
		sorts: state.all.sorts,
		shaderLoaders: parts.some((info) => info.shaderLoaders),
		maxCategories: null,
		hasFollows: parts.some((info) => info.hasFollows),
		categories: false,
	};
}

export function catalogTypesOf(info: Pick<SourceInfo, "types">): CatalogType[] {
	return CATALOG_TYPES.filter((type) => info.types.includes(type));
}

export function sortsOf(info: Pick<SourceInfo, "sorts">): SearchSort[] {
	return SEARCH_SORTS.filter((sort) => info.sorts.includes(sort));
}

/** `undefined` — поля нет. */
export function loaderOptions(
	type: CatalogType,
	info: Pick<SourceInfo, "shaderLoaders">,
): string[] | undefined {
	if (type === "shader" && !info.shaderLoaders) return undefined;
	return LOADER_FILTERS[type];
}

/* Состояние каталога */

export type CatalogFilters = {
	/** `null` — любая. */
	gameVersion: string | null;
	/** `null` — любой. */
	loader: string | null;
	categories: string[];
};

export type CatalogResults = {
	status: "idle" | "loading" | "ready" | "error";
	/** По нему видно, что ответ устарел. */
	key: string | null;
	/** Пока идёт новый запрос, старый список виден. */
	hitsKey: string | null;
	hits: ProjectHit[];
	/** В «Все» — верхняя оценка (сумма каталогов). */
	total: number;
	offline: boolean;
	cursor: string | null;
	partial: PartialFailure[];
	error: AppError | null;
	loadingMore: boolean;
	moreError: AppError | null;
};

export type CatalogState = {
	source: CatalogSource;
	type: CatalogType;
	query: string;
	/** Запрос, ушедший в ядро: через 300 мс после ввода или по Enter. */
	appliedQuery: string;
	/** Если источник её не поддерживает, действует «Популярные» (`effectiveSort`). */
	sort: SearchSort;
	compatibleOnly: boolean;
	/** Версия и загрузчик общие, категории свои у источника. */
	filters: CatalogFilters;
	contextId: string | null;
	/** Растёт при смене сборки не из шапки — кнопка контекста подсвечивается. */
	contextFlash: number;
	results: CatalogResults;
	scrollTop: number;
};

/** Своё у источника: категории, порции, прокрутка. Лежит, пока показан другой. */
type SourceStash = {
	type: CatalogType;
	categories: string[];
	results: CatalogResults;
	scrollTop: number;
	at: number;
};

function readSorts(): Partial<Record<CatalogSource, SearchSort>> {
	const stored = readStorage(SORT_KEY);
	if (!stored) return {};
	const valid = (value: unknown): value is SearchSort => SEARCH_SORTS.includes(value as SearchSort);
	try {
		const parsed: unknown = JSON.parse(stored);
		if (parsed && typeof parsed === "object") {
			const result: Partial<Record<CatalogSource, SearchSort>> = {};
			for (const id of CATALOG_SOURCES) {
				const value = (parsed as Record<string, unknown>)[id];
				if (valid(value)) result[id] = value;
			}
			return result;
		}
	} catch {
		// Раньше хранилась строка — это сортировка Modrinth.
	}
	return valid(stored) ? { modrinth: stored } : {};
}

function loadSort(source: CatalogSource): SearchSort {
	return readSorts()[source] ?? "relevance";
}

/** Без выбора — «Все»; если «Все» выключен, `setSources` переключит на главный источник, не записывая выбор. */
function loadSource(): CatalogSource {
	const stored = readStorage(SOURCE_KEY);
	if (stored === null) return "all";
	return CATALOG_SOURCES.includes(stored as CatalogSource) ? (stored as CatalogSource) : DEFAULT_SOURCE;
}

const EMPTY_FILTERS: CatalogFilters = { gameVersion: null, loader: null, categories: [] };
const EMPTY_RESULTS: CatalogResults = {
	status: "idle",
	key: null,
	hitsKey: null,
	hits: [],
	total: 0,
	offline: false,
	cursor: null,
	partial: [],
	error: null,
	loadingMore: false,
	moreError: null,
};

const initialSource = loadSource();
let state: CatalogState = {
	source: initialSource,
	type: "mod",
	query: "",
	appliedQuery: "",
	sort: loadSort(initialSource),
	compatibleOnly: true,
	filters: EMPTY_FILTERS,
	contextId: readStorage(INSTANCE_KEY),
	contextFlash: 0,
	results: EMPTY_RESULTS,
	scrollTop: 0,
};
let stash: Partial<Record<CatalogSource, SourceStash>> = {};
const listeners = new Set<() => void>();

function set(next: Partial<CatalogState>) {
	state = { ...state, ...next };
	for (const listener of listeners) listener();
}

function setResults(next: Partial<CatalogResults>) {
	set({ results: { ...state.results, ...next } });
}

let requestSeq = 0;

function switchSource(next: CatalogSource, persist: boolean) {
	if (persist) writeStorage(SOURCE_KEY, next);
	if (next === state.source) return;
	// Ответы, которые ещё идут, относятся к прежнему источнику — не применяем.
	requestSeq++;
	const current = state.results;
	stash = {
		...stash,
		[state.source]: {
			type: state.type,
			categories: state.filters.categories,
			// Недождавшийся запрос при возврате нужно повторить.
			results:
				current.status === "loading"
					? {
							...current,
							status: current.hitsKey ? "ready" : "idle",
							key: null,
							loadingMore: false,
						}
					: { ...current, loadingMore: false },
			scrollTop: state.scrollTop,
			at: Date.now(),
		},
	};
	const info = viewInfo(next);
	const type = info.types.includes(state.type) ? state.type : "mod";
	const saved = stash[next];
	const usable = saved && saved.type === type;
	const results = usable
		? Date.now() - saved.at < STASH_FRESH
			? saved.results
			: { ...saved.results, key: null }
		: EMPTY_RESULTS;
	set({
		source: next,
		type,
		sort: loadSort(next),
		// Категории источника лежат в его `stash` и вернутся вместе с ним.
		filters: { ...state.filters, categories: usable && info.categories ? saved.categories : [] },
		results,
		scrollTop: usable ? saved.scrollTop : 0,
	});
}

export function setCatalogSource(next: CatalogSource) {
	if (!viewInfo(next).enabled) return;
	switchSource(next, true);
}

export function setCatalogType(type: CatalogType) {
	if (type === state.type) return;
	// Категории и загрузчики у типов свои; версию игры оставляем.
	set({ type, filters: { ...state.filters, loader: null, categories: [] }, scrollTop: 0 });
}

export function setCatalogQuery(query: string) {
	set({ query });
}

export function applyCatalogQuery(query = state.query) {
	if (query.trim() === state.appliedQuery) return;
	set({ appliedQuery: query.trim(), scrollTop: 0 });
}

export function setCatalogSort(sort: SearchSort) {
	writeStorage(SORT_KEY, JSON.stringify({ ...readSorts(), [state.source]: sort }));
	set({ sort, scrollTop: 0 });
}

export function effectiveSort(
	s: CatalogState,
	info: Pick<SourceInfo, "sorts"> = viewInfo(s.source),
): SearchSort {
	return info.sorts.includes(s.sort) ? s.sort : "relevance";
}

export function setCompatibleOnly(value: boolean) {
	set({ compatibleOnly: value, scrollTop: 0 });
}

export function setCatalogFilters(patch: Partial<CatalogFilters>) {
	set({ filters: { ...state.filters, ...patch }, scrollTop: 0 });
}

export function resetCatalogFilters() {
	set({ filters: EMPTY_FILTERS, query: "", appliedQuery: "", scrollTop: 0 });
}

/** `persist` — запомнить выбор; без него — только на сессию. `flash` — подсветить кнопку контекста. */
export function setCatalogContext(id: string | null, options?: { persist?: boolean; flash?: boolean }) {
	if (options?.persist) writeStorage(INSTANCE_KEY, id);
	if (id === state.contextId) return;
	set({
		contextId: id,
		contextFlash: options?.flash ? state.contextFlash + 1 : state.contextFlash,
		scrollTop: 0,
	});
}

export function saveCatalogScroll(top: number) {
	state = { ...state, scrollTop: top };
}

export function contextInstance(instances: Instance[], id = state.contextId): Instance | null {
	return id ? (instances.find((i) => i.id === id) ?? null) : null;
}

export function instanceLoaders(type: CatalogType, instance: Instance, shaderLoaders = true): string[] {
	const kind = instance.loader.kind;
	if (type === "mod") {
		if (kind === "vanilla") return [];
		return kind === "quilt" ? ["quilt", "fabric"] : [kind];
	}
	if (type === "shader") return kind === "vanilla" || !shaderLoaders ? [] : ["iris"];
	return [];
}

/** У модпаков сборки-контекста нет: модпак всегда ставится новой сборкой. */
export function effectiveFilters(s: CatalogState, instance: Instance | null) {
	const info = viewInfo(s.source);
	const locked = Boolean(instance && s.compatibleOnly && s.type !== "modpack");
	if (locked && instance) {
		return {
			locked,
			gameVersions: [instance.gameVersion],
			loaders: instanceLoaders(s.type, instance, info.shaderLoaders),
		};
	}
	return {
		locked,
		gameVersions: s.filters.gameVersion ? [s.filters.gameVersion] : [],
		loaders: s.filters.loader && loaderOptions(s.type, info) ? [s.filters.loader] : [],
	};
}

export function searchParams(s: CatalogState, instance: Instance | null): SearchParams {
	const { gameVersions, loaders } = effectiveFilters(s, instance);
	return {
		source: s.source,
		type: s.type,
		query: s.appliedQuery || undefined,
		gameVersions: gameVersions.length ? gameVersions : undefined,
		loaders: loaders.length ? loaders : undefined,
		categories: s.source !== "all" && s.filters.categories.length ? s.filters.categories : undefined,
		sort: effectiveSort(s),
		limit: PAGE_SIZE,
		instanceId: s.type === "modpack" ? undefined : instance?.id,
	};
}

export async function searchCatalog(params: SearchParams, force = false) {
	const key = JSON.stringify(params);
	if (!force && state.results.key === key && state.results.status !== "error") return;
	const seq = ++requestSeq;
	setResults({ status: "loading", key, error: null, moreError: null, loadingMore: false });
	try {
		const result = await withDeadline(
			catalogApi.search(params.source === "all" ? params : { ...params, offset: 0 }),
		);
		if (seq !== requestSeq) return;
		setResults({
			status: "ready",
			hitsKey: key,
			hits: result.hits,
			total: result.total,
			offline: result.offline,
			cursor: result.cursor ?? null,
			partial: result.partial ?? [],
			error: null,
		});
	} catch (err) {
		if (seq !== requestSeq) return;
		console.error("catalog_search", err);
		setResults({
			status: "error",
			hitsKey: null,
			hits: [],
			total: 0,
			offline: false,
			cursor: null,
			partial: [],
			error: toAppError(err),
		});
	}
}

export function hasMoreResults(results: CatalogResults, source: CatalogSource): boolean {
	if (source === "all") return results.cursor !== null;
	const count = results.hits.length;
	return count < results.total && count < MAX_OFFSET;
}

/** Id у разных каталогов могут совпасть. */
export const hitKey = (hit: Pick<ProjectHit, "source" | "id">) => `${hit.source}:${hit.id}`;

export async function loadMoreCatalog(params: SearchParams) {
	const results = state.results;
	if (results.status !== "ready" || results.loadingMore) return;
	// Курсор и смещение относятся к показанному списку: другому запросу они не годятся.
	if (results.hitsKey !== JSON.stringify(params)) return;
	const all = params.source === "all";
	if (!hasMoreResults(results, all ? "all" : DEFAULT_SOURCE)) return;
	const seq = requestSeq;
	setResults({ loadingMore: true, moreError: null });
	try {
		const result = await withDeadline(
			catalogApi.search(
				all
					? { ...params, cursor: results.cursor ?? undefined }
					: { ...params, offset: results.hits.length },
			),
		);
		if (seq !== requestSeq || state.results.hitsKey !== results.hitsKey) return;
		// Проекты могут сдвинуться между порциями — убираем повторы.
		const seen = new Set(state.results.hits.map(hitKey));
		setResults({
			hits: [...state.results.hits, ...result.hits.filter((h) => !seen.has(hitKey(h)))],
			total: result.total,
			offline: state.results.offline || result.offline,
			cursor: result.cursor ?? null,
			partial: result.partial ?? [],
			loadingMore: false,
		});
	} catch (err) {
		if (seq !== requestSeq) return;
		setResults({ loadingMore: false, moreError: toAppError(err) });
	}
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

export function useCatalog(): CatalogState {
	return useSyncExternalStore(subscribe, () => state);
}

export function getCatalog(): CatalogState {
	return state;
}

/* Категории: кэш по источнику и типу на время работы. */

export type CategoriesEntry =
	{ status: "loading" } | { status: "ready"; items: CategoryTag[] } | { status: "error"; error: AppError };

let categories: Record<string, CategoriesEntry> = {};
const categoryListeners = new Set<() => void>();

function categoriesKey(type: CatalogType, source: ContentSource) {
	return `${source}:${type}`;
}

function setCategories(key: string, entry: CategoriesEntry) {
	categories = { ...categories, [key]: entry };
	for (const listener of categoryListeners) listener();
}

export function loadCategories(type: CatalogType, source: ContentSource, force = false) {
	const key = categoriesKey(type, source);
	const entry = categories[key];
	if (!force && entry && entry.status !== "error") return;
	setCategories(key, { status: "loading" });
	withDeadline(catalogApi.categories(type, source))
		.then((items) => setCategories(key, { status: "ready", items }))
		.catch((err: unknown) => setCategories(key, { status: "error", error: toAppError(err) }));
}

function subscribeCategories(listener: () => void) {
	categoryListeners.add(listener);
	return () => {
		categoryListeners.delete(listener);
	};
}

export function useCategories(type: CatalogType, source: ContentSource): CategoriesEntry | undefined {
	return useSyncExternalStore(subscribeCategories, () => categories[categoriesKey(type, source)]);
}

export function useCategoryCache(): Record<string, CategoriesEntry> {
	return useSyncExternalStore(subscribeCategories, () => categories);
}

type CategoryKey = keyof Dictionary["catalog"]["category"];
type CfCategories = Dictionary["catalog"]["cfCategory"];
type CfCategoryKey = {
	[T in keyof CfCategories]: `${T}.${keyof CfCategories[T] & string}`;
}[keyof CfCategories];

function sourceCategoryName(slug: string, source: ContentSource, type?: CatalogType): string | null {
	const own = type ? categories[categoriesKey(type, source)] : undefined;
	if (own?.status === "ready") {
		const tag = own.items.find((item) => item.slug === slug);
		if (tag) return tag.name || null;
	}
	for (const [key, entry] of Object.entries(categories)) {
		if (!key.startsWith(`${source}:`) || entry.status !== "ready") continue;
		const tag = entry.items.find((item) => item.slug === slug);
		if (tag) return tag.name || null;
	}
	return null;
}

function prettySlug(slug: string): string {
	const text = slug.replace(/-/g, " ");
	return text.charAt(0).toUpperCase() + text.slice(1);
}

/** Slug уникален только внутри типа. Названий модов и брендов в переводах нет — остаются как у источника. */
export function categoryLabel(
	slug: string,
	source: ContentSource = DEFAULT_SOURCE,
	type?: CatalogType,
): string {
	if (source === "modrinth") {
		if (Object.hasOwn(ru.catalog.category, slug))
			return i18n.t(`catalog.category.${slug as CategoryKey}`);
		return prettySlug(slug);
	}
	const name = sourceCategoryName(slug, source, type);
	if (i18n.language === "en" && name) return name;
	if (type && Object.hasOwn(ru.catalog.cfCategory[type], slug))
		return i18n.t(`catalog.cfCategory.${`${type}.${slug}` as CfCategoryKey}`);
	return name ?? prettySlug(slug);
}

export function searchModpacks(query: string, source: ContentSource = DEFAULT_SOURCE) {
	setCatalogSource(source);
	setCatalogType("modpack");
	set({ query, appliedQuery: query.trim(), scrollTop: 0 });
	navigate("/catalog");
}

/* Тип вкладки, под которым показывать проект (датапак бывает выложен модом). */

const projectTypes = new Map<string, CatalogType>();
const projectTabs = new Map<string, "versions">();
const projectUrls = new Map<string, string>();
const projectTwins = new Map<string, AlsoOn[]>();

/** `url` — страница на сайте на случай, если проект не нашёлся. `alsoOn` — двойники склеенной строки «Все». */
export function openProject(
	project: { source: ContentSource; id: string },
	type?: CatalogType,
	options?: { tab?: "versions"; url?: string; alsoOn?: AlsoOn[] },
) {
	if (type) projectTypes.set(project.id, type);
	if (options?.tab) projectTabs.set(project.id, options.tab);
	else projectTabs.delete(project.id);
	if (options?.url) projectUrls.set(project.id, options.url);
	if (options?.alsoOn?.length) projectTwins.set(project.id, options.alsoOn);
	else projectTwins.delete(project.id);
	navigate(projectPath(project.source, project.id));
}

export function projectUrlHint(id: string): string | undefined {
	return projectUrls.get(id);
}

export function projectAlsoOnHint(id: string): AlsoOn[] {
	return projectTwins.get(id) ?? [];
}

export function projectTabHint(id: string): "versions" | undefined {
	return projectTabs.get(id);
}

export function clearProjectTabHint(id: string) {
	projectTabs.delete(id);
}

export function projectTypeHint(id: string): CatalogType | undefined {
	return projectTypes.get(id);
}
