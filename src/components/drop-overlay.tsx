import { Braces, Download, Globe, OctagonX, Package, Palette, Sparkles, Upload } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { createElement, type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { InstanceThumb } from "@/components/instances/instance-cover";
import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { overlayOpen } from "@/hooks/use-create-shortcut";
import { contentApi, fileDrop, type ContentType, type Instance } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import { applyAddedContent } from "@/lib/content";
import { coverOf } from "@/lib/covers";
import { errorText, toAppError } from "@/lib/errors";
import { formatAgo } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { compareRecent, getInstances, useInstances } from "@/lib/instances";
import { versionLabel } from "@/lib/loaders";
import { activeAccount, getAccounts } from "@/lib/accounts";
import { addManualFiles, manualDialogTarget } from "@/lib/manual";
import { openModpackFile } from "@/lib/modpacks";
import { getRoute, instanceIdOf } from "@/lib/router";
import { addSkinFiles } from "@/lib/skins";
import { cn } from "@/lib/utils";
import { useWorlds } from "@/lib/worlds";

const PICK_TYPES = ["resourcepack", "shader", "datapack"] as const satisfies readonly ContentType[];
type PickType = (typeof PICK_TYPES)[number];

const TYPE_ICONS = { resourcepack: Palette, shader: Sparkles, datapack: Braces } as const;

type Step =
	| { kind: "instance"; paths: string[] }
	| { kind: "type"; instanceId: string; paths: string[] }
	| { kind: "world"; instanceId: string; paths: string[] };

function baseName(path: string) {
	return path.split(/[\\/]/).pop() ?? path;
}

function isModpackFile(path: string) {
	return path.toLowerCase().endsWith(".mrpack");
}

function isZip(path: string) {
	return path.toLowerCase().endsWith(".zip");
}

/** На «Скинах» файлы идут в библиотеку; `.mrpack` без PNG по-прежнему ставится как модпак. */
function isSkinDrop(paths: string[]) {
	if (getRoute() !== "/skins" || activeAccount(getAccounts())?.kind === "offline") return false;
	return paths.some((path) => path.toLowerCase().endsWith(".png")) || !paths.some(isModpackFile);
}

/** Без сборок `.zip` тоже пробуем как модпак: ресурспаку или датапаку всё равно некуда деться. */
function findModpack(paths: string[]): string | null {
	return (
		paths.find(isModpackFile) ??
		(getInstances().instances.length === 0 ? paths.find(isZip) : undefined) ??
		null
	);
}

/** В StrictMode и при HMR эффект подписывается заново, а отписка асинхронная: действует только последняя подписка. */
let subscription = 0;
/** Tauri может прислать одно и то же отпускание дважды (окну и webview). */
let lastDrop: { key: string; at: number } | null = null;
const adding = new Set<string>();

function dropKey(instanceId: string, paths: string[], projectType?: ContentType, world?: string) {
	return [instanceId, projectType ?? "", world ?? "", ...[...paths].sort()].join("|");
}

function currentInstance(): Instance | null {
	const id = instanceIdOf(getRoute());
	return id ? (getInstances().instances.find((i) => i.id === id) ?? null) : null;
}

export function DropOverlay() {
	const { t } = useTranslation();
	const { reduceMotion } = useAppearance();
	const [dragging, setDragging] = useState<{
		count: number;
		name: string | null;
		modpack: string | null;
		manual: boolean;
		skin: boolean;
		skinWrong?: boolean;
	} | null>(null);
	const [step, setStep] = useState<Step | null>(null);
	const [open, setOpen] = useState(false);

	const show = (next: Step) => {
		setStep(next);
		setOpen(true);
	};

	const add = async (instanceId: string, paths: string[], projectType?: ContentType, world?: string) => {
		const key = dropKey(instanceId, paths, projectType, world);
		if (adding.has(key)) return;
		adding.add(key);
		try {
			const result = await contentApi.addFiles(instanceId, paths, { projectType, world });
			applyAddedContent(instanceId, result.added);
			if (result.added.length) toast.success(i18n.t("drop.added", { count: result.added.length }));
			const modpack = result.unsupported.find(
				(u) => u.reason === "modpack" || u.reason === "curseforge",
			);
			const wrong = result.unsupported.filter((u) => u.reason === "type");
			if (modpack) void openModpackFile(modpack.path);
			if (wrong.length)
				toast.error(
					i18n.t("drop.unsupported", { names: wrong.map((u) => baseName(u.path)).join(", ") }),
				);
			if (result.needsType.length) show({ kind: "type", instanceId, paths: result.needsType });
		} catch (err) {
			toast.error(errorText(toAppError(err)));
		} finally {
			adding.delete(key);
		}
	};

	useEffect(() => {
		let unlisten: (() => void) | null = null;
		let disposed = false;
		let accepted = false;
		const token = ++subscription;
		fileDrop
			.listen((event) => {
				if (token !== subscription) return;
				if (event.type === "enter") {
					if (manualDialogTarget() && event.paths.length) {
						accepted = true;
						setDragging({
							count: event.paths.length,
							name: null,
							modpack: null,
							manual: true,
							skin: false,
						});
						return;
					}
					if (isSkinDrop(event.paths)) {
						accepted = event.paths.length > 0 && !overlayOpen();
						if (accepted)
							setDragging({
								count: event.paths.length,
								name: null,
								modpack: null,
								manual: false,
								skin: true,
								skinWrong: !event.paths.some((path) => path.toLowerCase().endsWith(".png")),
							});
						return;
					}
					const modpack = findModpack(event.paths);
					accepted =
						event.paths.length > 0 &&
						!overlayOpen() &&
						(modpack !== null || getInstances().instances.length > 0);
					if (!accepted) return;
					setDragging({
						count: event.paths.length,
						name: currentInstance()?.name ?? null,
						modpack: modpack ? baseName(modpack) : null,
						manual: false,
						skin: false,
					});
				} else if (event.type === "leave") {
					accepted = false;
					setDragging(null);
				} else if (event.type === "drop") {
					setDragging(null);
					if (!accepted || !event.paths.length) return;
					accepted = false;
					const key = [...event.paths].sort().join("|");
					const now = Date.now();
					if (lastDrop && lastDrop.key === key && now - lastDrop.at < 1500) return;
					lastDrop = { key, at: now };
					if (manualDialogTarget()) {
						void addManualFiles(event.paths);
						return;
					}
					if (isSkinDrop(event.paths)) {
						void addSkinFiles(event.paths);
						return;
					}
					const modpack = findModpack(event.paths);
					if (modpack) {
						if (event.paths.length > 1) toast(i18n.t("drop.modpackAlone"));
						void openModpackFile(
							modpack,
							undefined,
							isZip(modpack) ? i18n.t("drop.zipNotModpack") : undefined,
						);
						return;
					}
					const instance = currentInstance();
					if (instance) void add(instance.id, event.paths);
					else show({ kind: "instance", paths: event.paths });
				}
			})
			.then((fn) => {
				if (disposed) fn();
				else unlisten = fn;
			})
			.catch((err: unknown) => console.error("Не удалось подписаться на перетаскивание файлов", err));
		return () => {
			disposed = true;
			unlisten?.();
		};
		// Подписка одна на всё время: `add`/`show` меняют только состояние этого компонента.
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, []);

	return (
		<>
			<AnimatePresence>
				{dragging && (
					<motion.div
						key="drop"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: reduceMotion ? 0 : 0.12 }}
						className={cn(
							"pointer-events-none fixed inset-x-0 top-14 bottom-0 bg-background/88",
							dragging.manual ? "z-[60]" : "z-40",
						)}
					>
						<div
							className={cn(
								"absolute top-[round(down,50%,1px)] left-[round(down,50%,1px)] flex w-130 translate-x-[round(down,-50%,1px)] translate-y-[round(down,-50%,1px)] flex-col items-center rounded-2xl border bg-background px-8 py-9 text-center",
								dragging.skinWrong ? "border-destructive" : "border-primary",
							)}
						>
							<span
								className={cn(
									"flex size-14 items-center justify-center rounded-2xl",
									dragging.skinWrong
										? "bg-destructive/12 text-destructive"
										: "bg-primary-soft text-primary-text",
								)}
							>
								{dragging.skinWrong ? (
									<OctagonX className="size-6" />
								) : dragging.skin ? (
									<Upload className="size-6" />
								) : dragging.modpack && !dragging.manual ? (
									<Package className="size-6" />
								) : (
									<Download className="size-6" />
								)}
							</span>
							<h2 className="mt-5 text-lg/6 font-semibold text-balance">
								{dragging.skinWrong
									? t("skins.drop.wrongTitle")
									: dragging.skin
										? t("skins.drop.title")
										: dragging.manual
											? t("manual.dropTitle")
											: dragging.modpack
												? t("drop.modpackTitle")
												: dragging.name
													? t("drop.title", { name: dragging.name })
													: t("drop.titleAny")}
							</h2>
							<p className="mt-2 max-w-full truncate text-sm text-muted-foreground">
								{dragging.skin
									? t("skins.drop.hint")
									: dragging.manual
										? t("manual.strip.hint")
										: dragging.modpack
											? t("drop.modpackHint", { file: dragging.modpack })
											: t("drop.hint", { count: dragging.count })}
							</p>
						</div>
					</motion.div>
				)}
			</AnimatePresence>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent className="w-[448px] max-w-[calc(100vw-2rem)] gap-0 p-0 sm:max-w-[448px]">
					{step && (
						<StepBody
							step={step}
							onInstance={(id) => {
								setOpen(false);
								void add(id, step.paths);
							}}
							onType={(type) => {
								if (step.kind !== "type") return;
								if (type === "datapack") {
									setStep({
										kind: "world",
										instanceId: step.instanceId,
										paths: step.paths,
									});
									return;
								}
								setOpen(false);
								void add(step.instanceId, step.paths, type);
							}}
							onWorld={(world) => {
								if (step.kind !== "world") return;
								setOpen(false);
								void add(step.instanceId, step.paths, "datapack", world);
							}}
						/>
					)}
				</DialogContent>
			</Dialog>
		</>
	);
}

