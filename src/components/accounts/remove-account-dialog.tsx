import { LoaderCircle } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
	applyAccountsState,
	closeRemoveAccountDialog,
	reloadAccounts,
	restoreDialogFocus,
	useAccountDialogs,
	useAccounts,
} from "@/lib/accounts";
import { accountsApi } from "@/lib/api";
import { errorText, toAppError } from "@/lib/errors";

export function RemoveAccountDialog() {
	const { t } = useTranslation();
	const { remove } = useAccountDialogs();
	const store = useAccounts();
	const [busy, setBusy] = useState(false);
	const account = remove.account;
	if (!account) return null;

	const removingActive = account.id === store.activeId;
	const nextActive = removingActive ? store.accounts.find((a) => a.id !== account.id) : undefined;

	const confirm = async () => {
		setBusy(true);
		try {
			applyAccountsState(await accountsApi.remove(account.id));
			closeRemoveAccountDialog();
			toast(t("accounts.toast.removed", { username: account.username }));
		} catch (err) {
			closeRemoveAccountDialog();
			toast.error(errorText(toAppError(err)));
			void reloadAccounts();
		} finally {
			setBusy(false);
		}
	};

	return (
		<AlertDialog
			open={remove.open}
			onOpenChange={(open) => {
				if (!open && !busy) closeRemoveAccountDialog();
			}}
		>
			<AlertDialogContent
				className="max-w-[400px] gap-6 sm:max-w-[400px] data-[size=default]:sm:max-w-[400px]"
				onCloseAutoFocus={restoreDialogFocus}
				onEscapeKeyDown={(e) => busy && e.preventDefault()}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle className="break-words">
						{t("accounts.remove.title", { username: account.username })}
					</AlertDialogTitle>
					<AlertDialogDescription asChild>
						<div className="flex flex-col gap-2 text-sm text-muted-foreground">
							<p>
								{account.kind === "microsoft"
									? t("accounts.remove.descriptionMicrosoft")
									: t("accounts.remove.descriptionOffline")}
							</p>
							{nextActive && (
								<p>{t("accounts.remove.nextActive", { username: nextActive.username })}</p>
							)}
						</div>
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus disabled={busy}>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button variant="destructive" disabled={busy} onClick={() => void confirm()}>
						{busy && <LoaderCircle className="animate-spin" />}
						{t("accounts.remove.confirm")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
