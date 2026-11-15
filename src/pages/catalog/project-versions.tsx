import { ChevronRight, OctagonX, RotateCcw } from "lucide-react";
import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { GameVersionPicker } from "@/components/catalog/game-version-picker";
import { CenteredState } from "@/components/centered-state";
import { MarkdownView } from "@/components/markdown";
import { Segments } from "@/components/segments";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
	catalogApi,
	type ContentSource,
	type ContentVersion,
	type Instance,
	type VersionChannel,
} from "@/lib/api";
import { instanceLoaders, LOADER_FILTER_NAMES, visibleLoaders, type CatalogType } from "@/lib/catalog";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { formatAgo, formatBytes, formatCompact } from "@/lib/format";
import { compareGameVersions, LOADER_NAMES } from "@/lib/loaders";
import { cn } from "@/lib/utils";

type Loaded<T> = { status: "loading" } | { status: "ready"; value: T } | { status: "error"; error: AppError };
type Channel = "all" | VersionChannel;

const ANY = "__any__";
const PAGE = 50;
const CHANNELS: Channel[] = ["all", "release", "beta", "alpha"];
/** Уже этой ширины контейнера столбцы «Загрузчики» и «Загрузки» прячутся. */
const COLUMNS =
	"grid grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_112px_120px] items-center gap-4 @min-[720px]/project:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_minmax(0,1fr)_112px_88px_120px]";
const WIDE_ONLY = "hidden @min-[720px]/project:block";

function loaderName(loader: string) {
	return (LOADER_NAMES as Record<string, string>)[loader] ?? LOADER_FILTER_NAMES[loader] ?? loader;
}

function compactVersions(versions: string[]): string {
	const sorted = [...versions].sort(compareGameVersions);
	if (sorted.length <= 2) return sorted.join(", ");
	return `${sorted[sorted.length - 1]} – ${sorted[0]}`;
}

