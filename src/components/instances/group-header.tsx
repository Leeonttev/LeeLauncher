import { ChevronDown, ChevronRight, Ellipsis, Pencil, Plus, Trash2 } from "lucide-react";
import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { usePendingAction } from "@/hooks/use-pending-action";
import { groupKey, openGroupDialog, useGroupsUi } from "@/lib/groups";
import { openCreateInstanceDialog } from "@/lib/instances";
import { cn } from "@/lib/utils";

type GroupHeaderProps = {
	name: string | null;
	shown: number;
	total: number;
	filtered: boolean;
	collapsed: boolean;
	/** Alt+щелчок или Alt+Enter — то же со всеми группами. */
	onToggle: (all: boolean) => void;
	contentId: string;
	className?: string;
};

export function GroupHeader({
	name,
	shown,
	total,
	filtered,
	collapsed,
	onToggle,
	contentId,
	className,
}: GroupHeaderProps) {
	const { t } = useTranslation();
	const { flash } = useGroupsUi();
	const toggleRef = useRef<HTMLButtonElement>(null);
	const moreRef = useRef<HTMLButtonElement>(null);
	const title = name ?? t("groups.none");
	const flashing = flash !== null && flash === groupKey(name);

	const row = (
		<div className={cn("flex items-center gap-3", className)}>
			<h2 className="-ml-2 min-w-0">
				<button
					ref={toggleRef}
					type="button"
					aria-expanded={!collapsed}
					aria-controls={contentId}
					aria-label={t("groups.toggle", { name: title })}
					className={cn(
						"relative flex h-9 max-w-full items-center gap-2 rounded-lg border border-transparent pr-2.5 pl-2 transition-[background-color] duration-(--duration-base) ease-out hover:bg-accent",
						flashing && "bg-primary-soft hover:bg-primary-soft",
					)}
					onClick={(e) => onToggle(e.altKey)}
					onKeyDown={(e) => {
						if (e.key === "Enter" && e.altKey) {
							e.preventDefault();
							onToggle(true);
						}
					}}
				>
					{collapsed ? (
						<ChevronRight className="text-muted-foreground" />
					) : (
						<ChevronDown className="text-muted-foreground" />
					)}
					<span className="max-w-[480px] min-w-0 truncate text-lg/6 font-semibold">{title}</span>
					<span className="flex h-[22px] shrink-0 items-center rounded-md bg-muted px-[7px] text-[13px] font-semibold text-muted-foreground tabular-nums">
						{filtered
							? t("groups.countFiltered", { shown, total })
							: t("groups.count", { count: total })}
					</span>
				</button>
			</h2>
			<span aria-hidden className="h-px min-w-4 flex-1 bg-border" />
			{name ? (
				<DropdownMenu>
					<Tooltip>
						<TooltipTrigger asChild>
							<DropdownMenuTrigger asChild>
								<Button
									ref={moreRef}
									variant="ghost"
									size="icon-sm"
									aria-label={t("groups.menu")}
									className="text-muted-foreground hover:text-foreground aria-expanded:bg-accent aria-expanded:text-foreground"
								>
									<Ellipsis />
								</Button>
							</DropdownMenuTrigger>
						</TooltipTrigger>
						<TooltipContent>{t("groups.menu")}</TooltipContent>
					</Tooltip>
					<GroupMenuContent name={name} kind="dropdown" focusTarget={() => moreRef.current} />
				</DropdownMenu>
			) : (
				<span aria-hidden className="w-8 shrink-0" />
			)}
		</div>
	);

	if (!name) return row;
	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>{row}</ContextMenuTrigger>
			<GroupMenuContent name={name} kind="context" focusTarget={() => toggleRef.current} />
		</ContextMenu>
	);
}

function GroupMenuContent({
	name,
	kind,
	focusTarget,
}: {
	name: string;
	kind: "dropdown" | "context";
	focusTarget: () => HTMLElement | null;
}) {
	const { t } = useTranslation();
	const { defer, onCloseAutoFocus } = usePendingAction();
	const Item = kind === "dropdown" ? DropdownMenuItem : ContextMenuItem;
	const Separator = kind === "dropdown" ? DropdownMenuSeparator : ContextMenuSeparator;
	const items = (
		<>
			<Item
				className="h-9 gap-2.5 px-2.5"
				onSelect={() => defer(() => openCreateInstanceDialog(focusTarget(), { group: name }))}
			>
				<Plus />
				<span className="flex-1 truncate">{t("groups.createIn")}</span>
			</Item>
			<Item
				className="h-9 gap-2.5 px-2.5"
				onSelect={() =>
					defer(() => openGroupDialog("rename", { group: name, returnFocus: focusTarget() }))
				}
			>
				<Pencil />
				<span className="flex-1 truncate">{t("groups.rename")}</span>
			</Item>
			<Separator />
			<Item
				variant="destructive"
				className="h-9 gap-2.5 px-2.5"
				onSelect={() =>
					defer(() => openGroupDialog("delete", { group: name, returnFocus: focusTarget() }))
				}
			>
				<Trash2 />
				<span className="flex-1 truncate">{t("groups.delete")}</span>
			</Item>
		</>
	);
	return kind === "dropdown" ? (
		<DropdownMenuContent align="end" className="w-60" onCloseAutoFocus={onCloseAutoFocus}>
			{items}
		</DropdownMenuContent>
	) : (
		<ContextMenuContent className="w-60" onCloseAutoFocus={onCloseAutoFocus}>
			{items}
		</ContextMenuContent>
	);
}
