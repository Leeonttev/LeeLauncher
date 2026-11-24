import {
	FolderOpen,
	History,
	LoaderCircle,
	OctagonX,
	Plus,
	RotateCcw,
	RotateCw,
	Search,
	Shirt,
	Undo2,
	X,
} from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { AccountAvatar } from "@/components/accounts/account-avatar";
import { CenteredState } from "@/components/centered-state";
import { ErrorDetails } from "@/components/error-details";
import { onCoverButton } from "@/components/instances/play-button";
import { ScrollContainerContext, useScrollContainer } from "@/components/layout/scroll-container";
import { PixelGrid } from "@/components/pixel-grid";
import { SearchField } from "@/components/search-field";
import { Segments } from "@/components/segments";
import { SkinCard } from "@/components/skins/skin-card";
import { CapeFlat } from "@/components/skins/skin-flat";
import { SkinStage } from "@/components/skins/skin-stage";
import { SkinCardSkeleton } from "@/components/skins/skins-skeleton";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { overlayOpen, typingTarget } from "@/hooks/use-create-shortcut";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { activeAccount, openAddAccountDialog, openReloginDialog, useAccounts } from "@/lib/accounts";
import type { Account, SkinEntry, SkinModel } from "@/lib/api";
import { errorText } from "@/lib/errors";
import { ariaShortcut, isPrimaryMod, noModifiers } from "@/lib/platform";
import {
	inAccountHistory,
	isHistorySkin,
	loadProfile,
	loadSkinLibrary,
	pickSkinFiles,
	refreshSkins,
	revertDraft,
	SEARCH_FROM,
	setDraftCape,
	setDraftModel,
	setSkinsFilter,
	setSkinsPageAccount,
	setSkinsQuery,
	skinDisplayName,
	skinRecency,
	stageView,
	tryOnSkin,
	useSkins,
	waitTimeText,
	wearDraft,
	wearSkin,
	type SkinFilter,
	type SkinsStore,
	type StageView,
} from "@/lib/skins";
import { cn } from "@/lib/utils";

const COLUMN = 152;
const GAP = 12;
/** Высота карточки и зазор. */
const ROW_HEIGHT = 216 + GAP;
const VIRTUAL_FROM = 200;
const SKELETONS = 12;
/** Уже — «Добавить файл» только иконкой. */
const NARROW_LIBRARY = 600;
/** У `SelectItem` не может быть пустого значения. */
const NO_CAPE = "none";
const HINT_ID = "skins-main-hint";

/** Состояние аккаунта для подиума, кнопки и карточек. */
type Ctx = {
	store: SkinsStore;
	account: Account | null;
	needsRelogin: boolean;
	view: StageView;
};

/** Пока идёт первый запрос профиля, менять и снимать скин рано. */
function isFirstProfileLoad(store: SkinsStore, accountId: string) {
	const state = store.profiles[accountId];
	return !state || state.status === "loading";
}

/** Надеть с карточки можно, только когда ничего не мешает и главной кнопке. */
function wearableAccount({ store, account, needsRelogin }: Ctx): Account | null {
	if (!account || needsRelogin) return null;
	if (store.applying[account.id] || store.resetting[account.id] || store.waits[account.id]) return null;
	return isFirstProfileLoad(store, account.id) ? null : account;
}

function focusMainButton() {
	document.querySelector<HTMLElement>("[data-skin-main]")?.focus();
}

