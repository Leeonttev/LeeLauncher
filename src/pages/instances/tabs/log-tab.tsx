import {
	ArrowUpRight,
	Copy,
	FolderOpen,
	Info,
	OctagonX,
	Play,
	RotateCcw,
	ScrollText,
	type LucideIcon,
} from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { CenteredState } from "@/components/centered-state";
import { GameAccount } from "@/components/instances/game-account";
import {
	useSessionSeconds,
	useShownAccount,
	useShownGameAccount,
} from "@/components/instances/instance-text";
import { useScrollContainer } from "@/components/layout/scroll-container";
import { SearchField } from "@/components/search-field";
import { Segments } from "@/components/segments";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { contentApi, type GameAccountInfo, type Instance, type LogLine } from "@/lib/api";
import { errorText, toAppError } from "@/lib/errors";
import { formatDateTime, formatDuration, formatNumber } from "@/lib/format";
import { loadGameLog, useGameLog } from "@/lib/game-log";
import {
	launchInstance,
	openCrashDialog,
	openInstanceFolder,
	statusOf,
	useInstances,
	type InstancesStore,
} from "@/lib/instances";
import { cn } from "@/lib/utils";

type Level = "all" | "info" | "warn" | "error";

function levelOf(line: LogLine): Exclude<Level, "all"> {
	return line.level === "warn" ? "warn" : line.level === "error" ? "error" : "info";
}

function lineText(line: LogLine) {
	const head = [
		line.time && `[${line.time}]`,
		line.thread || line.level
			? `[${line.thread ?? ""}${line.level ? `/${line.level.toUpperCase()}` : ""}]`
			: null,
	]
		.filter(Boolean)
		.join(" ");
	return head ? `${head}: ${line.text}` : line.text;
}

type Match = { line: number; nth: number };

function countIn(text: string, needle: string) {
	let count = 0;
	for (let at = text.indexOf(needle); at !== -1; at = text.indexOf(needle, at + needle.length)) count++;
	return count;
}

/** До низа окна, чтобы прокручивался только лог. */
const MIN_AREA = 320;
const PAGE_BOTTOM = 40;

