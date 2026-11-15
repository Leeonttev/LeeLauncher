import {
	ArrowUpRight,
	Copy,
	Folder,
	FolderOpen,
	FolderPlus,
	Minus,
	Pencil,
	Play,
	RefreshCcwDot,
	Square,
	Trash2,
	type LucideIcon,
} from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuRadioGroup,
	ContextMenuRadioItem,
	ContextMenuSeparator,
	ContextMenuSub,
	ContextMenuSubContent,
	ContextMenuSubTrigger,
} from "@/components/ui/context-menu";
import {
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuSeparator,
	DropdownMenuSub,
	DropdownMenuSubContent,
	DropdownMenuSubTrigger,
} from "@/components/ui/dropdown-menu";
import { usePendingAction } from "@/hooks/use-pending-action";
import type { Instance } from "@/lib/api";
import { toAppError } from "@/lib/errors";
import { groupErrorText, groupKey, listGroups, openGroupDialog } from "@/lib/groups";
import {
	duplicateInstance,
	isBusy,
	launchInstance,
	moveInstanceToGroup,
	openInstanceDialog,
	openInstanceFolder,
	statusOf,
	useInstances,
} from "@/lib/instances";
import { instancePath, navigate } from "@/lib/router";
import { cn } from "@/lib/utils";

type MenuKind = "dropdown" | "context";

type ItemProps = {
	className?: string;
	disabled?: boolean;
	variant?: "default" | "destructive";
	onSelect?: () => void;
	children: ReactNode;
};

type Parts = {
	Item: ComponentType<ItemProps>;
	Separator: ComponentType<{ className?: string }>;
	Sub: ComponentType<{ children: ReactNode }>;
	SubTrigger: ComponentType<{ className?: string; children: ReactNode }>;
	SubContent: ComponentType<{ className?: string; children: ReactNode }>;
	RadioGroup: ComponentType<{ value?: string; children: ReactNode }>;
	RadioItem: ComponentType<{
		value: string;
		className?: string;
		onSelect?: () => void;
		children: ReactNode;
	}>;
};

const ITEMS: Record<MenuKind, Parts> = {
	dropdown: {
		Item: DropdownMenuItem,
		Separator: DropdownMenuSeparator,
		Sub: DropdownMenuSub,
		SubTrigger: DropdownMenuSubTrigger,
		SubContent: DropdownMenuSubContent,
		RadioGroup: DropdownMenuRadioGroup,
		RadioItem: DropdownMenuRadioItem,
	},
	context: {
		Item: ContextMenuItem,
		Separator: ContextMenuSeparator,
		Sub: ContextMenuSub,
		SubTrigger: ContextMenuSubTrigger,
		SubContent: ContextMenuSubContent,
		RadioGroup: ContextMenuRadioGroup,
		RadioItem: ContextMenuRadioItem,
	},
};

type InstanceMenuProps = {
	instance: Instance;
	kind: MenuKind;
	onPage?: boolean;
	focusTarget: () => HTMLElement | null;
};