export function SkinsPage() {
	const { t } = useTranslation();
	const store = useSkins();
	const accounts = useAccounts();
	const account = activeAccount(accounts);
	const searchRef = useRef<HTMLInputElement>(null);

	useEffect(() => {
		refreshSkins();
	}, []);

	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			const primary = isPrimaryMod(event);
			const find = event.code === "KeyF" && primary;
			const open = event.code === "KeyO" && primary;
			const slash =
				(event.code === "Slash" || event.code === "NumpadDivide") &&
				noModifiers(event) &&
				!event.shiftKey;
			if (!find && !open && !slash) return;
			// Иначе Ctrl+F и Ctrl+O перехватит сам WebView.
			if (find || open) event.preventDefault();
			if (overlayOpen() || event.repeat) return;
			if (open) {
				void pickSkinFiles();
				return;
			}
			if (typingTarget(event.target)) return;
			event.preventDefault();
			searchRef.current?.focus();
			searchRef.current?.select();
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);

	const accountId = account?.id ?? null;
	const view = stageView(store, accountId);
	const needsRelogin = Boolean(account && (account.needsRelogin || store.relogin[account.id]));
	const ctx: Ctx = { store, account, needsRelogin, view };

	// Профиль — при входе на страницу и смене аккаунта. Частоту запросов бережёт ядро.
	useEffect(() => {
		setSkinsPageAccount(accountId);
		if (accountId && !needsRelogin) void loadProfile(accountId);
		return () => setSkinsPageAccount(null);
	}, [accountId, needsRelogin]);

	return (
		<div className="@container/skins flex min-h-0 flex-1 gap-8 px-8 py-6">
			<h1 className="sr-only">{t("skins.title")}</h1>
			<div className="flex min-h-0 w-80 shrink-0 flex-col gap-4 @min-[1280px]/skins:w-[380px]">
				<Podium ctx={ctx} />
				<Controls ctx={ctx} />
			</div>
			<Library ctx={ctx} searchRef={searchRef} />
		</div>
	);
}

function RetryOnCover({
	account,
	wait,
	round,
}: {
	account: Account;
	wait: number | undefined;
	round?: boolean;
}) {
	const { t } = useTranslation();
	const label = wait ? t("skins.action.wait", { time: waitTimeText(wait) }) : t("common.retry");
	const retry = () => {
		if (!wait) void loadProfile(account.id, true);
	};
	if (!round)
		return (
			<button
				type="button"
				aria-disabled={wait ? true : undefined}
				className={cn(onCoverButton, "h-8 px-3 text-[13px] aria-disabled:opacity-70")}
				onClick={retry}
			>
				<RotateCw />
				{label}
			</button>
		);
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={label}
					aria-disabled={wait ? true : undefined}
					className={cn(onCoverButton, "size-8 aria-disabled:opacity-70")}
					onClick={retry}
				>
					<RotateCw />
				</button>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}