export function LogTab({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const log = useGameLog(instance.id, true);
	const store = useInstances();
	const status = statusOf(instance, store);
	const running = status === "running" || status === "launching";
	// Игру запустил прошлый экземпляр лаунчера: её вывода у нас нет.
	const recovered = running && store.processes[instance.id]?.recovered === true && log?.source !== "live";
	const [level, setLevel] = useState<Level>("all");
	const [query, setQuery] = useState("");
	const [current, setCurrent] = useState(0);
	const [follow, setFollow] = useState(true);
	const loading = !log || log.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);

	const lines = log?.status === "ready" ? log.lines : [];
	const counts = { warn: 0, error: 0 };
	for (const line of lines) {
		const l = levelOf(line);
		if (l !== "info") counts[l]++;
	}
	const filtered = level === "all" ? lines : lines.filter((line) => levelOf(line) === level);
	const needle = query.trim().toLocaleLowerCase();
	const matches: Match[] = [];
	if (needle) {
		filtered.forEach((line, index) => {
			const n = countIn(line.text.toLocaleLowerCase(), needle);
			for (let nth = 0; nth < n; nth++) matches.push({ line: index, nth });
		});
	}
	const active = matches.length ? Math.min(current, matches.length - 1) : -1;

	const copyAll = () => {
		navigator.clipboard.writeText(lines.map(lineText).join("\n")).then(
			() => toast.success(t("common.copied")),
			() => toast.error(t("errors.fallback")),
		);
	};
	const openFile = () => {
		contentApi.openLogFile(instance.id).catch((err: unknown) => toast.error(errorText(toAppError(err))));
	};

	const step = (delta: number) => {
		if (!matches.length) return;
		setFollow(false);
		setCurrent((active + delta + matches.length) % matches.length);
	};

	let body: ReactNode;
	if (log?.status === "error") {
		body = (
			<CenteredState
				icon={OctagonX}
				tone="destructive"
				heading="h2"
				title={t("log.error.title")}
				description={log.error ? errorText(log.error) : t("errors.fallback")}
				action={
					<Button variant="outline" onClick={() => void loadGameLog(instance.id)}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				}
			/>
		);
	} else if (recovered) {
		body = (
			<>
				<LogStatus instance={instance} store={store} lineCount={0} recovered />
				<p className="mt-3 flex max-w-prose items-start gap-2 text-[13px]/[18px] text-muted-foreground">
					<Info className="mt-px shrink-0" />
					{t("log.recovered")}
				</p>
			</>
		);
	} else if (loading) {
		body = showSkeleton ? <Skeleton className="mt-4 h-80 w-full rounded-xl" /> : null;
	} else if (!lines.length && !running) {
		body = (
			<CenteredState
				icon={ScrollText}
				heading="h2"
				title={t("log.empty.title")}
				description={t("log.empty.description")}
				action={
					status === "installed" && (
						<Button
							variant="outline"
							onClick={(e) => void launchInstance(instance.id, e.currentTarget)}
						>
							<Play fill="currentColor" />
							{t("instance.play")}
						</Button>
					)
				}
			/>
		);
	} else {
		body = (
			<>
				<LogStatus instance={instance} store={store} lineCount={lines.length} />
				<LogArea
					lines={filtered}
					needle={needle}
					matches={matches}
					active={active}
					follow={follow}
					onFollowChange={setFollow}
				/>
			</>
		);
	}

	const hasLog = !recovered && (lines.length > 0 || running);
	return (
		<div className="flex flex-1 flex-col">
			{hasLog && log?.status === "ready" && (
				<div className="flex flex-wrap items-center gap-3">
					<Segments
						label={t("log.levelLabel")}
						value={level}
						onChange={(value) => setLevel(value as Level)}
						layoutId="log-level"
						options={[
							{ value: "all", content: t("log.level.all") },
							{ value: "info", content: t("log.level.info") },
							{
								value: "warn",
								content: <LevelLabel label={t("log.level.warn")} count={counts.warn} />,
							},
							{
								value: "error",
								content: <LevelLabel label={t("log.level.error")} count={counts.error} />,
							},
						]}
					/>
					<SearchField
						value={query}
						onChange={(value) => {
							setQuery(value);
							setCurrent(0);
						}}
						placeholder={t("log.search")}
						className="w-55"
						extra={
							needle && (
								<span
									aria-live="polite"
									className="shrink-0 px-1 text-[13px]/[18px] whitespace-nowrap text-muted-foreground tabular-nums"
								>
									{matches.length
										? t("log.matches", { current: active + 1, total: matches.length })
										: t("log.noMatches")}
								</span>
							)
						}
						onKeyDown={(event) => {
							if (event.key !== "Enter") return;
							event.preventDefault();
							step(event.shiftKey ? -1 : 1);
						}}
					/>
					<div className="ml-auto flex items-center gap-2">
						<div className="mr-2 flex items-center gap-2">
							<Switch id="log-follow" checked={follow} onCheckedChange={setFollow} />
							<Label htmlFor="log-follow" className="text-sm font-medium">
								{t("log.follow")}
							</Label>
						</div>
						<IconAction icon={Copy} label={t("log.copyAll")} onClick={copyAll} />
						<IconAction icon={ArrowUpRight} label={t("log.openFile")} onClick={openFile} />
						<IconAction
							icon={FolderOpen}
							label={t("log.folder")}
							onClick={() => void openInstanceFolder(instance.id, "logs")}
						/>
					</div>
				</div>
			)}
			{body}
		</div>
	);
}

function LevelLabel({ label, count }: { label: string; count: number }) {
	return (
		<span className="flex items-center gap-1.5">
			{label}
			{count > 0 && <span className="text-muted-foreground tabular-nums">{formatNumber(count)}</span>}
		</span>
	);
}

