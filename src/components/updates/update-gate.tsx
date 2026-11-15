import { Check, Download, Info, LoaderCircle, OctagonX, RotateCw, Square, WifiOff } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { AppIcon } from "@/components/brand/identity";
import { ErrorDetails } from "@/components/error-details";
import { InstanceThumb } from "@/components/instances/instance-cover";
import { MarkdownView } from "@/components/markdown";
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
import { external, type Download as DownloadTask, type Instance, type UpdateProgress } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import { initDownloads, useDownloads } from "@/lib/downloads";
import type { AppError } from "@/lib/errors";
import { formatBytes, formatDuration } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { initInstances, useInstances } from "@/lib/instances";
import {
	RELEASES_URL,
	activeTasks,
	canSkip,
	cancelTasksAndUpdate,
	localizedNotes,
	skipUpdate,
	startUpdate,
	stopGames,
	useUpdates,
} from "@/lib/updates";
import { cn } from "@/lib/utils";

const easeOut = [0.22, 1, 0.36, 1] as const;

function etaText(seconds: number) {
	if (seconds < 60) return i18n.t("update.etaSeconds", { count: Math.max(1, Math.ceil(seconds)) });
	return formatDuration(Math.ceil(seconds / 60) * 60);
}

function taskName(task: DownloadTask, instances: Instance[]) {
	if (task.kind === "java") return i18n.t("downloads.java", { major: task.javaMajor ?? "" });
	const instance = instances.find((i) => i.id === task.instanceId);
	return instance?.name ?? task.title ?? i18n.t("downloads.instanceFallback");
}

function listText(names: string[]) {
	const unique = [...new Set(names)];
	const shown = unique.slice(0, 3);
	const text = new Intl.ListFormat(i18n.language, { type: "conjunction" }).format(shown);
	return unique.length > shown.length ? `${text}…` : text;
}

