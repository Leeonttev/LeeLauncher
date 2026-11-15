import { ArrowUpRight, Info, OctagonX, Unlink } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CoverRadioGroup } from "@/components/instances/cover-picker";
import { GroupCombobox } from "@/components/instances/group-combobox";
import { JavaSelect } from "@/components/instances/java-select";
import { MemoryField } from "@/components/instances/memory-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { Instance, ModpackLink, Resolution } from "@/lib/api";
import { openProject, SOURCE_NAMES } from "@/lib/catalog";
import { formatDate, formatMemory } from "@/lib/format";
import { catalogLink, openUnlinkModpack } from "@/lib/modpacks";
import { defaultMemory, initGameSettings, useGameSettings } from "@/lib/game-settings";
import { coverOf, type InstanceCover } from "@/lib/covers";
import {
	getInstances,
	INSTANCE_JAVA_FIELD_ID,
	isBusy,
	openInstanceDialog,
	statusOf,
	updateInstance,
	useInstances,
} from "@/lib/instances";
import { instanceJavaMajor } from "@/lib/loaders";
import { SettingRow } from "@/pages/settings/setting-row";

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
	return (
		<section aria-labelledby={id} className="mt-10">
			<h2 id={id} className="text-xl/[26px] font-semibold tracking-[-0.015em]">
				{title}
			</h2>
			<div className="mt-2 divide-y divide-border">{children}</div>
		</section>
	);
}

/** Всё сохраняется сразу, кнопки «Сохранить» нет. */
export function InstanceSettings({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const store = useInstances();
	const status = statusOf(instance, store);
	const running = status === "running" || status === "launching";

	useEffect(() => initGameSettings(), []);

	return (
		<>
			<AnimatePresence initial={false}>
				{running && (
					<motion.p
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: 0.2 }}
						className="mt-8 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground"
					>
						<Info className="shrink-0" />
						{t("instance.restartHint")}
					</motion.p>
				)}
			</AnimatePresence>

			<Section id="instance-general" title={t("instance.settings.general")}>
				<SettingRow
					inline
					htmlFor="instance-name"
					title={t("instance.settings.name.title")}
					description={t("instance.settings.name.description")}
				>
					<NameField key={instance.name} instance={instance} />
				</SettingRow>
				<SettingRow
					inline
					titleId="instance-group"
					title={t("groups.field.label")}
					description={t("groups.field.hint")}
				>
					<GroupCombobox
						value={instance.group}
						labelledBy="instance-group"
						className="h-9 w-80 shrink-0"
						onChange={(group) => void updateInstance(instance.id, { group })}
					/>
				</SettingRow>
				<SettingRow
					titleId="instance-cover"
					title={t("instance.settings.cover.title")}
					description={t("instance.settings.cover.description")}
				>
					<CoverRow instance={instance} />
				</SettingRow>
			</Section>

			<Section id="instance-game" title={t("instance.settings.game")}>
				<SettingRow
					titleId="instance-memory"
					title={t("instance.settings.memory.title")}
					description={t("instance.settings.memory.description")}
				>
					<MemoryRow instance={instance} />
				</SettingRow>
				<SettingRow
					inline
					htmlFor={INSTANCE_JAVA_FIELD_ID}
					title={t("instance.settings.java.title")}
					description={t("instance.settings.java.description")}
				>
					<JavaRow instance={instance} />
				</SettingRow>
				<ResolutionRow instance={instance} />
				<SettingRow
					htmlFor="instance-jvm-args"
					title={t("instance.settings.jvmArgs.title")}
					description={t("instance.settings.jvmArgs.description")}
				>
					<JvmArgsField key={instance.jvmArgs ?? ""} instance={instance} />
				</SettingRow>
			</Section>

			<AnimatePresence initial={false}>
				{instance.modpack && (
					<motion.div
						key="modpack"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: 0.12 }}
					>
						<ModpackSection instance={instance} link={instance.modpack} />
					</motion.div>
				)}
			</AnimatePresence>

			<Section id="instance-manage" title={t("instance.settings.manage")}>
				<ManageRow instance={instance} kind="reinstall" />
				<ManageRow instance={instance} kind="remove" />
			</Section>
		</>
	);
}

