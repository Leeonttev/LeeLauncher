import { FolderOpen, LoaderCircle, OctagonX, Plus, RotateCcw, Search, Shirt, X } from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useEffect, useRef, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { AccountAvatar } from "@/components/accounts/account-avatar";
import { useAccountStatus } from "@/components/accounts/use-account-status";
import { CenteredState } from "@/components/centered-state";
import { ErrorDetails } from "@/components/error-details";
import { ScrollContainerContext, useScrollContainer } from "@/components/layout/scroll-container";
import { PixelGrid } from "@/components/pixel-grid";
import { SearchField } from "@/components/search-field";
import { Segments } from "@/components/segments";
import { CapeFlat } from "@/components/skins/skin-flat";
import { SkinCard } from "@/components/skins/skin-card";
import { SkinStage } from "@/components/skins/skin-stage";
import { SkinCardSkeleton } from "@/components/skins/skins-skeleton";
import { Button } from "@/components/ui/button";
import { onCoverButton } from "@/components/instances/play-button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { overlayOpen, typingTarget } from "@/hooks/use-create-shortcut";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { activeAccount, openAddAccountDialog, openReloginDialog, useAccounts } from "@/lib/accounts";
import type { Account, SkinEntry, SkinModel } from "@/lib/api";
import { errorText } from "@/lib/errors";
import {
	loadProfile,
	loadSkinLibrary,
	openTakeOffDialog,
	pickSkinFiles,
	refreshSkins,
	revertDraft,
	saveCurrentSkin,
	SEARCH_FROM,
	setDraftCape,
	setDraftModel,
	setSkinsPageAccount,
	setSkinsQuery,
	stageView,
	tryOnSkin,
	useSkins,
	waitTimeText,
	wearDraft,
	type StageView,
	type SkinsStore,
} from "@/lib/skins";
import { cn } from "@/lib/utils";

const COLUMN = 152;
const GAP = 12;
/** Рамка, картинка 128, две строки текста. */
const ROW_HEIGHT = 204 + GAP;
const VIRTUAL_FROM = 200;
const CASCADE_LIMIT = 12;
const CASCADE_STEP = 0.02;
const SKELETONS = 12;
/** У `SelectItem` не может быть пустого значения. */
const NO_CAPE = "none";