function Podium({ ctx }: { ctx: Ctx }) {
	const { t } = useTranslation();
	const { store, account, needsRelogin, view } = ctx;
	const { current, trying } = view;
	const wait = account ? store.waits[account.id] : undefined;
	const applying = account ? Boolean(store.applying[account.id]) : false;
	const celebration = account && store.celebration?.accountId === account.id ? store.celebration : null;
	const state = account ? store.profiles[account.id] : undefined;
	const error = state?.error ?? null;
	const offline = error?.code === "network";
	const entryName = view.entry ? skinDisplayName(view.entry, account?.id ?? null) : null;
	const name = entryName ?? (view.texture && account ? account.username : null);

	let plate: { title: string; subtitle: string; retry: boolean } | null = null;
	let center: ReactNode = null;
	if (account) {
		if (trying) {
			plate = { title: t("skins.stage.trying"), subtitle: entryName ?? account.username, retry: false };
		} else if (current?.texture) {
			const stale = Boolean(error) && !needsRelogin;
			plate = {
				title: current.isDefault ? t("skins.stage.wornDefault") : t("skins.stage.worn"),
				subtitle: stale
					? offline
						? t("skins.stage.staleOffline")
						: t("skins.stage.staleError")
					: account.username,
				retry: stale,
			};
		} else if (!needsRelogin) {
			const loading = !state || state.status === "loading";
			center = loading ? (
				<p className="flex items-center gap-2 rounded-lg bg-cover-scrim/70 px-3 py-1.5 text-[13px]/[18px] font-semibold text-on-cover">
					<LoaderCircle className="animate-spin" />
					{t("skins.stage.loading")}
				</p>
			) : (
				<>
					<p className="rounded-lg bg-cover-scrim/70 px-3 py-1.5 text-center text-[13px]/[18px] font-semibold text-on-cover">
						{offline ? t("skins.stage.noProfileOffline") : t("skins.stage.noProfile")}
					</p>
					<RetryOnCover account={account} wait={wait} />
				</>
			);
		}
	}

	const revert = () => {
		revertDraft();
		focusMainButton();
	};

	return (
		<SkinStage
			texture={view.texture}
			model={view.model}
			cape={view.capeTexture}
			capeTurn={store.capeTurn}
			ariaLabel={name ? t("skins.stage.aria", { name }) : t("skins.stage.ariaEmpty")}
			celebration={celebration}
			onEscape={trying && !applying ? revertDraft : undefined}
			center={center}
			bottom={
				trying &&
				!applying && (
					<button
						type="button"
						className={cn(onCoverButton, "h-8 px-3 text-[13px]")}
						onClick={revert}
					>
						<Undo2 />
						{t("skins.action.revert")}
					</button>
				)
			}
			overlay={
				account &&
				plate && (
					<div className="flex items-center gap-1.5">
						<div
							title={plate.subtitle}
							className="flex min-w-0 items-center gap-2 rounded-lg bg-cover-scrim/70 py-1.5 pr-2.5 pl-1.5 text-on-cover"
						>
							<AccountAvatar account={{ ...account, needsRelogin }} size={24} surface="cover" />
							<span aria-live="polite" className="min-w-0">
								<span className="block truncate text-[13px]/4 font-semibold">
									{plate.title}
								</span>
								<span className="block truncate text-xs/[14px] opacity-80">
									{plate.subtitle}
								</span>
							</span>
						</div>
						{plate.retry && <RetryOnCover account={account} wait={wait} round />}
					</div>
				)
			}
		/>
	);
}