const rowClass =
	"rounded-lg border relative flex w-full items-center gap-3 border-transparent px-2.5 py-2 text-left transition-colors duration-(--duration-fast) ease-out hover:bg-accent";

function StepHeader({ title, description }: { title: string; description?: string }) {
	return (
		<div className="px-5 pt-5 pr-14 pb-3">
			<DialogTitle>{title}</DialogTitle>
			<DialogDescription
				className={cn("mt-1 text-[13px]/[18px] text-muted-foreground", !description && "sr-only")}
			>
				{description ?? title}
			</DialogDescription>
		</div>
	);
}

function StepBody({
	step,
	onInstance,
	onType,
	onWorld,
}: {
	step: Step;
	onInstance: (id: string) => void;
	onType: (type: PickType) => void;
	onWorld: (world: string) => void;
}) {
	const { t } = useTranslation();
	const names = step.paths.map(baseName).join(", ");

	if (step.kind === "instance") return <InstanceStep names={names} onPick={onInstance} />;
	if (step.kind === "type")
		return (
			<>
				<StepHeader title={t("drop.pickType")} description={names} />
				<List>
					{PICK_TYPES.map((type) => (
						<button key={type} type="button" className={rowClass} onClick={() => onType(type)}>
							<span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
								{createElement(TYPE_ICONS[type])}
							</span>
							<span className="text-sm/5 font-semibold">{t(`drop.type.${type}`)}</span>
						</button>
					))}
				</List>
			</>
		);
	return <WorldStep instanceId={step.instanceId} names={names} onPick={onWorld} />;
}

