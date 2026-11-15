import {
	ArrowRight,
	Check,
	ChevronRight,
	ExternalLink,
	LoaderCircle,
	type LucideIcon,
	Minus,
	OctagonX,
	Plus,
	RefreshCw,
	RotateCcw,
	TriangleAlert,
} from "lucide-react";
import { useEffect, useId, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AutoHeight } from "@/components/auto-height";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { ManualWarning } from "@/components/modpacks/install-modpack-dialog";
import { ModpackVersionPicker } from "@/components/modpacks/modpack-version-picker";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
	catalogApi,
	modpackApi,
	type ContentVersion,
	type Instance,
	type ModpackLink,
	type ModpackPreview,
	type ModpackUpdatePreview,
} from "@/lib/api";
import { openProject } from "@/lib/catalog";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { statusOf, useInstances } from "@/lib/instances";
import {
	backupPreference,
	cancelInspect,
	closeModpackDialog,
	defaultVersion,
	inspectRequestId,
	previewTarget,
	restoreModpackFocus,
	runModpackUpdate,
	setBackupPreference,
	useModpackDialogs,
	type UpdateSource,
} from "@/lib/modpacks";
import { cn } from "@/lib/utils";

const CONFIRM_ID = "update-modpack-confirm";

type Loaded<T> = { status: "loading" } | { status: "ready"; value: T } | { status: "error"; error: AppError };

