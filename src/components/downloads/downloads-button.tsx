import { Download } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useTranslation } from "react-i18next";
import { DownloadsPanel } from "@/components/downloads/downloads-panel";
import { Popover, PopoverTrigger } from "@/components/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
	buttonState,
	overallPercent,
	setDownloadsOpen,
	useDownloads,
	useDownloadsShortcut,
} from "@/lib/downloads";
import { cn } from "@/lib/utils";
import { ariaShortcut, shortcutLabel } from "@/lib/platform";

export function DownloadsButton() {
	const { t } = useTranslation();
	const store = useDownloads();
	const state = buttonState(store);
	const active = state === "active";
	const percent = active ? overallPercent(store.items) : null;
	const rounded = percent === null ? null : Math.round(percent);
	const busyCount = store.items.filter((d) => d.state === "running" || d.state === "queued").length;
	useDownloadsShortcut();

	let label: string;
	if (active) {
		label =
			rounded === null
				? t("downloads.buttonBusy")
				: busyCount > 1
					? t("downloads.buttonActiveMany", { count: busyCount, percent: rounded })
					: t("downloads.buttonActive", { percent: rounded });
	} else if (state === "failed") label = t("downloads.buttonFailed");
	else if (state === "new") label = t("downloads.buttonNew");
	else label = t("downloads.button");

	return (
		<Popover open={store.open} onOpenChange={setDownloadsOpen}>
			<Tooltip>
				<TooltipTrigger asChild>
					<PopoverTrigger asChild>
						<button
							type="button"
							aria-label={label}
							aria-keyshortcuts={ariaShortcut("J")}
							className={cn(
								"group/dl relative flex size-9 items-center justify-center rounded-full transition-[color,background-color,scale] duration-(--duration-fast) ease-out hover:bg-sidebar-accent hover:text-foreground active:scale-[0.98] aria-expanded:bg-sidebar-accent",
								active
									? "text-primary-text hover:text-primary-text"
									: "text-muted-foreground aria-expanded:text-foreground",
							)}
						>
							<Download />
							{active && (
								<span
									aria-hidden
									className="absolute inset-x-2.5 bottom-1 h-0.5 overflow-clip rounded-full bg-border"
								>
									{rounded === null ? (
										<span className="progress-indeterminate absolute inset-y-0 left-0 w-[30%] bg-primary" />
									) : (
										<span
											className="absolute inset-0 origin-left bg-primary transition-transform duration-200 ease-linear"
											style={{ transform: `scaleX(${rounded / 100})` }}
										/>
									)}
								</span>
							)}
							<AnimatePresence initial={false}>
								{(state === "failed" || state === "new") && (
									<motion.span
										key={state}
										aria-hidden
										initial={{ opacity: 0 }}
										animate={{ opacity: 1 }}
										exit={{ opacity: 0 }}
										transition={{ duration: 0.2 }}
										className="absolute top-[3px] right-[3px] flex size-3 items-center justify-center rounded-full bg-sidebar group-hover/dl:bg-sidebar-accent group-aria-expanded/dl:bg-sidebar-accent"
									>
										<span
											className={cn(
												"size-2 rounded-full",
												state === "failed" ? "bg-destructive" : "bg-primary",
											)}
										/>
									</motion.span>
								)}
							</AnimatePresence>
						</button>
					</PopoverTrigger>
				</TooltipTrigger>
				<TooltipContent side="bottom" sideOffset={6}>
					{label}
					<span className="ml-2 opacity-70">{shortcutLabel("J")}</span>
				</TooltipContent>
			</Tooltip>
			<DownloadsPanel />
		</Popover>
	);
}
