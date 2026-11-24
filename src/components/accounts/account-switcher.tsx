import { Check, ChevronDown, OctagonX, Plus, Trash2, User } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import {
	useEffect,
	useRef,
	useState,
	type ComponentProps,
	type ReactNode,
	type Ref,
	type SyntheticEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { AccountAvatar } from "@/components/accounts/account-avatar";
import { useAccountStatus } from "@/components/accounts/use-account-status";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
	ACCOUNT_SWITCHER_ATTR,
	activeAccount,
	openAddAccountDialog,
	openReloginDialog,
	openRemoveAccountDialog,
	setActiveAccount,
	useAccounts,
	type AccountsStore,
} from "@/lib/accounts";
import type { Account } from "@/lib/api";
import { gamesOfAccount, useInstances } from "@/lib/instances";
import { cn } from "@/lib/utils";
import { titlebarClasses } from "@/components/layout/titlebar-layout";

const spring = { type: "spring", stiffness: 500, damping: 35 } as const;

export function AccountSwitcher() {
	const store = useAccounts();
	const loading = store.status === "idle" || store.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);

	if (loading) {
		return (
			<div data-tauri-drag-region aria-busy className="flex h-10 items-center gap-2.5 pr-2.5 pl-1">
				{showSkeleton && (
					<>
						<Skeleton className="pointer-events-none size-8 shrink-0 rounded-sm bg-sidebar-accent" />
						<Skeleton className="pointer-events-none h-3.5 w-20 bg-sidebar-accent" />
					</>
				)}
			</div>
		);
	}

	return (
		<motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: 0.15 }}>
			{store.accounts.length === 0 ? <AddAccountButton /> : <SwitcherMenu store={store} />}
		</motion.div>
	);
}

type SwitcherButtonProps = ComponentProps<"button"> & {
	contentKey: string;
	avatar: ReactNode;
	title: string;
	titleClassName?: string;
	chevron?: boolean;
};

/** Голова 32 (`rounded-sm`, 6) в 4 px от края: скругление кнопки 6 + 4 = 10, углы идут параллельно. */
function SwitcherButton({
	contentKey,
	avatar,
	title,
	titleClassName,
	chevron = true,
	className,
	...props
}: SwitcherButtonProps) {
	return (
		<button
			type="button"
			{...{ [ACCOUNT_SWITCHER_ATTR]: "" }}
			className={cn(
				"relative flex h-10 items-center gap-2.5 overflow-clip rounded-[10px] pr-2.5 pl-1 text-left transition-[background-color,scale] duration-(--duration-fast) ease-out hover:bg-sidebar-accent active:scale-[0.98] aria-expanded:bg-sidebar-accent",
				className,
			)}
			{...props}
		>
			<AnimatePresence mode="popLayout" initial={false}>
				<motion.span
					key={contentKey}
					initial={{ opacity: 0 }}
					animate={{ opacity: 1, transition: spring }}
					exit={{ opacity: 0, transition: { duration: 0.1 } }}
					className="flex min-w-0 items-center gap-2.5"
				>
					{avatar}
					<span
						className={cn(
							"max-w-28 truncate text-sm/5 font-semibold whitespace-nowrap text-foreground",
							titlebarClasses.nick,
							titleClassName,
						)}
					>
						{title}
					</span>
				</motion.span>
			</AnimatePresence>
			{chevron && <ChevronDown className="text-muted-foreground" />}
		</button>
	);
}

function EmptyAvatar() {
	return (
		<span
			aria-hidden
			className="flex size-8 shrink-0 items-center justify-center rounded-sm bg-foreground/10 text-muted-foreground"
		>
			<User />
		</span>
	);
}

function AddAccountButton() {
	const { t } = useTranslation();
	const label = t("accounts.switcher.addAccount");
	return (
		<SwitcherButton
			contentKey="none"
			avatar={<EmptyAvatar />}
			title={label}
			titleClassName={cn("max-w-none text-primary-text", titlebarClasses.addAccount)}
			chevron={false}
			onClick={(e) => openAddAccountDialog(e.currentTarget)}
		/>
	);
}

