import {
	ArrowUpRight,
	Check,
	Download,
	ExternalLink,
	Globe,
	LoaderCircle,
	Plus,
	RefreshCw,
	RotateCcw,
	Trash2,
} from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { InstallTargetContent } from "@/components/catalog/install-target-popover";
import { RemoveContentDialog } from "@/components/catalog/remove-content-dialog";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { usePendingAction } from "@/hooks/use-pending-action";
import type { ContentInstallState, ContentItem, Instance } from "@/lib/api";
import { SOURCE_NAMES } from "@/lib/catalog";
import { useInstanceContent } from "@/lib/content";
import {
	installButtonState,
	installProject,
	updateProjectItem,
	useInstallUi,
	type InstallButtonState,
	type ProjectRef,
} from "@/lib/content-install";
import { setDownloadsOpen, useDownloads } from "@/lib/downloads";
import { errorText } from "@/lib/errors";
import { useInstances } from "@/lib/instances";
import { openInstancePage } from "@/lib/instance-page";
import { cn } from "@/lib/utils";
import { useInstanceWorlds } from "@/lib/worlds";

export type InstallButtonSize = "sm" | "page";

type InstallButtonProps = {
	project: ProjectRef & { install?: ContentInstallState };
	/** Сборка, куда ставим; `null` — спросить («Куда установить?»). */
	instance: Instance | null;
	size?: InstallButtonSize;
	/** Кнопка конкретной версии (вкладка «Версии»). */
	versionId?: string;
	versionInstalled?: boolean;
	/** Сборку, выбранную в «Куда установить?», каталог берёт своей. */
	onPickInstance?: (id: string) => void;
	onIncompatibleClick?: () => void;
	/** В узкой карточке — только иконка, подпись в подсказке. */
	compact?: boolean;
	tone?: "primary";
	className?: string;
};

/** Ширина кнопки постоянная, смена состояния — crossfade, фокус переносится на новую кнопку. */
export function InstallButton({
	project,
	instance,
	size = "sm",
	versionId,
	versionInstalled,
	onPickInstance,
	onIncompatibleClick,
	compact,
	tone,
	className,
}: InstallButtonProps) {
	const instances = useInstances();
	const content = useInstanceContent(instance?.id ?? null);
	const worlds = useInstanceWorlds(project.type === "world" ? (instance?.id ?? null) : null);
	const downloads = useDownloads();
	const ui = useInstallUi();
	const state = installButtonState({
		project,
		instance,
		instances,
		content,
		worlds,
		downloads,
		ui,
		versionId,
		versionInstalled,
	});
	const containerRef = useRef<HTMLDivElement>(null);
	const stateKey = state.kind;

	useLayoutEffect(() => {
		const container = containerRef.current;
		if (!container?.contains(document.activeElement)) return;
		const next = container.querySelector<HTMLElement>(`[data-install-state="${stateKey}"] button`);
		if (next && !next.contains(document.activeElement)) next.focus({ preventScroll: true });
	}, [stateKey]);

	return (
		<div ref={containerRef} className={cn("relative z-10 grid shrink-0", className)}>
			<AnimatePresence mode="popLayout" initial={false}>
				<motion.div
					key={stateKey}
					data-install-state={stateKey}
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={{ duration: 0.12 }}
					className="flex justify-end"
				>
					<StateButton
						state={state}
						project={project}
						instance={instance}
						size={size}
						versionId={versionId}
						onPickInstance={onPickInstance}
						onIncompatibleClick={onIncompatibleClick}
						compact={compact}
						primary={tone === "primary"}
					/>
				</motion.div>
			</AnimatePresence>
		</div>
	);
}

