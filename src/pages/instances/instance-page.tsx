import {
	ArrowLeft,
	Ellipsis,
	ExternalLink,
	FolderOpen,
	Globe,
	Image,
	LoaderCircle,
	OctagonX,
	Puzzle,
	RotateCcw,
	ScrollText,
	Search,
	Settings,
	ShieldAlert,
	X,
	type LucideIcon,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { CenteredState } from "@/components/centered-state";
import { ErrorDetails } from "@/components/error-details";
import { InstanceCover } from "@/components/instances/instance-cover";
import { InstanceMenuContent } from "@/components/instances/instance-menu";
import { InstanceMeta } from "@/components/instances/instance-meta";
import { onCoverButton, PlayButton } from "@/components/instances/play-button";
import { ProgressBar } from "@/components/instances/progress-bar";
import { PageTabs } from "@/components/page-tabs";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { TabsContent } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { overlayOpen, typingTarget } from "@/hooks/use-create-shortcut";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { contentApi, type Instance } from "@/lib/api";
import { useInstanceContent } from "@/lib/content";
import { coverOf } from "@/lib/covers";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { formatBytes, formatNumber } from "@/lib/format";
import { initGameSettings, openAppFolder, useGameSettings } from "@/lib/game-settings";
import { INSTANCE_TABS, setInstanceTab, useInstancePageState, type InstanceTab } from "@/lib/instance-page";
import {
	cancelInstall,
	focusJavaSetting,
	isDiskProblem,
	openInstanceFolder,
	progressValue,
	reloadInstances,
	stageText,
	statusOf,
	useInstances,
} from "@/lib/instances";
import { goBack, instanceIdOf, navigate, useRoute } from "@/lib/router";
import { useScreenshots } from "@/lib/screenshots";
import { dismissFontVuln, openFontVulnInfo, useFontVulnDismissed } from "@/lib/security";
import { cn } from "@/lib/utils";
import { useWorlds } from "@/lib/worlds";
import { InstanceSettings } from "@/pages/instances/instance-settings";
import { ContentTab } from "@/pages/instances/tabs/content-tab";
import { LogTab } from "@/pages/instances/tabs/log-tab";
import { ScreenshotsTab } from "@/pages/instances/tabs/screenshots-tab";
import { WorldsTab } from "@/pages/instances/tabs/worlds-tab";
import { ariaBackShortcut, isBackShortcut } from "@/lib/platform";

const TAB_ICONS: Record<InstanceTab, LucideIcon> = {
	content: Puzzle,
	worlds: Globe,
	screenshots: Image,
	log: ScrollText,
	settings: Settings,
};

const scrim =
	"pointer-events-none absolute inset-0 -z-10 bg-linear-to-r from-cover-scrim/80 via-cover-scrim/35 to-transparent";

export function InstancePage() {
	const { t } = useTranslation();
	const route = useRoute();
	const id = instanceIdOf(route);
	const store = useInstances();
	const loading = store.status === "idle" || store.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);
	const instance = store.instances.find((i) => i.id === id) ?? null;
	const exists = instance !== null;

	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (!isBackShortcut(event)) return;
			if (typingTarget(event.target) || overlayOpen()) return;
			event.preventDefault();
			goBack("/instances");
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);

	// Ядро следит за папками открытой сборки и шлёт события изменений.
	useEffect(() => {
		if (!id || !exists) return;
		contentApi.watch(id).catch((err: unknown) => {
			// Сборку удалили, пока страница открыта: страница покажет «не найдена».
			if (toAppError(err).code !== "instance_not_found") console.error("instance_watch", err);
		});
		return () => {
			contentApi.watch(null).catch((err: unknown) => console.error("instance_watch", err));
		};
	}, [id, exists]);

	if (instance) return <InstanceView instance={instance} />;

	let body;
	if (store.status === "error") {
		body = (
			<CenteredState
				icon={OctagonX}
				tone="destructive"
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
		return showSkeleton ? <PageSkeleton /> : null;
	} else {
		body = (
			<CenteredState
				icon={Search}
				title={t("instance.notFound.title")}
				description={t("instance.notFound.description")}
				action={
					<Button variant="outline" onClick={() => navigate("/instances", { replace: true })}>
						{t("instance.notFound.action")}
					</Button>
				}
			/>
		);
	}

	return (
		<div className="flex flex-1 flex-col px-8 pt-7 pb-10">
			<div>
				<Button
					variant="ghost"
					size="sm"
					aria-keyshortcuts={ariaBackShortcut}
					className="-ml-2.5 px-2.5 has-[>svg]:px-2.5"
					onClick={() => goBack("/instances")}
				>
					<ArrowLeft />
					{t("instance.back")}
				</Button>
			</div>
			{body}
		</div>
	);
}

