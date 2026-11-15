import { ArrowUpRight, Check, Download, LoaderCircle, RotateCcw } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { usePendingAction } from "@/hooks/use-pending-action";
import { setDownloadsOpen, useDownloads } from "@/lib/downloads";
import { errorText } from "@/lib/errors";
import { openInstancePage } from "@/lib/instance-page";
import { useInstances } from "@/lib/instances";
import {
	modpackButtonState,
	openInstallModpack,
	type ModpackButtonState,
	type ModpackRef,
} from "@/lib/modpacks";
import { cn } from "@/lib/utils";

/** В меню «Установлен» — не больше стольких сборок. */
const MENU_INSTANCES = 5;

/** С `versionId` кнопка всегда «Установить» эту версию. */
export function ModpackInstallButton({
	project,
	size = "sm",
	versionId,
	tone,
	className,
}: {
	project: ModpackRef;
	size?: "sm" | "page";
	tone?: "primary";
	versionId?: string;
	className?: string;
}) {
	const instances = useInstances();
	const downloads = useDownloads();
	const state: ModpackButtonState = versionId
		? { kind: "install" }
		: modpackButtonState(project, instances, downloads);
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
						size={size}
						versionId={versionId}
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
	size,
	versionId,
	primary,
}: {
	state: ModpackButtonState;
	project: ModpackRef;
	size: "sm" | "page";
	versionId?: string;
	primary?: boolean;
}) {
	const { t } = useTranslation();
	const page = size === "page";
	const base = page ? "h-10 min-w-29 px-5" : "h-8 min-w-29 gap-1.5 px-3 text-[13px] font-semibold";
	const muted =
		"border-transparent bg-muted text-muted-foreground hover:bg-muted dark:bg-muted dark:hover:bg-muted";
	const withHint = (button: ReactNode, hint: string) => (
		<Tooltip>
			<TooltipTrigger asChild>{button}</TooltipTrigger>
			<TooltipContent className="max-w-72">{hint}</TooltipContent>
		</Tooltip>
	);
	const install = (target: HTMLElement) =>
		openInstallModpack({ kind: "catalog", project, versionId }, target);

	switch (state.kind) {
		case "install":
			return (
				<Button
					variant={page || primary ? "default" : "outline"}
					className={base}
					onClick={(e) => install(e.currentTarget)}
				>
					<Download />
					{t("modpack.install")}
				</Button>
			);
		case "installing": {
			const percent = state.percent === null ? null : Math.round(state.percent);
			return withHint(
				<Button
					variant="secondary"
					aria-disabled
					aria-label={t("modpack.installing", { name: state.name })}
					className={cn(base, muted, "text-foreground")}
					onClick={() => setDownloadsOpen(true)}
				>
					<LoaderCircle className="animate-spin text-primary-text" />
					{percent !== null && <span className="tabular-nums">{percent}%</span>}
				</Button>,
				t("modpack.installing", { name: state.name }),
			);
		}
		case "installed":
			return <InstalledMenu className={cn(base, muted)} project={project} state={state} />;
		case "error":
			return withHint(
				<Button
					variant="outline"
					className={cn(base, "border-destructive text-destructive hover:text-destructive")}
					onClick={state.retry}
				>
					<RotateCcw />
					{t("catalog.install.retry")}
				</Button>,
				errorText(state.error),
			);
	}
}

function InstalledMenu({
	className,
	project,
	state,
}: {
	className: string;
	project: ModpackRef;
	state: Extract<ModpackButtonState, { kind: "installed" }>;
}) {
	const { t } = useTranslation();
	const pending = usePendingAction();
	const triggerRef = useRef<HTMLButtonElement>(null);
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					ref={triggerRef}
					variant="secondary"
					className={className}
					aria-label={t("modpack.menuLabel", { title: project.title })}
				>
					<Check />
					{t("modpack.installed")}
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="max-w-72" onCloseAutoFocus={pending.onCloseAutoFocus}>
				{state.instances.slice(0, MENU_INSTANCES).map((instance) => (
					<DropdownMenuItem
						key={instance.id}
						onSelect={() => pending.defer(() => openInstancePage(instance.id, "content"))}
					>
						<ArrowUpRight />
						<span className="truncate">{t("modpack.openInstance", { name: instance.name })}</span>
					</DropdownMenuItem>
				))}
				<DropdownMenuSeparator />
				<DropdownMenuItem
					onSelect={() =>
						pending.defer(() =>
							openInstallModpack({ kind: "catalog", project }, triggerRef.current),
						)
					}
				>
					<Download />
					{t("modpack.installAgain")}
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
