import { cn } from "@/lib/utils";
import { Info, LoaderCircle, OctagonX, WifiOff } from "lucide-react";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { AutoHeight } from "@/components/auto-height";
import {
	CoverStudio,
	STUDIO_DIALOG_CLASS,
	STUDIO_GRID_CLASS,
	STUDIO_SCROLL_CLASS,
} from "@/components/instances/cover-studio";
import { GroupCombobox } from "@/components/instances/group-combobox";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { LOADER_KINDS, type LoaderKind } from "@/lib/api";
import { toAppError } from "@/lib/errors";
import { groupErrorText } from "@/lib/groups";
import { leastUsedCover, type InstanceCover } from "@/lib/covers";
import {
	closeInstanceDialog,
	createInstance,
	getInstanceDialogs,
	getInstances,
	restoreInstanceDialogFocus,
	uniqueName,
	useInstanceDialogs,
	useInstances,
} from "@/lib/instances";
import { LOADER_NAMES } from "@/lib/loaders";
import { instancePath, navigate } from "@/lib/router";
import { readStorage, writeStorage } from "@/lib/storage";
import {
	prefetchLoaderGameVersions,
	retryVersions,
	useGameVersions,
	useLoaderGameVersions,
	useLoaderVersions,
} from "@/lib/versions";
import { VersionCombobox, type VersionOption } from "@/pages/instances/version-combobox";
import { segmentGroup, segmentItem, segmentThumb } from "@/components/segment";
import { SlidingMarker } from "@/components/sliding-marker";

const SNAPSHOTS_KEY = "show-snapshots";
const CONFIRM_ID = "create-instance-confirm";

function defaultLoader(): LoaderKind {
	const last = [...getInstances().instances].sort(
		(a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt),
	)[0];
	return last?.loader.kind ?? "fabric";
}

export function CreateInstanceDialog() {
	const dialogs = useInstanceDialogs();
	const open = dialogs.open && dialogs.kind === "create";
	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) closeInstanceDialog();
			}}
		>
			<DialogContent
				zoom={false}
				className={STUDIO_DIALOG_CLASS}
				onCloseAutoFocus={restoreInstanceDialogFocus}
				onOpenAutoFocus={(e) => {
					e.preventDefault();
					const input = document.getElementById("create-instance-name") as HTMLInputElement | null;
					input?.focus();
					input?.select();
				}}
			>
				<div className={STUDIO_SCROLL_CLASS}>
					{dialogs.kind === "create" && <CreateForm key={dialogs.session} />}
				</div>
			</DialogContent>
		</Dialog>
	);
}