function IconAction({
	icon: Icon,
	label,
	onClick,
}: {
	icon: LucideIcon;
	label: string;
	onClick: () => void;
}) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button variant="outline" size="icon" aria-label={label} onClick={onClick}>
					<Icon />
				</Button>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}

function LogStatus({
	instance,
	store,
	lineCount,
	recovered = false,
}: {
	instance: Instance;
	store: InstancesStore;
	lineCount: number;
	recovered?: boolean;
}) {
	const { t } = useTranslation();
	const log = useGameLog(instance.id, false);
	const status = statusOf(instance, store);
	const session = useSessionSeconds(instance, store);
	const exit = store.lastExits[instance.id];
	const runningAccount = useShownGameAccount(instance.id, store);
	const exitAccount = useShownAccount(exit?.account ?? null);

	if (status === "running" || status === "launching") {
		return (
			<p className="mt-3.5 flex items-center gap-2 text-[13px]/[18px] text-primary-text">
				{runningAccount ? (
					<AccountPrefix account={runningAccount} />
				) : (
					<span aria-hidden className="size-2 shrink-0 rounded-full bg-primary" />
				)}
				<span className="truncate tabular-nums">
					{recovered
						? t("log.runningNoLog", { duration: formatDuration(session ?? 0) })
						: t("log.running", { duration: formatDuration(session ?? 0), count: lineCount })}
				</span>
			</p>
		);
	}
	if (log?.source === "live" && exit?.crashed) {
		return (
			<div className="mt-2 flex items-center gap-2 text-[13px]/[18px] text-destructive">
				<OctagonX className="shrink-0" />
				{exitAccount && <AccountPrefix account={exitAccount} />}
				<span className="truncate">
					{exit.exitCode !== null
						? t("log.crashed", { code: exit.exitCode })
						: t("log.crashedNoCode")}
				</span>
				<Button
					variant="ghost"
					size="sm"
					onClick={() => void openCrashDialog(instance.id, exit.exitCode)}
				>
					{t("common.details")}
				</Button>
			</div>
		);
	}
	let text: string | null = null;
	if (log?.source === "live" && exit) {
		text = t("log.last", { when: formatDateTime(exit.at), duration: formatDuration(exit.durationSecs) });
	} else if (log?.source === "file" && log.file) {
		text = t("log.lastShort", { when: formatDateTime(log.file.modified) });
	} else if (instance.lastPlayedAt) {
		text = t("log.lastShort", { when: formatDateTime(Date.parse(instance.lastPlayedAt)) });
	}
	if (!text) return null;
	return <p className="mt-3.5 truncate text-[13px]/[18px] text-muted-foreground">{text}</p>;
}

/** «◼ ник ·» перед статусом: обрезается ник, а не статус. */
function AccountPrefix({ account }: { account: GameAccountInfo }) {
	return (
		<span className="flex max-w-[45%] min-w-0 shrink items-center gap-2 text-muted-foreground">
			<GameAccount account={account} />
			<span aria-hidden className="shrink-0">
				{"·"}
			</span>
			<span className="sr-only">{account.username}</span>
		</span>
	);
}