function InstanceView({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const page = useInstancePageState();
	const store = useInstances();
	const tab = page.tabs[instance.id] ?? "content";
	const content = useInstanceContent(instance.id);
	const worlds = useWorlds(instance.id);
	const screenshots = useScreenshots(instance.id, false);
	const status = statusOf(instance, store);
	const running = status === "running" || status === "launching";

	const counts: Partial<Record<InstanceTab, number | null>> = {
		content: content?.status === "ready" ? content.items.length : null,
		worlds: worlds?.status === "ready" ? worlds.items.length : null,
		screenshots: screenshots?.status === "ready" ? screenshots.total : null,
	};

	const tabs = INSTANCE_TABS.map((value) => ({
		value,
		label: t(`instance.tabs.${value}`),
		icon: TAB_ICONS[value],
		count: counts[value] ?? null,
		extra:
			value === "log" && running ? (
				<span aria-hidden className="size-2 shrink-0 rounded-full bg-primary" />
			) : undefined,
	}));

	const content_ = (value: InstanceTab) =>
		cn(
			"mt-5 flex flex-1 flex-col data-[state=active]:animate-in data-[state=active]:duration-(--duration-fast) data-[state=active]:fade-in-0",
			value === "settings" && "[&>*:first-child]:mt-0",
		);

	return (
		<div data-instance-page data-instance-id={instance.id} className="flex flex-1 flex-col">
			<div className="px-8 pt-7 pb-3">
				<Banner instance={instance} />
			</div>
			<PageTabs
				className="flex-1"
				listClassName="sticky top-0 z-20 bg-background px-8"
				label={t("instance.tabsLabel")}
				layoutId="instance-tab"
				value={tab}
				onValueChange={(next) => setInstanceTab(instance.id, next)}
				tabs={tabs}
			>
				<div className="flex flex-1 flex-col px-8 pb-10">
					<FontVulnWarning instance={instance} />
					<StatusBlock instance={instance} />
					<TabsContent value="content" className={content_("content")}>
						<ContentTab instance={instance} />
					</TabsContent>
					<TabsContent value="worlds" className={content_("worlds")}>
						<WorldsTab instance={instance} />
					</TabsContent>
					<TabsContent value="screenshots" className={content_("screenshots")}>
						<ScreenshotsTab instance={instance} />
					</TabsContent>
					<TabsContent value="log" className={content_("log")}>
						<LogTab instance={instance} />
					</TabsContent>
					<TabsContent value="settings" className={content_("settings")}>
						<InstanceSettings instance={instance} />
					</TabsContent>
				</div>
			</PageTabs>
		</div>
	);
}

function Banner({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const moreRef = useRef<HTMLButtonElement>(null);
	const actionsLabel = t("instance.actions", { name: instance.name });

	return (
		<section
			aria-labelledby="instance-title"
			className="relative isolate flex h-52 shrink-0 items-end gap-6 overflow-hidden rounded-2xl bg-muted px-8 pb-7"
		>
			<InstanceCover scene={coverOf(instance)} variant="wide" className="absolute inset-0 -z-10" />
			<div aria-hidden className={scrim} />
			<button
				type="button"
				aria-keyshortcuts={ariaBackShortcut}
				className={cn(onCoverButton, "absolute top-5 left-6 h-8 px-3")}
				onClick={() => goBack("/instances")}
			>
				<ArrowLeft />
				{t("instance.back")}
			</button>
			<div className="min-w-0 flex-1">
				<h1
					id="instance-title"
					className="truncate text-[28px]/[34px] font-semibold tracking-[-0.025em] text-on-cover"
				>
					{instance.name}
				</h1>
				<InstanceMeta instance={instance} variant="banner" className="mt-1" />
			</div>
			<div className="flex shrink-0 items-center gap-2">
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							aria-label={t("instance.openFolder")}
							className={cn(onCoverButton, "size-11")}
							onClick={() => void openInstanceFolder(instance.id)}
						>
							<FolderOpen />
						</button>
					</TooltipTrigger>
					<TooltipContent>{t("instance.openFolder")}</TooltipContent>
				</Tooltip>
				<DropdownMenu>
					<Tooltip>
						<TooltipTrigger asChild>
							<DropdownMenuTrigger asChild>
								<button
									ref={moreRef}
									type="button"
									aria-label={actionsLabel}
									className={cn(onCoverButton, "size-11 aria-expanded:bg-cover-scrim/55")}
								>
									<Ellipsis />
								</button>
							</DropdownMenuTrigger>
						</TooltipTrigger>
						<TooltipContent>{actionsLabel}</TooltipContent>
					</Tooltip>
					<InstanceMenuContent
						instance={instance}
						kind="dropdown"
						onPage
						focusTarget={() => moreRef.current}
					/>
				</DropdownMenu>
				<PlayButton instance={instance} size="banner" />
			</div>
		</section>
	);
}

function FontVulnWarning({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const settings = useGameSettings().settings;
	const dismissed = useFontVulnDismissed(instance.id);
	const show = instance.fontVulnerable === true && !dismissed;
	const protectedPacks = settings.status === "ready" && (settings.value?.protectServerPacks ?? true);

	useEffect(() => initGameSettings(), []);

	function dismiss() {
		dismissFontVuln(instance.id);
		// Крестик исчезает вместе с плашкой: фокус — на текущую вкладку, а не на body.
		document
			.querySelector<HTMLElement>("[data-instance-page] [role=tab][aria-selected=true]")
			?.focus({ preventScroll: true });
	}

	return (
		<AnimatePresence initial={false}>
			{show && (
				<motion.div
					key="font-vuln"
					role="note"
					aria-labelledby="font-vuln-title"
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={{ duration: 0.2 }}
					className="mt-5 flex items-start gap-3 border-b pb-4"
				>
					<ShieldAlert className="mt-0.5 shrink-0 text-warning" />
					<div className="min-w-0 flex-1">
						<p id="font-vuln-title" className="text-sm/5 font-semibold">
							{t("security.fontVuln.title")}
						</p>
						<p className="mt-0.5 max-w-prose text-[13px]/[18px] text-muted-foreground">
							{protectedPacks
								? t("security.fontVuln.descriptionProtected")
								: t("security.fontVuln.description")}
						</p>
					</div>
					<Button variant="outline" size="sm" className="shrink-0" onClick={openFontVulnInfo}>
						<ExternalLink />
						{t("common.details")}
					</Button>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon-sm"
								aria-label={t("security.fontVuln.dismiss")}
								className="shrink-0"
								onClick={dismiss}
							>
								<X />
							</Button>
						</TooltipTrigger>
						<TooltipContent>{t("security.fontVuln.dismiss")}</TooltipContent>
					</Tooltip>
				</motion.div>
			)}
		</AnimatePresence>
	);
}

function StatusBlock({ instance }: { instance: Instance }) {
	const store = useInstances();
	const status = statusOf(instance, store);
	const error = store.installErrors[instance.id];
	const show = status === "queued" || status === "installing" || status === "launching";
	return (
		<AnimatePresence initial={false} mode="wait">
			{show && (
				<motion.div
					key="progress"
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={{ duration: 0.2 }}
					className="mt-5"
				>
					<ProgressBlock instance={instance} />
				</motion.div>
			)}
			{status === "install_failed" && (
				<motion.div
					key="error"
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={{ duration: 0.2 }}
					className="mt-5"
				>
					<InstallError instance={instance} error={error} />
				</motion.div>
			)}
		</AnimatePresence>
	);
}

function ProgressBlock({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const store = useInstances();
	const status = statusOf(instance, store);
	const progress = store.progress[instance.id];
	const value = status === "launching" && !progress ? null : progressValue(instance, store);
	const cancellable = status === "queued" || status === "installing";
	const cancelling = Boolean(store.cancelling[instance.id]);

	const details: string[] = [];
	if (progress && progress.stage !== "queued") {
		if (progress.filesTotal > 0)
			details.push(
				t("install.files", {
					current: formatNumber(progress.filesDone),
					total: formatNumber(progress.filesTotal),
				}),
			);
		if (progress.bytesTotal > 0)
			details.push(
				t("install.bytes", {
					current: formatBytes(progress.bytesDone),
					total: formatBytes(progress.bytesTotal),
				}),
			);
	}

	return (
		<div>
			<div className="flex items-baseline gap-4">
				<p className="min-w-0 flex-1 truncate text-sm/5 font-medium">
					{status === "queued" ? t("install.stage.queued") : stageText(instance, store)}
				</p>
				{value !== null && (
					<span className="text-sm text-muted-foreground tabular-nums">{`${Math.round(value * 100)}%`}</span>
				)}
			</div>
			<ProgressBar
				variant="thin"
				value={value}
				ariaLabel={t("install.progressLabel", { name: instance.name })}
				className="mt-2"
			/>
			<div className="mt-2 flex min-h-8 items-center gap-4">
				<p className="min-w-0 flex-1 truncate text-[13px]/[18px] text-muted-foreground tabular-nums">
					{details.join(" · ")}
				</p>
				{cancellable && (
					<Button
						variant="ghost"
						size="sm"
						disabled={cancelling}
						onClick={() => void cancelInstall(instance.id)}
					>
						{cancelling ? <LoaderCircle className="animate-spin" /> : <X />}
						{t("instance.cancel")}
					</Button>
				)}
			</div>
		</div>
	);
}

function InstallError({ instance, error }: { instance: Instance; error?: AppError }) {
	const { t } = useTranslation();
	const javaProblem = error?.code === "java_not_found" || error?.code === "java_invalid";
	const diskProblem = error ? isDiskProblem(error) : false;
	return (
		<div>
			<p className="flex items-center gap-2 text-sm/5 font-medium text-destructive">
				<OctagonX className="shrink-0" />
				{t("install.failed")}
			</p>
			{error && (
				<p className="mt-1 max-w-prose text-[13px]/[18px] text-muted-foreground">
					{errorText(error)}
				</p>
			)}
			{javaProblem && (
				<Button
					variant="outline"
					size="sm"
					className="mt-3"
					onClick={() => focusJavaSetting(instance.id)}
				>
					{t("install.javaSettings")}
				</Button>
			)}
			{diskProblem && (
				<Button
					variant="outline"
					size="sm"
					className="mt-3"
					onClick={() => void openAppFolder("data")}
				>
					<FolderOpen />
					{t("install.openDataFolder")}
				</Button>
			)}
			{error?.details && <ErrorDetails details={error.details} />}
		</div>
	);
}

function PageSkeleton() {
	return (
		<div aria-busy className="flex flex-1 flex-col">
			<Skeleton className="h-48 w-full" />
			<div className="flex h-11 items-center gap-4 border-b px-8">
				{[0, 1, 2, 3, 4].map((i) => (
					<Skeleton key={i} className="h-4 w-20" />
				))}
			</div>
			<div className="flex flex-col gap-3 px-8 pt-5">
				<Skeleton className="h-9 w-65" />
				{[0, 1, 2, 3, 4, 5].map((i) => (
					<div key={i} className="flex h-14 items-center gap-4">
						<Skeleton className="size-10 rounded-lg" />
						<div className="flex flex-1 flex-col gap-1.5">
							<Skeleton className="h-4 w-40" />
							<Skeleton className="h-3 w-24" />
						</div>
					</div>
				))}
			</div>
		</div>
	);
}
