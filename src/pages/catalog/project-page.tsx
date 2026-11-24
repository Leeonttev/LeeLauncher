import {
	ArrowLeft,
	ArrowRight,
	BookOpen,
	Bug,
	Code,
	ExternalLink,
	Heart,
	type LucideIcon,
	MessageCircle,
	OctagonX,
	RotateCcw,
	Search,
	WifiOff,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { InstallButton } from "@/components/catalog/install-button";
import { InstanceContextPicker } from "@/components/catalog/instance-context-picker";
import { ModpackInstallButton } from "@/components/catalog/modpack-install-button";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { ProjectMeta } from "@/components/catalog/project-meta";
import { SlowNotice } from "@/components/catalog/slow-notice";
import { CenteredState } from "@/components/centered-state";
import { useScrollContainer } from "@/components/layout/scroll-container";
import { PageTabs } from "@/components/page-tabs";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { TabsContent } from "@/components/ui/tabs";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import {
	catalogApi,
	external,
	type AlsoOn,
	type ContentSource,
	type ContentVersion,
	type Instance,
	type Project,
} from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import {
	clearProjectTabHint,
	contextInstance,
	DEFAULT_SOURCE,
	isUnreachable,
	loadCategories,
	openProject,
	projectAlsoOnHint,
	projectTabHint,
	projectTypeHint,
	projectUrlHint,
	setCatalogContext,
	SOURCE_NAMES,
	useCatalog,
	withDeadline,
	type CatalogType,
} from "@/lib/catalog";
import { installButtonState, useInstallUi, type ProjectRef } from "@/lib/content-install";
import { findProjectItem, useInstanceContent } from "@/lib/content";
import { useDownloads } from "@/lib/downloads";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";
import { useInstances } from "@/lib/instances";
import { openInstancePage } from "@/lib/instance-page";
import { LOADER_NAMES } from "@/lib/loaders";
import { defaultVersion, instancesOfModpack, versionTarget, type ModpackRef } from "@/lib/modpacks";
import { useRequiredDeps } from "@/lib/project-page";
import { goBack, projectRouteOf, useRoute } from "@/lib/router";
import { cn } from "@/lib/utils";
import { useInstanceWorlds } from "@/lib/worlds";
import { PROJECT_GRID, ProjectOverview } from "@/pages/catalog/project-description";
import { ProjectGallery } from "@/pages/catalog/project-gallery";
import { ProjectVersions } from "@/pages/catalog/project-versions";
import { ariaBackShortcut, isBackShortcut } from "@/lib/platform";

type Tab = "description" | "versions" | "gallery";

type Loaded<T> = { status: "loading" } | { status: "ready"; value: T } | { status: "error"; error: AppError };

/** Высота липкой полосы вкладок: ряд кнопки под ней уже не виден. */
const TABS_BAR = 44;
const RIGHT_CELL =
	"mt-5 pl-30 @min-[1040px]/project:mt-0 @min-[1040px]/project:pl-0 @min-[1040px]/project:pt-1";

function openExternal(url: string) {
	external.open(url).catch((err: unknown) => {
		console.error("open-url", err);
		toast.error(i18n.t("common.openLinkFailed"));
	});
}

export function ProjectPage() {
	const route = useRoute();
	const ref = projectRouteOf(route);
	const catalog = useCatalog();
	const store = useInstances();
	const id = ref?.id ?? "";
	// Модпак ставится новой сборкой: сборка-контекст ему не нужна.
	const modpackHint = projectTypeHint(id) === "modpack";
	const instance = modpackHint ? null : contextInstance(store.instances, catalog.contextId);
	const instancesReady = store.status === "ready" || store.status === "error";
	const source = (ref?.source ?? DEFAULT_SOURCE) as ContentSource;
	const instanceId = instance?.id;
	const [project, setProject] = useState<Loaded<Project>>({ status: "loading" });
	const [versions, setVersions] = useState<Loaded<ContentVersion[]>>({ status: "loading" });
	const [attempt, setAttempt] = useState(0);
	const [versionsAttempt, setVersionsAttempt] = useState(0);

	useEffect(() => {
		if (!id || !instancesReady) return;
		let alive = true;
		const hint = projectTypeHint(id);
		withDeadline(catalogApi.project({ id, source, instanceId, projectType: hint }))
			.then((value) => alive && setProject({ status: "ready", value }))
			.catch((err: unknown) => alive && setProject({ status: "error", error: toAppError(err) }));
		return () => {
			alive = false;
		};
	}, [id, source, instanceId, instancesReady, attempt]);

	useEffect(() => {
		if (!id || !instancesReady) return;
		let alive = true;
		withDeadline(catalogApi.versions({ projectId: id, source, instanceId }))
			.then((value) => alive && setVersions({ status: "ready", value }))
			.catch((err: unknown) => alive && setVersions({ status: "error", error: toAppError(err) }));
		return () => {
			alive = false;
		};
	}, [id, source, instanceId, instancesReady, attempt, versionsAttempt]);

	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (isBackShortcut(event)) {
				event.preventDefault();
				goBack("/catalog");
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);

	const retry = () => {
		setProject({ status: "loading" });
		setVersions({ status: "loading" });
		setAttempt((n) => n + 1);
	};

	// Страница остаётся на месте: перечитываются только версии.
	const retryVersions = () => {
		setVersions({ status: "loading" });
		setVersionsAttempt((n) => n + 1);
	};

	return (
		<div className="@container/project flex flex-1 flex-col">
			<BackButton />
			{project.status === "ready" ? (
				<ProjectView
					key={project.value.id}
					project={project.value}
					versions={versions}
					instance={instance}
					onRetryVersions={retryVersions}
				/>
			) : project.status === "error" ? (
				<ProjectError
					error={project.error}
					source={source}
					url={projectUrlHint(id)}
					alsoOn={projectAlsoOnHint(id)}
					onRetry={retry}
				/>
			) : (
				<ProjectSkeleton source={source} />
			)}
		</div>
	);
}

function BackButton() {
	const { t } = useTranslation();
	return (
		<Button
			variant="ghost"
			size="sm"
			className="-ml-3 self-start"
			aria-keyshortcuts={ariaBackShortcut}
			onClick={() => goBack("/catalog")}
		>
			<ArrowLeft />
			{t("project.back")}
		</Button>
	);
}

function ProjectError({
	error,
	source,
	url,
	alsoOn,
	onRetry,
}: {
	error: AppError;
	source: ContentSource;
	url?: string;
	alsoOn: AlsoOn[];
	onRetry: () => void;
}) {
	const { t } = useTranslation();
	const siteButton = url ? (
		<Button variant="outline" onClick={() => openExternal(url)}>
			<ExternalLink />
			{t("project.openOnSite")}
		</Button>
	) : null;
	if (error.code === "source_disabled") {
		// Источник выключен в этой сборке лаунчера или не принимает запросы.
		const name = SOURCE_NAMES[source];
		return (
			<CenteredState
				icon={WifiOff}
				heading="h1"
				title={t("catalog.error.disabledTitle", { source: name })}
				description={
					error.details === "denied"
						? t("catalog.error.deniedDescription", { source: name })
						: t("project.sourceDisabled.description", { source: name })
				}
				action={siteButton ?? undefined}
			/>
		);
	}
	if (error.code === "content_not_found") {
		return (
			<CenteredState
				icon={Search}
				heading="h1"
				title={t("project.notFound.title")}
				description={t("project.notFound.description")}
				action={
					<div className="flex flex-wrap justify-center gap-2">
						<Button variant="outline" onClick={() => goBack("/catalog")}>
							{t("project.back")}
						</Button>
						{siteButton}
					</div>
				}
			/>
		);
	}
	const unreachable = isUnreachable(error);
	const twin = unreachable ? alsoOn.find((item) => item.source !== source) : undefined;
	return (
		<CenteredState
			icon={error.code === "network" ? WifiOff : OctagonX}
			tone="destructive"
			heading="h1"
			title={
				unreachable
					? t("catalog.error.unreachableTitle", { source: SOURCE_NAMES[source] })
					: t("project.error.title")
			}
			description={unreachable ? t("catalog.error.unreachableDescription") : errorText(error)}
			action={
				<div className="flex flex-wrap justify-center gap-2">
					<Button variant="outline" onClick={onRetry}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
					{twin && (
						<Button
							variant="outline"
							onClick={() => openProject(twin, projectTypeHint(twin.id), { url: twin.url })}
						>
							<ArrowRight />
							{t("project.alsoOn", { source: SOURCE_NAMES[twin.source] })}
						</Button>
					)}
				</div>
			}
		/>
	);
}

function ProjectSkeleton({ source }: { source: ContentSource }) {
	const show = useDelayedFlag(true, 150);
	return (
		<>
			<SlowNotice loading source={source} className="mt-3" />
			{show && <SkeletonBody />}
		</>
	);
}

function SkeletonBody() {
	return (
		<div aria-busy>
			<div className={cn(PROJECT_GRID, "mt-3")}>
				<div className="flex items-start gap-6">
					<Skeleton className="size-24 shrink-0 rounded-xl" />
					<div className="min-w-0 flex-1">
						<Skeleton className="h-8 w-64" />
						<Skeleton className="mt-2 h-4 w-32" />
						<Skeleton className="mt-3 h-4 w-[90%]" />
						<Skeleton className="mt-2 h-4 w-[70%]" />
						<Skeleton className="mt-3 h-3 w-72" />
					</div>
				</div>
				<div className={RIGHT_CELL}>
					<div className="flex gap-2">
						<Skeleton className="h-10 w-65 rounded-lg" />
						<Skeleton className="h-10 w-32 rounded-lg" />
					</div>
					<Skeleton className="mt-3 h-3 w-56" />
				</div>
			</div>
			<div className="relative -mx-8 mt-7 flex h-11 items-center gap-6 px-11 after:absolute after:inset-x-0 after:bottom-0 after:h-px after:bg-border after:content-['']">
				<Skeleton className="h-4 w-20" />
				<Skeleton className="h-4 w-20" />
				<Skeleton className="h-4 w-20" />
			</div>
			<div className={cn(PROJECT_GRID, "mt-7")}>
				<div>
					<Skeleton className="h-4 w-[92%]" />
					<Skeleton className="mt-3 h-4 w-[84%]" />
					<Skeleton className="mt-3 h-4 w-[88%]" />
					<Skeleton className="mt-3 h-4 w-[60%]" />
				</div>
				<div className="mt-8 grid grid-cols-2 gap-x-8 gap-y-6 border-t pt-7 @min-[1040px]/project:mt-0 @min-[1040px]/project:grid-cols-[repeat(auto-fill,minmax(200px,1fr))] @min-[1040px]/project:border-t-0 @min-[1040px]/project:pt-0">
					{Array.from({ length: 4 }, (_, i) => (
						<Skeleton key={i} className="h-10" />
					))}
				</div>
			</div>
		</div>
	);
}

function targetLabel(instance: Instance) {
	return instance.loader.kind === "vanilla"
		? instance.gameVersion
		: `${instance.gameVersion} ${LOADER_NAMES[instance.loader.kind]}`;
}

type ProjectLink = { key: string; label: string; url: string; icon: LucideIcon };

function useHeaderHidden(node: HTMLElement | null) {
	const scroller = useScrollContainer();
	const [hidden, setHidden] = useState(false);
	useEffect(() => {
		const root = scroller.current;
		if (!node || !root) return;
		const observer = new IntersectionObserver(
			([entry]) => {
				const top = entry.rootBounds?.top ?? 0;
				setHidden(!entry.isIntersecting && entry.boundingClientRect.top < top + TABS_BAR);
			},
			{ root, rootMargin: `-${TABS_BAR}px 0px 0px 0px` },
		);
		observer.observe(node);
		return () => observer.disconnect();
	}, [node, scroller]);
	return hidden;
}

function ProjectView({
	project,
	versions,
	instance,
	onRetryVersions,
}: {
	project: Project;
	versions: Loaded<ContentVersion[]>;
	instance: Instance | null;
	onRetryVersions: () => void;
}) {
	const { t } = useTranslation();
	const { reduceMotion } = useAppearance();
	const store = useInstances();
	const catalog = useCatalog();
	const scroller = useScrollContainer();
	const [tab, setTab] = useState<Tab>(() => projectTabHint(project.id) ?? "description");
	// `seq` пересоздаёт вкладку «Версии» с этой версией.
	const [focusVersion, setFocusVersion] = useState<{ id: string; seq: number } | null>(null);
	const [installRow, setInstallRow] = useState<HTMLDivElement | null>(null);
	const miniRef = useRef<HTMLDivElement>(null);
	const tabsAnchor = useRef<HTMLDivElement>(null);
	const headerHidden = useHeaderHidden(installRow);
	useEffect(() => clearProjectTabHint(project.id), [project.id]);
	// Мини-установка уходит вместе с фокусом — переносим его на кнопку в шапке.
	useLayoutEffect(() => {
		if (headerHidden || !miniRef.current?.contains(document.activeElement)) return;
		const buttons = installRow?.querySelectorAll<HTMLElement>("button");
		buttons?.[buttons.length - 1]?.focus({ preventScroll: true });
	}, [headerHidden, installRow]);
	const type: CatalogType = projectTypeHint(project.id) ?? project.type;
	const modpack = type === "modpack";
	const contentInstance = modpack ? null : instance;
	const [alsoOn] = useState(() => projectAlsoOnHint(project.id));
	const ref: ProjectRef & { install?: Project["install"] } = {
		source: project.source,
		id: project.id,
		title: project.title,
		type: type === "modpack" ? "mod" : type,
		iconUrl: project.iconUrl,
		gameVersions: project.gameVersions,
		loaders: project.loaders,
		siteOnly: project.siteOnly,
		install: project.install,
		alsoOn,
	};
	// У некоторых источников подписи категорий берутся из их списка категорий.
	useEffect(() => loadCategories(type, project.source), [type, project.source]);
	const candidates: (Omit<ProjectLink, "url"> & { url?: string })[] = [
		{ key: "site", label: SOURCE_NAMES[project.source], url: project.links.site, icon: ExternalLink },
		{ key: "source", label: t("project.links.source"), url: project.links.source, icon: Code },
		{ key: "wiki", label: t("project.links.wiki"), url: project.links.wiki, icon: BookOpen },
		{ key: "discord", label: "Discord", url: project.links.discord, icon: MessageCircle },
		{ key: "issues", label: t("project.links.issues"), url: project.links.issues, icon: Bug },
		{
			key: "donate",
			label: t("project.links.donate"),
			url: project.links.donate?.[0]?.url,
			icon: Heart,
		},
	];
	const links = candidates.filter((link): link is ProjectLink => Boolean(link.url));

	const tabs = [
		{ value: "description" as const, label: t("project.tabs.description") },
		{ value: "versions" as const, label: t("project.tabs.versions"), count: project.versionsCount },
		...(project.gallery.length
			? [{ value: "gallery" as const, label: t("project.tabs.gallery"), count: project.gallery.length }]
			: []),
	];

	const switchTab = (next: Tab) => {
		if (next === tab) return;
		setTab(next);
		setFocusVersion(null);
	};

	/** Ссылка, с которой ушли, пропала вместе с «Обзором» — фокус на вкладку. */
	const openTab = (next: Tab, versionId?: string) => {
		switchTab(next);
		if (versionId) setFocusVersion((prev) => ({ id: versionId, seq: (prev?.seq ?? 0) + 1 }));
		requestAnimationFrame(() => {
			const root = scroller.current;
			const anchor = tabsAnchor.current;
			if (root && anchor) {
				const offset = anchor.getBoundingClientRect().top - root.getBoundingClientRect().top;
				if (offset < 0 && !versionId) root.scrollTop += offset;
			}
			if (!versionId)
				anchor?.nextElementSibling
					?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')
					?.focus();
		});
	};

	const mini = headerHidden && (
		<motion.div
			key="mini"
			ref={miniRef}
			role="group"
			aria-label={t("project.mini.label")}
			initial={{ opacity: 0 }}
			animate={{ opacity: 1 }}
			exit={{ opacity: 0 }}
			transition={{ duration: reduceMotion ? 0 : 0.12 }}
			className="absolute inset-y-0 right-8 flex items-center gap-2.5"
		>
			<ProjectIcon url={project.iconUrl} type={project.type} size={24} />
			<span title={project.title} className="max-w-60 truncate text-sm/5 font-semibold">
				{project.title}
			</span>
			{modpack ? (
				<ModpackInstallButton project={modpackRefOf(project)} tone="primary" />
			) : (
				<>
					<span className="hidden @min-[1040px]/project:block">
						<InstanceContextPicker
							size="compact"
							instances={store.instances}
							value={contentInstance}
							onChange={(id) => setCatalogContext(id, { persist: true })}
						/>
					</span>
					<InstallButton
						project={ref}
						instance={contentInstance}
						tone="primary"
						onPickInstance={(id) => setCatalogContext(id, { persist: true })}
						onIncompatibleClick={() => openTab("versions")}
					/>
				</>
			)}
		</motion.div>
	);

	return (
		<>
			<div className={cn(PROJECT_GRID, "mt-3")}>
				<div className="flex min-w-0 items-start gap-6">
					<ProjectIcon url={project.iconUrl} type={project.type} size={96} />
					<div className="min-w-0 flex-1">
						<h1
							title={project.title}
							className="truncate text-[28px]/[34px] font-semibold tracking-[-0.025em]"
						>
							{project.title}
						</h1>
						{project.author && (
							<p className="mt-0.5 truncate text-sm text-muted-foreground">{project.author}</p>
						)}
						{project.description && (
							<p className="mt-2 line-clamp-3 max-w-160 text-sm">{project.description}</p>
						)}
						<ProjectMeta hit={project} type={type} updatedPrefix className="mt-2.5" />
					</div>
				</div>
				<div className={cn(RIGHT_CELL, "min-w-0")}>
					<div ref={setInstallRow} className="flex flex-wrap items-center gap-2">
						{modpack ? (
							<ModpackInstallButton project={modpackRefOf(project)} size="page" />
						) : (
							<>
								<InstanceContextPicker
									size="page"
									instances={store.instances}
									value={contentInstance}
									flash={catalog.contextFlash}
									onChange={(id) => setCatalogContext(id, { persist: true })}
								/>
								<InstallButton
									project={ref}
									instance={contentInstance}
									size="page"
									onPickInstance={(id) => setCatalogContext(id, { persist: true })}
									onIncompatibleClick={() => switchTab("versions")}
								/>
							</>
						)}
					</div>
					{modpack ? (
						<ModpackNote project={project} versions={versions} />
					) : (
						contentInstance && (
							<InstallNote
								project={ref}
								instance={contentInstance}
								version={project.bestVersion ?? undefined}
							/>
						)
					)}
					{(links.length > 0 || alsoOn.length > 0) && (
						<div className="mt-3 -ml-2 flex flex-wrap gap-0.5">
							{alsoOn.map((twin) => (
								<Button
									key={`${twin.source}:${twin.id}`}
									variant="ghost"
									size="sm"
									className="px-2 text-[13px] text-foreground"
									onClick={() => openProject(twin, type, { url: twin.url })}
								>
									<ArrowRight />
									{t("project.alsoOn", { source: SOURCE_NAMES[twin.source] })}
								</Button>
							))}
							{links.map((link) => (
								<Button
									key={link.key}
									variant="ghost"
									size="sm"
									className="px-2 text-[13px] text-foreground"
									onClick={() => openExternal(link.url)}
								>
									<link.icon />
									{link.label}
								</Button>
							))}
						</div>
					)}
				</div>
			</div>
			<div ref={tabsAnchor} aria-hidden />
			<PageTabs
				className="mt-7"
				barClassName="sticky top-0 z-20 -mx-8 bg-background"
				listClassName="px-8"
				label={t("project.tabsLabel")}
				layoutId="project-tab"
				value={tab}
				onValueChange={switchTab}
				tabs={tabs}
				aside={<AnimatePresence initial={false}>{mini}</AnimatePresence>}
			>
				<TabsContent value="description" className="mt-7">
					<ProjectOverview
						project={project}
						versions={versions}
						instance={contentInstance}
						type={type}
						onOpenVersions={(versionId) => openTab("versions", versionId)}
						onOpenGallery={() => openTab("gallery")}
					/>
				</TabsContent>
				<TabsContent value="versions" className="mt-7">
					<ProjectVersions
						key={focusVersion?.seq ?? 0}
						source={project.source}
						type={type}
						versions={versions}
						instance={contentInstance}
						onRetry={onRetryVersions}
						focusVersionId={focusVersion?.id}
						action={(version) =>
							modpack ? (
								<ModpackInstallButton
									project={modpackRefOf(project)}
									versionId={version.id}
								/>
							) : (
								<InstallButton
									project={{
										...ref,
										gameVersions: version.gameVersions,
										loaders: version.loaders,
										siteOnly: version.siteOnly,
									}}
									instance={contentInstance}
									versionId={version.id}
									versionInstalled={version.installed}
								/>
							)
						}
					/>
				</TabsContent>
				{project.gallery.length > 0 && (
					<TabsContent value="gallery" className="mt-7">
						<ProjectGallery project={project} />
					</TabsContent>
				)}
			</PageTabs>
		</>
	);
}

function modpackRefOf(project: Project): ModpackRef {
	return { source: project.source, id: project.id, title: project.title, iconUrl: project.iconUrl };
}

function ModpackNote({ project, versions }: { project: Project; versions: Loaded<ContentVersion[]> }) {
	const { t } = useTranslation();
	const store = useInstances();
	const installed = instancesOfModpack(store.instances, project.id)[0];
	const version = versions.status === "ready" ? defaultVersion(versions.value) : null;
	return (
		<>
			{version && (
				<p className="mt-2 max-w-120 truncate text-[13px]/[18px] text-muted-foreground">
					{t("modpack.willInstall", { version: version.number, target: versionTarget(version) })}
				</p>
			)}
			{installed && (
				<button
					type="button"
					className="relative mt-1 block max-w-120 truncate rounded-md border border-transparent text-left text-[13px]/[18px] text-primary-text underline-offset-2 hover:underline"
					onClick={() => openInstancePage(installed.id, "content")}
				>
					{t("modpack.alreadyInstalled", { name: installed.name })}
				</button>
			)}
		</>
	);
}

function InstallNote({
	project,
	instance,
	version,
}: {
	project: ProjectRef & { install?: Project["install"] };
	instance: Instance;
	version: ContentVersion | undefined;
}) {
	const { t } = useTranslation();
	const instances = useInstances();
	const content = useInstanceContent(instance.id);
	const downloads = useDownloads();
	const ui = useInstallUi();
	const deps = useRequiredDeps(version, project.source);
	const worlds = useInstanceWorlds(project.type === "world" ? instance.id : null);
	const state = installButtonState({ project, instance, instances, content, downloads, ui, worlds });
	const target = targetLabel(instance);
	const className = "mt-2 max-w-120 text-[13px]/[18px] text-muted-foreground";
	if (state.kind === "siteOnly") {
		return <p className={className}>{t("catalog.install.siteOnlyNote", { name: instance.name })}</p>;
	}
	let text: string | null = null;
	if (state.kind === "incompatible")
		text =
			state.target === null
				? t("catalog.where.needsLoader")
				: t("catalog.install.noVersionHint", { target });
	else if (state.kind === "install" && version) {
		const missing =
			deps.status === "ready" ? deps.value.filter((dep) => !findProjectItem(content, dep.id)) : [];
		const base = { version: version.number, target };
		text =
			missing.length === 1
				? t("project.withDeps", { ...base, name: missing[0].title })
				: missing.length > 1
					? t("project.withDepsMany", { ...base, count: missing.length })
					: t("catalog.install.willInstall", base);
	}
	if (!text) return null;
	return <p className={className}>{text}</p>;
}