function LogArea({
	lines,
	needle,
	matches,
	active,
	follow,
	onFollowChange,
}: {
	lines: LogLine[];
	needle: string;
	matches: Match[];
	active: number;
	follow: boolean;
	onFollowChange: (follow: boolean) => void;
}) {
	const { t } = useTranslation();
	const mainRef = useScrollContainer();
	const frameRef = useRef<HTMLDivElement>(null);
	const scrollRef = useRef<HTMLDivElement>(null);
	const [height, setHeight] = useState(MIN_AREA);
	const activeMatch = active >= 0 ? matches[active] : null;

	useLayoutEffect(() => {
		const main = mainRef.current;
		const frame = frameRef.current;
		if (!main || !frame) return;
		const update = () => {
			const top = frame.getBoundingClientRect().top - main.getBoundingClientRect().top + main.scrollTop;
			setHeight(Math.max(MIN_AREA, Math.floor(main.clientHeight - top - PAGE_BOTTOM)));
		};
		update();
		const observer = new ResizeObserver(update);
		observer.observe(main);
		const page = frame.closest("[data-instance-page]");
		if (page) observer.observe(page);
		return () => observer.disconnect();
	}, [mainRef]);

	// Компилятор React не мемоизирует компонент с виртуализатором — так и нужно.
	// eslint-disable-next-line react-hooks/incompatible-library
	const virtualizer = useVirtualizer({
		count: lines.length,
		getScrollElement: () => scrollRef.current,
		estimateSize: () => 20,
		overscan: 20,
		paddingStart: 8,
		paddingEnd: 8,
	});

	useEffect(() => {
		if (!follow || !lines.length) return;
		const frame = requestAnimationFrame(() => {
			const el = scrollRef.current;
			if (el) el.scrollTop = el.scrollHeight;
		});
		return () => cancelAnimationFrame(frame);
	}, [follow, lines.length, height]);

	useEffect(() => {
		if (activeMatch) virtualizer.scrollToIndex(activeMatch.line, { align: "center" });
	}, [activeMatch, virtualizer]);

	return (
		<div ref={frameRef} className="mt-3 overflow-clip rounded-xl bg-muted" style={{ height }}>
			<div
				ref={scrollRef}
				role="log"
				aria-label={t("log.areaLabel")}
				tabIndex={0}
				className="size-full overflow-auto font-mono text-[13px]/5 outline-none select-text focus-visible:bg-accent/40"
				onScroll={(event) => {
					const el = event.currentTarget;
					const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
					if (atBottom !== follow) onFollowChange(atBottom);
				}}
			>
				<div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
					{virtualizer.getVirtualItems().map((item) => (
						<div
							key={item.index}
							ref={virtualizer.measureElement}
							data-index={item.index}
							className="absolute inset-x-0 top-0 flex gap-3 px-3"
							style={{ transform: `translateY(${item.start}px)` }}
						>
							<Line
								line={lines[item.index]}
								needle={needle}
								activeNth={activeMatch?.line === item.index ? activeMatch.nth : -1}
							/>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}

const LEVEL_CLASS: Record<string, string> = {
	info: "text-muted-foreground",
	debug: "text-muted-foreground",
	warn: "text-warning",
	error: "text-destructive",
};

function Line({ line, needle, activeNth }: { line: LogLine; needle: string; activeNth: number }) {
	const tone =
		line.level === "warn" ? "text-warning" : line.level === "error" ? "text-destructive" : undefined;
	return (
		<>
			<span className="w-16 shrink-0 text-muted-foreground">{line.time ?? ""}</span>
			<span className={cn("w-12 shrink-0 font-semibold", line.level && LEVEL_CLASS[line.level])}>
				{line.level?.toUpperCase() ?? ""}
			</span>
			<span className="w-30 shrink-0 truncate text-muted-foreground">{line.thread ?? ""}</span>
			<span className={cn("min-w-0 flex-1 break-all whitespace-pre-wrap", tone)}>
				<Highlight text={line.text} needle={needle} activeNth={activeNth} />
			</span>
		</>
	);
}

function Highlight({ text, needle, activeNth }: { text: string; needle: string; activeNth: number }) {
	if (!needle) return text;
	const lower = text.toLocaleLowerCase();
	const parts: ReactNode[] = [];
	let from = 0;
	let nth = 0;
	for (let at = lower.indexOf(needle); at !== -1; at = lower.indexOf(needle, at + needle.length)) {
		if (at > from) parts.push(text.slice(from, at));
		parts.push(
			<mark
				key={at}
				className={
					nth === activeNth ? "bg-primary text-primary-foreground" : "bg-primary-soft text-inherit"
				}
			>
				{text.slice(at, at + needle.length)}
			</mark>,
		);
		from = at + needle.length;
		nth++;
	}
	if (from < text.length) parts.push(text.slice(from));
	return parts;
}