function StateButton({
	state,
	project,
	instance,
	size,
	versionId,
	onPickInstance,
	onIncompatibleClick,
	compact,
	primary,
}: {
	state: InstallButtonState;
	project: ProjectRef;
	instance: Instance | null;
	size: InstallButtonSize;
	versionId?: string;
	onPickInstance?: (id: string) => void;
	onIncompatibleClick?: () => void;
	compact?: boolean;
	primary?: boolean;
}) {
	const { t } = useTranslation();
	const page = size === "page";
	const main = page || Boolean(primary);
	const plain = page ? "h-10 min-w-29 px-5" : "h-8 min-w-29 gap-1.5 px-3 text-[13px] font-semibold";
	const base = cn(
		plain,
		compact && "@max-[240px]/card:w-8 @max-[240px]/card:min-w-8 @max-[240px]/card:px-0",
	);
	const text = (label: string) =>
		compact ? <span className="@max-[240px]/card:sr-only">{label}</span> : label;
	const withCompactHint = (button: ReactNode, hint: string) =>
		compact ? (
			<Tooltip>
				<TooltipTrigger asChild>{button}</TooltipTrigger>
				<TooltipContent>{hint}</TooltipContent>
			</Tooltip>
		) : (
			button
		);
	const muted =
		"border-transparent bg-muted text-muted-foreground hover:bg-muted dark:bg-muted dark:hover:bg-muted";
	const name = instance?.name ?? "";
	const [open, setOpen] = useState(false);

	const install = (target: HTMLElement) => {
		if (!instance) return;
		void installProject({ instanceId: instance.id, project, versionId }, target);
	};

	const withHint = (button: ReactNode, hint: string) => (
		<Tooltip>
			<TooltipTrigger asChild>{button}</TooltipTrigger>
			<TooltipContent className="max-w-72">{hint}</TooltipContent>
		</Tooltip>
	);

	const withTargetPopover = (button: ReactNode) => (
		<Popover open={open} onOpenChange={setOpen}>
			{withCompactHint(<PopoverTrigger asChild>{button}</PopoverTrigger>, t("catalog.install.install"))}
			<PopoverContent align="end" sideOffset={6} fit className="w-100">
				<InstallTargetContent
					project={project}
					instanceId={instance?.id}
					onPick={(instanceId, world) => {
						setOpen(false);
						if (!instance) onPickInstance?.(instanceId);
						void installProject({ instanceId, project, versionId, world });
					}}
				/>
			</PopoverContent>
		</Popover>
	);

	switch (state.kind) {
		case "pick":
		case "install":
		case "siteOnly": {
			const siteOnly = Boolean(project.siteOnly);
			const source = SOURCE_NAMES[project.source];
			const needsPopover = state.kind === "pick" || project.type === "datapack";
			if (siteOnly) {
				// Сборки нет — сначала «Куда установить?», потом «Скачайте вручную».
				const hint = t("catalog.install.siteOnlyHint", { title: project.title, source });
				const button = (
					<Button
						variant={main ? "default" : "outline"}
						className={base}
						aria-label={page ? undefined : `${t("catalog.install.siteOnly")} · ${hint}`}
						onClick={needsPopover ? undefined : (e) => install(e.currentTarget)}
					>
						<ExternalLink />
						{page
							? t("catalog.install.siteOnlyPage", { source })
							: text(t("catalog.install.siteOnly"))}
					</Button>
				);
				return needsPopover ? withTargetPopover(button) : withHint(button, hint);
			}
			// Сборка видна в поле выбора рядом, поэтому в подписи её нет (имя — в `aria-label`).
			const label = text(t("catalog.install.install"));
			const button = (
				<Button
					variant={main ? "default" : "outline"}
					className={base}
					aria-label={instance ? t("catalog.install.installTo", { name }) : undefined}
					onClick={needsPopover ? undefined : (e) => install(e.currentTarget)}
				>
					<Download />
					{label}
				</Button>
			);
			return needsPopover
				? withTargetPopover(button)
				: withCompactHint(button, t("catalog.install.install"));
		}
		case "installing": {
			const percent = state.percent === null ? null : Math.round(state.percent);
			return withHint(
				<Button
					variant="secondary"
					aria-disabled
					aria-label={t("catalog.install.installing", { name })}
					className={cn(base, muted, "text-foreground")}
					onClick={() => setDownloadsOpen(true)}
				>
					<LoaderCircle className="animate-spin text-primary-text" />
					{percent !== null && <span className="tabular-nums">{percent}%</span>}
				</Button>,
				t("catalog.install.installing", { name }),
			);
		}
		case "installed":
			return (
				<InstalledMenu
					className={cn(base, muted)}
					label={text(versionId ? t("project.versions.installed") : t("catalog.install.installed"))}
					project={project}
					instance={instance}
					item={state.item}
				/>
			);
		case "update":
			return withCompactHint(
				<Button
					variant="secondary"
					className={cn(
						base,
						"border-transparent bg-primary-soft text-primary-text hover:bg-primary hover:text-primary-foreground",
					)}
					aria-label={
						state.item?.update
							? t("catalog.install.updateTo", { version: state.item.update.number })
							: undefined
					}
					onClick={(e) => {
						if (!instance) return;
						if (state.item) void updateProjectItem(instance.id, state.item, e.currentTarget);
						else install(e.currentTarget);
					}}
				>
					<RefreshCw />
					{text(t("catalog.install.update"))}
				</Button>,
				t("catalog.install.update"),
			);
		case "incompatible": {
			const label =
				state.target === null
					? t("catalog.install.needsLoader")
					: t("catalog.install.noVersionFor", { target: state.target });
			const hint =
				state.target === null
					? t("catalog.where.needsLoader")
					: t("catalog.install.noVersionHint", { target: state.target });
			return withHint(
				<Button
					variant="secondary"
					aria-disabled
					className={cn(plain, muted, "opacity-75")}
					onClick={onIncompatibleClick}
				>
					{label}
				</Button>,
				hint,
			);
		}
		case "error":
			return withHint(
				<Button
					variant="outline"
					className={cn(base, "border-destructive text-destructive hover:text-destructive")}
					onClick={state.retry}
				>
					<RotateCcw />
					{text(t("catalog.install.retry"))}
				</Button>,
				errorText(state.error),
			);
		case "waitInstance":
			return withHint(
				<Button variant="secondary" aria-disabled className={cn(base, muted, "opacity-75")}>
					<Download />
					{text(t("catalog.install.install"))}
				</Button>,
				t("catalog.install.waitInstance"),
			);
	}
}

