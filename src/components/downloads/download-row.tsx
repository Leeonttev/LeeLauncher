import {
	Boxes,
	CircleCheck,
	Coffee,
	LoaderCircle,
	OctagonX,
	RotateCcw,
	TriangleAlert,
	X,
} from "lucide-react";
import { useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { InstanceThumb } from "@/components/instances/instance-cover";
import { linkFocusRing, stretchedLink } from "@/components/instances/instance-link";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { Download, Instance } from "@/lib/api";
import { coverOf } from "@/lib/covers";
import { cancelDownload, dismissDownload, retryDownload, setDownloadsOpen } from "@/lib/downloads";
import { errorText, toAppError } from "@/lib/errors";
import { formatAgo, formatBytes, formatDuration } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { openInstancePage } from "@/lib/instance-page";
import { isSourceEnabled } from "@/lib/catalog";
import { LOADER_NAMES, javaMajorFor } from "@/lib/loaders";
import { openManualDialog } from "@/lib/manual";
import { openFailedFiles } from "@/lib/modpacks";
import { cn } from "@/lib/utils";

export type DownloadRowKind = "active" | "queued" | "done";

type DownloadRowProps = {
	download: Download;
	kind: DownloadRowKind;
	instance: Instance | null;
	now: number;
};

export function DownloadRow({ download, kind, instance, now }: DownloadRowProps) {
	const { t } = useTranslation();
	const content = download.kind === "content" ? download.content : undefined;
	const title = content
		? contentTitle(download)
		: instance
			? instance.name
			: download.kind === "java"
				? t("downloads.java", { major: download.javaMajor ?? "" })
				: (download.title ?? t("downloads.instanceFallback"));
	const target =
		content && instance ? (
			<p className="truncate text-[13px]/[18px] text-muted-foreground">
				{t("downloads.contentTo", { name: instance.name })}
			</p>
		) : null;
	const failed = download.state === "failed";
	const partial = !failed && kind === "done" ? (download.warnings?.failed.length ?? 0) : 0;
	const manual =
		!failed && kind === "done"
			? (download.warnings?.manual ?? []).filter((file) => isSourceEnabled(file.source)).length
			: 0;
	const [retrying, setRetrying] = useState(false);

	const retry = () => {
		if (retrying) return;
		setRetrying(true);
		void retryDownload(download.id).finally(() => setRetrying(false));
	};

	const rowAction = () => {
		if (kind === "done") void dismissDownload(download.id);
		else void cancelDownload(download.id);
	};
	const actionLabel =
		kind === "active"
			? t("downloads.cancel")
			: kind === "queued"
				? t("downloads.unqueue")
				: t("downloads.dismiss");

	const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
		if (event.key !== "Delete") return;
		event.preventDefault();
		rowAction();
	};

	const heading = instance ? (
		<button
			type="button"
			data-link
			aria-keyshortcuts="Delete"
			className={cn("block max-w-full truncate text-sm/5 font-semibold", stretchedLink)}
			onClick={() => {
				setDownloadsOpen(false);
				openInstancePage(
					instance.id,
					download.kind === "content" || download.kind === "modpack" ? "content" : undefined,
				);
			}}
			onKeyDown={onKeyDown}
		>
			{title}
		</button>
	) : (
		<p className="truncate text-sm/5 font-semibold">{title}</p>
	);

	const close = (
		<RowIconButton
			label={actionLabel}
			onClick={rowAction}
			className={
				kind === "done" && !failed && !partial && !manual
					? "opacity-0 group-focus-within/row:opacity-100 group-hover/row:opacity-100"
					: undefined
			}
		/>
	);

	return (
		<div
			className={cn(
				"group/row relative mx-1.5 flex gap-3 rounded-md border border-transparent px-2.5",
				kind === "active" ? "pt-2.5 pb-3" : "items-center py-2",
				instance && "hover:bg-accent",
				instance && linkFocusRing,
			)}
		>
			<RowIcon download={download} instance={instance} />
			{kind === "active" ? (
				<div className="min-w-0 flex-1">
					<div className="flex items-start gap-2">
						<div className="min-w-0 flex-1">
							{heading}
							{target}
						</div>
						<span className="-my-1.5">{close}</span>
					</div>
					<ActiveDetails download={download} instance={instance} title={title} />
				</div>
			) : (
				<>
					<div className="min-w-0 flex-1">
						{heading}
						{target}
						{kind === "queued" ? (
							<p className="text-[13px]/[18px] text-muted-foreground">
								{t("downloads.queued")}
							</p>
						) : failed ? (
							<p className="mt-0.5 flex items-start gap-1.5 text-[13px]/[18px] text-destructive">
								<OctagonX className="mt-px shrink-0" />
								<span className="line-clamp-2">
									{errorText(download.error ?? toAppError(null))}
								</span>
							</p>
						) : partial || manual ? (
							<>
								{partial > 0 && (
									<p className="mt-0.5 flex items-start gap-1.5 text-[13px]/[18px] text-warning">
										<TriangleAlert className="mt-px shrink-0" />
										<span className="line-clamp-2">
											{t("modpack.result.partial", { count: partial })}
										</span>
									</p>
								)}
								{manual > 0 && (
									<p className="mt-0.5 flex items-start gap-1.5 text-[13px]/[18px] text-warning">
										<TriangleAlert className="mt-px shrink-0" />
										<span className="line-clamp-2">
											{download.kind === "modpack"
												? t("modpack.result.manual", { count: manual })
												: t("downloads.manual", { count: manual })}
										</span>
									</p>
								)}
							</>
						) : (
							<p className="mt-0.5 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
								<CircleCheck className="shrink-0 text-primary-text" />
								<span className="truncate">
									{t(
										download.kind === "java"
											? "downloads.javaInstalled"
											: download.kind === "content"
												? "downloads.contentInstalled"
												: download.kind === "modpack" && download.mode === "update"
													? "modpack.result.updated"
													: "downloads.installed",
										{
											when: formatAgo(download.finishedAt ?? now, now),
										},
									)}
								</span>
							</p>
						)}
					</div>
					{manual > 0 && download.instanceId && (
						<Button
							variant="outline"
							size="sm"
							className="relative z-10"
							onClick={() => {
								const id = download.instanceId;
								if (!id) return;
								// Сначала закрыть панель, иначе поповер и диалог спорят за фокус.
								setDownloadsOpen(false);
								requestAnimationFrame(() => void openManualDialog(id, undefined, null));
							}}
						>
							{t("downloads.manualAction")}
						</Button>
					)}
					{partial > 0 && (
						<Button
							variant="outline"
							size="sm"
							className="relative z-10"
							onClick={() => {
								// Сначала закрыть панель, иначе поповер и диалог спорят за фокус.
								setDownloadsOpen(false);
								requestAnimationFrame(() => openFailedFiles(download.id));
							}}
						>
							{t("common.details")}
						</Button>
					)}
					{failed && (
						<Button
							variant="outline"
							size="sm"
							className="relative z-10 aria-disabled:opacity-50"
							aria-disabled={retrying || undefined}
							onClick={retry}
						>
							{retrying ? <LoaderCircle className="animate-spin" /> : <RotateCcw />}
							{t("downloads.retry")}
						</Button>
					)}
					{close}
				</>
			)}
		</div>
	);
}

