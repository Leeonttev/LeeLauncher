import { LoaderCircle, OctagonX } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
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
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { closeModpackDialog, restoreModpackFocus, unlinkModpack, useModpackDialogs } from "@/lib/modpacks";

/** Данные не пропадают, поэтому кнопка обычная, не `destructive`. */
export function UnlinkModpackDialog() {
	const { t } = useTranslation();
	const dialog = useModpackDialogs().unlink;
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<{ session: number; value: AppError } | null>(null);
	const shownError = error?.session === dialog.session ? error.value : null;

	const confirm = async () => {
		if (!dialog.instanceId || busy) return;
		setBusy(true);
		try {
			await unlinkModpack(dialog.instanceId);
		} catch (err) {
			setError({ session: dialog.session, value: toAppError(err) });
		} finally {
			setBusy(false);
		}
	};

	return (
		<AlertDialog
			open={dialog.open}
			onOpenChange={(open) => {
				if (!open && !busy) closeModpackDialog("unlink");
			}}
		>
			<AlertDialogContent
				className="max-w-[400px] gap-6 sm:max-w-[400px] data-[size=default]:sm:max-w-[400px]"
				onCloseAutoFocus={(e) => restoreModpackFocus("unlink", e)}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle>{t("modpack.unlink.title")}</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{t("modpack.unlink.description")}
					</AlertDialogDescription>
					{shownError && (
						<p className="flex items-start gap-1.5 text-[13px]/[18px] text-destructive">
							<OctagonX className="mt-px shrink-0" />
							<span className="min-w-0">{errorText(shownError)}</span>
						</p>
					)}
				</AlertDialogHeader>
				<AlertDialogFooter>
					{/* `aria-disabled`, а не `disabled`: иначе фокус уйдёт на body. */}
					<AlertDialogCancel
						autoFocus
						aria-disabled={busy || undefined}
						className="aria-disabled:opacity-50"
						onClick={(e) => {
							if (busy) e.preventDefault();
						}}
					>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button
						aria-disabled={busy || undefined}
						className="aria-disabled:opacity-50"
						onClick={() => void confirm()}
					>
						{busy && <LoaderCircle className="animate-spin" />}
						{t("modpack.unlink.confirm")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
