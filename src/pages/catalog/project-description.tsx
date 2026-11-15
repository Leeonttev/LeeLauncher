import { ArrowRight, User } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { MarkdownView } from "@/components/markdown";
import { PixelGrid } from "@/components/pixel-grid";
import { Skeleton } from "@/components/ui/skeleton";
import { external, type ContentVersion, type Instance, type Project, type Side } from "@/lib/api";
import {
	LOADER_FILTER_NAMES,
	openProject,
	SOURCE_NAMES,
	visibleLoaders,
	type CatalogType,
} from "@/lib/catalog";
import { findProjectItem, useInstanceContent } from "@/lib/content";
import { formatAgo, formatDate } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { compareGameVersions, LOADER_NAMES } from "@/lib/loaders";
import { useRequiredDeps } from "@/lib/project-page";
import { cn } from "@/lib/utils";
import { GalleryPreview } from "@/pages/catalog/project-gallery";
import { ChannelChip } from "@/pages/catalog/project-versions";

type Loaded<T> = { status: "loading" } | { status: "ready"; value: T } | { status: "error" };

/** Общая для шапки и «Обзора», чтобы колонки совпали. */
export const PROJECT_GRID =
	"flex flex-col @min-[1040px]/project:grid @min-[1040px]/project:grid-cols-[minmax(0,800px)_minmax(352px,1fr)] @min-[1040px]/project:items-start @min-[1040px]/project:gap-x-12";

const RELEASE = /^\d+\.\d+(\.\d+)?$/;
const LATEST_COUNT = 3;

/** От старой к новой, по релизам; нет релизов — как есть, через запятую. */
function versionRange(versions: string[]): string {
	const releases = versions.filter((v) => RELEASE.test(v)).sort(compareGameVersions);
	if (releases.length === 0) return versions.slice(0, 3).join(", ");
	if (releases.length === 1) return releases[0];
	return `${releases[releases.length - 1]} – ${releases[0]}`;
}

function loaderName(loader: string) {
	return (LOADER_NAMES as Record<string, string>)[loader] ?? LOADER_FILTER_NAMES[loader] ?? loader;
}

function openLink(url: string) {
	external.open(url).catch(() => toast.error(i18n.t("common.openLinkFailed")));
}

export function ProjectOverview({
	project,
	versions,
	instance,
	type,
	onOpenVersions,
	onOpenGallery,
}: {
	project: Project;
	versions: Loaded<ContentVersion[]>;
	instance: Instance | null;
	type: CatalogType;
	onOpenVersions: (versionId?: string) => void;
	onOpenGallery: () => void;
}) {
	const { t } = useTranslation();
	const version =
		project.bestVersion ??
		(versions.status === "ready"
			? (versions.value.find((v) => v.channel === "release") ?? versions.value[0])
			: undefined);
	return (
		<div className={PROJECT_GRID}>
			<div className="min-w-0 select-text">
				{project.body.trim() ? (
					<MarkdownView source={project.body} />
				) : (
					<p className="text-sm text-muted-foreground">{t("project.emptyDescription")}</p>
				)}
			</div>
			<div className="mt-8 flex min-w-0 flex-col gap-8 border-t pt-7 @min-[1040px]/project:mt-0 @min-[1040px]/project:border-t-0 @min-[1040px]/project:pt-0">
				<Facts project={project} version={version} type={type} />
				{version && type !== "modpack" && (
					<Dependencies project={project} version={version} instance={instance} />
				)}
				<LatestVersions versions={versions} count={project.versionsCount} onOpen={onOpenVersions} />
				{project.gallery.length > 0 && (
					<Block
						title={t("project.galleryPreview.title")}
						count={project.gallery.length}
						action={{ label: t("project.galleryPreview.all"), onClick: onOpenGallery }}
					>
						<GalleryPreview project={project} />
					</Block>
				)}
				{project.team.length > 1 && <Team team={project.team} />}
			</div>
		</div>
	);
}

function Block({
	title,
	count,
	action,
	children,
	label,
}: {
	title: string;
	count?: number;
	action?: { label: string; onClick: () => void };
	children: ReactNode;
	label?: string;
}) {
	return (
		<section aria-label={label ?? title}>
			<div className="flex min-h-6 items-center gap-3">
				<h2 className="text-[13px]/[18px] font-semibold">
					{title}
					{count !== undefined && (
						<span className="ml-1.5 font-medium text-muted-foreground tabular-nums">{count}</span>
					)}
				</h2>
				{action && (
					<button
						type="button"
						className="relative ml-auto flex shrink-0 items-center gap-1 rounded-md border border-transparent px-1 text-[13px]/[18px] font-semibold text-primary-text hover:underline"
						onClick={action.onClick}
					>
						{action.label}
						<ArrowRight />
					</button>
				)}
			</div>
			<div className="mt-3">{children}</div>
		</section>
	);
}

