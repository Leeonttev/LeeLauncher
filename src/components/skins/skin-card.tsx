import { Ellipsis, FolderOpen, Pencil, Shirt, Trash2 } from "lucide-react";
import { motion } from "motion/react";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useRef, useState, type ComponentType, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { SkinFlat } from "@/components/skins/skin-flat";
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
import { usePendingAction } from "@/hooks/use-pending-action";
import type { Account, SkinEntry, SkinModel } from "@/lib/api";
import { openSkinDialog, openTakeOffDialog, revealSkin } from "@/lib/skins";

type ItemProps = {
	variant?: "default" | "destructive";
	disabled?: boolean;
	onSelect?: () => void;
	children: ReactNode;
};

type MenuProps = {
	skin: SkinEntry;
	wornBy: Account | null;
	takeOffLocked: boolean;
	focusTarget: () => HTMLElement | null;
};

/** Диалоги открываются после закрытия меню. */
function MenuItems({
	Item,
	Separator,
	defer,
	skin,
	wornBy,
	takeOffLocked,
	focusTarget,
}: MenuProps & {
	Item: ComponentType<ItemProps>;
	Separator: ComponentType;
	defer: (action: () => void) => void;
}) {
	const { t } = useTranslation();
	const dialog = (kind: "rename" | "remove") => defer(() => openSkinDialog(kind, skin, focusTarget()));
	return (
		<>
			<Item onSelect={() => dialog("rename")}>
				<Pencil />
				{t("skins.menu.rename")}
			</Item>
			<Item onSelect={() => void revealSkin(skin.id)}>
				<FolderOpen />
				{t("skins.menu.showFile")}
			</Item>
			{wornBy && (
				<Item
					disabled={takeOffLocked}
					onSelect={() => defer(() => openTakeOffDialog(wornBy.id, focusTarget()))}
				>
					<Shirt />
					{t("skins.menu.takeOff")}
				</Item>
			)}
			<Separator />
			<Item variant="destructive" onSelect={() => dialog("remove")}>
				<Trash2 />
				{t("skins.menu.remove")}
			</Item>
		</>
	);
}

/** Щелчок и стрелки только примеряют. */
export function SkinCard({
	skin,
	model,
	wornBy,
	takeOffLocked,
	menuTabbable,
	onMoveVertical,
	onEnter,
	fadeDelay,
}: {
	skin: SkinEntry;
	model: SkinModel;
	wornBy: Account | null;
	takeOffLocked: boolean;
	/** «⋯» в порядке Tab только у текущей карточки. */
	menuTabbable: boolean;
	/** Стрелка вверх/вниз — на ряд, а не на один скин. */
	onMoveVertical: (id: string, direction: -1 | 1) => void;
	onEnter: () => void;
	/** `null` — без анимации появления (ряды виртуального списка монтируются при прокрутке). */
	fadeDelay: number | null;
}) {
	const { t } = useTranslation();
	const radioRef = useRef<HTMLButtonElement>(null);
	const [menuOpen, setMenuOpen] = useState(false);
	/** После Shift+F10 фокус возвращается на карточку, а не на «⋯». */
	const openedFromCard = useRef(false);
	const dropdown = usePendingAction();
	const context = usePendingAction();
	const focusTarget = () => radioRef.current;

	const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
		if (event.key === "F10" && event.shiftKey) {
			event.preventDefault();
			openedFromCard.current = true;
			setMenuOpen(true);
		} else if (event.code === "F2") {
			event.preventDefault();
			openSkinDialog("rename", skin, event.currentTarget);
		} else if (event.code === "Delete") {
			event.preventDefault();
			openSkinDialog("remove", skin, event.currentTarget);
		} else if (event.code === "Enter" || event.code === "NumpadEnter") {
			event.preventDefault();
			onEnter();
		} else if (event.code === "ArrowUp" || event.code === "ArrowDown") {
			// Radix ходит по порядку; `preventDefault` отключает его обработчик.
			event.preventDefault();
			onMoveVertical(skin.id, event.code === "ArrowUp" ? -1 : 1);
		}
	};

	const shared = { skin, wornBy, takeOffLocked, focusTarget };
	const armsLabel = model === "slim" ? t("skins.card.slim") : t("skins.card.classic");
	const body = (
		<div className="relative w-38">
			<RadioGroupPrimitive.Item
				ref={radioRef}
				value={skin.id}
				data-skin-card={skin.id}
				aria-label={skin.name}
				aria-keyshortcuts="Shift+F10 F2 Delete"
				className="relative flex w-full cursor-pointer flex-col rounded-xl border border-border px-3 pt-3 pb-2.5 text-left transition-[background-color,border-color] duration-(--duration-fast) ease-out hover:border-border-hover hover:bg-accent focus-visible:outline-foreground aria-checked:bg-primary-soft aria-checked:outline-2 aria-checked:outline-offset-2 aria-checked:outline-primary"
				onKeyDown={onKeyDown}
			>
				<SkinFlat texture={skin.texture} model={model} scale={4} className="mx-auto" />
				<span title={skin.name} className="mt-2 block truncate text-sm/5 font-semibold">
					{skin.name}
				</span>
				<span
					title={skin.legacy ? `${armsLabel} · ${t("skins.card.legacy")}` : undefined}
					className="block truncate text-[13px]/[18px] text-muted-foreground"
				>
					{wornBy ? (
						<span className="font-semibold text-primary-text">{t("skins.card.worn")}</span>
					) : (
						armsLabel
					)}
					{skin.legacy && ` · ${t("skins.card.legacy")}`}
				</span>
			</RadioGroupPrimitive.Item>
			<DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
				<DropdownMenuTrigger asChild>
					<Button
						variant="ghost"
						size="icon-xs"
						tabIndex={menuTabbable ? 0 : -1}
						aria-label={t("skins.card.menu", { name: skin.name })}
						className="absolute top-1.5 right-1.5 size-7 text-muted-foreground"
					>
						<Ellipsis />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent
					align="end"
					className="w-64"
					onCloseAutoFocus={(event) => {
						dropdown.onCloseAutoFocus(event);
						const fromCard = openedFromCard.current;
						openedFromCard.current = false;
						if (fromCard && !event.defaultPrevented) {
							event.preventDefault();
							radioRef.current?.focus();
						}
					}}
				>
					<MenuItems
						{...shared}
						Item={DropdownMenuItem}
						Separator={DropdownMenuSeparator}
						defer={dropdown.defer}
					/>
				</DropdownMenuContent>
			</DropdownMenu>
		</div>
	);

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				{fadeDelay === null ? (
					body
				) : (
					<motion.div
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						transition={{ duration: 0.12, delay: fadeDelay }}
						className="w-38"
					>
						{body}
					</motion.div>
				)}
			</ContextMenuTrigger>
			<ContextMenuContent className="w-64" onCloseAutoFocus={context.onCloseAutoFocus}>
				<MenuItems
					{...shared}
					Item={ContextMenuItem}
					Separator={ContextMenuSeparator}
					defer={context.defer}
				/>
			</ContextMenuContent>
		</ContextMenu>
	);
}
