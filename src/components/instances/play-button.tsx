import { Download, LoaderCircle, Play, RotateCcw, Square, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useLayoutEffect, useRef, type MouseEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useShownGameAccount } from "@/components/instances/instance-text";
import { ProgressBar } from "@/components/instances/progress-bar";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { activeAccount, useAccounts } from "@/lib/accounts";
import type { Instance } from "@/lib/api";
import {
	cancelInstall,
	installInstance,
	launchInstance,
	openInstanceDialog,
	progressValue,
	statusOf,
	useInstances,
} from "@/lib/instances";
import { cn } from "@/lib/utils";

export type PlayButtonSize = "row" | "cover" | "banner" | "page";

const soft =
	"bg-primary-soft text-primary-text transition-[color,background-color] duration-(--duration-base) hover:bg-primary hover:text-primary-foreground";
const stopHover = "hover:border-destructive/40 hover:bg-destructive/10 hover:text-destructive";

/** Своя таблетка, не вариант `Button`: тёмная подложка с размытием читается и на снегу, и на песке. */
export const onCoverButton =
	"relative inline-flex shrink-0 items-center justify-center gap-1.5 rounded-full bg-cover-scrim/40 text-sm font-medium whitespace-nowrap text-on-cover backdrop-blur-md transition-[color,background-color,scale] duration-(--duration-fast) ease-out hover:bg-cover-scrim/55 active:scale-[0.98] [&_svg]:pointer-events-none [&_svg]:shrink-0";
const onCoverPlay = "bg-on-cover text-cover-scrim backdrop-blur-none hover:bg-on-cover/90";
const onCoverStop = "hover:bg-destructive/80 hover:text-on-cover";