/** Закрыть нельзя: только обновиться, а после двух неудач подряд — продолжить без обновления. */
export function UpdateGate() {
	const { t, i18n: i18next } = useTranslation();
	const store = useUpdates();
	const instancesStore = useInstances();
	const downloads = useDownloads();
	const { reduceMotion } = useAppearance();
	const mainRef = useRef<HTMLButtonElement>(null);
	const rootRef = useRef<HTMLDivElement>(null);
	const [confirm, setConfirm] = useState<"stop" | "tasks" | null>(null);

	useEffect(() => {
		initInstances();
		initDownloads();
		mainRef.current?.focus();
	}, []);

	const phase = store.phase;
	const installing = phase === "installing";
	const wasInstalling = useRef(false);
	useEffect(() => {
		// Если установка не удалась, фокус возвращается на «Повторить».
		if (installing) rootRef.current?.focus();
		else if (wasInstalling.current) mainRef.current?.focus();
		wasInstalling.current = installing;
	}, [installing]);

	const info = store.info;
	if (!info) return null;

	const gameIds = Object.keys(instancesStore.processes);
	const games = gameIds
		.map((id) => instancesStore.instances.find((i) => i.id === id))
		.filter((i): i is Instance => Boolean(i));
	const playing = gameIds.length > 0 || store.coreGame;
	const tasks = activeTasks(downloads.items);
	const waitingTasks = phase === "ready" && !playing && tasks.length > 0;
	const notes = installing ? null : localizedNotes(info.notes, i18next.language);
	const size = info.size ?? store.progress?.total ?? null;

	const title =
		phase === "installing"
			? t("update.titleInstalling")
			: phase === "ready"
				? t("update.titleReady")
				: t("update.title");
	const meta = installing
		? t("update.restartHint")
		: size !== null && phase !== "ready"
			? t("update.metaSize", {
					version: info.version,
					current: info.currentVersion,
					size: formatBytes(size),
				})
			: t("update.meta", { version: info.version, current: info.currentVersion });

	let blockKey = "none";
	let block: ReactNode = null;
	if (installing) {
		blockKey = "installing";
		block = (
			<div
				role="progressbar"
				aria-label={t("update.titleInstalling")}
				className="relative h-2 overflow-clip rounded-full bg-border"
			>
				<div className="progress-indeterminate absolute inset-y-0 left-0 w-[30%] rounded-full bg-primary" />
			</div>
		);
	} else if (phase === "error" && store.error) {
		blockKey = "error";
		block = <ErrorBlock error={store.error} />;
	} else {
		const parts: ReactNode[] = [];
		if (phase === "downloading") {
			parts.push(<DownloadProgress key="progress" progress={store.progress} />);
		}
		if (playing) {
			parts.push(
				<GameRow
					key="game"
					games={games}
					count={gameIds.length}
					onStop={gameIds.length > 0 ? () => setConfirm("stop") : null}
				/>,
			);
		} else if (waitingTasks) {
			parts.push(
				<p key="tasks" className="flex items-center gap-2 text-[13px]/[18px]">
					<LoaderCircle className="shrink-0 animate-spin text-primary-text" />
					<span className="min-w-0">
						{t("update.waitTasks", {
							count: tasks.length,
							names: listText(tasks.map((task) => taskName(task, instancesStore.instances))),
						})}
					</span>
				</p>,
			);
		}
		if (parts.length) {
			blockKey = [phase === "downloading" && "progress", playing && "game", waitingTasks && "tasks"]
				.filter(Boolean)
				.join("-");
			block = <div className="flex flex-col gap-4">{parts}</div>;
		}
	}

	let mainLabel = t("update.action");
	let mainIcon = <Download />;
	let mainBusy = false;
	if (phase === "error") {
		mainLabel = t("update.retry");
		mainIcon = <RotateCw />;
	} else if (phase === "downloading") {
		mainLabel = t("update.downloadingAction");
		mainIcon = <LoaderCircle className="animate-spin" />;
		mainBusy = true;
	} else if (phase === "ready") {
		mainLabel = playing ? t("update.downloadedWaitGame") : t("update.downloaded");
		mainIcon = <Check />;
		mainBusy = true;
	}

	const skip = canSkip(store);
	const hint = phase === "found" ? t("update.mandatoryHint") : skip ? t("update.skipHint") : null;
	const fade = { duration: reduceMotion ? 0 : 0.16, ease: easeOut };

	return (
		<div className="min-h-0 flex-1 [scrollbar-gutter:stable] overflow-y-auto">
			<div
				ref={rootRef}
				tabIndex={-1}
				className={cn(
					"mx-auto w-[560px] max-w-full px-8 pb-10 outline-none",
					installing ? "pt-[180px]" : "pt-12",
				)}
			>
				<header className="flex items-center gap-5">
					<AppIcon size={64} />
					<div className="min-w-0">
						<h1 className="text-[28px]/[34px] font-semibold tracking-[-0.025em]">{title}</h1>
						<p className="mt-1 text-[13px]/[18px] text-muted-foreground">{meta}</p>
					</div>
				</header>

				{notes && (
					<section aria-labelledby="update-notes" className="mt-8">
						<h2
							id="update-notes"
							className="mb-1.5 text-[13px]/[18px] font-medium text-muted-foreground"
						>
							{t("update.notes")}
						</h2>
						<div className="max-h-60 overflow-y-auto border-y border-border pt-1 pb-3 select-text [@media(max-height:639px)]:max-h-40">
							<MarkdownView source={notes} className="md-compact" />
						</div>
					</section>
				)}

				<div aria-live="polite" className={cn(block !== null && "mt-6")}>
					<AnimatePresence mode="wait" initial={false}>
						{block !== null && (
							<motion.div
								key={blockKey}
								initial={{ opacity: 0 }}
								animate={{ opacity: 1 }}
								exit={{ opacity: 0 }}
								transition={fade}
							>
								{block}
							</motion.div>
						)}
					</AnimatePresence>
				</div>

				{!installing && (
					<>
						<div className="mt-6 flex flex-wrap gap-2">
							<Button
								ref={mainRef}
								size="xl"
								aria-disabled={mainBusy || undefined}
								className="aria-disabled:opacity-60"
								onClick={() => {
									if (!mainBusy) void startUpdate();
								}}
							>
								{mainIcon}
								{mainLabel}
							</Button>
							{waitingTasks && (
								<Button
									variant="ghost"
									size="lg"
									className="self-center"
									onClick={() => setConfirm("tasks")}
								>
									{t("update.cancelTasks")}
								</Button>
							)}
							{skip && (
								<Button
									variant="ghost"
									size="lg"
									className="self-center"
									onClick={skipUpdate}
								>
									{t("update.skip")}
								</Button>
							)}
						</div>
						{hint && <p className="mt-3 text-[13px]/[18px] text-muted-foreground">{hint}</p>}
					</>
				)}
			</div>

			<ConfirmDialogs
				kind={confirm}
				gameCount={gameIds.length}
				onClose={() => setConfirm(null)}
				returnFocus={mainRef}
			/>
		</div>
	);
}

