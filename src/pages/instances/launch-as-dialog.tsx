import { X } from "lucide-react";
import { useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { AccountAvatar } from "@/components/accounts/account-avatar";
import { useAccountStatus } from "@/components/accounts/use-account-status";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useAccounts } from "@/lib/accounts";
import {
	closeLaunchAsDialog,
	gamesOfAccount,
	getInstances,
	launchInstance,
	setSameAccountWarning,
	useInstances,
	useLaunchAsDialog,
	type LaunchAsDialog as DialogState,
} from "@/lib/instances";
import { cn } from "@/lib/utils";

/** Второй запуск с того же аккаунта: предупредить и предложить другой. Один экземпляр на приложение. */
export function LaunchAsDialog() {
	const { t } = useTranslation();
	const dialog = useLaunchAsDialog();
	const confirmRef = useRef<HTMLButtonElement>(null);
	// Запуск — после закрытия окна, когда фокус уже вернулся: иначе следующий диалог (вход) спорит за фокус.
	const pending = useRef<(() => void) | null>(null);

	return (
		<Dialog
			open={dialog.open}
			onOpenChange={(next) => {
				if (!next) closeLaunchAsDialog();
			}}
		>
			<DialogContent
				showCloseButton={false}
				className="w-[440px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[440px]"
				onOpenAutoFocus={(event) => {
					event.preventDefault();
					confirmRef.current?.focus();
				}}
				onCloseAutoFocus={(event) => {
					const target = dialog.returnFocus;
					if (target?.isConnected) {
						event.preventDefault();
						target.focus();
					}
					const action = pending.current;
					pending.current = null;
					action?.();
				}}
			>
				<DialogClose className="absolute top-3 right-3 inline-flex size-8 items-center justify-center rounded-full text-muted-foreground transition-[color,background-color,scale] duration-(--duration-fast) ease-out hover:bg-accent hover:text-foreground active:scale-[0.98]">
					<X />
					<span className="sr-only">{t("common.close")}</span>
				</DialogClose>
				{dialog.instanceId && dialog.accountId && (
					<LaunchAsForm
						key={dialog.session}
						dialog={dialog}
						confirmRef={confirmRef}
						onConfirm={(action) => {
							pending.current = action;
							closeLaunchAsDialog();
						}}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}

function LaunchAsForm({
	dialog,
	confirmRef,
	onConfirm,
}: {
	dialog: DialogState;
	confirmRef: React.RefObject<HTMLButtonElement | null>;
	onConfirm: (action: () => void) => void;
}) {
	const { t } = useTranslation();
	const accounts = useAccounts();
	const store = useInstances();
	const statusOf = useAccountStatus();
	const instanceId = dialog.instanceId ?? "";
	const targetId = dialog.accountId ?? "";
	const [dontAsk, setDontAsk] = useState(false);
	// Снимок на момент открытия: если игра закроется, заголовок и текст не пропадут.
	const [intro] = useState(() => {
		const current = getInstances();
		const busy = gamesOfAccount(targetId, current, instanceId);
		return {
			nick: accounts.accounts.find((a) => a.id === targetId)?.username ?? "",
			name: busy[0]?.name ?? "",
		};
	});
	const [selected, setSelected] = useState(() => {
		if (dialog.explicit) return targetId;
		const current = getInstances();
		const free = accounts.accounts.find((a) => !gamesOfAccount(a.id, current, instanceId).length);
		return free?.id ?? targetId;
	});
	const showPicker = accounts.accounts.length >= 2;

	const submit = (event: FormEvent) => {
		event.preventDefault();
		if (dontAsk) setSameAccountWarning(false);
		const accountId = showPicker ? selected : targetId;
		const world = dialog.world ?? undefined;
		const returnFocus = dialog.returnFocus;
		onConfirm(() => void launchInstance(instanceId, returnFocus, { accountId, world, confirmed: true }));
	};

	const onKeyDown = (event: KeyboardEvent<HTMLFormElement>) => {
		// Radix не обрабатывает Enter на радиокнопках.
		if (event.key !== "Enter" || !(event.target as HTMLElement).closest('[role="radio"]')) return;
		event.preventDefault();
		event.currentTarget.requestSubmit();
	};

	return (
		<form onSubmit={submit} onKeyDown={onKeyDown} noValidate>
			<DialogTitle className="pr-10 break-words">
				{t("launchAs.title", { nick: intro.nick })}
			</DialogTitle>
			<DialogDescription className="mt-2 text-sm text-muted-foreground">
				{t("launchAs.description", { name: intro.name })}
			</DialogDescription>

			{showPicker && (
				<>
					<p id="launch-as-pick" className="mt-5 mb-2 text-sm font-medium">
						{t("launchAs.pick")}
					</p>
					<RadioGroup
						aria-labelledby="launch-as-pick"
						value={selected}
						onValueChange={setSelected}
						className={cn(
							"gap-0",
							accounts.accounts.length > 5 &&
								"-mx-2.5 max-h-60 overflow-x-hidden overflow-y-auto px-2.5",
						)}
					>
						{accounts.accounts.map((account) => {
							const games = gamesOfAccount(account.id, store, instanceId);
							const id = `launch-as-${account.id}`;
							return (
								<label
									key={account.id}
									htmlFor={id}
									className="-mx-2.5 flex h-12 cursor-pointer items-center gap-3 rounded-lg px-2.5 transition-[background-color] duration-(--duration-fast) ease-out hover:bg-accent"
								>
									<RadioGroupItem id={id} value={account.id} />
									<AccountAvatar account={account} size={32} surface="popover" />
									<span className="flex min-w-0 flex-1 flex-col">
										<span className="truncate text-sm/5 font-medium">
											{account.username}
										</span>
										{games.length ? (
											<span className="flex items-center gap-1.5 text-xs text-primary-text">
												<span
													aria-hidden
													className="size-1.5 shrink-0 rounded-full bg-primary"
												/>
												<span className="truncate">
													{games.length === 1
														? t("launchAs.busy", { name: games[0].name })
														: t("accounts.menu.inGameCount", {
																count: games.length,
															})}
												</span>
											</span>
										) : (
											<span
												className={cn(
													"truncate text-xs",
													account.needsRelogin
														? "text-destructive"
														: "text-muted-foreground",
												)}
											>
												{statusOf(account)}
											</span>
										)}
									</span>
								</label>
							);
						})}
					</RadioGroup>
				</>
			)}

			<div className="mt-4 flex items-center gap-2.5">
				<Checkbox
					id="launch-as-dont-ask"
					checked={dontAsk}
					onCheckedChange={(value) => setDontAsk(value === true)}
				/>
				<Label htmlFor="launch-as-dont-ask" className="text-sm font-normal">
					{t("launchAs.dontAsk")}
				</Label>
			</div>

			<div className="mt-6 flex justify-end gap-2">
				<Button type="button" variant="ghost" onClick={closeLaunchAsDialog}>
					{t("common.cancel")}
				</Button>
				<Button ref={confirmRef} type="submit">
					{t("launchAs.confirm")}
				</Button>
			</div>
		</form>
	);
}
