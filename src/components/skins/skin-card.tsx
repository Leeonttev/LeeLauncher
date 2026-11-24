import { Check, Ellipsis, FolderOpen, Pencil, Shirt, Trash2, UserRoundX } from "lucide-react";
import { motion } from "motion/react";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useRef, useState, type ComponentType, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { SkinFlat } from "@/components/skins/skin-flat";
import { Badge } from "@/components/ui/badge";
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
import { formatDate } from "@/lib/format";
import { ariaDeleteKey, ariaShortcut, isDeleteKey, isPrimaryMod } from "@/lib/platform";
import {
	isHistorySkin,
	openSkinDialog,
	openTakeOffDialog,
	revealSkin,
	skinDisplayName,
	skinFirstSeen,
	tryOnSkin,
} from "@/lib/skins";

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
	/** `null` — надеть сейчас нельзя (пауза, идёт изменение, нужен вход). */
	onWear: (() => void) | null;
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
	onWear,
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
			<Item onSelect={() => tryOnSkin(skin)}>
				<Shirt />
				{t("skins.menu.tryOn")}
			</Item>
			<Item disabled={!onWear || Boolean(wornBy)} onSelect={() => onWear?.()}>
				<Check />
				{t("skins.menu.wear")}
			</Item>
			<Item onSelect={() => dialog("rename")}>
				<Pencil />
				{t("skins.menu.rename")}
			</Item>
			<Item onSelect={() => void revealSkin(skin.id)}>
				<FolderOpen />
				{t("skins.menu.showFile")}
			</Item>
			<Separator />
			{wornBy && (
				<Item
					disabled={takeOffLocked}
					onSelect={() => defer(() => openTakeOffDialog(wornBy.id, focusTarget()))}
				>
					<UserRoundX />
					{t("skins.menu.takeOff")}
				</Item>
			)}
			<Item variant="destructive" onSelect={() => dialog("remove")}>
				<Trash2 />
				{isHistorySkin(skin) ? t("skins.menu.removeHistory") : t("skins.menu.remove")}
			</Item>
		</>
	);
}

/** Щелчок и стрелки примеряют; двойной щелчок и Ctrl/⌘+Enter — сразу надевают. */
export function SkinCard({
	skin,
	model,
	accountId,
	wornBy,
	takeOffLocked,
	onWear,
	menuTabbable,
	onMoveVertical,
	fresh,
}: {
	skin: SkinEntry;
	model: SkinModel;
	accountId: string | null;
	wornBy: Account | null;
	takeOffLocked: boolean;
	onWear: ((focus: HTMLElement | null) => void) | null;
	/** «⋯» в порядке Tab только у текущей карточки. */
	menuTabbable: boolean;
	/** Стрелка вверх/вниз — на ряд, а не на один скин. */
	onMoveVertical: (id: string, direction: -1 | 1) => void;
	/** Добавлен пользователем только что: появляется прозрачностью. */
	fresh: boolean;
}) {
	const { t } = useTranslation();
	const radioRef = useRef<HTMLButtonElement>(null);
	const [menuOpen, setMenuOpen] = useState(false);
	const [hover, setHover] = useState(false);
	/** После Shift+F10 фокус возвращается на карточку, а не на «⋯». */
	const openedFromCard = useRef(false);
	const dropdown = usePendingAction();
	const context = usePendingAction();
	const focusTarget = () => radioRef.current;
	const name = skinDisplayName(skin, accountId);
	const wear = onWear ? () => onWear(radioRef.current) : null;

	const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
		if (event.key === "F10" && event.shiftKey) {
			event.preventDefault();
			openedFromCard.current = true;
			setMenuOpen(true);
		} else if (event.code === "F2") {
			event.preventDefault();
			openSkinDialog("rename", skin, event.currentTarget);
		} else if (isDeleteKey(event)) {
			event.preventDefault();
			openSkinDialog("remove", skin, event.currentTarget);
		} else if (event.code === "Enter" || event.code === "NumpadEnter") {
			event.preventDefault();
			if (isPrimaryMod(event)) wear?.();
			else tryOnSkin(skin);
		} else if (event.code === "ArrowUp" || event.code === "ArrowDown") {
			// Radix ходит по порядку; `preventDefault` отключает его обработчик.
			event.preventDefault();
			onMoveVertical(skin.id, event.code === "ArrowUp" ? -1 : 1);
		}
	};

	const armsLabel = model === "slim" ? t("skins.card.slim") : t("skins.card.classic");
	const seenAt = skinFirstSeen(skin, accountId);
	// Дата — в подсказке: в узкой карточке она не помещается рядом с текстом.
	const subtitle = wornBy
		? t("skins.card.onAccount")
		: isHistorySkin(skin)
			? t("skins.card.wasWornPlain")
			: skin.legacy
				? `${armsLabel} · ${t("skins.card.legacy")}`
				: armsLabel;

	const shared = { skin, wornBy, takeOffLocked, onWear: wear, focusTarget };
	const body = (
		<div className="relative w-38">
			<RadioGroupPrimitive.Item
				ref={radioRef}
				value={skin.id}
				data-skin-card={skin.id}
				aria-label={wornBy ? `${name} · ${t("skins.card.worn")}` : name}
				aria-keyshortcuts={`Shift+F10 F2 ${ariaDeleteKey} ${ariaShortcut("Enter")}`}
				className="relative flex h-54 w-full cursor-pointer flex-col rounded-xl border border-border bg-card px-3 pt-8 pb-2.5 text-left transition-[border-color,box-shadow] duration-(--duration-fast) ease-out not-aria-checked:hover:border-border-hover focus-visible:outline-foreground aria-checked:border-primary aria-checked:ring-1 aria-checked:ring-primary aria-checked:ring-inset"
				onKeyDown={onKeyDown}
				onDoubleClick={() => wear?.()}
				onPointerEnter={() => setHover(true)}
				onPointerLeave={() => setHover(false)}
			>
				<SkinFlat
					texture={skin.texture}
					model={model}
					scale={4}
					side={hover ? "back" : "front"}
					className="mx-auto"
				/>
				<span title={name} className="mt-2 block truncate text-sm/5 font-medium">
					{name}
				</span>
				<span
					title={
						isHistorySkin(skin) ? t("skins.card.wasWorn", { date: formatDate(seenAt) }) : subtitle
					}
					className="block truncate text-xs/4 text-muted-foreground"
				>
					{subtitle}
				</span>
			</RadioGroupPrimitive.Item>
			{wornBy && (
				<Badge className="pointer-events-none absolute top-2 left-2">{t("skins.card.worn")}</Badge>
			)}
			<DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
				<DropdownMenuTrigger asChild>
					<Button
						variant="ghost"
						size="icon-xs"
						tabIndex={menuTabbable ? 0 : -1}
						aria-label={t("skins.card.menu", { name })}
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
				{/* Обёртка есть всегда: смена на голый `div` пересоздала бы карточку и сбросила фокус. */}
				<motion.div
					initial={fresh ? { opacity: 0 } : false}
					animate={{ opacity: 1 }}
					transition={{ duration: 0.2 }}
					className="w-38"
				>
					{body}
				</motion.div>
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