export function SkinsPage() {
	const store = useSkins();
	const accounts = useAccounts();
	const account = activeAccount(accounts);
	const searchRef = useRef<HTMLInputElement>(null);

	useEffect(() => {
		refreshSkins();
	}, []);

	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			const plainCtrl = event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey;
			const ctrlF = event.code === "KeyF" && plainCtrl;
			const ctrlO = event.code === "KeyO" && plainCtrl;
			const slash =
				(event.code === "Slash" || event.code === "NumpadDivide") &&
				!event.ctrlKey &&
				!event.altKey &&
				!event.shiftKey;
			if (!ctrlF && !ctrlO && !slash) return;
			// Иначе Ctrl+F и Ctrl+O откроет сам WebView2.
			if (ctrlF || ctrlO) event.preventDefault();
			if (overlayOpen() || event.repeat) return;
			if (ctrlO) {
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

	// Профиль — при входе на страницу и смене аккаунта. Частоту запросов бережёт ядро.
	useEffect(() => {
		setSkinsPageAccount(accountId);
		if (accountId && !needsRelogin) void loadProfile(accountId);
		return () => setSkinsPageAccount(null);
	}, [accountId, needsRelogin]);

	return (
		<div className="@container/skins flex min-h-0 flex-1">
			<div className="flex w-85 shrink-0 flex-col border-r @min-[1100px]/skins:w-100">
				<Podium store={store} account={account} needsRelogin={needsRelogin} view={view} />
				<Panel store={store} account={account} needsRelogin={needsRelogin} view={view} />
			</div>
			<Library store={store} account={account} view={view} searchRef={searchRef} />
		</div>
	);
}

function useStageCaption(
	store: SkinsStore,
	account: Account | null,
	needsRelogin: boolean,
	view: StageView,
): { caption: string | null; retry: boolean } {
	const { t } = useTranslation();
	if (view.trying) return { caption: t("skins.stage.trying"), retry: false };
	if (!account) return { caption: null, retry: false };
	const { current } = view;
	const worn = current?.isDefault ? t("skins.stage.wornDefault") : t("skins.stage.worn");
	if (needsRelogin) return { caption: current?.texture ? worn : null, retry: false };
	const state = store.profiles[account.id];
	const error = state?.error ?? null;
	const offline = error?.code === "network";
	if (state?.status === "error" || !state) {
		if (state && current?.texture)
			return {
				caption: offline ? t("skins.stage.staleOffline") : t("skins.stage.staleError"),
				retry: true,
			};
		if (state)
			return {
				caption: offline ? t("skins.stage.noProfileOffline") : t("skins.stage.noProfile"),
				retry: true,
			};
		return { caption: current?.texture ? worn : t("skins.stage.loading"), retry: false };
	}
	if (state.status === "loading")
		return { caption: current?.texture ? worn : t("skins.stage.loading"), retry: false };
	if (!current?.texture) return { caption: t("skins.stage.noProfile"), retry: true };
	if (error)
		return {
			caption: offline ? t("skins.stage.staleOffline") : t("skins.stage.staleError"),
			retry: true,
		};
	return { caption: worn, retry: false };
}

/** Пока идёт первый запрос профиля, менять и снимать скин рано. */
function isFirstProfileLoad(store: SkinsStore, accountId: string) {
	const state = store.profiles[accountId];
	return !state || state.status === "loading";
}

function Podium({
	store,
	account,
	needsRelogin,
	view,
}: {
	store: SkinsStore;
	account: Account | null;
	needsRelogin: boolean;
	view: StageView;
}) {
	const { t } = useTranslation();
	const statusOf = useAccountStatus();
	const { caption, retry } = useStageCaption(store, account, needsRelogin, view);
	const wait = account ? store.waits[account.id] : undefined;
	const name = view.entry?.name ?? (view.texture && account ? account.username : null);
	const celebration = account && store.celebration?.accountId === account.id ? store.celebration : null;

	return (
		<SkinStage
			texture={view.texture}
			model={view.model}
			cape={view.capeTexture}
			capeTurn={store.capeTurn}
			caption={caption}
			captionAction={
				retry &&
				account && (
					<button
						type="button"
						aria-disabled={wait ? true : undefined}
						className={cn(onCoverButton, "h-7 px-2.5 text-[13px] aria-disabled:opacity-70")}
						onClick={() => {
							if (!wait) void loadProfile(account.id, true);
						}}
					>
						{wait ? t("skins.action.wait", { time: waitTimeText(wait) }) : t("common.retry")}
					</button>
				)
			}
			ariaLabel={name ? t("skins.stage.aria", { name }) : t("skins.stage.ariaEmpty")}
			celebration={celebration}
			overlay={
				account && (
					<div
						title={account.username}
						className="flex w-fit max-w-full items-center gap-2 rounded-lg bg-cover-scrim/70 py-1.5 pr-2.5 pl-1.5 text-on-cover"
					>
						<AccountAvatar account={{ ...account, needsRelogin }} size={24} surface="cover" />
						<span className="min-w-0">
							<span className="block truncate text-[13px]/4 font-semibold">
								{account.username}
							</span>
							<span className="block truncate text-xs/[14px] opacity-80">
								{statusOf({ ...account, needsRelogin })}
							</span>
						</span>
					</div>
				)
			}
		/>
	);
}

function CapeRow({ view, loading }: { view: StageView; loading: boolean }) {
	const { t } = useTranslation();
	if (loading)
		return (
			<div className="flex items-center justify-between gap-3">
				<span className="text-[13px]/[18px] font-semibold">{t("skins.cape.label")}</span>
				<Skeleton className="h-8 w-44 rounded-lg" />
			</div>
		);
	if (!view.capes.length) return null;
	const selected = view.capes.find((cape) => cape.id === view.capeId);
	return (
		<div className="flex items-center justify-between gap-3">
			<span className="text-[13px]/[18px] font-semibold">{t("skins.cape.label")}</span>
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

function NotSavedRow({
	account,
	takeOffLocked,
	onSaved,
}: {
	account: Account;
	takeOffLocked: boolean;
	onSaved: () => void;
}) {
	const { t } = useTranslation();
	const [saving, setSaving] = useState(false);
	return (
		<div className="flex items-center justify-between gap-3">
			<span className="min-w-0 truncate text-[13px]/[18px] text-muted-foreground">
				{t("skins.current.notSaved")}
			</span>
			<div className="flex shrink-0 items-center gap-1">
				<Button
					variant="outline"
					size="sm"
					aria-disabled={saving || undefined}
					onClick={async () => {
						if (saving) return;
						setSaving(true);
						const saved = await saveCurrentSkin(account.id);
						setSaving(false);
						if (saved) onSaved();
					}}
				>
					{saving && <LoaderCircle className="animate-spin" />}
					{t("skins.current.save")}
				</Button>
				<Button
					variant="ghost"
					size="sm"
					aria-disabled={takeOffLocked || undefined}
					className="aria-disabled:cursor-default aria-disabled:opacity-50"
					onClick={(event) => {
						if (!takeOffLocked) openTakeOffDialog(account.id, event.currentTarget);
					}}
				>
					{t("skins.current.takeOff")}
				</Button>
			</div>
		</div>
	);
}

function Panel({
	store,
	account,
	needsRelogin,
	view,
}: {
	store: SkinsStore;
	account: Account | null;
	needsRelogin: boolean;
	view: StageView;
}) {
	const { t } = useTranslation();
	const mainRef = useRef<HTMLButtonElement>(null);
	const { current, trying } = view;
	const applying = account ? Boolean(store.applying[account.id]) : false;
	const resetting = account ? Boolean(store.resetting[account.id]) : false;
	const wait = account ? store.waits[account.id] : undefined;
	const profileState = account ? store.profiles[account.id] : undefined;
	const loading = !current && (!profileState || profileState.status === "loading");
	// Неизвестно, что на аккаунте и не потеряется ли это при замене.
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

	return (
		<div className="flex shrink-0 flex-col gap-2.5 border-t px-5 pt-3.5 pb-4">
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
			{account && !needsRelogin && <CapeRow view={view} loading={loading} />}
			{account && current && !current.skinId && !current.isDefault && !trying && (
				<NotSavedRow
					account={account}
					takeOffLocked={firstLoad}
					onSaved={() => mainRef.current?.focus()}
				/>
			)}
			<div className="mt-0.5 flex gap-2">
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							ref={mainRef}
							data-skin-main
							aria-disabled={inactive || undefined}
							className="h-9 min-w-0 flex-1 aria-disabled:cursor-default aria-disabled:opacity-50 aria-disabled:hover:bg-primary aria-disabled:active:translate-y-0"
							onClick={() => run?.()}
						>
							{busy && <LoaderCircle className="animate-spin" />}
							<span className="truncate">{label}</span>
						</Button>
					</TooltipTrigger>
					{hint && <TooltipContent>{hint}</TooltipContent>}
				</Tooltip>
				{trying && (
					<Button
						variant="outline"
						className="h-9 w-24"
						onClick={() => {
							revertDraft();
							mainRef.current?.focus();
						}}
					>
						{t("skins.action.revert")}
					</Button>
				)}
			</div>
		</div>
	);
}

function Library({
	store,
	account,
	view,
	searchRef,
}: {
	store: SkinsStore;
	account: Account | null;
	view: StageView;
	searchRef: RefObject<HTMLInputElement | null>;
}) {
	const { t } = useTranslation();
	const scrollRef = useRef<HTMLDivElement>(null);
	const loading = store.status === "idle" || store.status === "loading";
	const lateLoading = useDelayedFlag(loading, 150);
	const [cascade] = useState(() => store.status !== "ready");
	const total = store.items.length;
	const query = store.query.trim().toLocaleLowerCase();
	const visible = query
		? store.items.filter((skin) => skin.name.toLocaleLowerCase().includes(query))
		: store.items;

	useEffect(() => {
		if (store.addedSeq) scrollRef.current?.scrollTo({ top: 0 });
	}, [store.addedSeq]);

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
			<div aria-busy className="-mx-8 min-h-0 flex-1 overflow-hidden px-8 pt-5">
				<div className="grid grid-cols-[repeat(auto-fill,152px)] gap-3">
					{Array.from({ length: SKELETONS }, (_, index) => (
						<SkinCardSkeleton key={index} />
					))}
				</div>
			</div>
		) : (
			<div className="flex-1" />
		);
	} else if (!total) {
		body = (
			<CenteredState
				heading="h2"
				icon={Shirt}
				title={t("skins.empty.title")}
				description={t("skins.empty.text")}
				action={
					<Button variant="outline" onClick={() => void pickSkinFiles()}>
						<FolderOpen />
						{t("skins.empty.pick")}
					</Button>
				}
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
			<div ref={scrollRef} className="-mx-8 min-h-0 flex-1 overflow-y-auto px-8 pt-5 pb-10">
				<ScrollContainerContext value={scrollRef}>
					<SkinGrid
						items={visible}
						account={account}
						view={view}
						takeOffLocked={Boolean(account && isFirstProfileLoad(store, account.id))}
						cascade={cascade}
					/>
				</ScrollContainerContext>
			</div>
		);
	}

	return (
		<div className="flex min-w-0 flex-1 flex-col px-8 pt-7">
			<div className="flex shrink-0 items-center justify-between gap-6">
				<div className="flex min-w-0 items-baseline gap-2.5">
					<h1 className="text-[28px]/[34px] font-semibold tracking-[-0.025em]">
						{t("skins.title")}
					</h1>
					{total > 0 && <span className="text-sm text-muted-foreground">{total}</span>}
				</div>
				<div className="flex shrink-0 items-center gap-2">
					{(total >= SEARCH_FROM || store.query) && (
						<SearchField
							inputRef={searchRef}
							value={store.query}
							onChange={setSkinsQuery}
							placeholder={t("skins.search")}
							keyShortcuts="Control+F /"
							className="w-50"
						/>
					)}
					<Button
						variant="outline"
						data-skins-add
						aria-keyshortcuts="Control+O"
						onClick={() => void pickSkinFiles()}
					>
						<Plus />
						{t("skins.add")}
					</Button>
				</div>
			</div>
			{body}
		</div>
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

function SkinGrid({
	items,
	account,
	view,
	takeOffLocked,
	cascade,
}: {
	items: SkinEntry[];
	account: Account | null;
	view: StageView;
	takeOffLocked: boolean;
	cascade: boolean;
}) {
	const { t } = useTranslation();
	const rootRef = useRef<HTMLDivElement>(null);
	const columns = useColumns(rootRef);
	const checkedId = view.entry?.id ?? null;
	const checkedVisible = items.some((skin) => skin.id === checkedId);
	const tabStop = checkedVisible ? checkedId : items[0]?.id;
	const virtual = items.length > VIRTUAL_FROM;

	const card = (skin: SkinEntry, fadeDelay: number | null) => (
		<SkinCard
			key={skin.id}
			skin={skin}
			model={checkedId === skin.id ? view.model : skin.model}
			wornBy={view.current?.skinId === skin.id ? account : null}
			takeOffLocked={takeOffLocked}
			menuTabbable={tabStop === skin.id}
			fadeDelay={fadeDelay}
			onEnter={() => document.querySelector<HTMLElement>("[data-skin-main]")?.focus()}
			onMoveVertical={(id, direction) => {
				const index = items.findIndex((other) => other.id === id);
				moveToCard(items[index + direction * columns]?.id);
			}}
		/>
	);

	return (
		<RadioGroupPrimitive.Root
			ref={rootRef}
			aria-label={t("skins.title")}
			value={checkedId ?? ""}
			loop={false}
			onValueChange={(id) => {
				const skin = items.find((other) => other.id === id);
				if (skin) tryOnSkin(skin);
			}}
		>
			{virtual ? (
				<VirtualRows items={items} columns={columns} render={(skin) => card(skin, null)} />
			) : (
				<PixelGrid
					columns={columns}
					minColumn={COLUMN}
					maxColumn={COLUMN}
					gap={GAP}
					className="grid-cols-[repeat(auto-fill,152px)] gap-y-3"
				>
					{items.map((skin, index) =>
						card(skin, cascade && index < CASCADE_LIMIT ? index * CASCADE_STEP : 0),
					)}
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
	render: (skin: SkinEntry) => React.ReactNode;
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