export function InstanceMenuContent({ instance, kind, onPage = false, focusTarget }: InstanceMenuProps) {
	const { t } = useTranslation();
	const store = useInstances();
	const status = statusOf(instance, store);
	const busy = isBusy(status);
	const { defer, onCloseAutoFocus } = usePendingAction();
	const parts = ITEMS[kind];
	const { Item, Separator } = parts;
	const busyNote = busy
		? status === "running" || status === "launching"
			? t("instance.menu.busyRunning")
			: t("instance.menu.busyInstalling")
		: null;

	const dialog = (name: "rename" | "reinstall" | "remove" | "stop") =>
		defer(() =>
			openInstanceDialog(name, instance.id, { returnFocus: focusTarget(), leaveAfterRemove: onPage }),
		);

	const items = (
		<>
			{!onPage && (
				<>
					{status === "installed" && (
						<MenuRow
							Item={Item}
							icon={Play}
							label={t("instance.play")}
							onSelect={() => defer(() => void launchInstance(instance.id, focusTarget()))}
						/>
					)}
					{status === "running" && (
						<MenuRow
							Item={Item}
							icon={Square}
							label={t("instance.stop")}
							onSelect={() => dialog("stop")}
						/>
					)}
					<MenuRow
						Item={Item}
						icon={ArrowUpRight}
						label={t("instance.menu.open")}
						onSelect={() => navigate(instancePath(instance.id))}
					/>
					<Separator className="-mx-1.5 my-1" />
				</>
			)}
			<MenuRow
				Item={Item}
				icon={FolderOpen}
				label={t("common.openFolder")}
				onSelect={() => void openInstanceFolder(instance.id)}
			/>
			{!onPage && (
				<MenuRow
					Item={Item}
					icon={Pencil}
					label={t("instance.menu.rename")}
					onSelect={() => dialog("rename")}
				/>
			)}
			<MenuRow
				Item={Item}
				icon={Copy}
				label={t("instance.menu.duplicate")}
				note={busyNote}
				disabled={busy}
				onSelect={() => void duplicateInstance(instance.id)}
			/>
			<MoveToGroup
				instance={instance}
				parts={parts}
				openNewGroup={() =>
					defer(() =>
						openGroupDialog("new", { instanceId: instance.id, returnFocus: focusTarget() }),
					)
				}
			/>
			<MenuRow
				Item={Item}
				icon={RefreshCcwDot}
				label={t("instance.menu.reinstall")}
				note={busyNote}
				disabled={busy}
				onSelect={() => dialog("reinstall")}
			/>
			<Separator className="-mx-1.5 my-1" />
			<MenuRow
				Item={Item}
				icon={Trash2}
				label={t("instance.menu.delete")}
				note={busyNote}
				disabled={busy}
				destructive
				onSelect={() => dialog("remove")}
			/>
		</>
	);

	const className = "w-60";
	return kind === "dropdown" ? (
		<DropdownMenuContent align="end" className={className} onCloseAutoFocus={onCloseAutoFocus}>
			{items}
		</DropdownMenuContent>
	) : (
		<ContextMenuContent className={className} onCloseAutoFocus={onCloseAutoFocus}>
			{items}
		</ContextMenuContent>
	);
}

function moveTo(id: string, group: string | null) {
	void moveInstanceToGroup(id, group).catch((err: unknown) => toast.error(groupErrorText(toAppError(err))));
}

function MoveToGroup({
	instance,
	parts,
	openNewGroup,
}: {
	instance: Instance;
	parts: Parts;
	openNewGroup: () => void;
}) {
	const { t, i18n } = useTranslation();
	const store = useInstances();
	const groups = listGroups(store.instances, i18n.language);
	const { Item, Separator, Sub, SubTrigger, SubContent, RadioGroup, RadioItem } = parts;

	if (!groups.length) {
		return (
			<MenuRow
				Item={Item}
				icon={FolderPlus}
				label={t("instance.menu.toNewGroup")}
				onSelect={openNewGroup}
			/>
		);
	}

	return (
		<Sub>
			<SubTrigger className="h-9 gap-2.5 px-2.5">
				<Folder className="text-muted-foreground" />
				<span className="flex-1 truncate">{t("instance.menu.moveToGroup")}</span>
			</SubTrigger>
			<SubContent className="max-w-80 min-w-52">
				<div className="max-h-72 overflow-y-auto">
					<RadioGroup value={groupKey(instance.group)}>
						{groups.map((group) => (
							<RadioItem
								key={groupKey(group)}
								value={groupKey(group)}
								className="h-9 pr-2.5"
								onSelect={() => {
									if (groupKey(group) !== groupKey(instance.group))
										moveTo(instance.id, group);
								}}
							>
								<span className="truncate">{group}</span>
							</RadioItem>
						))}
					</RadioGroup>
				</div>
				<Separator />
				<MenuRow
					Item={Item}
					icon={FolderPlus}
					label={t("instance.menu.newGroup")}
					onSelect={openNewGroup}
				/>
				{instance.group && (
					<MenuRow
						Item={Item}
						icon={Minus}
						label={t("instance.menu.removeFromGroup")}
						onSelect={() => moveTo(instance.id, null)}
					/>
				)}
			</SubContent>
		</Sub>
	);
}

function MenuRow({
	Item,
	icon: Icon,
	label,
	note,
	disabled,
	destructive,
	onSelect,
}: {
	Item: ComponentType<ItemProps>;
	icon: LucideIcon;
	label: string;
	note?: string | null;
	disabled?: boolean;
	destructive?: boolean;
	onSelect: () => void;
}) {
	return (
		<Item
			className={cn("h-9 gap-2.5 px-2.5", destructive && "focus:bg-destructive/10")}
			disabled={disabled}
			variant={destructive ? "destructive" : "default"}
			onSelect={onSelect}
		>
			<Icon className={cn(!destructive && "text-muted-foreground")} />
			<span className="flex-1 truncate">{label}</span>
			{disabled && note && <span className="ml-2 shrink-0 text-xs text-muted-foreground">{note}</span>}
		</Item>
	);
}
