import { Ellipsis } from "lucide-react";
import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { linkFocusRing, onLinkKeyDown, stretchedLink } from "@/components/instances/instance-link";
import { InstanceThumb } from "@/components/instances/instance-cover";
import { InstanceMenuContent } from "@/components/instances/instance-menu";
import { InstanceMeta } from "@/components/instances/instance-meta";
import { VulnBadge } from "@/components/instances/vuln-badge";
import { PlayButton } from "@/components/instances/play-button";
import { Button } from "@/components/ui/button";
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu";
import { DropdownMenu, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { Instance } from "@/lib/api";
import { coverOf } from "@/lib/covers";
import { statusOf, useInstances } from "@/lib/instances";
import { versionLabel } from "@/lib/loaders";
import { instancePath, navigate } from "@/lib/router";
import { cn } from "@/lib/utils";
import { ariaDeleteKey } from "@/lib/platform";

export function InstanceRow({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const store = useInstances();
	const status = statusOf(instance, store);
	const titleRef = useRef<HTMLButtonElement>(null);
	const moreRef = useRef<HTMLButtonElement>(null);
	const actionsLabel = t("instances.card.actions", { name: instance.name });

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div
					className={cn(
						"group relative flex h-16 items-center gap-4 rounded-lg border border-transparent px-3 hover:bg-accent",
						linkFocusRing,
					)}
				>
					<InstanceThumb scene={coverOf(instance)} size={48} />
					<div className="min-w-0 flex-1">
						<button
							ref={titleRef}
							type="button"
							data-link
							aria-keyshortcuts={ariaDeleteKey}
							className={cn("block max-w-full truncate text-sm/5 font-semibold", stretchedLink)}
							onClick={() => navigate(instancePath(instance.id))}
							onKeyDown={onLinkKeyDown(instance, status)}
						>
							{instance.name}
						</button>
						<p className="flex min-w-0 items-center gap-1 text-[13px]/[18px] text-muted-foreground">
							<span className="truncate">{versionLabel(instance)}</span>
							<VulnBadge instance={instance} />
						</p>
					</div>
					<InstanceMeta instance={instance} variant="row" className="w-52 shrink-0" />
					<PlayButton instance={instance} size="row" />
					<DropdownMenu>
						<Tooltip>
							<TooltipTrigger asChild>
								<DropdownMenuTrigger asChild>
									<Button
										ref={moreRef}
										variant="ghost"
										size="icon-sm"
										aria-label={actionsLabel}
										className="relative z-10 text-muted-foreground group-hover:text-foreground aria-expanded:bg-accent aria-expanded:text-foreground"
									>
										<Ellipsis />
									</Button>
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
			</ContextMenuTrigger>
			<InstanceMenuContent instance={instance} kind="context" focusTarget={() => titleRef.current} />
		</ContextMenu>
	);
}