function InstalledMenu({
	className,
	label,
	project,
	instance,
	item,
}: {
	className: string;
	label: ReactNode;
	project: ProjectRef;
	instance: Instance | null;
	item: ContentItem | null;
}) {
	const { t } = useTranslation();
	const pending = usePendingAction();
	const name = instance?.name ?? "";
	const [confirm, setConfirm] = useState(false);
	const triggerRef = useRef<HTMLButtonElement>(null);
	// Мир удаляют на вкладке «Миры»: в нём прогресс игрока.
	const world = project.type === "world";
	return (
		<>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<Button
						ref={triggerRef}
						variant="secondary"
						className={className}
						aria-label={t("catalog.install.menuLabel", { title: project.title })}
					>
						<Check />
						{label}
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end" onCloseAutoFocus={pending.onCloseAutoFocus}>
					{instance && world && (
						<>
							<DropdownMenuItem
								onSelect={() => pending.defer(() => openInstancePage(instance.id, "worlds"))}
							>
								<Globe />
								{t("catalog.install.openWorlds")}
							</DropdownMenuItem>
							<DropdownMenuItem
								onSelect={() =>
									pending.defer(
										() =>
											void installProject(
												{ instanceId: instance.id, project },
												triggerRef.current,
											),
									)
								}
							>
								<Plus />
								{t("catalog.install.installAgain")}
							</DropdownMenuItem>
						</>
					)}
					{instance && !world && (
						<DropdownMenuItem
							onSelect={() => pending.defer(() => openInstancePage(instance.id, "content"))}
						>
							<ArrowUpRight />
							{t("catalog.install.openInstance")}
						</DropdownMenuItem>
					)}
					{instance && item && !world && (
						<DropdownMenuItem
							variant="destructive"
							onSelect={() => pending.defer(() => setConfirm(true))}
						>
							<Trash2 />
							{t("catalog.install.remove", { name })}
						</DropdownMenuItem>
					)}
				</DropdownMenuContent>
			</DropdownMenu>
			{instance && item && (
				<RemoveContentDialog
					open={confirm}
					onOpenChange={setConfirm}
					title={project.title}
					instanceId={instance.id}
					instanceName={name}
					items={[item]}
					returnFocus={triggerRef}
				/>
			)}
		</>
	);
}