function DownloadProgress({ progress }: { progress: UpdateProgress | null }) {
	const { t } = useTranslation();
	const done = progress?.downloaded ?? 0;
	const total = progress?.total ?? null;
	const percent = total ? Math.min(100, Math.floor((done / total) * 100)) : null;
	const speed = progress?.speedBps ?? null;

	let detail: string;
	if (progress?.reconnect) {
		detail = t("update.reconnecting", progress.reconnect);
	} else if (total) {
		detail =
			speed !== null
				? t("update.bytes", {
						done: formatBytes(done),
						total: formatBytes(total),
						speed: t("update.speed", { value: formatBytes(speed) }),
					})
				: t("update.waitingServer", { done: formatBytes(done), total: formatBytes(total) });
	} else {
		detail =
			speed !== null
				? t("update.speed", { value: formatBytes(speed) })
				: t("update.waitingServerShort");
	}

	return (
		<div>
			<div className="flex items-baseline justify-between gap-4" aria-hidden>
				<span className="text-sm font-semibold tabular-nums">
					{percent !== null
						? t("update.progress", { percent })
						: t("update.progressUnknown", { done: formatBytes(done) })}
				</span>
				{progress?.etaSecs != null && !progress.reconnect && (
					<span className="text-[13px]/[18px] text-muted-foreground tabular-nums">
						{t("update.eta", { time: etaText(progress.etaSecs) })}
					</span>
				)}
			</div>
			<div
				role="progressbar"
				aria-label={t("update.progressLabel")}
				aria-valuemin={0}
				aria-valuemax={100}
				aria-valuenow={percent === null ? undefined : Math.floor(percent / 10) * 10}
				className="relative mt-2 h-2 overflow-clip rounded-full bg-border"
			>
				{percent === null ? (
					<div className="progress-indeterminate absolute inset-y-0 left-0 w-[30%] rounded-full bg-primary" />
				) : (
					<div
						className="absolute inset-0 origin-left bg-primary transition-transform duration-(--duration-fast) ease-out"
						style={{ transform: `scaleX(${percent / 100})` }}
					/>
				)}
			</div>
			<p className="mt-2 text-[13px]/[18px] text-muted-foreground tabular-nums" aria-hidden>
				{detail}
			</p>
			{progress?.slow && !progress.reconnect && (
				<p className="mt-3 flex items-start gap-2 text-[13px]/[18px] text-warning">
					<Info className="mt-px shrink-0" />
					<span className="min-w-0">{t("update.slow")}</span>
				</p>
			)}
		</div>
	);
}