function Fact({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
	return (
		<div className="min-w-0">
			<p className="text-[13px]/[18px] font-semibold">{label}</p>
			<div className="mt-1 text-sm/5">{children}</div>
			{hint && <p className="mt-0.5 text-[13px]/[18px] text-muted-foreground">{hint}</p>}
		</div>
	);
}

/** Если источник не указывает это у проекта — берём метки версии. */
function envText(project: Project, version: ContentVersion | undefined): string | null {
	let clientSide: Side = project.clientSide;
	let serverSide: Side = project.serverSide;
	if (clientSide === "unknown" && serverSide === "unknown" && version?.side) {
		clientSide = version.side.client;
		serverSide = version.side.server;
	}
	if (clientSide === "unknown" && serverSide === "unknown") return null;
	const client = clientSide === "required" || clientSide === "optional";
	const server = serverSide === "required" || serverSide === "optional";
	if (client && server) return i18n.t("project.facts.envBoth");
	if (client) return i18n.t("project.facts.envClient");
	if (server) return i18n.t("project.facts.envServer");
	return i18n.t("project.facts.envUnknown");
}

function Facts({
	project,
	version,
	type,
}: {
	project: Project;
	version: ContentVersion | undefined;
	type: CatalogType;
}) {
	const { t } = useTranslation();
	const loaders = visibleLoaders(project.loaders).map(loaderName);
	const license = project.license;
	const env =
		type !== "datapack" && type !== "resourcepack" && type !== "world" ? envText(project, version) : null;
	return (
		<section
			aria-label={t("project.facts.label")}
			className="grid grid-cols-2 gap-x-8 gap-y-6 @min-[1040px]/project:grid-cols-[repeat(auto-fill,minmax(200px,1fr))]"
		>
			{(project.gameVersions.length > 0 || loaders.length > 0) && (
				<Fact
					label={t("project.facts.compatibility")}
					hint={
						project.gameVersions.length > 0 && loaders.length > 0
							? loaders.join(" · ")
							: undefined
					}
				>
					{project.gameVersions.length > 0
						? `Minecraft ${versionRange(project.gameVersions)}`
						: loaders.join(" · ")}
				</Fact>
			)}
			{env && <Fact label={t("project.facts.env")}>{env}</Fact>}
			{license && (
				<Fact label={t("project.facts.license")}>
					{license.url ? (
						<button
							type="button"
							className="relative -mx-0.5 max-w-full truncate rounded-md border border-transparent px-0.5 text-left text-primary-text underline underline-offset-2 hover:no-underline"
							onClick={() => openLink(license.url ?? "")}
						>
							{license.name || license.id}
						</button>
					) : (
						<p className="truncate">{license.name || license.id}</p>
					)}
				</Fact>
			)}
			<Fact
				label={t("project.facts.source")}
				hint={
					project.publishedAt > 0
						? t("project.facts.publishedShort", {
								date: formatDate(new Date(project.publishedAt).toISOString()),
							})
						: undefined
				}
			>
				{SOURCE_NAMES[project.source]}
			</Fact>
			{project.siteOnly && (
				<Fact label={t("project.facts.download")} hint={t("project.facts.siteOnlyHint")}>
					{t("project.facts.siteOnly")}
				</Fact>
			)}
		</section>
	);
}

function Dependencies({
	project,
	version,
	instance,
}: {
	project: Project;
	version: ContentVersion;
	instance: Instance | null;
}) {
	const { t } = useTranslation();
	const content = useInstanceContent(instance?.id ?? null);
	const deps = useRequiredDeps(version, project.source);
	if (deps.status === "error" || (deps.status === "ready" && !deps.value.length)) return null;
	return (
		<Block title={t("project.facts.dependencies")}>
			{deps.status === "loading" ? (
				<div aria-busy className="flex items-center gap-2.5">
					<Skeleton className="size-8 rounded-lg" />
					<Skeleton className="h-3.5 w-28" />
				</div>
			) : (
				<PixelGrid minColumn={240} gap={24} className="gap-y-2.5">
					{deps.value.map((dep) => {
						const installed = Boolean(findProjectItem(content, dep.id));
						return (
							<div key={dep.id} className="flex min-w-0 items-center gap-2.5">
								<ProjectIcon url={dep.iconUrl} type={dep.type} size={32} />
								<span className="min-w-0 flex-1">
									<button
										type="button"
										className="relative -m-0.5 block max-w-[calc(100%+4px)] rounded-md border border-transparent text-left text-sm/5 font-semibold text-primary-text hover:underline"
										onClick={() => openProject(dep)}
									>
										<span className="block truncate">{dep.title}</span>
									</button>
									<span className="block text-[13px]/[18px] text-muted-foreground">
										{installed
											? t("project.facts.dependencyInstalled")
											: t("project.facts.dependencyRequired")}
									</span>
								</span>
							</div>
						);
					})}
				</PixelGrid>
			)}
		</Block>
	);
}

function LatestVersions({
	versions,
	count,
	onOpen,
}: {
	versions: Loaded<ContentVersion[]>;
	count: number;
	onOpen: (versionId?: string) => void;
}) {
	const { t } = useTranslation();
	if (versions.status === "error") return null;
	if (versions.status === "ready" && versions.value.length === 0) return null;
	const latest =
		versions.status === "ready"
			? [...versions.value].sort((a, b) => b.publishedAt - a.publishedAt).slice(0, LATEST_COUNT)
			: [];
	return (
		<Block
			title={t("project.latest.title")}
			action={{ label: t("project.latest.all", { count }), onClick: () => onOpen() }}
		>
			{versions.status === "loading" ? (
				<div aria-busy className="flex flex-col">
					{Array.from({ length: LATEST_COUNT }, (_, i) => (
						<div key={i} className="flex h-11 items-center border-b">
							<Skeleton className="h-4 w-[60%]" />
						</div>
					))}
				</div>
			) : (
				<ul>
					{latest.map((version) => (
						<li key={version.id} className="border-b">
							<button
								type="button"
								aria-label={t("project.versions.expand", { version: version.number })}
								className="relative -mx-2 flex h-11 w-[calc(100%+1rem)] items-center gap-2 rounded-md border border-transparent px-2 text-left transition-colors duration-(--duration-fast) ease-out hover:bg-accent"
								onClick={() => onOpen(version.id)}
							>
								<span className="max-w-40 shrink-0 truncate text-sm/5 font-semibold">
									{version.number}
								</span>
								<ChannelChip channel={version.channel} />
								<span className="min-w-0 flex-1 truncate text-[13px]/[18px] text-muted-foreground">
									{[
										[...version.gameVersions].sort(compareGameVersions)[0],
										visibleLoaders(version.loaders).map(loaderName).join(", "),
									]
										.filter(Boolean)
										.join(" · ")}
								</span>
								<span className="shrink-0 text-[13px]/[18px] text-muted-foreground">
									{formatAgo(version.publishedAt)}
								</span>
							</button>
						</li>
					))}
				</ul>
			)}
		</Block>
	);
}

function Team({ team }: { team: Project["team"] }) {
	const { t } = useTranslation();
	return (
		<Block title={t("project.team.title")}>
			<PixelGrid minColumn={240} gap={24} className="gap-y-2.5">
				{team.map((member) => (
					<div key={member.id} className="flex min-w-0 items-center gap-2.5">
						<TeamAvatar url={member.avatarUrl} />
						<span className="min-w-0 flex-1">
							<span className="block truncate text-sm/5 font-semibold">{member.username}</span>
							{member.role && (
								<span className="block truncate text-[13px]/[18px] text-muted-foreground">
									{member.role}
								</span>
							)}
						</span>
					</div>
				))}
			</PixelGrid>
		</Block>
	);
}

function TeamAvatar({ url }: { url: string | null }) {
	return url ? <LoadedAvatar key={url} url={url} /> : <AvatarFallback />;
}

function LoadedAvatar({ url }: { url: string }) {
	const [state, setState] = useState<"loading" | "ready" | "error">("loading");
	if (state === "error") return <AvatarFallback />;
	return (
		<span aria-hidden className="relative block size-8 shrink-0 rounded-lg">
			{state === "loading" && <span className="absolute inset-0 animate-pulse bg-foreground/10" />}
			<img
				src={url}
				alt=""
				loading="lazy"
				decoding="async"
				referrerPolicy="no-referrer"
				draggable={false}
				className={cn("size-full object-cover", state === "loading" && "opacity-0")}
				onLoad={() => setState("ready")}
				onError={() => setState("error")}
			/>
		</span>
	);
}

function AvatarFallback() {
	return (
		<span
			aria-hidden
			className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
		>
			<User />
		</span>
	);
}