function SwitcherMenu({ store }: { store: AccountsStore }) {
	const { t } = useTranslation();
	const statusOf = useAccountStatus();
	const instances = useInstances();
	const [open, setOpen] = useState(false);
	const triggerRef = useRef<HTMLButtonElement>(null);
	const activeItemRef = useRef<HTMLDivElement>(null);
	const keyboardOpen = useRef(false);
	/** Диалог открываем после закрытия меню, когда оно уже вернуло фокус. */
	const pending = useRef<(() => void) | null>(null);
	const active = activeAccount(store);

	useEffect(() => {
		if (!open || !keyboardOpen.current) return;
		const frame = requestAnimationFrame(() => activeItemRef.current?.focus());
		return () => cancelAnimationFrame(frame);
	}, [open]);

	const afterClose = (action: () => void, close = false) => {
		pending.current = action;
		if (close) setOpen(false);
	};

	const requestRemove = (account: Account) =>
		afterClose(() => openRemoveAccountDialog(account, triggerRef.current), true);

	let button: ReactNode;
	let tooltip: string;
	if (active) {
		const status = statusOf(active);
		tooltip = t("accounts.switcher.tooltip", { username: active.username, status });
		button = (
			<SwitcherButton
				ref={triggerRef}
				contentKey={active.id}
				avatar={<AccountAvatar account={active} size={32} surface="sidebar" />}
				title={active.username}
				aria-label={t("accounts.switcher.ariaLabel", { username: active.username, status })}
			/>
		);
	} else {
		tooltip = t("accounts.switcher.selectAccount");
		button = (
			<SwitcherButton
				ref={triggerRef}
				contentKey="none"
				avatar={<EmptyAvatar />}
				title={t("accounts.switcher.notSelected")}
				aria-label={tooltip}
			/>
		);
	}

	return (
		<DropdownMenu open={open} onOpenChange={setOpen}>
			<Tooltip>
				<TooltipTrigger asChild>
					<DropdownMenuTrigger
						asChild
						onPointerDown={() => (keyboardOpen.current = false)}
						onKeyDown={(e) => {
							if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown")
								keyboardOpen.current = true;
						}}
					>
						{button}
					</DropdownMenuTrigger>
				</TooltipTrigger>
				<TooltipContent side="bottom" sideOffset={6} hidden={open}>
					{tooltip}
				</TooltipContent>
			</Tooltip>
			<DropdownMenuContent
				side="bottom"
				align="end"
				sideOffset={8}
				className="w-72 p-1.5"
				onCloseAutoFocus={(e) => {
					const action = pending.current;
					if (!action) return;
					pending.current = null;
					e.preventDefault();
					action();
				}}
			>
				<DropdownMenuLabel className="px-2.5 py-1.5 text-xs font-medium text-muted-foreground">
					{t("accounts.menu.label")}
				</DropdownMenuLabel>
				<div className="max-h-[264px] overflow-y-auto">
					{store.accounts.map((account) => {
						const isActive = account.id === store.activeId;
						const games = gamesOfAccount(account.id, instances);
						return (
							<AccountItem
								key={account.id}
								ref={isActive ? activeItemRef : undefined}
								account={account}
								active={isActive}
								status={statusOf(account)}
								inGame={
									games.length === 0
										? null
										: games.length === 1
											? t("accounts.menu.inGame", { name: games[0].name })
											: t("accounts.menu.inGameCount", { count: games.length })
								}
								onSelect={() => {
									if (!isActive) void setActiveAccount(account.id);
									if (account.needsRelogin)
										afterClose(() => openReloginDialog(account, triggerRef.current));
								}}
								onRemove={() => requestRemove(account)}
							/>
						);
					})}
				</div>
				<DropdownMenuSeparator className="-mx-1.5 my-1" />
				<DropdownMenuItem
					className="h-12 gap-3 px-2.5"
					onSelect={() => afterClose(() => openAddAccountDialog(triggerRef.current))}
				>
					<span className="flex size-8 items-center justify-center rounded-sm bg-foreground/[0.06] text-foreground">
						<Plus className="text-foreground" />
					</span>
					<span className="text-sm font-medium">{t("accounts.menu.add")}</span>
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

type AccountItemProps = {
	ref?: Ref<HTMLDivElement>;
	account: Account;
	active: boolean;
	status: string;
	/** Подпись «В игре · …»; «нужно войти снова» важнее. */
	inGame: string | null;
	onSelect: () => void;
	onRemove: () => void;
};

function AccountItem({ ref, account, active, status, inGame, onSelect, onRemove }: AccountItemProps) {
	const { t } = useTranslation();
	const removeLabel = t("accounts.menu.removeTooltip");
	const stop = (e: SyntheticEvent) => {
		e.preventDefault();
		e.stopPropagation();
	};
	return (
		<DropdownMenuItem
			ref={ref}
			role="menuitemradio"
			aria-checked={active}
			aria-keyshortcuts="Delete"
			textValue={account.username}
			className="group/item h-12 gap-3 px-2.5"
			onSelect={onSelect}
			onKeyDown={(e) => {
				if (e.key === "Delete" || e.key === "Backspace") {
					e.preventDefault();
					onRemove();
				}
			}}
		>
			<AccountAvatar account={account} size={32} surface="popover" />
			<span className="flex min-w-0 flex-1 flex-col">
				<span className={cn("truncate text-sm/5", active ? "font-semibold" : "font-medium")}>
					{account.username}
				</span>
				{inGame && !account.needsRelogin ? (
					<span className="flex items-center gap-1.5 text-xs text-primary-text">
						<span aria-hidden className="size-1.5 shrink-0 rounded-full bg-primary" />
						<span className="truncate">{inGame}</span>
					</span>
				) : (
					<span
						className={cn(
							"flex items-center gap-1 text-xs",
							account.needsRelogin ? "text-destructive" : "text-muted-foreground",
						)}
					>
						{account.needsRelogin && <OctagonX className="shrink-0" />}
						<span className="truncate">{status}</span>
					</span>
				)}
			</span>
			<Tooltip>
				<TooltipTrigger asChild>
					<button
						type="button"
						tabIndex={-1}
						aria-label={removeLabel}
						className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-[opacity,color,background-color] duration-(--duration-fast) ease-out group-focus-within/item:opacity-100 group-hover/item:opacity-100 group-data-[highlighted]/item:opacity-100 hover:bg-destructive/10 hover:text-destructive"
						onPointerDown={stop}
						onPointerUp={stop}
						onClick={(e) => {
							stop(e);
							onRemove();
						}}
					>
						<Trash2 className="text-current" />
					</button>
				</TooltipTrigger>
				<TooltipContent side="top" sideOffset={4}>
					{removeLabel}
				</TooltipContent>
			</Tooltip>
			{active && <Check className="text-primary-text" />}
		</DropdownMenuItem>
	);
}