function CapeRow({ view, loading }: { view: StageView; loading: boolean }) {
	const { t } = useTranslation();
	const label = <span className="text-[13px]/[18px] font-semibold">{t("skins.cape.label")}</span>;
	if (loading)
		return (
			<div className="flex items-center justify-between gap-3">
				{label}
				<Skeleton className="h-8 w-44 rounded-lg" />
			</div>
		);
	if (!view.capes.length) return null;
	const selected = view.capes.find((cape) => cape.id === view.capeId);
	return (
		<div className="flex items-center justify-between gap-3">
			{label}
			<Select
				value={selected?.id ?? NO_CAPE}
				onValueChange={(value) => setDraftCape(value === NO_CAPE ? null : value)}
			>
				<SelectTrigger aria-label={t("skins.cape.label")} className="h-8 w-44">
					<SelectValue>
						<span className="truncate">{selected?.name ?? t("skins.cape.none")}</span>
					</SelectValue>
				</SelectTrigger>
				<SelectContent position="popper">
					<SelectItem value={NO_CAPE} className="h-10">
						<span className="flex h-8 w-5 shrink-0 items-center justify-center">
							<X />
						</span>
						{t("skins.cape.none")}
					</SelectItem>
					{view.capes.map((cape) => (
						<SelectItem key={cape.id} value={cape.id} className="h-10">
							<CapeFlat texture={cape.texture} />
							<span className="truncate">{cape.name}</span>
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);
}

/** Одна главная кнопка: не пересоздаётся, меняются текст и `aria-disabled`. */
function Controls({ ctx }: { ctx: Ctx }) {
	const { t } = useTranslation();
	const { store, account, needsRelogin, view } = ctx;
	const mainRef = useRef<HTMLButtonElement>(null);
	const { current, trying } = view;
	const applying = account ? Boolean(store.applying[account.id]) : false;
	const resetting = account ? Boolean(store.resetting[account.id]) : false;
	const wait = account ? store.waits[account.id] : undefined;
	const profileState = account ? store.profiles[account.id] : undefined;
	const capesLoading = !current && (!profileState || profileState.status === "loading");
	const firstLoad = Boolean(account && isFirstProfileLoad(store, account.id));
	// Рук нет у манекена; обычный скин загружать как свой незачем.
	const armsLocked = !view.texture || (!view.picked && Boolean(current?.isDefault));

	let label: string;
	let busy = false;
	let inactive = false;
	let hint: string | null = null;
	let run: (() => void) | null = null;

	if (!account) {
		label = t("skins.action.addAccount");
		run = () => openAddAccountDialog(mainRef.current);
	} else if (needsRelogin) {
		label = t("skins.action.relogin");
		run = () => openReloginDialog(account, mainRef.current);
	} else {
		label = t("skins.action.wear");
		if (applying || resetting) {
			label = resetting ? t("skins.action.takingOff") : t("skins.action.wearing");
			busy = true;
			inactive = true;
		} else if (wait) {
			label = t("skins.action.wait", { time: waitTimeText(wait) });
			inactive = true;
		} else if (firstLoad) {
			inactive = true;
			hint = t("skins.hint.loading");
		} else if (trying) {
			run = () => wearDraft(account, mainRef.current);
		} else {
			inactive = true;
			hint = current?.texture ? t("skins.hint.alreadyWorn") : t("skins.hint.pickFirst");
		}
	}
	if (account && store.wornHint === account.id && !trying) hint = t("skins.hint.wornInGame");

	return (
		<div className="relative flex shrink-0 flex-col gap-3">
			<div className="flex items-center justify-between gap-3">
				<span className="text-[13px]/[18px] font-semibold">{t("skins.model.label")}</span>
				<div aria-disabled={armsLocked || undefined} className={cn(armsLocked && "opacity-50")}>
					<Segments
						label={t("skins.model.label")}
						value={view.model}
						layoutId="skin-model"
						disabled={armsLocked}
						onChange={(value) => setDraftModel(value as SkinModel)}
						options={[
							{ value: "classic", content: t("skins.model.classic") },
							{ value: "slim", content: t("skins.model.slim") },
						]}
					/>
				</div>
			</div>
			{account && !needsRelogin && <CapeRow view={view} loading={capesLoading} />}
			<Button
				ref={mainRef}
				size="lg"
				data-skin-main
				aria-disabled={inactive || undefined}
				aria-describedby={hint ? HINT_ID : undefined}
				className="w-full aria-disabled:cursor-default aria-disabled:opacity-50 aria-disabled:hover:bg-primary aria-disabled:active:scale-100"
				onClick={() => run?.()}
			>
				{busy && <LoaderCircle className="animate-spin" />}
				<span className="truncate">{label}</span>
			</Button>
			{/* Строка лежит в нижнем отступе страницы: иначе сцена прыгала бы по высоте, когда она появляется. */}
			<p
				id={HINT_ID}
				className="absolute inset-x-0 top-full mt-1 truncate text-center text-xs/4 text-muted-foreground"
			>
				{hint}
			</p>
		</div>
	);
}

/** Надетый — первым, дальше новые сверху: по «последний раз видели» или дате добавления. */
function sortedSkins(items: SkinEntry[], wornId: string | null | undefined) {
	return items
		.map((skin, index) => ({ skin, index, recency: skinRecency(skin) }))
		.sort((a, b) => {
			const worn = Number(b.skin.id === wornId) - Number(a.skin.id === wornId);
			return worn || b.recency - a.recency || a.index - b.index;
		})
		.map((entry) => entry.skin);
}

function useWidth(ref: RefObject<HTMLElement | null>) {
	const [width, setWidth] = useState<number | null>(null);
	useEffect(() => {
		const node = ref.current;
		if (!node) return;
		const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
		observer.observe(node);
		return () => observer.disconnect();
	}, [ref]);
	return width;
}

const chipClass =
	"inline-flex h-8 shrink-0 items-center rounded-full border border-border bg-card px-3 text-[13px] font-medium whitespace-nowrap tabular-nums transition-[background-color,border-color,color] duration-(--duration-fast) ease-out hover:border-border-hover aria-checked:border-transparent aria-checked:bg-primary-soft aria-checked:text-primary-text aria-checked:focus-visible:outline-foreground";

function FilterChips({ value, counts }: { value: SkinFilter; counts: Record<SkinFilter, number> }) {
	const { t } = useTranslation();
	const options: { value: SkinFilter; label: string }[] = [
		{ value: "all", label: t("skins.filter.all", { count: counts.all }) },
		{ value: "uploaded", label: t("skins.filter.uploaded", { count: counts.uploaded }) },
		{ value: "history", label: t("skins.filter.history", { count: counts.history }) },
	];
	return (
		<RadioGroupPrimitive.Root
			aria-label={t("skins.filter.label")}
			value={value}
			orientation="horizontal"
			onValueChange={(next) => setSkinsFilter(next as SkinFilter)}
			className="flex min-w-0 items-center gap-2"
		>
			{options.map((option) => (
				<RadioGroupPrimitive.Item key={option.value} value={option.value} className={chipClass}>
					{option.label}
				</RadioGroupPrimitive.Item>
			))}
		</RadioGroupPrimitive.Root>
	);
}

function AddFileButton({ narrow }: { narrow: boolean }) {
	const { t } = useTranslation();
	const button = (
		<Button
			variant="outline"
			size={narrow ? "icon-sm" : "sm"}
			data-skins-add
			aria-label={narrow ? t("skins.add") : undefined}
			aria-keyshortcuts={ariaShortcut("O")}
			onClick={() => void pickSkinFiles()}
		>
			<Plus />
			{!narrow && t("skins.add")}
		</Button>
	);
	if (!narrow) return button;
	return (
		<Tooltip>
			<TooltipTrigger asChild>{button}</TooltipTrigger>
			<TooltipContent>{t("skins.add")}</TooltipContent>
		</Tooltip>
	);
}

function Library({ ctx, searchRef }: { ctx: Ctx; searchRef: RefObject<HTMLInputElement | null> }) {
	const { t } = useTranslation();
	const { store, account, view } = ctx;
	const rootRef = useRef<HTMLDivElement>(null);
	const scrollRef = useRef<HTMLDivElement>(null);
	const width = useWidth(rootRef);
	const narrow = width !== null && width < NARROW_LIBRARY;
	const loading = store.status === "idle" || store.status === "loading";
	const lateLoading = useDelayedFlag(loading, 150);
	const accountId = account?.id ?? null;

	const items = sortedSkins(store.items, view.current?.skinId);
	const counts: Record<SkinFilter, number> = {
		all: items.length,
		uploaded: items.filter((skin) => !isHistorySkin(skin)).length,
		history: items.filter((skin) => inAccountHistory(skin, accountId)).length,
	};
	// Чипов нет, пока история пуста: фильтр тогда — «Все».
	const showFilters = counts.history > 0;
	const filter = showFilters ? store.filter : "all";
	const inFilter =
		filter === "uploaded"
			? items.filter((skin) => !isHistorySkin(skin))
			: filter === "history"
				? items.filter((skin) => inAccountHistory(skin, accountId))
				: items;
	const query = store.query.trim().toLocaleLowerCase();
	const visible = query
		? inFilter.filter((skin) => skinDisplayName(skin, accountId).toLocaleLowerCase().includes(query))
		: inFilter;

	useEffect(() => {
		if (store.addedSeq) scrollRef.current?.scrollTo({ top: 0 });
	}, [store.addedSeq]);

	const pickButton = (
		<Button variant="outline" onClick={() => void pickSkinFiles()}>
			<FolderOpen />
			{t("skins.empty.pick")}
		</Button>
	);

	let body;
	if (store.status === "error") {
		body = (
			<CenteredState
				heading="h2"
				icon={OctagonX}
				tone="destructive"
				title={t("skins.libraryError")}
				description={store.error ? errorText(store.error) : t("errors.fallback")}
				action={
					<Button variant="outline" onClick={() => void loadSkinLibrary()}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				}
				footer={store.error?.details ? <ErrorDetails details={store.error.details} /> : undefined}
			/>
		);
	} else if (loading) {
		body = lateLoading ? (
			<div aria-busy className="min-h-0 flex-1 overflow-hidden pt-1">
				<div className="grid grid-cols-[repeat(auto-fill,152px)] gap-3">
					{Array.from({ length: SKELETONS }, (_, index) => (
						<SkinCardSkeleton key={index} />
					))}
				</div>
			</div>
		) : (
			<div className="flex-1" />
		);
	} else if (!items.length) {
		body = (
			<CenteredState
				heading="h2"
				icon={Shirt}
				title={t("skins.empty.title")}
				description={t("skins.empty.text")}
				action={pickButton}
			/>
		);
	} else if (!inFilter.length) {
		body =
			filter === "history" ? (
				<CenteredState
					heading="h2"
					icon={History}
					title={t("skins.empty.historyTitle")}
					description={t("skins.empty.historyText")}
				/>
			) : (
				<CenteredState
					heading="h2"
					icon={Shirt}
					title={t("skins.empty.uploadedTitle")}
					description={t("skins.empty.text")}
					action={pickButton}
				/>
			);
	} else if (!visible.length) {
		body = (
			<CenteredState
				heading="h2"
				icon={Search}
				title={t("skins.notFound")}
				action={
					<Button variant="outline" onClick={() => setSkinsQuery("")}>
						{t("skins.clearSearch")}
					</Button>
				}
			/>
		);
	} else {
		body = (
			// Отступ слева и сверху — место под кольцо фокуса крайних карточек.
			<div ref={scrollRef} className="-mr-8 -ml-1 min-h-0 flex-1 overflow-y-auto pt-1 pr-8 pb-1 pl-1">
				<ScrollContainerContext value={scrollRef}>
					<SkinGrid items={visible} ctx={ctx} />
				</ScrollContainerContext>
			</div>
		);
	}

	return (
		<section
			ref={rootRef}
			data-skins-drop-zone
			aria-labelledby="skins-mine-title"
			className="flex min-w-0 flex-1 flex-col"
		>
			<div className="flex h-11 shrink-0 items-center justify-between gap-3">
				<h2
					id="skins-mine-title"
					className="flex min-w-0 items-baseline gap-2 text-xl/[26px] font-semibold tracking-[-0.015em]"
				>
					<span className="truncate">{t("skins.mine")}</span>
					{items.length > 0 && (
						<span className="text-[13px] font-normal tracking-normal text-muted-foreground tabular-nums">
							{items.length}
						</span>
					)}
				</h2>
				<div className="flex shrink-0 items-center gap-2">
					{(items.length >= SEARCH_FROM || store.query) && (
						<SearchField
							inputRef={searchRef}
							value={store.query}
							onChange={setSkinsQuery}
							placeholder={t("skins.search")}
							keyShortcuts={`${ariaShortcut("F")} /`}
							className={narrow ? "w-44" : "w-56"}
						/>
					)}
					<AddFileButton narrow={narrow} />
				</div>
			</div>
			{loading && lateLoading ? (
				<div className="flex h-12 shrink-0 items-center gap-2">
					{[0, 1, 2].map((index) => (
						<Skeleton key={index} className="h-8 w-20 rounded-full" />
					))}
				</div>
			) : showFilters && store.status === "ready" ? (
				<div className="flex h-12 shrink-0 items-center gap-2">
					<FilterChips value={filter} counts={counts} />
				</div>
			) : (
				<div className="h-3 shrink-0" />
			)}
			{body}
		</section>
	);
}

/** Нужно и `PixelGrid`, и стрелкам вверх/вниз, и рядам виртуального списка. */
function useColumns(ref: RefObject<HTMLElement | null>) {
	const [columns, setColumns] = useState(1);
	useEffect(() => {
		const node = ref.current;
		if (!node) return;
		const observer = new ResizeObserver(([entry]) => {
			setColumns(Math.max(1, Math.floor((entry.contentRect.width + GAP) / (COLUMN + GAP))));
		});
		observer.observe(node);
		return () => observer.disconnect();
	}, [ref]);
	return columns;
}

/** Как и стрелки Radix, сразу примеряет. */
function moveToCard(id: string | undefined) {
	if (!id) return;
	const card = document.querySelector<HTMLElement>(`[data-skin-card="${id}"]`);
	card?.focus();
	card?.click();
}

function SkinGrid({ items, ctx }: { items: SkinEntry[]; ctx: Ctx }) {
	const { t } = useTranslation();
	const { store, account, view } = ctx;
	const rootRef = useRef<HTMLDivElement>(null);
	const columns = useColumns(rootRef);
	const checkedId = view.entry?.id ?? null;
	const checkedVisible = items.some((skin) => skin.id === checkedId);
	const tabStop = checkedVisible ? checkedId : items[0]?.id;
	const virtual = items.length > VIRTUAL_FROM;
	const takeOffLocked = Boolean(account && isFirstProfileLoad(store, account.id));
	const wearable = wearableAccount(ctx);
	const fresh = new Set(store.fresh);

	const card = (skin: SkinEntry) => (
		<SkinCard
			key={skin.id}
			skin={skin}
			model={checkedId === skin.id ? view.model : skin.model}
			accountId={account?.id ?? null}
			wornBy={view.current?.skinId === skin.id ? account : null}
			takeOffLocked={takeOffLocked}
			onWear={wearable ? (focus) => wearSkin(wearable, skin, focus) : null}
			menuTabbable={tabStop === skin.id}
			fresh={fresh.has(skin.id)}
			onMoveVertical={(id, direction) => {
				const index = items.findIndex((other) => other.id === id);
				moveToCard(items[index + direction * columns]?.id);
			}}
		/>
	);

	return (
		<RadioGroupPrimitive.Root
			ref={rootRef}
			aria-label={t("skins.mine")}
			value={checkedId ?? ""}
			loop={false}
			onValueChange={(id) => {
				const skin = items.find((other) => other.id === id);
				if (skin) tryOnSkin(skin);
			}}
		>
			{virtual ? (
				<VirtualRows items={items} columns={columns} render={card} />
			) : (
				<PixelGrid
					columns={columns}
					minColumn={COLUMN}
					maxColumn={COLUMN}
					gap={GAP}
					className="grid-cols-[repeat(auto-fill,152px)] gap-y-3"
				>
					{items.map(card)}
				</PixelGrid>
			)}
		</RadioGroupPrimitive.Root>
	);
}

function VirtualRows({
	items,
	columns,
	render,
}: {
	items: SkinEntry[];
	columns: number;
	render: (skin: SkinEntry) => ReactNode;
}) {
	const scrollRef = useScrollContainer();
	const count = Math.ceil(items.length / columns);
	// Компилятор React не мемоизирует компонент с виртуализатором — так и нужно.
	// eslint-disable-next-line react-hooks/incompatible-library
	const virtualizer = useVirtualizer({
		count,
		getScrollElement: () => scrollRef.current,
		estimateSize: () => ROW_HEIGHT,
		overscan: 4,
	});
	return (
		<div className="relative" style={{ height: virtualizer.getTotalSize() }}>
			{virtualizer.getVirtualItems().map((row) => (
				<div
					key={row.key}
					className="absolute inset-x-0 top-0 flex gap-3"
					style={{ transform: `translateY(${row.start}px)` }}
				>
					{items.slice(row.index * columns, row.index * columns + columns).map(render)}
				</div>
			))}
		</div>
	);
}