function CreateForm() {
	const { t } = useTranslation();
	const store = useInstances();
	const ids = useId();
	// Предвыбор читается один раз: у формы `key={session}`.
	const [preset] = useState(() => getInstanceDialogs().createPreset);
	// Наименее занятая обложка, уходит в ядро явно.
	const [cover, setCover] = useState<InstanceCover>(
		() => preset?.cover ?? leastUsedCover(getInstances().instances),
	);
	const [loader, setLoader] = useState<LoaderKind>(() => preset?.loader ?? defaultLoader());
	// Только на эту сессию: в настройку пишет лишь переключатель.
	const [snapshots, setSnapshots] = useState(
		() => Boolean(preset?.showSnapshots) || readStorage(SNAPSHOTS_KEY) === "true",
	);
	const [pickedVersion, setPickedVersion] = useState<string | null>(() => preset?.gameVersion ?? null);
	const [group, setGroup] = useState<string | null>(() => preset?.group ?? null);
	const [pickedLoaderVersion, setPickedLoaderVersion] = useState<{ key: string; id: string } | null>(null);
	const [customName, setCustomName] = useState<string | null>(null);
	const [nameBlurred, setNameBlurred] = useState(false);
	const [submitted, setSubmitted] = useState(false);
	const [busy, setBusy] = useState(false);
	const focusAfterBusy = useRef<string | null>(null);

	useEffect(() => {
		if (busy || !focusAfterBusy.current) return;
		document.getElementById(focusAfterBusy.current)?.focus();
		focusAfterBusy.current = null;
	}, [busy]);

	// Заранее, чтобы смена загрузчика сразу знала версию и форма менялась один раз.
	useEffect(prefetchLoaderGameVersions, []);

	const gameVersions = useGameVersions();
	const loaderGames = useLoaderGameVersions(loader);

	const versionOptions = useMemo<VersionOption[] | null>(() => {
		if (gameVersions?.status !== "ready" || loaderGames?.status !== "ready") return null;
		const supported = loader === "vanilla" ? null : new Set(loaderGames.value.items);
		const latest = gameVersions.value.latest.release;
		return gameVersions.value.versions
			.filter((v) => v.kind === "release" || (snapshots && v.kind === "snapshot"))
			.filter((v) => !supported || supported.has(v.id))
			.map((v) => ({
				id: v.id,
				badges: [
					...(v.id === latest ? [{ label: t("createInstance.badge.latest"), accent: true }] : []),
					...(v.kind === "snapshot" ? [{ label: t("createInstance.badge.snapshot") }] : []),
					...(v.downloaded ? [{ label: t("createInstance.badge.downloaded") }] : []),
				],
			}));
	}, [gameVersions, loaderGames, loader, snapshots, t]);

	const defaultVersion = useMemo(() => {
		if (!versionOptions || gameVersions?.status !== "ready") return null;
		const releases = new Set(
			gameVersions.value.versions.filter((v) => v.kind === "release").map((v) => v.id),
		);
		return versionOptions.find((o) => releases.has(o.id))?.id ?? versionOptions[0]?.id ?? null;
	}, [versionOptions, gameVersions]);

	const pickedSupported = Boolean(pickedVersion && versionOptions?.some((o) => o.id === pickedVersion));
	const gameVersion = pickedSupported ? pickedVersion : defaultVersion;
	// Подсказка — только если версию убрал загрузчик, а не фильтр снапшотов.
	const droppedByLoader =
		loader !== "vanilla" &&
		loaderGames?.status === "ready" &&
		!loaderGames.value.items.includes(pickedVersion ?? "");
	const switchedFrom =
		pickedVersion && versionOptions && !pickedSupported && droppedByLoader ? pickedVersion : null;

	const loaderVersions = useLoaderVersions(loader, gameVersion);
	const loaderKey = `${loader}:${gameVersion}`;
	const loaderOptions = useMemo<VersionOption[] | null>(() => {
		if (loaderVersions?.status !== "ready") return null;
		const hasStability = loaderVersions.value.items.some((v) => v.stable);
		return loaderVersions.value.items.map((v) => ({
			id: v.id,
			badges: [
				...(v.recommended ? [{ label: t("createInstance.badge.recommended"), accent: true }] : []),
				...(hasStability && !v.stable ? [{ label: t("createInstance.badge.beta") }] : []),
			],
		}));
	}, [loaderVersions, t]);
	const recommendedLoader =
		loaderVersions?.status === "ready"
			? ((loaderVersions.value.items.find((v) => v.recommended) ?? loaderVersions.value.items[0])?.id ??
				null)
			: null;
	const loaderVersion =
		pickedLoaderVersion?.key === loaderKey && loaderOptions?.some((o) => o.id === pickedLoaderVersion.id)
			? pickedLoaderVersion.id
			: recommendedLoader;

	const listsFailed = gameVersions?.status === "error" || loaderGames?.status === "error";
	const listsLoading = !listsFailed && !versionOptions;

	// Пока грузятся версии нового загрузчика, автоназвание держится на прошлой, а не пустеет.
	const [heldVersion, setHeldVersion] = useState<string | null>(null);
	if (gameVersion && gameVersion !== heldVersion) setHeldVersion(gameVersion);
	const nameVersion = gameVersion ?? (listsLoading ? heldVersion : null);
	const autoName = nameVersion ? uniqueName(`${LOADER_NAMES[loader]} ${nameVersion}`, store.instances) : "";
	const name = customName ?? autoName;
	const nameEmpty = name.trim() === "";
	// Пустое автоназвание, пока грузятся списки, — не ошибка.
	const showNameError = customName !== null && nameEmpty && (nameBlurred || submitted);
	const loaderFailed = loader !== "vanilla" && loaderVersions?.status === "error";
	const loaderLoading = loader !== "vanilla" && !loaderFailed && !loaderOptions;
	const offlineNow =
		(gameVersions?.status === "ready" && gameVersions.value.offline) ||
		(loaderGames?.status === "ready" && loaderGames.value.offline) ||
		(loader !== "vanilla" && loaderVersions?.status === "ready" && loaderVersions.value.offline);
	// Пока списки грузятся, держим последнее известное значение.
	const [heldOffline, setHeldOffline] = useState(false);
	const offlineKnown = !listsLoading && !loaderLoading;
	if (offlineKnown && offlineNow !== heldOffline) setHeldOffline(offlineNow);
	const offline = offlineKnown ? offlineNow : heldOffline;
	const ready = Boolean(gameVersion) && (loader === "vanilla" || Boolean(loaderVersion));
	const canSubmit = ready && !nameEmpty && !busy;

	const rearmName = () => {
		if (customName === "") setCustomName(null);
	};

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		setSubmitted(true);
		if (!canSubmit || !gameVersion) return;
		// Поля сейчас станут `disabled`, и Chromium сбросит фокус на body.
		document.getElementById(CONFIRM_ID)?.focus();
		setBusy(true);
		const session = getInstanceDialogs().session;
		try {
			const instance = await createInstance({
				name: name.trim(),
				gameVersion,
				loader: { kind: loader, version: loader === "vanilla" ? null : loaderVersion },
				cover,
				group,
			});
			preset?.afterCreate?.(instance);
			// Диалог закрыли или открыли заново, пока сборка создавалась.
			const dialogs = getInstanceDialogs();
			if (!dialogs.open || dialogs.session !== session) return;
			closeInstanceDialog();
			navigate(instancePath(instance.id));
		} catch (err) {
			const error = toAppError(err);
			toast.error(groupErrorText(error));
			focusAfterBusy.current =
				error.code === "invalid_name" ? (error.details === "group" ? groupId : nameId) : CONFIRM_ID;
			setBusy(false);
		}
	};

	const nameId = "create-instance-name";
	const nameErrorId = `${ids}-name-error`;
	const loaderLabelId = `${ids}-loader`;
	const versionLabelId = `${ids}-version`;
	const loaderVersionLabelId = `${ids}-loader-version`;
	const groupLabelId = `${ids}-group`;
	const groupId = `${ids}-group-field`;

	return (
		<form onSubmit={(e) => void submit(e)} noValidate>
			<AutoHeight>
				<div className={STUDIO_GRID_CLASS}>
					{/* Кнопки вне `fieldset`, чтобы фокус не терялся. */}
					<fieldset disabled={busy} className="contents">
						<CoverStudio
							cover={cover}
							onCoverChange={setCover}
							eyebrow={t("createInstance.title")}
							title={name}
						/>
					</fieldset>

					<div className="flex flex-col p-6">
						<DialogTitle className="pr-10">{t("createInstance.title")}</DialogTitle>
						<DialogDescription className="sr-only">
							{t("createInstance.description")}
						</DialogDescription>

						<fieldset disabled={busy} className="contents">
							<div className="mt-5">
								<Label htmlFor={nameId} className="text-[13px]/[18px] font-medium">
									{t("createInstance.name")}
								</Label>
								<Input
									id={nameId}
									value={name}
									maxLength={64}
									spellCheck={false}
									autoComplete="off"
									aria-invalid={showNameError || undefined}
									aria-describedby={showNameError ? nameErrorId : undefined}
									className="mt-2 h-10"
									onChange={(e) => setCustomName(e.target.value)}
									onBlur={() => setNameBlurred(true)}
								/>
							</div>
							{showNameError && (
								<p
									id={nameErrorId}
									className="mt-2 flex items-center gap-1.5 text-[13px]/[18px] text-destructive"
								>
									<OctagonX className="shrink-0" />
									{t("createInstance.nameRequired")}
								</p>
							)}

							<div className="mt-6">
								<Label id={loaderLabelId} className="text-[13px]/[18px] font-medium">
									{t("createInstance.loader")}
								</Label>
								<RadioGroupPrimitive.Root
									aria-labelledby={loaderLabelId}
									value={loader}
									orientation="horizontal"
									onValueChange={(next) => {
										setLoader(next as LoaderKind);
										rearmName();
									}}
									className={cn(segmentGroup, "mt-2 grid w-fit grid-cols-[repeat(5,68px)]")}
								>
									{LOADER_KINDS.map((kind) => (
										<RadioGroupPrimitive.Item
											key={kind}
											value={kind}
											className={segmentItem}
										>
											{kind === loader && (
												<SlidingMarker
													group={`${ids}loader`}
													value={loader}
													className={segmentThumb}
												/>
											)}
											<span className="relative">{LOADER_NAMES[kind]}</span>
										</RadioGroupPrimitive.Item>
									))}
								</RadioGroupPrimitive.Root>
							</div>

							<div className="mt-5">
								<Label id={versionLabelId} className="text-[13px]/[18px] font-medium">
									{t("createInstance.gameVersion")}
								</Label>
								{listsFailed ? (
									<FailedRow onRetry={() => retryVersions(loader, gameVersion)} />
								) : (
									<VersionCombobox
										options={versionOptions ?? []}
										value={gameVersion}
										labelledBy={versionLabelId}
										loading={listsLoading}
										onChange={(id) => {
											setPickedVersion(id);
											rearmName();
										}}
										footer={
											<label className="flex h-11 shrink-0 cursor-pointer items-center justify-between gap-3 border-t px-3 text-sm">
												{t("createInstance.showSnapshots")}
												<Switch
													checked={snapshots}
													onCheckedChange={(checked) => {
														setSnapshots(checked);
														writeStorage(SNAPSHOTS_KEY, checked ? "true" : null);
														rearmName();
													}}
												/>
											</label>
										}
									/>
								)}
								{switchedFrom && gameVersion && (
									<p className="mt-2 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
										<Info className="shrink-0" />
										{t("createInstance.versionSwitched", {
											loader: LOADER_NAMES[loader],
											version: switchedFrom,
											newVersion: gameVersion,
										})}
									</p>
								)}
								{offline && !listsFailed && (
									<p className="mt-2 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
										<WifiOff className="shrink-0" />
										{t("createInstance.offlineCached")}
									</p>
								)}
							</div>

							{loader !== "vanilla" && !listsFailed && (
								<div className="mt-5">
									<Label
										id={loaderVersionLabelId}
										className="text-[13px]/[18px] font-medium"
									>
										{t("createInstance.loaderVersion", { loader: LOADER_NAMES[loader] })}
									</Label>
									{loaderFailed ? (
										<FailedRow onRetry={() => retryVersions(loader, gameVersion)} />
									) : (
										<VersionCombobox
											options={loaderOptions ?? []}
											value={loaderVersion}
											labelledBy={loaderVersionLabelId}
											loading={loaderLoading}
											onChange={(id) => setPickedLoaderVersion({ key: loaderKey, id })}
										/>
									)}
								</div>
							)}

							<div className="mt-5">
								<Label id={groupLabelId} className="text-[13px]/[18px] font-medium">
									{t("groups.field.label")}
								</Label>
								<GroupCombobox
									id={groupId}
									value={group}
									labelledBy={groupLabelId}
									className="mt-2 w-full"
									side="top"
									onChange={setGroup}
								/>
							</div>
							{/* Место поля держим, чтобы высота окна не менялась при смене загрузчика. */}
							{(loader === "vanilla" || listsFailed) && (
								<div aria-hidden className="mt-5 h-[66px]" />
							)}
						</fieldset>

						<div className="mt-auto flex justify-end gap-2 pt-6">
							<Button
								type="button"
								variant="ghost"
								aria-disabled={busy || undefined}
								className="aria-disabled:opacity-50"
								onClick={() => {
									if (!busy) closeInstanceDialog();
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
								{busy && <LoaderCircle className="animate-spin" />}
								{busy ? t("createInstance.submitting") : t("createInstance.submit")}
							</Button>
						</div>
					</div>
				</div>
			</AutoHeight>
		</form>
	);
}

function FailedRow({ onRetry }: { onRetry: () => void }) {
	const { t } = useTranslation();
	return (
		<div className="mt-2 flex h-10 items-center gap-2">
			<OctagonX className="shrink-0 text-destructive" />
			<span className="text-[13px]/[18px]">{t("createInstance.versionsFailed")}</span>
			<Button type="button" variant="ghost" size="sm" className="ml-auto" onClick={onRetry}>
				{t("common.retry")}
			</Button>
		</div>
	);
}