function GameRow({
	games,
	count,
	onStop,
}: {
	games: Instance[];
	count: number;
	/** `null` — игру видит только ядро, остановить её отсюда нельзя. */
	onStop: (() => void) | null;
}) {
	const { t } = useTranslation();
	const first = games[0];
	const name = first?.name ?? t("downloads.instanceFallback");
	const heading =
		count === 0
			? t("update.gameUnknown")
			: count > 1
				? t("update.gameMore", { name, count: count - 1 })
				: t("update.game", { name });
	return (
		<div className="flex items-center gap-3 border-y border-border py-3">
			{first && <InstanceThumb scene={first.cover} size={40} />}
			<div className="min-w-0 flex-1">
				<p className="truncate text-sm font-semibold">{heading}</p>
				<p className="text-[13px]/[18px] text-muted-foreground">{t("update.gameText")}</p>
			</div>
			{onStop && (
				<Button variant="outline" size="sm" className="shrink-0" onClick={onStop}>
					<Square />
					{count > 1 ? t("update.stopGames") : t("update.stopGame")}
				</Button>
			)}
		</div>
	);
}

function ErrorBlock({ error }: { error: AppError }) {
	const { t } = useTranslation();
	const stalled = error.code === "update_stalled";
	const network = stalled || error.code === "update_unavailable" || error.code === "network";
	const corrupt = error.code === "update_bad_signature";
	const disk = error.code === "not_enough_space";
	const heading = network
		? t("update.error.network")
		: corrupt
			? t("update.error.corrupt")
			: disk
				? t("update.error.disk")
				: t("update.error.install");
	const text = stalled
		? t("update.error.stalledText")
		: network
			? t("update.error.networkText")
			: corrupt
				? t("update.error.corruptText")
				: disk
					? t("update.error.diskText")
					: t("update.error.installText");
	const details = error.details ?? (network || corrupt || disk ? null : error.message);

	return (
		<div className="flex gap-3">
			<span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-destructive/10 text-destructive">
				{network ? <WifiOff className="size-5" /> : <OctagonX className="size-5" />}
			</span>
			<div className="min-w-0 flex-1">
				<p className="text-sm font-semibold">{heading}</p>
				<p className="mt-0.5 text-[13px]/[18px] text-muted-foreground">{text}</p>
				{corrupt && (
					<Button
						variant="link"
						className="mt-1 h-auto px-0 text-[13px]"
						onClick={() => void external.open(RELEASES_URL).catch(() => {})}
					>
						{t("update.fromGithub")}
					</Button>
				)}
				{details && <ErrorDetails details={details} className="mt-1" />}
			</div>
		</div>
	);
}

function ConfirmDialogs({
	kind,
	gameCount,
	onClose,
	returnFocus,
}: {
	kind: "stop" | "tasks" | null;
	gameCount: number;
	onClose: () => void;
	returnFocus: RefObject<HTMLButtonElement | null>;
}) {
	const { t } = useTranslation();
	const [busy, setBusy] = useState(false);
	const stop = kind === "stop";

	const confirm = async () => {
		setBusy(true);
		if (stop) await stopGames();
		else await cancelTasksAndUpdate();
		setBusy(false);
		onClose();
	};

	return (
		<AlertDialog open={kind !== null} onOpenChange={(open) => !open && !busy && onClose()}>
			<AlertDialogContent
				className="max-w-[448px] gap-6 sm:max-w-[448px] data-[size=default]:sm:max-w-[448px]"
				onEscapeKeyDown={(e) => busy && e.preventDefault()}
				onCloseAutoFocus={(event) => {
					const target = returnFocus.current;
					if (!target?.isConnected) return;
					event.preventDefault();
					target.focus();
				}}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle>
						{stop ? t("instance.stopDialog.title") : t("update.cancelTasksTitle")}
					</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{stop ? t("instance.stopDialog.description") : t("update.cancelTasksText")}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel
						autoFocus
						aria-disabled={busy || undefined}
						onClick={(event) => busy && event.preventDefault()}
					>
						{stop ? t("common.cancel") : t("update.wait")}
					</AlertDialogCancel>
					<Button
						variant="destructive"
						aria-disabled={busy || undefined}
						onClick={() => !busy && void confirm()}
					>
						{busy && <LoaderCircle className="animate-spin" />}
						{stop
							? gameCount > 1
								? t("update.stopGames")
								: t("instance.stopDialog.confirm")
							: t("update.cancelTasksConfirm")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
