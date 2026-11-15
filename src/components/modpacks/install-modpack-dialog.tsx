import {
	Check,
	ChevronRight,
	Download,
	LoaderCircle,
	OctagonX,
	RotateCcw,
	TriangleAlert,
} from "lucide-react";
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AutoHeight } from "@/components/auto-height";
import { ProjectIcon } from "@/components/catalog/project-icon";
import {
	CoverStudio,
	STUDIO_DIALOG_CLASS,
	STUDIO_GRID_CLASS,
	STUDIO_SCROLL_CLASS,
} from "@/components/instances/cover-studio";
import { GroupCombobox } from "@/components/instances/group-combobox";
import { ModpackVersionPicker } from "@/components/modpacks/modpack-version-picker";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
	catalogApi,
	modpackApi,
	type ContentVersion,
	type ModpackOptionalFile,
	type ModpackPreview,
} from "@/lib/api";
import { SOURCE_NAMES } from "@/lib/catalog";
import { leastUsedCover, type InstanceCover } from "@/lib/covers";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { groupErrorText } from "@/lib/groups";
import { getInstances, uniqueName, useInstances } from "@/lib/instances";
import {
	cancelInspect,
	closeModpackDialog,
	defaultVersion,
	fileSource,
	inspectRequestId,
	installModpack,
	previewTarget,
	restoreModpackFocus,
	useModpackDialogs,
	type InstallSource,
} from "@/lib/modpacks";
import { cn } from "@/lib/utils";

const CONFIRM_ID = "install-modpack-confirm";
const NAME_MAX = 64;
const OPTIONAL_COLLAPSED_FROM = 6;

type Loaded<T> = { status: "loading" } | { status: "ready"; value: T } | { status: "error"; error: AppError };

function baseName(path: string) {
	return path.split(/[\\/]/).pop() ?? path;
}

