import {
	CircleCheck,
	ExternalLink,
	Folder,
	FolderOpen,
	Hourglass,
	OctagonX,
	RotateCcw,
	TriangleAlert,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { external, type ContentSource, type ManualFile } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import { DEFAULT_SOURCE, otherSource, SOURCE_NAMES } from "@/lib/catalog";
import { errorText, toAppError } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { useInstances } from "@/lib/instances";
import {
	closeManualDialog,
	isWaiting,
	pickManualFiles,
	restoreManualFocus,
	retryManualDialog,
	useManualDialog,
	type ManualDialog as ManualDialogState,
} from "@/lib/manual";
import { cn } from "@/lib/utils";

/** «Открыть все» открывает не больше стольких страниц сразу. */
const OPEN_ALL_LIMIT = 10;
const DONE_ID = "manual-dialog-done";

function openPage(url: string) {
	external.open(url).catch((err: unknown) => {
		console.error("open-url", err);
		toast.error(i18n.t("common.openLinkFailed"));
	});
}

function fileKey(file: ManualFile) {
	return `${file.source}:${file.projectId}:${file.fileId}`;
}

/** Пока диалог открыт, ядро само находит скачанное в «Загрузках». Один экземпляр на приложение. */
export function ManualDialog() {
	const { t } = useTranslation();
	const dialog = useManualDialog();
	const store = useInstances();
	const instance = store.instances.find((i) => i.id === dialog.instanceId) ?? null;
	const files = dialog.status === "ready" ? dialog.files : (dialog.extra ?? []);
	const source: ContentSource = files[0]?.source ?? otherSource(DEFAULT_SOURCE) ?? DEFAULT_SOURCE;

	return (
		<Dialog open={dialog.open} onOpenChange={(open) => !open && closeManualDialog()}>
			<DialogContent
				className="flex max-h-[min(640px,calc(100svh-96px))] w-[560px] max-w-[calc(100vw-2rem)] flex-col gap-0 p-6 sm:max-w-[560px]"
				onCloseAutoFocus={restoreManualFocus}
				onOpenAutoFocus={(event) => {
					if (dialog.status === "ready") event.preventDefault();
				}}
			>
				<DialogTitle className="pr-10">
					{t("manual.title", { count: Math.max(1, files.length) })}
				</DialogTitle>
				<DialogDescription className="mt-2">
					{t("manual.description", { source: SOURCE_NAMES[source], name: instance?.name ?? "" })}
				</DialogDescription>
				{dialog.status === "ready" ? (
					<ManualBody key={dialog.session} dialog={dialog} />
				) : dialog.status === "error" ? (
					<div className="mt-5 flex items-start gap-2 text-[13px]/[18px] text-destructive">
						<OctagonX className="mt-px shrink-0" />
						<span className="min-w-0 flex-1">
							{t("manual.loadFailed")} · {errorText(dialog.error ?? toAppError(null))}
						</span>
						<Button variant="outline" size="sm" onClick={retryManualDialog}>
							<RotateCcw />
							{t("common.retry")}
						</Button>
					</div>
				) : (
					<ManualSkeleton count={Math.max(1, Math.min(files.length, 4))} />
				)}
			</DialogContent>
		</Dialog>
	);
}

function ManualSkeleton({ count }: { count: number }) {
	return (
		<div aria-busy className="mt-3.5 border-t">
			{Array.from({ length: count }, (_, i) => (
				<div key={i} className="flex h-14 items-center gap-3 border-b">
					<Skeleton className="size-8 rounded-lg" />
					<div className="flex flex-1 flex-col gap-1.5">
						<Skeleton className="h-4 w-40" />
						<Skeleton className="h-3 w-56" />
					</div>
				</div>
			))}
		</div>
	);
}

function ManualBody({ dialog }: { dialog: ManualDialogState }) {
	const { t } = useTranslation();
	const files = dialog.files;
	const listRef = useRef<HTMLUListElement>(null);
	const footerRef = useRef<HTMLDivElement>(null);
	/** Кнопка «Открыть» с фокусом пропадёт, когда файл добавится. */
	const focusedRow = useRef<string | null>(null);
	const done = files.filter((file) => file.state === "added").length;
	const waiting = files.filter(isWaiting);
	const allDone = files.length > 0 && waiting.length === 0;
	const openable = waiting.slice(0, OPEN_ALL_LIMIT);

	const [previous, setPrevious] = useState(files);
	const [announce, setAnnounce] = useState("");
	if (previous !== files) {
		const found = files.find(
			(file) =>
				file.state === "added" &&
				previous.find((old) => fileKey(old) === fileKey(file))?.state !== "added",
		);
		if (found) setAnnounce(t("manual.addedAria", { name: found.title }));
		setPrevious(files);
	}

	useEffect(() => {
		listRef.current?.querySelector<HTMLElement>("[data-manual-open]")?.focus();
	}, []);
	useEffect(() => {
		if (allDone) document.getElementById(DONE_ID)?.focus();
	}, [allDone]);

	// «Открыть» пропала вместе с фокусом — переносим его на следующую или на первую кнопку низа.
	useLayoutEffect(() => {
		const key = focusedRow.current;
		const list = listRef.current;
		if (!key || !list) return;
		const active = document.activeElement;
		if (active && active !== document.body && active.isConnected) return;
		focusedRow.current = null;
		const rows = Array.from(list.querySelectorAll<HTMLElement>("[data-manual-row]"));
		const index = rows.findIndex((row) => row.dataset.manualRow === key);
		const next = rows
			.slice(index + 1)
			.map((row) => row.querySelector<HTMLElement>("[data-manual-open]"))
			.find(Boolean);
		(next ?? footerRef.current?.querySelector<HTMLElement>("button"))?.focus();
	}, [files]);

	const openAll = () => {
		for (const file of openable) openPage(file.pageUrl);
	};

	return (
		<>
			<p aria-live="polite" className="sr-only">
				{announce}
			</p>
			<ul
				ref={listRef}
				className="mt-3.5 min-h-0 flex-1 overflow-y-auto border-t"
				onFocus={(event) => {
					const row = (event.target as HTMLElement).closest<HTMLElement>("[data-manual-row]");
					focusedRow.current = row?.dataset.manualRow ?? null;
				}}
				onBlur={(event) => {
					if (event.relatedTarget) focusedRow.current = null;
				}}
			>
				{files.map((file) => (
					<ManualRow key={fileKey(file)} file={file} />
				))}
			</ul>
			<div className="mt-3 flex min-w-0 items-center gap-2 text-[13px]/[18px]">
				{dialog.watching ? (
					<p className="flex min-w-0 flex-1 items-center gap-2 text-muted-foreground">
						<Folder className="shrink-0" />
						<Tooltip>
							<TooltipTrigger asChild>
								<span className="min-w-0 truncate">
									{t("manual.watching", { path: dialog.path ?? "" })}
								</span>
							</TooltipTrigger>
							<TooltipContent className="max-w-100 break-all">{dialog.path}</TooltipContent>
						</Tooltip>
					</p>
				) : (
					<p className="flex min-w-0 flex-1 items-start gap-2 text-warning">
						<TriangleAlert className="mt-px shrink-0" />
						{t("manual.watchFailed")}
					</p>
				)}
				<Button
					variant="ghost"
					size="sm"
					className="shrink-0 text-primary-text"
					onClick={() => void pickManualFiles()}
				>
					<FolderOpen />
					{t("manual.pickFiles")}
				</Button>
			</div>
			<div ref={footerRef} className="mt-5 flex items-center gap-2">
				{allDone ? (
					<p className="flex min-w-0 flex-1 items-center gap-1.5 text-[13px]/[18px] text-primary-text">
						<CircleCheck className="shrink-0" />
						{t("manual.allDone")}
					</p>
				) : (
					<p className="min-w-0 flex-1 truncate text-[13px]/[18px] text-muted-foreground tabular-nums">
						{t("manual.progress", { done, total: files.length })}
					</p>
				)}
				{allDone ? (
					<Button id={DONE_ID} onClick={closeManualDialog}>
						{t("manual.done")}
					</Button>
				) : (
					<>
						{openable.length > 0 && (
							<Button variant="ghost" onClick={openAll}>
								<ExternalLink />
								{waiting.length > OPEN_ALL_LIMIT
									? t("manual.openSome", { count: OPEN_ALL_LIMIT, total: waiting.length })
									: t("manual.openAll", { count: waiting.length })}
							</Button>
						)}
						<Button variant="outline" onClick={closeManualDialog}>
							{t("manual.later")}
						</Button>
					</>
				)}
			</div>
		</>
	);
}

function ManualRow({ file }: { file: ManualFile }) {
	const { t } = useTranslation();
	const { reduceMotion } = useAppearance();
	const added = file.state === "added";
	const detail =
		file.state === "wrong" ? (
			<span className="block truncate text-[13px]/[18px] text-warning">
				{t("manual.wrongFileHint", { got: file.got ?? file.error?.details ?? "", file: file.file })}
			</span>
		) : file.state === "failed" ? (
			<span className="block truncate text-[13px]/[18px] text-destructive">
				{errorText(file.error ?? toAppError(null))}
			</span>
		) : (
			<span className="block truncate text-[13px]/[18px] text-muted-foreground">
				{`${file.file} · ${formatBytes(file.size)}`}
			</span>
		);

	return (
		<li data-manual-row={fileKey(file)} className="flex h-14 items-center gap-3 border-b">
			<ProjectIcon url={file.iconUrl} type={file.type} size={32} />
			<span className="min-w-0 flex-1">
				<span className="block truncate text-sm/5 font-semibold">{file.title}</span>
				{detail}
			</span>
			<span className="grid w-29 shrink-0">
				<AnimatePresence mode="popLayout" initial={false}>
					<motion.span
						key={file.state}
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: reduceMotion ? 0 : 0.12 }}
						className="col-start-1 row-start-1"
					>
						<RowState file={file} />
					</motion.span>
				</AnimatePresence>
			</span>
			{/* Кнопки нет, но место остаётся: колонки не прыгают. */}
			<span className="flex w-24 shrink-0 justify-end">
				{!added && (
					<Button
						variant="outline"
						size="sm"
						data-manual-open
						aria-label={t("manual.openAria", { name: file.title })}
						onClick={() => openPage(file.pageUrl)}
					>
						<ExternalLink />
						{t("manual.open")}
					</Button>
				)}
			</span>
		</li>
	);
}

function RowState({ file }: { file: ManualFile }) {
	const { t } = useTranslation();
	const base = "flex items-center gap-1.5 text-[13px]/[18px]";
	switch (file.state) {
		case "added":
			return (
				<span className={cn(base, "text-primary-text")}>
					<CircleCheck className="shrink-0" />
					{t("manual.added")}
				</span>
			);
		case "wrong":
			return (
				<span className={cn(base, "text-warning")}>
					<TriangleAlert className="shrink-0" />
					{t("manual.wrongFile")}
				</span>
			);
		case "failed":
			return (
				<span className={cn(base, "text-destructive")}>
					<OctagonX className="shrink-0" />
					{t("manual.failed")}
				</span>
			);
		default:
			return (
				<span className={cn(base, "text-muted-foreground")}>
					<Hourglass className="shrink-0" />
					{t("manual.waiting")}
				</span>
			);
	}
}