/** `focusVersionId` — фильтры не прячут эту версию, строка раскрыта и в фокусе. */
export function ProjectVersions({
	type,
	source,
	versions,
	instance,
	onRetry,
	action,
	focusVersionId,
}: {
	type: CatalogType;
	source: ContentSource;
	versions: Loaded<ContentVersion[]>;
	instance: Instance | null;
	onRetry: () => void;
	action: (version: ContentVersion) => ReactNode;
	focusVersionId?: string;
}) {
	const { t } = useTranslation();
	const ids = useId();
	const all = versions.status === "ready" ? versions.value : [];
	const hasRelease = all.some((v) => v.channel === "release");
	const focused = focusVersionId ? all.find((v) => v.id === focusVersionId) : undefined;
	const fitsInstance = (v: ContentVersion) => {
		if (!instance) return true;
		if (!v.gameVersions.includes(instance.gameVersion)) return false;
		const wanted = instanceLoaders(type, instance);
		return !(type === "mod" && wanted.length && !v.loaders.some((l) => wanted.includes(l)));
	};
	const [channel, setChannel] = useState<Channel | null>(() =>
		focused && focused.channel !== "release" ? "all" : null,
	);
	const [gameVersion, setGameVersion] = useState<string | null>(null);
	const [loader, setLoader] = useState<string | null>(null);
	const [onlyFor, setOnlyFor] = useState(() => !(focused && !fitsInstance(focused)));
	const [limit, setLimit] = useState(PAGE);
	const listRef = useRef<HTMLUListElement>(null);
	const rootRef = useRef<HTMLDivElement>(null);
	const retryRef = useRef<HTMLButtonElement>(null);
	const retrying = useRef(false);
	const activeChannel: Channel = channel ?? (hasRelease ? "release" : "all");
	const locked = Boolean(instance && onlyFor);
	const loaders = [...new Set(all.flatMap((v) => visibleLoaders(v.loaders)))];

	const filtered = all.filter((v) => {
		if (activeChannel !== "all" && v.channel !== activeChannel) return false;
		if (locked) return fitsInstance(v);
		if (gameVersion && !v.gameVersions.includes(gameVersion)) return false;
		if (loader && !v.loaders.includes(loader)) return false;
		return true;
	});
	const focusIndex = focused ? filtered.indexOf(focused) : -1;
	const shown = filtered.slice(0, Math.max(limit, focusIndex + 1));

	const scrolledRef = useRef(false);
	useLayoutEffect(() => {
		if (scrolledRef.current || !focusVersionId) return;
		const trigger = listRef.current?.querySelector<HTMLElement>(
			`[data-version-id="${CSS.escape(focusVersionId)}"] [data-version-trigger]`,
		);
		if (!trigger) return;
		scrolledRef.current = true;
		trigger.focus({ preventScroll: true });
		trigger.scrollIntoView({ block: "center" });
	});

	// «Повторить» пропадает на время загрузки: фокус ждёт на вкладке, при новой ошибке возвращается.
	useLayoutEffect(() => {
		if (!retrying.current || versions.status === "loading") return;
		retrying.current = false;
		if (versions.status === "error") retryRef.current?.focus();
	}, [versions.status]);

	if (versions.status === "loading") {
		return (
			<div ref={rootRef} tabIndex={-1} aria-busy className="flex flex-col gap-3 outline-none">
				{Array.from({ length: 6 }, (_, i) => (
					<Skeleton key={i} className="h-10 w-full rounded-lg" />
				))}
			</div>
		);
	}
	if (versions.status === "error") {
		return (
			<div ref={rootRef} tabIndex={-1} className="outline-none">
				<CenteredState
					icon={OctagonX}
					tone="destructive"
					heading="h2"
					fill={false}
					className="py-12"
					title={t("project.versions.failed")}
					description={errorText(versions.error)}
					action={
						<Button
							ref={retryRef}
							variant="outline"
							onClick={() => {
								retrying.current = true;
								rootRef.current?.focus();
								onRetry();
							}}
						>
							<RotateCcw />
							{t("common.retry")}
						</Button>
					}
				/>
			</div>
		);
	}

	return (
		<div ref={rootRef} tabIndex={-1} className="outline-none">
			<div className="flex flex-wrap items-center gap-3">
				<span id={`${ids}-version`} className="sr-only">
					{t("catalog.filter.gameVersion")}
				</span>
				<div className={cn("w-44", locked && "opacity-60")}>
					<GameVersionPicker
						labelledBy={`${ids}-version`}
						value={locked && instance ? instance.gameVersion : gameVersion}
						onChange={(value) => {
							setGameVersion(value);
							setLimit(PAGE);
						}}
						disabled={locked}
					/>
				</div>
				{loaders.length > 1 && (
					<Select
						value={loader ?? ANY}
						onValueChange={(value) => {
							setLoader(value === ANY ? null : value);
							setLimit(PAGE);
						}}
						disabled={locked}
					>
						<SelectTrigger
							className={cn("h-9 w-40", locked && "opacity-60")}
							aria-label={t("catalog.filter.loader")}
						>
							<SelectValue />
						</SelectTrigger>
						<SelectContent position="popper">
							<SelectItem value={ANY}>{t("catalog.filter.anyLoader")}</SelectItem>
							{loaders.map((l) => (
								<SelectItem key={l} value={l}>
									{loaderName(l)}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				)}
				<Segments
					label={t("project.versions.channelLabel")}
					value={activeChannel}
					onChange={(value) => {
						setChannel(value as Channel);
						setLimit(PAGE);
					}}
					layoutId="project-channel"
					options={CHANNELS.map((c) => ({ value: c, content: t(`project.versions.channel.${c}`) }))}
				/>
				{instance && (
					<label className="ml-auto flex items-center gap-2.5 text-sm">
						<span className="max-w-60 truncate">
							{t("project.versions.onlyFor", { name: instance.name })}
						</span>
						<Switch checked={onlyFor} onCheckedChange={setOnlyFor} />
					</label>
				)}
			</div>
			<div
				aria-hidden
				className={cn(COLUMNS, "mt-5 h-8 border-b text-[13px]/[18px] text-muted-foreground")}
			>
				<span>{t("project.versions.col.version")}</span>
				<span>{t("project.versions.col.game")}</span>
				<span className={WIDE_ONLY}>{t("project.versions.col.loaders")}</span>
				<span>{t("project.versions.col.published")}</span>
				<span className={WIDE_ONLY}>{t("project.versions.col.downloads")}</span>
				<span />
			</div>
			{shown.length === 0 ? (
				<p className="py-10 text-center text-[13px]/[18px] text-muted-foreground">
					{t("project.versions.empty")}
				</p>
			) : (
				<ul ref={listRef} className="mt-1">
					{shown.map((version) => (
						<VersionRow
							key={version.id}
							version={version}
							source={source}
							action={action(version)}
							defaultOpen={version.id === focusVersionId}
						/>
					))}
				</ul>
			)}
			{filtered.length > shown.length && (
				<div className="mt-4 flex justify-center">
					<Button variant="outline" onClick={() => setLimit((n) => n + PAGE)}>
						{t("project.versions.more")}
					</Button>
				</div>
			)}
		</div>
	);
}

export function ChannelChip({ channel }: { channel: VersionChannel }) {
	const { t } = useTranslation();
	return (
		<span
			className={cn(
				"inline-flex h-[22px] shrink-0 items-center rounded-md px-[7px] text-[13px] font-medium",
				channel === "release" ? "bg-primary-soft text-primary-text" : "bg-warning/18 text-warning",
			)}
		>
			{t(`project.versions.chip.${channel}`)}
		</span>
	);
}

function VersionRow({
	version,
	source,
	action,
	defaultOpen = false,
}: {
	version: ContentVersion;
	source: ContentSource;
	action: ReactNode;
	defaultOpen?: boolean;
}) {
	const { t } = useTranslation();
	const [open, setOpen] = useState(defaultOpen);
	return (
		<li data-version-id={version.id}>
			<Collapsible open={open} onOpenChange={setOpen}>
				<div className="relative -mx-3 -my-0.5 rounded-lg border border-transparent px-2.5 transition-colors duration-(--duration-base) hover:bg-accent has-[[data-version-trigger]:focus-visible]:outline-2 has-[[data-version-trigger]:focus-visible]:outline-offset-2 has-[[data-version-trigger]:focus-visible]:outline-ring">
					<div className={cn(COLUMNS, "h-13")}>
						<CollapsibleTrigger
							data-version-trigger
							aria-label={t("project.versions.expand", { version: version.number })}
							className="flex min-w-0 items-center gap-2 text-left outline-none after:absolute after:inset-0 after:content-['']"
						>
							<ChevronRight
								className={cn(
									"shrink-0 text-muted-foreground transition-transform",
									open && "rotate-90",
								)}
							/>
							<span title={version.number} className="truncate text-sm/5 font-semibold">
								{version.number}
							</span>
							<ChannelChip channel={version.channel} />
						</CollapsibleTrigger>
						<span className="truncate text-[13px]/[18px]">
							{compactVersions(version.gameVersions)}
						</span>
						<span className={cn(WIDE_ONLY, "truncate text-[13px]/[18px]")}>
							{visibleLoaders(version.loaders).map(loaderName).join(", ")}
						</span>
						<span className="truncate text-[13px]/[18px] text-muted-foreground">
							{formatAgo(version.publishedAt)}
						</span>
						<span
							className={cn(
								WIDE_ONLY,
								"truncate text-[13px]/[18px] text-muted-foreground tabular-nums",
							)}
						>
							{formatCompact(version.downloads)}
						</span>
						{action}
					</div>
				</div>
				<CollapsibleContent>
					{open && <VersionDetails version={version} source={source} />}
				</CollapsibleContent>
			</Collapsible>
		</li>
	);
}

function VersionDetails({ version, source }: { version: ContentVersion; source: ContentSource }) {
	const { t } = useTranslation();
	const [full, setFull] = useState<Loaded<ContentVersion>>({ status: "loading" });

	useEffect(() => {
		let alive = true;
		catalogApi
			.version(version.id, source)
			.then((value) => alive && setFull({ status: "ready", value }))
			.catch((err: unknown) => alive && setFull({ status: "error", error: toAppError(err) }));
		return () => {
			alive = false;
		};
	}, [version.id, source]);

	return (
		<div className="flex flex-col gap-5 pt-2 pb-5 pl-6">
			<div>
				<p className="text-[13px]/[18px] font-semibold">{t("project.versions.changelog")}</p>
				<div className="mt-2 max-h-80 overflow-y-auto select-text">
					{full.status === "loading" ? (
						<div aria-busy>
							<Skeleton className="h-3.5 w-[60%]" />
							<Skeleton className="mt-2 h-3.5 w-[40%]" />
						</div>
					) : full.status === "error" ? (
						<p className="flex items-start gap-1.5 text-[13px]/[18px] text-destructive">
							<OctagonX className="mt-px shrink-0" />
							<span className="min-w-0">{errorText(full.error)}</span>
						</p>
					) : full.value.changelog?.trim() ? (
						<MarkdownView source={full.value.changelog} />
					) : (
						<p className="text-[13px]/[18px] text-muted-foreground">
							{t("project.versions.noChangelog")}
						</p>
					)}
				</div>
			</div>
			<div>
				<p className="text-[13px]/[18px] font-semibold">{t("project.versions.files")}</p>
				<ul className="mt-2 flex flex-col gap-1 text-[13px]/[18px]">
					{version.files.map((file) => (
						<li key={file.name} className="flex gap-3">
							<span className="min-w-0 truncate font-mono select-text">{file.name}</span>
							<span className="shrink-0 text-muted-foreground">{formatBytes(file.size)}</span>
						</li>
					))}
				</ul>
			</div>
		</div>
	);
}