export function InstallModpackDialog() {
	const dialog = useModpackDialogs().install;
	const [busy, setBusy] = useState(false);
	return (
		<Dialog
			open={dialog.open}
			onOpenChange={(next) => {
				if (!next && !busy) closeModpackDialog("install");
			}}
		>
			<DialogContent
				zoom={false}
				className={STUDIO_DIALOG_CLASS}
				onCloseAutoFocus={(e) => restoreModpackFocus("install", e)}
				onOpenAutoFocus={(e) => {
					e.preventDefault();
					document.getElementById(CONFIRM_ID)?.focus();
				}}
			>
				<div className={STUDIO_SCROLL_CLASS}>
					{dialog.source && (
						<InstallForm key={dialog.session} source={dialog.source} onBusyChange={setBusy} />
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
}

function InstallForm({
	source,
	onBusyChange,
}: {
	source: InstallSource;
	onBusyChange: (busy: boolean) => void;
}) {
	const { t } = useTranslation();
	const store = useInstances();
	const ids = useId();
	const projectId = source.kind === "catalog" ? source.project.id : null;
	const catalogSource = source.kind === "catalog" ? source.project.source : undefined;

	const [versions, setVersions] = useState<{ key: number; value: Loaded<ContentVersion[]> } | null>(null);
	const [versionsAttempt, setVersionsAttempt] = useState(0);
	const [pickedVersion, setPickedVersion] = useState<string | null>(
		source.kind === "catalog" ? (source.versionId ?? null) : null,
	);
	const [inspectAttempt, setInspectAttempt] = useState(0);
	const [inspected, setInspected] = useState<{ key: string; value: Loaded<ModpackPreview> } | null>(null);
	const [progress, setProgress] = useState<{ key: string; percent: number | null } | null>(null);

	const [cover, setCover] = useState<InstanceCover>(() => leastUsedCover(getInstances().instances));
	const [customName, setCustomName] = useState<string | null>(null);
	const [group, setGroup] = useState<string | null>(null);
	/** По токену модпака: смена версии начинает заново. */
	const [unchecked, setUnchecked] = useState<{ token: string; paths: string[] } | null>(null);
	const [nameTouched, setNameTouched] = useState(false);
	const [submitted, setSubmitted] = useState(false);
	const [busy, setBusy] = useState(false);
	const [submitError, setSubmitError] = useState<AppError | null>(null);
	/** Куда вернуть фокус после ошибки `modpack_install`. */
	const focusAfterBusy = useRef<string | null>(null);

	useEffect(() => {
		if (busy || !focusAfterBusy.current) return;
		document.getElementById(focusAfterBusy.current)?.focus();
		focusAfterBusy.current = null;
	}, [busy]);

	useEffect(() => {
		if (!projectId) return;
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
	}, [projectId, catalogSource, versionsAttempt]);

	const versionsState: Loaded<ContentVersion[]> | null =
		source.kind !== "catalog"
			? null
			: versions && versions.key === versionsAttempt
				? versions.value
				: { status: "loading" };
	const versionList = versionsState?.status === "ready" ? versionsState.value : [];
	const versionId =
		source.kind === "catalog"
			? (pickedVersion ??
				(versionsState?.status === "ready" ? (defaultVersion(versionList)?.id ?? null) : null))
			: null;

	const inspectKey = source.kind === "catalog" ? `${versionId ?? ""}:${inspectAttempt}` : "file";
	useEffect(() => {
		if (!projectId || !versionId) return;
		let alive = true;
		const key = inspectKey;
		const requestId = inspectRequestId();
		let done = false;
		let unlisten: (() => void) | null = null;
		void modpackApi
			.onInspectProgress((event) => {
				if (!alive || event.requestId !== requestId) return;
				setProgress({
					key,
					percent:
						event.bytesTotal > 0
							? Math.min(100, (event.bytesDone / event.bytesTotal) * 100)
							: null,
				});
			})
			.then((fn) => {
				if (alive) unlisten = fn;
				else fn();
			});
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
			unlisten?.();
			if (!done) cancelInspect(requestId);
		};
	}, [projectId, catalogSource, versionId, inspectKey]);

	const preview: Loaded<ModpackPreview> =
		source.kind === "file"
			? { status: "ready", value: source.preview }
			: versionsState?.status === "ready" && !versionId
				? { status: "error", error: { code: "content_not_found", message: "" } }
				: inspected && inspected.key === inspectKey
					? inspected.value
					: { status: "loading" };
	const percent = progress && progress.key === inspectKey ? progress.percent : null;
	const ready = preview.status === "ready" ? preview.value : null;
	// Пока читается другая версия, прошлая остаётся неактивной: высота формы меняется один раз.
	const [heldPreview, setHeldPreview] = useState<ModpackPreview | null>(null);
	if (ready && ready !== heldPreview) setHeldPreview(ready);

	const title =
		source.kind === "catalog"
			? source.project.title
			: (source.preview.project?.title ?? source.preview.name);
	const iconUrl =
		source.kind === "catalog" ? source.project.iconUrl : (source.preview.project?.iconUrl ?? null);
	const autoName = uniqueName(title.slice(0, NAME_MAX).trim() || title, store.instances);
	const name = customName ?? autoName;
	const nameEmpty = name.trim() === "";
	const groupInvalid = submitError?.code === "invalid_name" && submitError.details === "group";
	const nameInvalid = submitError?.code === "invalid_name" && !groupInvalid;
	const showNameError = (nameEmpty && (nameTouched || submitted)) || nameInvalid;

	const optional = ready?.optional ?? [];
	const off = new Set(unchecked && ready && unchecked.token === ready.token ? unchecked.paths : []);
	const toggleOptional = (path: string, checked: boolean) => {
		if (!ready) return;
		const next = new Set(off);
		if (checked) next.delete(path);
		else next.add(path);
		setUnchecked({ token: ready.token, paths: [...next] });
	};

	const canSubmit = Boolean(ready) && !busy;

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		setSubmitted(true);
		if (!ready || busy || nameEmpty) return;
		// Поля сейчас станут `disabled`, и Chromium сбросит фокус на body.
		document.getElementById(CONFIRM_ID)?.focus();
		setBusy(true);
		onBusyChange(true);
		setSubmitError(null);
		try {
			await installModpack(
				{
					token: ready.token,
					name: name.trim(),
					group,
					cover,
					optional: optional.filter((f) => !off.has(f.path)).map((f) => f.path),
				},
				source.kind === "file",
			);
		} catch (err) {
			const error = toAppError(err);
			setSubmitError(error);
			focusAfterBusy.current =
				error.code === "invalid_name" ? (error.details === "group" ? groupId : nameId) : CONFIRM_ID;
		} finally {
			setBusy(false);
			onBusyChange(false);
		}
	};

	const nameId = `${ids}-name`;
	const nameErrorId = `${ids}-name-error`;
	const versionLabelId = `${ids}-version`;
	const groupLabelId = `${ids}-group`;
	const groupId = `${ids}-group-field`;
	const sourceName =
		SOURCE_NAMES[
			source.kind === "catalog"
				? source.project.source
				: (source.preview.project?.source ?? fileSource(source.path))
		];

	return (
		<form onSubmit={(e) => void submit(e)} noValidate>
			<AutoHeight>
				<div className={STUDIO_GRID_CLASS}>
					{/* Кнопки вне `fieldset`, чтобы фокус не терялся. */}
					<fieldset disabled={busy} className="contents">
						<CoverStudio
							cover={cover}
							onCoverChange={setCover}
							eyebrow={t("modpack.dialog.eyebrow")}
							title={name}
							media={<ProjectIcon url={iconUrl} type="modpack" size={40} />}
						/>
					</fieldset>

					<div className="flex min-w-0 flex-col p-6">
						<DialogTitle className="pr-10">{t("modpack.dialog.title")}</DialogTitle>
						<DialogDescription className="mt-1 truncate text-sm text-muted-foreground">
							{source.kind === "catalog"
								? t("modpack.dialog.subtitle", { title })
								: t("modpack.dialog.subtitleFile", { file: baseName(source.path) })}
						</DialogDescription>

						<fieldset disabled={busy} className="contents">
							<div className="mt-5">
								<Label htmlFor={nameId} className="text-[13px]/[18px] font-medium">
									{t("modpack.dialog.name")}
								</Label>
								<Input
									id={nameId}
									value={name}
									maxLength={NAME_MAX}
									spellCheck={false}
									autoComplete="off"
									aria-invalid={showNameError || undefined}
									aria-describedby={showNameError ? nameErrorId : undefined}
									className="mt-2 h-10"
									onChange={(e) => {
										setCustomName(e.target.value);
										if (nameInvalid) setSubmitError(null);
									}}
									onBlur={() => setNameTouched(true)}
								/>
								{showNameError && (
									<p
										id={nameErrorId}
										className="mt-2 flex items-center gap-1.5 text-[13px]/[18px] text-destructive"
									>
										<OctagonX className="shrink-0" />
										{nameEmpty
											? t("createInstance.nameRequired")
											: errorText(submitError ?? toAppError(null))}
									</p>
								)}
							</div>

							<div className="mt-5">
								<Label id={versionLabelId} className="text-[13px]/[18px] font-medium">
									{t("modpack.dialog.version")}
								</Label>
								{source.kind === "catalog" ? (
									versionsState?.status === "error" ? (
										<FailedLine
											className="mt-2 h-10"
											text={t("modpack.dialog.versionsFailed")}
											onRetry={() => setVersionsAttempt((n) => n + 1)}
										/>
									) : (
										<ModpackVersionPicker
											className="mt-2"
											versions={versionList}
											value={versionId}
											labelledBy={versionLabelId}
											loading={versionsState?.status === "loading"}
											onChange={setPickedVersion}
										/>
									)
								) : (
									<>
										<p
											aria-labelledby={versionLabelId}
											className="mt-2 flex h-9 min-w-0 items-center gap-2 text-sm"
										>
											<span className="truncate font-semibold">
												{source.preview.versionNumber}
											</span>
											<span className="min-w-0 truncate text-[13px]/[18px] text-muted-foreground">
												{previewTarget(source.preview)}
											</span>
										</p>
										<p className="flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
											{source.preview.project && <Check className="shrink-0" />}
											{source.preview.project
												? t("modpack.dialog.foundOnSource", { source: sourceName })
												: t("modpack.dialog.notFoundOnSource", {
														source: sourceName,
													})}
										</p>
									</>
								)}
							</div>

							<div className="mt-5">
								<Label id={groupLabelId} className="text-[13px]/[18px] font-medium">
									{t("modpack.dialog.folder")}
								</Label>
								<GroupCombobox
									id={groupId}
									value={group}
									labelledBy={groupLabelId}
									className="mt-2 w-full"
									side="top"
									onChange={(next) => {
										setGroup(next);
										if (groupInvalid) setSubmitError(null);
									}}
								/>
								{groupInvalid && submitError && (
									<p className="mt-2 flex items-center gap-1.5 text-[13px]/[18px] text-destructive">
										<OctagonX className="shrink-0" />
										{groupErrorText(submitError)}
									</p>
								)}
							</div>

							{versionsState?.status !== "error" && (
								<PreviewBlock
									preview={preview}
									held={heldPreview}
									percent={percent}
									off={off}
									onToggle={toggleOptional}
									onRetry={() => setInspectAttempt((n) => n + 1)}
								/>
							)}
						</fieldset>

						{submitError && !nameInvalid && !groupInvalid && (
							<p className="mt-5 flex items-start gap-1.5 text-[13px]/[18px] text-destructive">
								<OctagonX className="mt-px shrink-0" />
								{errorText(submitError)}
							</p>
						)}

						<div className="mt-auto flex justify-end gap-2 pt-6">
							<Button
								type="button"
								variant="ghost"
								aria-disabled={busy || undefined}
								className="aria-disabled:opacity-50"
								onClick={() => {
									if (!busy) closeModpackDialog("install");
								}}
							>
								{t("common.cancel")}
							</Button>
							<Button
								id={CONFIRM_ID}
								type="submit"
								aria-disabled={!canSubmit || undefined}
								className="aria-disabled:opacity-50"
							>
								{busy ? <LoaderCircle className="animate-spin" /> : <Download />}
								{t("modpack.dialog.confirm")}
							</Button>
						</div>
					</div>
				</div>
			</AutoHeight>
		</form>
	);
}

function FailedLine({ text, onRetry, className }: { text: string; onRetry: () => void; className?: string }) {
	const { t } = useTranslation();
	return (
		<div className={cn("flex items-center gap-2", className)}>
			<OctagonX className="shrink-0 text-destructive" />
			<span className="min-w-0 flex-1 text-[13px]/[18px] text-destructive">{text}</span>
			<Button type="button" variant="outline" size="sm" onClick={onRetry}>
				<RotateCcw />
				{t("common.retry")}
			</Button>
		</div>
	);
}

function PreviewBlock({
	preview,
	held,
	percent,
	off,
	onToggle,
	onRetry,
}: {
	preview: Loaded<ModpackPreview>;
	held: ModpackPreview | null;
	percent: number | null;
	off: Set<string>;
	onToggle: (path: string, checked: boolean) => void;
	onRetry: () => void;
}) {
	const { t } = useTranslation();
	const reading = (
		<>
			<LoaderCircle className="animate-spin text-primary-text" />
			{percent !== null
				? t("modpack.dialog.readingPercent", { percent: Math.round(percent) })
				: t("modpack.dialog.reading")}
		</>
	);
	if (preview.status === "error") {
		return <FailedLine className="mt-5" text={errorText(preview.error)} onRetry={onRetry} />;
	}
	const pending = preview.status === "loading";
	const value = preview.status === "ready" ? preview.value : held;
	if (!value) {
		return (
			<div aria-busy className="mt-5">
				<p className="flex items-center gap-2 text-[13px]/[18px] text-muted-foreground">{reading}</p>
				<Skeleton className="mt-3 h-3 w-56" />
			</div>
		);
	}
	const skipped = value.optional.filter((f) => off.has(f.path));
	// Модом считается только `mods/*.jar`, как в ядре.
	const skippedMods = skipped.filter((f) => f.type === "mod" && f.path.startsWith("mods/")).length;
	const mods = Math.max(0, value.mods - skippedMods);
	const bytes = Math.max(0, value.downloadBytes - skipped.reduce((sum, f) => sum + f.size, 0));
	const summary = [
		t("modpack.dialog.summary", { count: mods, size: formatBytes(bytes) }),
		value.javaMajor ? t("modpack.dialog.java", { major: value.javaMajor }) : null,
	]
		.filter(Boolean)
		.join(" · ");
	const dim = pending && "opacity-50";
	return (
		<div inert={pending || undefined} aria-busy={pending || undefined}>
			{value.optional.length > 0 && (
				<OptionalFiles files={value.optional} off={off} onToggle={onToggle} className={cn(dim)} />
			)}
			{pending ? (
				<p className="mt-3.5 flex items-center gap-2 text-[13px]/[18px] text-muted-foreground">
					{reading}
				</p>
			) : (
				<p className="mt-3.5 text-[13px]/[18px] text-muted-foreground tabular-nums">{summary}</p>
			)}
			{value.manualCount > 0 && <ManualWarning count={value.manualCount} className={cn(dim)} />}
		</div>
	);
}

export function ManualWarning({ count, className }: { count: number; className?: string }) {
	const { t } = useTranslation();
	return (
		<p className={cn("mt-2 flex items-start gap-1.5 text-[13px]/[18px] text-warning", className)}>
			<TriangleAlert className="mt-px shrink-0" />
			{t("modpack.dialog.manual", { count })}
		</p>
	);
}

function OptionalFiles({
	files,
	off,
	onToggle,
	className,
}: {
	files: ModpackOptionalFile[];
	off: Set<string>;
	onToggle: (path: string, checked: boolean) => void;
	className?: string;
}) {
	const { t } = useTranslation();
	const ids = useId();
	const [open, setOpen] = useState(files.length < OPTIONAL_COLLAPSED_FROM);
	const selected = files.length - files.filter((f) => off.has(f.path)).length;
	return (
		<Collapsible open={open} onOpenChange={setOpen} className={cn("mt-5", className)}>
			<CollapsibleTrigger className="relative -mx-1 flex w-[calc(100%+8px)] items-center gap-2 rounded-md border border-transparent px-1 text-left text-[13px]/[18px]">
				<ChevronRight
					className={cn(
						"shrink-0 text-muted-foreground transition-transform duration-(--duration-fast)",
						open && "rotate-90",
					)}
				/>
				<span className="font-semibold">{t("modpack.dialog.optional")}</span>
				<span className="text-muted-foreground tabular-nums">
					{t("modpack.dialog.optionalCount", { selected, total: files.length })}
				</span>
				<span aria-hidden className="h-px flex-1 bg-border" />
			</CollapsibleTrigger>
			<CollapsibleContent>
				<p className="mt-1.5 text-[13px]/[18px] text-muted-foreground">
					{t("modpack.dialog.optionalHint")}
				</p>
				<ul className="mt-1.5 flex flex-col">
					{files.map((file, index) => {
						const id = `${ids}-${index}`;
						return (
							<OptionalRow key={file.path} id={id}>
								<Checkbox
									id={id}
									checked={!off.has(file.path)}
									onCheckedChange={(value) => onToggle(file.path, value === true)}
								/>
								<ProjectIcon url={file.iconUrl} type={file.type ?? "mod"} size={32} />
								<span className="min-w-0 flex-1 truncate text-sm">
									{file.title ?? baseName(file.path)}
								</span>
								<span className="shrink-0 text-[13px]/[18px] text-muted-foreground tabular-nums">
									{formatBytes(file.size)}
								</span>
							</OptionalRow>
						);
					})}
				</ul>
			</CollapsibleContent>
		</Collapsible>
	);
}

function OptionalRow({ id, children }: { id: string; children: ReactNode }) {
	return (
		<li>
			<label
				htmlFor={id}
				className="-mx-2 flex h-11 cursor-pointer items-center gap-3 rounded-md px-2 transition-colors duration-(--duration-fast) hover:bg-accent"
			>
				{children}
			</label>
		</li>
	);
}