function RowIcon({ download, instance }: { download: Download; instance: Instance | null }) {
	if (download.kind === "content") {
		return <ProjectIcon url={download.content?.iconUrl ?? null} type="mod" size={40} />;
	}
	if (download.kind === "modpack") {
		return <ProjectIcon url={download.iconUrl ?? null} type="modpack" size={40} />;
	}
	if (instance) {
		return <InstanceThumb scene={coverOf(instance)} size={40} />;
	}
	return (
		<span
			aria-hidden
			className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary-soft text-primary-text"
		>
			{download.kind === "java" ? <Coffee /> : <Boxes />}
		</span>
	);
}

function contentTitle(download: Download): string {
	const content = download.content;
	const title = content?.projectTitle ?? i18n.t("downloads.contentUpdate");
	return content?.extraCount ? i18n.t("downloads.content", { title, count: content.extraCount }) : title;
}

function RowIconButton({
	label,
	onClick,
	className,
}: {
	label: string;
	onClick: () => void;
	className?: string;
}) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant="ghost"
					size="icon-sm"
					aria-label={label}
					className={cn("relative z-10 text-muted-foreground hover:text-foreground", className)}
					onClick={onClick}
				>
					<X />
				</Button>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}

function stageLabel(download: Download, instance: Instance | null): string {
	if (download.kind === "content") {
		const stage = i18n.t("downloads.stage.downloading");
		return download.filesTotal
			? i18n.t("downloads.files", { stage, done: download.filesDone ?? 0, total: download.filesTotal })
			: stage;
	}
	if (download.kind === "java") return i18n.t("downloads.stage.downloading");
	switch (download.stage) {
		case "java": {
			const major = instance
				? (instance.installed?.javaMajor ?? javaMajorFor(instance.gameVersion))
				: null;
			return major
				? i18n.t("install.stage.java", { version: major })
				: i18n.t("install.stage.javaGeneric");
		}
		case "processors": {
			const stage = i18n
				.t("install.stage.loader", { loader: instance ? LOADER_NAMES[instance.loader.kind] : "" })
				.trim();
			return download.stepsTotal
				? i18n.t("downloads.step", {
						stage,
						done: download.stepsDone ?? 0,
						total: download.stepsTotal,
					})
				: stage;
		}
		case "modpack_files":
			return download.filesTotal
				? i18n.t("modpack.stage.filesProgress", {
						done: download.filesDone ?? 0,
						total: download.filesTotal,
					})
				: i18n.t("modpack.stage.files");
		case "copying": {
			const stage = i18n.t("install.stage.copying");
			return download.filesTotal
				? i18n.t("downloads.files", {
						stage,
						done: download.filesDone ?? 0,
						total: download.filesTotal,
					})
				: stage;
		}
		case undefined:
			return i18n.t("install.stage.metadata");
		default:
			return i18n.t(`install.stage.${download.stage}`);
	}
}

