import { Ellipsis } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { coverFrame, InstanceCover } from "@/components/instances/instance-cover";
import { coverFocusRing, onLinkKeyDown, stretchedLink } from "@/components/instances/instance-link";
import { InstanceMenuContent } from "@/components/instances/instance-menu";
import { InstanceMeta } from "@/components/instances/instance-meta";
import { useSessionSeconds } from "@/components/instances/instance-text";
import { onCoverButton, PlayButton } from "@/components/instances/play-button";
import { ProgressBar } from "@/components/instances/progress-bar";
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu";
import { DropdownMenu, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { Instance } from "@/lib/api";
import { coverOf } from "@/lib/covers";
import { formatDuration } from "@/lib/format";
import { progressValue, statusOf, useInstances } from "@/lib/instances";
import { instancePath, navigate } from "@/lib/router";
import { cn } from "@/lib/utils";

export function InstanceCard({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const store = useInstances();
	const status = statusOf(instance, store);
	const session = useSessionSeconds(instance, store);
	const titleRef = useRef<HTMLButtonElement>(null);
	const moreRef = useRef<HTMLButtonElement>(null);
	const actionsLabel = t("instances.card.actions", { name: instance.name });
	const installing = status === "queued" || status === "installing";

	// Порядок в DOM — порядок Tab, обложку поднимает `-order-1`. Она выше `::after` названия, но пропускает щелчки к ссылке.
	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div className="group relative flex flex-col">
					<div className="pt-3">
						<button
							ref={titleRef}
							type="button"
							data-link
							aria-keyshortcuts="Delete"
							className={cn("block max-w-full truncate text-sm/5 font-semibold", stretchedLink)}
							onClick={() => navigate(instancePath(instance.id))}
							onKeyDown={onLinkKeyDown(instance, status)}
						>
							{instance.name}
						</button>
						<InstanceMeta instance={instance} variant="card" />
					</div>
					<div
						className={cn(
							"pointer-events-none relative z-[2] -order-1 h-40",
							coverFrame,
							coverFocusRing,
						)}
					>
						<InstanceCover scene={coverOf(instance)} parallax className="absolute inset-0" />

						<AnimatePresence initial={false}>
							{status === "running" && (
								<motion.span
									key="playing"
									initial={{ opacity: 0 }}
									animate={{ opacity: 1 }}
									exit={{ opacity: 0 }}
									transition={{ duration: 0.12 }}
									className="absolute top-3 left-3 flex h-6 max-w-[calc(100%-4rem)] items-center gap-1.5 rounded-full bg-cover-scrim/55 px-2.5 text-xs font-medium text-on-cover backdrop-blur-md"
								>
									<span aria-hidden className="size-2 shrink-0 rounded-full bg-primary" />
									<span className="truncate">
										{t("instances.card.playing", {
											duration: formatDuration(session ?? 0),
										})}
									</span>
								</motion.span>
							)}
						</AnimatePresence>

						{installing && (
							<ProgressBar
								variant="edge"
								value={progressValue(instance, store)}
								ariaLabel={t("install.progressLabel", { name: instance.name })}
								className="absolute inset-x-0 bottom-0"
							/>
						)}

						<PlayButton
							instance={instance}
							size="cover"
							className="pointer-events-auto absolute right-2.5 bottom-2.5"
						/>

						<DropdownMenu>
							<Tooltip>
								<TooltipTrigger asChild>
									<DropdownMenuTrigger asChild>
										<button
											ref={moreRef}
											type="button"
											aria-label={actionsLabel}
											className={cn(
												onCoverButton,
												"pointer-events-auto absolute top-2.5 right-2.5 z-10 size-8 bg-cover-scrim/45 hover:bg-cover-scrim/60 aria-expanded:bg-cover-scrim/60",
											)}
										>
											<Ellipsis />
										</button>
									</DropdownMenuTrigger>
								</TooltipTrigger>
								<TooltipContent>{actionsLabel}</TooltipContent>
							</Tooltip>
							<InstanceMenuContent
								instance={instance}
								kind="dropdown"
								focusTarget={() => moreRef.current}
							/>
						</DropdownMenu>
					</div>
				</div>
			</ContextMenuTrigger>
			<InstanceMenuContent instance={instance} kind="context" focusTarget={() => titleRef.current} />
		</ContextMenu>
	);
}

export function InstanceCardSkeleton() {
	return (
		<div>
			<Skeleton className="h-40 rounded-xl" />
			<Skeleton className="mt-3 h-3.5 w-32" />
			<Skeleton className="mt-2 h-3 w-24" />
		</div>
	);
}