function List({ children }: { children: ReactNode }) {
	return <div className="max-h-80 overflow-y-auto px-2.5 pb-2.5">{children}</div>;
}

function InstanceStep({ names, onPick }: { names: string; onPick: (id: string) => void }) {
	const { t } = useTranslation();
	const store = useInstances();
	const instances = [...store.instances].sort(compareRecent);
	return (
		<>
			<StepHeader title={t("drop.pickInstance")} description={names} />
			{instances.length ? (
				<List>
					{instances.map((instance) => (
						<button
							key={instance.id}
							type="button"
							className={rowClass}
							onClick={() => onPick(instance.id)}
						>
							<InstanceThumb scene={coverOf(instance)} size={32} />
							<span className="min-w-0 flex-1">
								<span className="block truncate text-sm/5 font-semibold">
									{instance.name}
								</span>
								<span className="block truncate text-[13px]/[18px] text-muted-foreground">
									{versionLabel(instance)}
								</span>
							</span>
						</button>
					))}
				</List>
			) : (
				<p className="px-5 pb-5 text-[13px]/[18px] text-muted-foreground">{t("drop.noInstances")}</p>
			)}
		</>
	);
}

function WorldStep({
	instanceId,
	names,
	onPick,
}: {
	instanceId: string;
	names: string;
	onPick: (world: string) => void;
}) {
	const { t } = useTranslation();
	const worlds = useWorlds(instanceId);
	const items = worlds?.status === "ready" ? worlds.items.filter((w) => !w.damaged) : [];

	let body: ReactNode;
	if (!worlds || worlds.status === "loading") {
		body = (
			<div aria-busy className="flex flex-col gap-2 px-5 pb-5">
				<Skeleton className="h-8 w-full" />
				<Skeleton className="h-8 w-full" />
			</div>
		);
	} else if (worlds.status === "error") {
		body = (
			<p className="flex items-start gap-1.5 px-5 pb-5 text-[13px]/[18px] text-destructive">
				<OctagonX className="mt-px shrink-0" />
				<span className="min-w-0">
					{worlds.error ? errorText(worlds.error) : t("errors.fallback")}
				</span>
			</p>
		);
	} else if (!items.length) {
		body = (
			<p className="px-5 pb-5 text-[13px]/[18px] text-muted-foreground">
				{t("catalog.where.noWorlds")}
			</p>
		);
	} else {
		body = (
			<List>
				{items.map((world) => (
					<button
						key={world.folder}
						type="button"
						className={rowClass}
						onClick={() => onPick(world.folder)}
					>
						{world.iconUrl ? (
							<img
								src={world.iconUrl}
								alt=""
								className="size-8 shrink-0 rounded-lg bg-muted [image-rendering:pixelated]"
							/>
						) : (
							<span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
								<Globe />
							</span>
						)}
						<span className="min-w-0 flex-1">
							<span className="block truncate text-sm/5 font-semibold">{world.name}</span>
							{world.lastPlayedAt !== null && (
								<span className="block truncate text-[13px]/[18px] text-muted-foreground">
									{formatAgo(world.lastPlayedAt)}
								</span>
							)}
						</span>
					</button>
				))}
			</List>
		);
	}

	return (
		<>
			<StepHeader title={t("catalog.where.world")} description={names} />
			{body}
			{!items.length && worlds?.status !== "loading" && (
				<div className="flex justify-end px-5 pb-5">
					<DialogClose asChild>
						<Button variant="ghost">{t("common.cancel")}</Button>
					</DialogClose>
				</div>
			)}
		</>
	);
}