function bytesLine(download: Download): string | null {
	const downloading =
		download.kind === "java" ||
		download.kind === "content" ||
		download.stage === "java" ||
		download.stage === "downloading" ||
		download.stage === "modpack_files";
	if (!downloading) return null;
	if (download.bytesDone <= 0 && download.bytesTotal === null) return null;
	const parts = [
		download.bytesTotal !== null
			? i18n.t("downloads.bytes", {
					done: formatBytes(download.bytesDone),
					total: formatBytes(download.bytesTotal),
				})
			: formatBytes(download.bytesDone),
	];
	if (download.speed !== null && download.speed > 0)
		parts.push(i18n.t("downloads.speed", { value: formatBytes(download.speed) }));
	if (download.etaSecs !== null)
		parts.push(
			i18n.t("downloads.eta", {
				// Вверх до минуты: «ещё ≈ 3 мин» не должно обещать меньше, чем осталось.
				time:
					download.etaSecs < 60
						? i18n.t("downloads.etaSoon")
						: formatDuration(Math.ceil(download.etaSecs / 60) * 60),
			}),
		);
	return parts.join(" · ");
}

function ActiveDetails({
	download,
	instance,
	title,
}: {
	download: Download;
	instance: Instance | null;
	title: string;
}) {
	const stage = stageLabel(download, instance);
	const percent = download.percent === null ? null : Math.round(download.percent);
	const bytes = bytesLine(download);
	return (
		<>
			<div className="mt-0.5 flex items-center gap-3 text-[13px]/[18px]">
				<span className="min-w-0 flex-1 truncate text-muted-foreground">{stage}</span>
				{percent !== null && (
					<span className="shrink-0 font-medium text-foreground tabular-nums">{percent}%</span>
				)}
			</div>
			<div
				role="progressbar"
				aria-label={title}
				aria-valuemin={0}
				aria-valuemax={100}
				aria-valuenow={percent ?? undefined}
				aria-valuetext={percent === null ? stage : `${stage} · ${percent}%`}
				className="relative mt-2 h-1 overflow-clip bg-foreground/12"
			>
				{percent === null ? (
					<div className="progress-indeterminate absolute inset-y-0 left-0 w-[30%] bg-primary" />
				) : (
					<div
						className="absolute inset-0 origin-left bg-primary transition-transform duration-200 ease-linear"
						style={{ transform: `scaleX(${Math.min(1, Math.max(0, percent / 100))})` }}
					/>
				)}
			</div>
			{bytes && (
				<p className="mt-1.5 truncate text-[13px]/[18px] text-muted-foreground tabular-nums">
					{bytes}
				</p>
			)}
		</>
	);
}