function NameField({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const [value, setValue] = useState(instance.name);
	const [error, setError] = useState(false);

	useEffect(() => {
		if (!error) return;
		const timer = setTimeout(() => setError(false), 3000);
		return () => clearTimeout(timer);
	}, [error]);

	const save = () => {
		const trimmed = value.trim();
		if (!trimmed) {
			setValue(instance.name);
			setError(true);
			return;
		}
		if (trimmed !== instance.name)
			void updateInstance(instance.id, { name: trimmed }).then((r) => !r && setValue(instance.name));
		else setValue(instance.name);
	};

	return (
		<div className="relative shrink-0">
			<Input
				id="instance-name"
				value={value}
				maxLength={64}
				spellCheck={false}
				autoComplete="off"
				aria-invalid={error || undefined}
				aria-describedby={error ? "instance-name-error" : undefined}
				className="h-9 w-80"
				onChange={(e) => setValue(e.target.value)}
				onBlur={save}
				onKeyDown={(e) => {
					if (e.key === "Enter") {
						e.preventDefault();
						e.currentTarget.blur();
					} else if (e.key === "Escape") {
						e.preventDefault();
						setValue(instance.name);
						requestAnimationFrame(() => (e.target as HTMLInputElement).blur());
					}
				}}
			/>
			{error && (
				<p
					id="instance-name-error"
					className="absolute top-full left-0 mt-1.5 flex items-center gap-1.5 text-[13px]/[18px] text-destructive"
				>
					<OctagonX className="shrink-0" />
					{t("createInstance.nameRequired")}
				</p>
			)}
		</div>
	);
}

/** Стрелками можно пройти несколько вариантов, сохранится последний. */
const COVER_SAVE_DELAY = 300;

function savedCover(id: string): InstanceCover | null {
	const instance = getInstances().instances.find((i) => i.id === id);
	return instance ? coverOf(instance) : null;
}

function saveCover(id: string, cover: InstanceCover) {
	if (savedCover(id) === cover) return Promise.resolve(true);
	return updateInstance(id, { cover }).then(Boolean);
}

function CoverRow({ instance }: { instance: Instance }) {
	const saved = coverOf(instance);
	const [draft, setDraft] = useState(saved);
	const [lastSaved, setLastSaved] = useState(saved);
	const pending = useRef<{ timer: number; cover: InstanceCover } | null>(null);
	const id = instance.id;

	if (saved !== lastSaved) {
		setLastSaved(saved);
		setDraft(saved);
	}

	// Уходя со страницы, не теряем выбор, который ещё ждёт сохранения.
	useEffect(
		() => () => {
			const job = pending.current;
			if (!job) return;
			clearTimeout(job.timer);
			pending.current = null;
			void saveCover(id, job.cover);
		},
		[id],
	);

	const change = (cover: InstanceCover) => {
		setDraft(cover);
		if (pending.current) clearTimeout(pending.current.timer);
		const timer = window.setTimeout(() => {
			pending.current = null;
			void saveCover(id, cover).then((ok) => {
				if (!ok) setDraft(savedCover(id) ?? cover);
			});
		}, COVER_SAVE_DELAY);
		pending.current = { timer, cover };
	};

	return <CoverRadioGroup value={draft} labelledBy="instance-cover" layout="row" onChange={change} />;
}

function MemoryRow({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const game = useGameSettings();
	const fallback = defaultMemory(game);
	const useDefault = instance.memory === null;
	const totalMb =
		game.memory.status === "ready"
			? (game.memory.value?.totalMb ?? 0)
			: game.memory.status === "error"
				? 0
				: null;

	return (
		<div>
			<div className="flex items-center gap-3">
				<Switch
					id="instance-memory-default"
					checked={useDefault}
					disabled={!useDefault && !fallback}
					onCheckedChange={(checked) => {
						if (checked) void updateInstance(instance.id, { memory: null });
						else if (fallback) void updateInstance(instance.id, { memory: { ...fallback } });
					}}
				/>
				<Label htmlFor="instance-memory-default" className="text-sm font-normal">
					{t("instance.settings.memory.useDefault", {
						value: fallback ? formatMemory(fallback.maxMb) : "…",
					})}
				</Label>
			</div>
			<AnimatePresence initial={false}>
				{instance.memory && (
					<motion.div
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: 0.15 }}
						className="mt-4"
					>
						<MemoryField
							valueMb={instance.memory.maxMb}
							totalMb={totalMb}
							labelledBy="instance-memory"
							hint={t("instance.settings.memory.hint")}
							onCommit={(mb) =>
								void updateInstance(instance.id, {
									memory: { minMb: Math.min(instance.memory?.minMb ?? 512, mb), maxMb: mb },
								})
							}
						/>
					</motion.div>
				)}
			</AnimatePresence>
		</div>
	);
}