export function PlayButton({
	instance,
	size,
	className,
}: {
	instance: Instance;
	size: PlayButtonSize;
	className?: string;
}) {
	const { t } = useTranslation();
	const store = useInstances();
	const accounts = useAccounts();
	const status = statusOf(instance, store);
	const noAccount = accounts.status !== "idle" && accounts.status !== "loading" && !activeAccount(accounts);
	const stateKey = status === "queued" ? "installing" : status;
	const containerRef = useRef<HTMLDivElement>(null);
	const cancelling = Boolean(store.cancelling[instance.id]);
	const name = instance.name;
	const gameAccount = useShownGameAccount(instance.id, store);
	const stopLabel = gameAccount
		? t("instances.card.aria.stopAs", { name, nick: gameAccount.username })
		: t("instances.card.aria.stop", { name });

	// Кнопка пересоздаётся при смене состояния: переносим фокус на новую, иначе он уйдёт на body.
	useLayoutEffect(() => {
		const container = containerRef.current;
		if (!container?.contains(document.activeElement)) return;
		const next = container.querySelector<HTMLElement>(
			`[data-play-state="${stateKey}"] button:not([disabled])`,
		);
		if (next && !next.contains(document.activeElement)) next.focus({ preventScroll: true });
	}, [stateKey]);

	const launch = (e: MouseEvent<HTMLButtonElement>) => void launchInstance(instance.id, e.currentTarget);
	const install = () => void installInstance(instance.id);
	const cancel = () => {
		if (!cancelling) void cancelInstall(instance.id);
	};
	const stop = (e: MouseEvent<HTMLButtonElement>) =>
		openInstanceDialog("stop", instance.id, { returnFocus: e.currentTarget });

	const withAccountHint = (button: ReactNode) =>
		noAccount ? (
			<Tooltip>
				<TooltipTrigger asChild>{button}</TooltipTrigger>
				<TooltipContent>{t("instance.needAccount")}</TooltipContent>
			</Tooltip>
		) : (
			button
		);

	const content = size === "cover" ? coverContent() : size === "banner" ? bannerContent() : plainContent();
	if (content === null) return null;

	function coverContent(): ReactNode {
		const base = cn(onCoverButton, "h-9 px-4");
		switch (status) {
			case "installed":
				return withAccountHint(
					<button
						type="button"
						aria-label={t("instances.card.aria.play", { name })}
						className={cn(base, onCoverPlay)}
						onClick={launch}
					>
						<Play fill="currentColor" />
						{t("instance.play")}
					</button>,
				);
			case "not_installed":
				return (
					<button
						type="button"
						aria-label={t("instances.card.aria.install", { name })}
						className={base}
						onClick={install}
					>
						<Download />
						{t("instance.install")}
					</button>
				);
			case "install_failed":
				return (
					<button
						type="button"
						aria-label={t("instances.card.aria.retry", { name })}
						className={base}
						onClick={install}
					>
						<RotateCcw />
						{t("common.retry")}
					</button>
				);
			case "queued":
			case "installing":
				return (
					<button
						type="button"
						aria-label={t("instances.card.aria.cancel", { name })}
						aria-disabled={cancelling || undefined}
						className={cn(base, "aria-disabled:pointer-events-none aria-disabled:opacity-60")}
						onClick={cancel}
					>
						{cancelling ? <LoaderCircle className="animate-spin" /> : <X />}
						{t("instance.cancel")}
					</button>
				);
			case "launching":
				return (
					<button type="button" aria-disabled className={cn(base, "pointer-events-none")}>
						<LoaderCircle className="animate-spin" />
						{t("instance.launching")}
					</button>
				);
			case "running":
				return (
					<button
						type="button"
						aria-label={stopLabel}
						className={cn(base, onCoverStop)}
						onClick={stop}
					>
						<Square />
						{t("instance.stopShort")}
					</button>
				);
		}
	}

	function bannerContent(): ReactNode {
		const big = "relative z-10";
		switch (status) {
			case "installed":
				return withAccountHint(
					<Button size="xl" className={big} onClick={launch}>
						<Play fill="currentColor" />
						{t("instance.play")}
					</Button>,
				);
			case "not_installed":
				return (
					<Button size="xl" className={big} onClick={install}>
						<Download />
						{t("instance.install")}
					</Button>
				);
			case "install_failed":
				return (
					<Button size="xl" className={big} onClick={install}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				);
			case "queued":
			case "installing":
				return (
					<div className="relative z-10 flex items-center gap-1 rounded-full bg-cover-scrim/40 p-1 backdrop-blur-md">
						<ProgressBar
							variant="compact"
							tone="cover"
							value={progressValue(instance, store)}
							label={status === "queued" ? t("install.stage.queued") : undefined}
							ariaLabel={t("install.progressLabel", { name })}
							className="h-9 w-48"
						/>
						<Tooltip>
							<TooltipTrigger asChild>
								<button
									type="button"
									aria-label={t("instance.cancelInstall")}
									aria-disabled={cancelling || undefined}
									className={cn(
										onCoverButton,
										"size-9 bg-transparent hover:bg-on-cover/16 aria-disabled:pointer-events-none aria-disabled:opacity-60",
									)}
									onClick={cancel}
								>
									{cancelling ? <LoaderCircle className="animate-spin" /> : <X />}
								</button>
							</TooltipTrigger>
							<TooltipContent>{t("instance.cancelInstall")}</TooltipContent>
						</Tooltip>
					</div>
				);
			case "launching":
				return (
					<button
						type="button"
						aria-disabled
						className={cn(onCoverButton, "pointer-events-none z-10 h-11 px-6 text-[15px]")}
					>
						<LoaderCircle className="animate-spin" />
						{t("instance.launching")}
					</button>
				);
			case "running":
				return (
					<button
						type="button"
						aria-label={gameAccount ? stopLabel : undefined}
						className={cn(onCoverButton, onCoverStop, "z-10 h-11 px-6 text-[15px]")}
						onClick={stop}
					>
						<Square />
						{t("instance.stop")}
					</button>
				);
		}
	}

	function plainContent(): ReactNode {
		const row = size === "row";
		const base = row ? "h-8 gap-1.5 px-3" : "h-10 px-5";
		const main = row ? cn(base, soft) : base;
		const variant = row ? "secondary" : "default";
		switch (status) {
			case "installed":
				return withAccountHint(
					<Button
						className={main}
						variant={variant}
						aria-label={row ? t("instances.card.aria.play", { name }) : undefined}
						onClick={launch}
					>
						<Play fill="currentColor" />
						{t("instance.play")}
					</Button>,
				);
			case "not_installed":
				return (
					<Button
						className={main}
						variant={variant}
						aria-label={row ? t("instances.card.aria.install", { name }) : undefined}
						onClick={install}
					>
						<Download />
						{t("instance.install")}
					</Button>
				);
			case "install_failed":
				return (
					<Button
						className={main}
						variant={variant}
						aria-label={row ? t("instances.card.aria.retry", { name }) : undefined}
						onClick={install}
					>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				);
			case "queued":
			case "installing":
				if (!row) return null;
				return (
					<div className="flex items-center gap-1">
						<ProgressBar
							variant="compact"
							value={progressValue(instance, store)}
							label={status === "queued" ? t("install.stage.queued") : undefined}
							ariaLabel={t("install.progressLabel", { name })}
							className="w-[120px]"
						/>
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon-sm"
									aria-label={t("instances.card.aria.cancel", { name })}
									aria-disabled={cancelling || undefined}
									className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
									onClick={cancel}
								>
									{cancelling ? <LoaderCircle className="animate-spin" /> : <X />}
								</Button>
							</TooltipTrigger>
							<TooltipContent>{t("instance.cancelInstall")}</TooltipContent>
						</Tooltip>
					</div>
				);
			case "launching":
				return (
					<Button
						className={cn(base, "pointer-events-none opacity-50")}
						variant="outline"
						aria-disabled
					>
						<LoaderCircle className="animate-spin" />
						{t("instance.launching")}
					</Button>
				);
			case "running":
				return (
					<Button
						className={cn(base, stopHover)}
						variant="outline"
						aria-label={row || gameAccount ? stopLabel : t("instance.stop")}
						onClick={stop}
					>
						<Square />
						{row ? t("instance.stopShort") : t("instance.stop")}
					</Button>
				);
		}
	}

	return (
		<div ref={containerRef} className={cn("relative z-10 shrink-0", className)}>
			<AnimatePresence mode="popLayout" initial={false}>
				<motion.div
					key={stateKey}
					data-play-state={stateKey}
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={{ duration: 0.12 }}
				>
					{content}
				</motion.div>
			</AnimatePresence>
		</div>
	);
}