export function UpdateModpackDialog() {
	const dialog = useModpackDialogs().update;
	const store = useInstances();
	const instance = store.instances.find((i) => i.id === dialog.instanceId) ?? null;
	const link = instance?.modpack ?? null;
	const [busy, setBusy] = useState(false);
	return (
		<Dialog
			open={dialog.open && Boolean(instance && link)}
			onOpenChange={(next) => {
				if (!next && !busy) closeModpackDialog("update");
			}}
		>
			<DialogContent
				className="w-[448px] max-w-[calc(100vw-2rem)] gap-0 p-0 sm:max-w-[448px]"
				onCloseAutoFocus={(e) => restoreModpackFocus("update", e)}
				onOpenAutoFocus={(e) => {
					e.preventDefault();
					document.getElementById(CONFIRM_ID)?.focus();
				}}
			>
				<div className="max-h-[calc(100vh-2rem-8px)] overflow-y-auto">
					{instance && link && dialog.source && (
						<UpdateForm
							key={dialog.session}
							instance={instance}
							link={link}
							source={dialog.source}
							onBusyChange={setBusy}
						/>
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
}

function UpdateForm({
	instance,
	link,
	source,
	onBusyChange,
}: {
	instance: Instance;
	link: ModpackLink;
	source: UpdateSource;
	onBusyChange: (busy: boolean) => void;
}) {
	const { t } = useTranslation();
	const store = useInstances();
	const ids = useId();
	const projectId = link.source !== "file" ? link.projectId : null;
	const catalogSource = link.source !== "file" ? link.source : undefined;
	const pick = source.kind === "version" && source.pick;

	const [versions, setVersions] = useState<{ key: number; value: Loaded<ContentVersion[]> } | null>(null);
	const [versionsAttempt, setVersionsAttempt] = useState(0);
	const [picked, setPicked] = useState<string | null>(null);
	const [attempt, setAttempt] = useState(0);
	const [inspected, setInspected] = useState<{ key: string; value: Loaded<ModpackPreview> } | null>(null);
	const [diff, setDiff] = useState<{ key: string; value: Loaded<ModpackUpdatePreview> } | null>(null);
	const [backup, setBackup] = useState(backupPreference);
	const [busy, setBusy] = useState(false);
	const [submitError, setSubmitError] = useState<AppError | null>(null);

	useEffect(() => {
		if (!pick || !projectId) return;
		let alive = true;
		const key = versionsAttempt;
		catalogApi
			.versions({ projectId, source: catalogSource })
			.then((value) => alive && setVersions({ key, value: { status: "ready", value } }))
			.catch(
				(err: unknown) =>
					alive && setVersions({ key, value: { status: "error", error: toAppError(err) } }),
			);
		return () => {
			alive = false;
		};
	}, [pick, projectId, catalogSource, versionsAttempt]);

	const versionsState: Loaded<ContentVersion[]> | null = !pick
		? null
		: versions && versions.key === versionsAttempt
			? versions.value
			: { status: "loading" };
	const versionList = versionsState?.status === "ready" ? versionsState.value : [];
	const versionId =
		source.kind === "file"
			? null
			: pick
				? (picked ??
					(versionsState?.status === "ready" ? (defaultVersion(versionList)?.id ?? null) : null))
				: source.versionId;

	const inspectKey = `${versionId ?? ""}:${attempt}`;
	useEffect(() => {
		if (source.kind !== "version" || !projectId || !versionId) return;
		let alive = true;
		const key = inspectKey;
		const requestId = inspectRequestId();
		let done = false;
		modpackApi
			.inspect({ requestId, source: catalogSource, projectId, versionId })
			.then((value) => alive && setInspected({ key, value: { status: "ready", value } }))
			.catch(
				(err: unknown) =>
					alive && setInspected({ key, value: { status: "error", error: toAppError(err) } }),
			)
			.finally(() => {
				done = true;
			});
		// Закрыли диалог или сменили версию — прервать скачивание и чтение.
		return () => {
			alive = false;
			if (!done) cancelInspect(requestId);
		};
	}, [source.kind, projectId, catalogSource, versionId, inspectKey]);

	const preview: Loaded<ModpackPreview> =
		source.kind === "file"
			? { status: "ready", value: source.preview }
			: inspected && inspected.key === inspectKey
				? inspected.value
				: { status: "loading" };
	const token = preview.status === "ready" ? preview.value.token : null;
	const same =
		source.kind === "file"
			? preview.status === "ready" &&
				preview.value.project?.versionId === link.versionId &&
				link.versionId !== null
			: versionId !== null && versionId === link.versionId;

	const diffKey = `${token ?? ""}:${attempt}`;
	useEffect(() => {
		if (!token || same) return;
		let alive = true;
		const key = diffKey;
		modpackApi
			.updatePreview(instance.id, token)
			.then((value) => alive && setDiff({ key, value: { status: "ready", value } }))
			.catch(
				(err: unknown) =>
					alive && setDiff({ key, value: { status: "error", error: toAppError(err) } }),
			);
		return () => {
			alive = false;
		};
	}, [instance.id, token, same, diffKey]);

	const diffState: Loaded<ModpackUpdatePreview> =
		preview.status === "error"
			? preview
			: diff && diff.key === diffKey
				? diff.value
				: { status: "loading" };
	const ready = diffState.status === "ready" ? diffState.value : null;
	// Пока считается разница для другой версии, прошлая остаётся неактивной: высота окна меняется один раз.
	const [held, setHeld] = useState<{ diff: ModpackUpdatePreview; manual: number } | null>(null);
	if (ready && preview.status === "ready" && ready !== held?.diff)
		setHeld({ diff: ready, manual: preview.value.manualCount });
	const pending = diffState.status === "loading" && !same;
	const shownDiff = ready ?? (pending ? (held?.diff ?? null) : null);
	const shownManual =
		preview.status === "ready" ? preview.value.manualCount : pending ? (held?.manual ?? 0) : 0;
	const dim = pending && "opacity-50";

	const selected = versionList.find((v) => v.id === versionId) ?? null;
	const current = versionList.find((v) => v.id === link.versionId) ?? null;
	const downgrade = Boolean(selected && current && selected.publishedAt < current.publishedAt);
	const newNumber = preview.status === "ready" ? preview.value.versionNumber : (selected?.number ?? null);
	const otherPack =
		source.kind === "file" && preview.status === "ready"
			? preview.value.project
				? link.projectId !== preview.value.project.projectId
					? (preview.value.project.title ?? preview.value.name)
					: null
				: link.source !== "file" || preview.value.name !== link.title
					? preview.value.name
					: null
			: null;

	const status = statusOf(instance, store);
	const running = status === "running" || status === "launching";
	const canSubmit = Boolean(ready && token) && !same && !busy && !running;

	const submit = async () => {
		if (!canSubmit || !token) return;
		setBusy(true);
		onBusyChange(true);
		setSubmitError(null);
		try {
			await runModpackUpdate({ id: instance.id, token, backup, version: newNumber ?? "" });
		} catch (err) {
			setSubmitError(toAppError(err));
		} finally {
			setBusy(false);
			onBusyChange(false);
		}
	};

	const retry = () => {
		if (versionsState?.status === "error") setVersionsAttempt((n) => n + 1);
		else setAttempt((n) => n + 1);
	};

	const versionLabelId = `${ids}-version`;
	const iconUrl =
		source.kind === "file" && otherPack ? (source.preview.project?.iconUrl ?? null) : link.iconUrl;
	const title = otherPack ?? link.title;

	const confirm = (
		<Button
			id={CONFIRM_ID}
			aria-disabled={!canSubmit || undefined}
			className="aria-disabled:opacity-50"
			onClick={() => void submit()}
		>
			{busy ? <LoaderCircle className="animate-spin" /> : <RefreshCw />}
			{downgrade ? t("modpack.update.confirmDowngrade") : t("modpack.update.confirm")}
		</Button>
	);

	return (
		<AutoHeight className="p-6">
			<DialogTitle className="pr-10">
				{downgrade ? t("modpack.update.titleDowngrade") : t("modpack.update.title")}
			</DialogTitle>

			{pick && (
				<div className="mt-5">
					<Label id={versionLabelId} className="text-[13px]/[18px] font-medium">
						{t("modpack.dialog.version")}
					</Label>
					{versionsState?.status === "error" ? (
						<ErrorLine className="mt-2" error={versionsState.error} onRetry={retry} />
					) : (
						<ModpackVersionPicker
							className="mt-2"
							versions={versionList}
							value={versionId}
							labelledBy={versionLabelId}
							loading={versionsState?.status === "loading"}
							onChange={setPicked}
						/>
					)}
				</div>
			)}

			<div className="mt-5 flex items-center gap-3">
				<ProjectIcon url={iconUrl} type="modpack" size={40} />
				<div className="min-w-0 flex-1">
					<p className="truncate text-sm/5 font-semibold">{title}</p>
					<DialogDescription asChild>
						<p className="flex min-w-0 items-center gap-1 truncate text-[13px]/[18px] text-muted-foreground">
							<span className="shrink-0">{link.versionNumber}</span>
							<ArrowRight className="shrink-0" />
							{newNumber ? (
								<span className="shrink-0 font-semibold text-primary-text">{newNumber}</span>
							) : (
								<Skeleton className="h-3 w-12" />
							)}
							{preview.status === "ready" && (
								<span className="truncate">{` · ${previewTarget(preview.value)}`}</span>
							)}
						</p>
					</DialogDescription>
				</div>
			</div>

			{same ? (
				<p className="mt-5 text-[13px]/[18px] text-muted-foreground">{t("modpack.update.same")}</p>
			) : diffState.status === "error" ? (
				<ErrorLine className="mt-5" error={diffState.error} onRetry={retry} />
			) : (
				<div inert={pending || undefined} aria-busy={pending || undefined} className={cn(dim)}>
					<DiffSummary diff={shownDiff} />
				</div>
			)}

			{shownManual > 0 && !same && <ManualWarning className={cn("mt-4", dim)} count={shownManual} />}

			{otherPack && (
				<p className="mt-4 flex items-start gap-1.5 text-[13px]/[18px] text-warning">
					<TriangleAlert className="mt-px shrink-0" />
					{t("modpack.update.otherPack", { name: otherPack })}
				</p>
			)}

			{shownDiff && !same && (
				<label
					inert={pending || undefined}
					className={cn("mt-5 flex cursor-pointer items-center gap-2.5 text-sm", dim)}
				>
					<Checkbox
						checked={backup}
						onCheckedChange={(value) => {
							const next = value === true;
							setBackup(next);
							setBackupPreference(next);
						}}
					/>
					<span className="min-w-0 truncate">
						{t("modpack.update.backup")}
						<span className="text-muted-foreground">{` · ${formatBytes(shownDiff.backupBytes)}`}</span>
					</span>
				</label>
			)}

			{submitError && (
				<p className="mt-5 flex items-start gap-1.5 text-[13px]/[18px] text-destructive">
					<OctagonX className="mt-px shrink-0" />
					{errorText(submitError)}
				</p>
			)}

			<div className="mt-6 flex items-center gap-2">
				{projectId && (
					<Button
						variant="ghost"
						className="-ml-3"
						onClick={() => {
							closeModpackDialog("update");
							openProject({ source: catalogSource ?? "modrinth", id: projectId }, "modpack", {
								tab: "versions",
							});
						}}
					>
						<ExternalLink />
						{t("modpack.update.changelog")}
					</Button>
				)}
				<div className="ml-auto flex gap-2">
					<Button variant="ghost" onClick={() => closeModpackDialog("update")}>
						{t("common.cancel")}
					</Button>
					{running ? (
						<Tooltip>
							<TooltipTrigger asChild>{confirm}</TooltipTrigger>
							<TooltipContent>{t("modpack.strip.gameRunning")}</TooltipContent>
						</Tooltip>
					) : (
						confirm
					)}
				</div>
			</div>
		</AutoHeight>
	);
}

function ErrorLine({
	error,
	onRetry,
	className,
}: {
	error: AppError;
	onRetry: () => void;
	className?: string;
}) {
	const { t } = useTranslation();
	return (
		<div className={cn("flex items-center gap-2", className)}>
			<OctagonX className="shrink-0 text-destructive" />
			<span className="min-w-0 flex-1 text-[13px]/[18px] text-destructive">{errorText(error)}</span>
			<Button variant="outline" size="sm" onClick={onRetry}>
				<RotateCcw />
				{t("common.retry")}
			</Button>
		</div>
	);
}

function DiffSummary({ diff }: { diff: ModpackUpdatePreview | null }) {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	if (!diff) {
		return (
			<div aria-busy className="mt-5 flex gap-5">
				<Skeleton className="h-4 w-24" />
				<Skeleton className="h-4 w-24" />
				<Skeleton className="h-4 w-24" />
			</div>
		);
	}
	const total = diff.added.length + diff.updated.length + diff.removed.length;
	return (
		<>
			<div className="mt-5 flex flex-wrap gap-x-5 gap-y-1 text-sm tabular-nums">
				<Count icon={Plus} text={t("modpack.update.added", { count: diff.added.length })} />
				<Count icon={RefreshCw} text={t("modpack.update.updated", { count: diff.updated.length })} />
				<Count icon={Minus} text={t("modpack.update.removed", { count: diff.removed.length })} />
			</div>
			{total > 0 && (
				<Collapsible open={open} onOpenChange={setOpen} className="mt-3">
					<CollapsibleTrigger className="relative -ml-1 flex items-center gap-1 rounded-md border border-transparent px-1 text-[13px]/[18px] font-semibold text-primary-text">
						<ChevronRight
							className={cn(
								"transition-transform duration-(--duration-fast)",
								open && "rotate-90",
							)}
						/>
						{t("modpack.update.files")}
					</CollapsibleTrigger>
					<CollapsibleContent>
						<ul className="mt-2 max-h-48 overflow-y-auto rounded-lg bg-muted px-3 py-2 font-mono text-[13px]/6 select-text">
							<FileLines sign="+" paths={diff.added} />
							<FileLines sign="↻" paths={diff.updated} />
							<FileLines sign="−" paths={diff.removed} />
						</ul>
					</CollapsibleContent>
				</Collapsible>
			)}
			<p className="mt-4 flex items-start gap-1.5 text-[13px]/[18px] text-muted-foreground">
				<Check className="mt-px shrink-0" />
				{t("modpack.update.keep", { count: diff.userFiles })}
			</p>
			{diff.modifiedOverrides.length > 0 && (
				<p className="mt-2 flex items-start gap-1.5 text-[13px]/[18px] text-warning">
					<TriangleAlert className="mt-px shrink-0" />
					{t("modpack.update.overwrite", { count: diff.modifiedOverrides.length })}
				</p>
			)}
		</>
	);
}

function Count({ icon: Icon, text }: { icon: LucideIcon; text: string }) {
	return (
		<span className="flex items-center gap-1.5">
			<Icon className="text-muted-foreground" />
			{text}
		</span>
	);
}

function FileLines({ sign, paths }: { sign: ReactNode; paths: string[] }) {
	return (
		<>
			{paths.map((path) => (
				<li key={`${String(sign)}${path}`} className="flex min-w-0 gap-2">
					<span aria-hidden className="w-3 shrink-0 text-muted-foreground">
						{sign}
					</span>
					<span className="min-w-0 truncate">{path}</span>
				</li>
			))}
		</>
	);
}