function JavaRow({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const game = useGameSettings();
	const major = instanceJavaMajor(instance);
	return (
		<div className="flex min-w-0 shrink-0 flex-col items-end">
			<JavaSelect
				id={INSTANCE_JAVA_FIELD_ID}
				value={instance.javaPath}
				installs={game.java.value}
				major={major}
				autoLabel={
					major
						? t("instance.settings.java.useDefault", { major })
						: t("instance.settings.java.useDefaultUnknown")
				}
				foundLabel={t("instance.settings.java.found")}
				incompatibleNote={t("instance.settings.java.incompatible", { version: instance.gameVersion })}
				className="w-80"
				onChange={(path) => void updateInstance(instance.id, { javaPath: path })}
			/>
			{instance.javaPath && (
				<p className="mt-2 w-80 truncate font-mono text-xs/4 text-muted-foreground select-text">
					{instance.javaPath}
				</p>
			)}
		</div>
	);
}

const RES_LIMITS = { width: [640, 7680], height: [480, 4320] } as const;
const DEFAULT_RESOLUTION: Resolution = { width: 1280, height: 720 };

function clampSide(value: number, side: keyof typeof RES_LIMITS) {
	const [min, max] = RES_LIMITS[side];
	return Math.min(max, Math.max(min, Math.round(value)));
}

type WindowMode = "default" | "fullscreen" | "custom";

function ResolutionRow({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const gameWindow = instance.resolution;
	const resolution = gameWindow && gameWindow !== "fullscreen" ? gameWindow : null;
	const mode: WindowMode = gameWindow === "fullscreen" ? "fullscreen" : resolution ? "custom" : "default";
	const savedWidth = String(resolution?.width ?? DEFAULT_RESOLUTION.width);
	const savedHeight = String(resolution?.height ?? DEFAULT_RESOLUTION.height);
	const [draft, setDraft] = useState({ width: savedWidth, height: savedHeight });
	const [synced, setSynced] = useState({ width: savedWidth, height: savedHeight });
	// Черновик берёт новое значение сборки, строка не пересоздаётся.
	if (synced.width !== savedWidth || synced.height !== savedHeight) {
		setSynced({ width: savedWidth, height: savedHeight });
		setDraft({ width: savedWidth, height: savedHeight });
	}

	const commit = () => {
		if (!resolution) return;
		const width = clampSide(Number.parseInt(draft.width, 10) || resolution.width, "width");
		const height = clampSide(Number.parseInt(draft.height, 10) || resolution.height, "height");
		setDraft({ width: String(width), height: String(height) });
		if (width !== resolution.width || height !== resolution.height)
			void updateInstance(instance.id, { resolution: { width, height } }).then(
				(saved) => !saved && setDraft({ width: savedWidth, height: savedHeight }),
			);
	};

	const sideInput = (side: "width" | "height") => (
		<Input
			inputMode="numeric"
			value={draft[side]}
			aria-label={t(`instance.settings.resolution.${side}`)}
			className="h-9 w-24"
			inputClassName="tabular-nums"
			onChange={(e) => setDraft((d) => ({ ...d, [side]: e.target.value.replace(/\D/g, "") }))}
			onBlur={commit}
			onKeyDown={(e) => {
				if (e.key === "Enter") {
					e.preventDefault();
					commit();
				}
			}}
		/>
	);

	return (
		<div className="py-5">
			<div className="flex items-center gap-6">
				<div className="min-w-0 flex-1">
					<Label htmlFor="instance-resolution" className="text-sm font-medium">
						{t("instance.settings.resolution.title")}
					</Label>
					<p className="mt-0.5 max-w-prose text-[13px]/[18px] text-muted-foreground">
						{t("instance.settings.resolution.description")}
					</p>
				</div>
				<Select
					value={mode}
					onValueChange={(value) => {
						const next = value as WindowMode;
						if (next === mode) return;
						void updateInstance(instance.id, {
							resolution:
								next === "custom"
									? DEFAULT_RESOLUTION
									: next === "fullscreen"
										? "fullscreen"
										: null,
						});
					}}
				>
					<SelectTrigger id="instance-resolution" className="h-9 w-48 shrink-0">
						<SelectValue />
					</SelectTrigger>
					<SelectContent position="popper" align="end">
						<SelectItem value="default">{t("instance.settings.resolution.default")}</SelectItem>
						<SelectItem value="fullscreen">
							{t("instance.settings.resolution.fullscreen")}
						</SelectItem>
						<SelectItem value="custom">{t("instance.settings.resolution.custom")}</SelectItem>
					</SelectContent>
				</Select>
			</div>
			{resolution && (
				<motion.div
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					transition={{ duration: 0.15 }}
					className="mt-4 flex items-center gap-2"
				>
					{sideInput("width")}
					<span aria-hidden className="text-muted-foreground">
						{"×"}
					</span>
					{sideInput("height")}
					<span className="text-[13px]/[18px] text-muted-foreground">
						{t("instance.settings.resolution.unit")}
					</span>
				</motion.div>
			)}
		</div>
	);
}

function JvmArgsField({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const [value, setValue] = useState(instance.jvmArgs ?? "");
	const hasMemory = /(^|\s)-Xm[xs]/.test(value);

	const save = () => {
		const next = value.trim() ? value.trim() : null;
		if (next !== instance.jvmArgs) void updateInstance(instance.id, { jvmArgs: next });
	};

	return (
		<div>
			<Textarea
				id="instance-jvm-args"
				value={value}
				spellCheck={false}
				autoComplete="off"
				placeholder="-XX:+UseG1GC -XX:MaxGCPauseMillis=50"
				className="min-h-20 w-full font-mono text-xs/[18px] md:text-xs/[18px]"
				onChange={(e) => setValue(e.target.value)}
				onBlur={save}
			/>
			{hasMemory && (
				<p className="mt-2 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
					<Info className="shrink-0" />
					{t("instance.settings.jvmArgs.memoryOverride")}
				</p>
			)}
		</div>
	);
}

function ManageRow({ instance, kind }: { instance: Instance; kind: "reinstall" | "remove" }) {
	const { t } = useTranslation();
	const store = useInstances();
	const status = statusOf(instance, store);
	const busy = isBusy(status);
	const busyText =
		status === "running" || status === "launching"
			? t("instance.settings.busyRunning")
			: t("instance.settings.busyInstalling");
	const remove = kind === "remove";

	const button = (
		<Button
			variant="outline"
			size="sm"
			disabled={busy}
			className={
				remove
					? "border-destructive/40 text-destructive hover:bg-destructive/10 hover:text-destructive"
					: undefined
			}
			onClick={(e) =>
				openInstanceDialog(kind, instance.id, {
					returnFocus: e.currentTarget,
					leaveAfterRemove: true,
				})
			}
		>
			{remove ? t("instance.deleteDialog.confirm") : t("instance.settings.reinstall.title")}
		</Button>
	);

	return (
		<SettingRow
			inline
			title={remove ? t("instance.settings.delete.title") : t("instance.settings.reinstall.title")}
			description={
				remove
					? t("instance.settings.delete.description")
					: t("instance.settings.reinstall.description")
			}
		>
			{busy ? (
				<Tooltip>
					<TooltipTrigger asChild>
						<span tabIndex={0} className="relative shrink-0 rounded-lg border border-transparent">
							{button}
						</span>
					</TooltipTrigger>
					<TooltipContent>{busyText}</TooltipContent>
				</Tooltip>
			) : (
				<span className="shrink-0">{button}</span>
			)}
		</SettingRow>
	);
}

function ModpackSection({ instance, link }: { instance: Instance; link: ModpackLink }) {
	const { t } = useTranslation();
	const catalog = catalogLink(instance);
	const date = formatDate(new Date(link.installedAt).toISOString());
	return (
		<Section id="instance-modpack" title={t("modpack.settings.title")}>
			<SettingRow
				inline
				title={`${link.title} · ${link.versionNumber}`}
				description={
					catalog
						? t("modpack.settings.source", { date, source: SOURCE_NAMES[catalog.source] })
						: t("modpack.settings.sourceFile", { date })
				}
			>
				{catalog && (
					<Button
						variant="outline"
						size="sm"
						className="shrink-0"
						onClick={() =>
							openProject({ source: catalog.source, id: catalog.projectId }, "modpack")
						}
					>
						<ArrowUpRight />
						{t("modpack.settings.openPage")}
					</Button>
				)}
			</SettingRow>
			<SettingRow
				inline
				title={t("modpack.settings.unlinkTitle")}
				description={t("modpack.settings.unlinkHint")}
			>
				<Button
					variant="outline"
					size="sm"
					className="shrink-0"
					onClick={(e) => openUnlinkModpack(instance.id, e.currentTarget)}
				>
					<Unlink />
					{t("modpack.settings.unlink")}
				</Button>
			</SettingRow>
		</Section>
	);
}
