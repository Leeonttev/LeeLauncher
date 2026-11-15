import { LoaderCircle } from "lucide-react";
import { useState, type RefObject } from "react";
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
import type { ContentItem } from "@/lib/api";
import { removeContent } from "@/lib/content";

export function RemoveContentDialog({
	open,
	onOpenChange,
	title,
	instanceId,
	instanceName,
	items,
	returnFocus,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	title: string;
	instanceId: string;
	instanceName: string;
	items: ContentItem[];
	returnFocus?: RefObject<HTMLElement | null>;
}) {
	const { t } = useTranslation();
	const [busy, setBusy] = useState(false);

	const confirm = async () => {
		setBusy(true);
		const ok = await removeContent(instanceId, items);
		setBusy(false);
		if (!ok) return;
		onOpenChange(false);
		toast(t("catalog.install.removed", { title, name: instanceName }));
	};

	return (
		<AlertDialog open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
			<AlertDialogContent
				className="max-w-[448px] gap-6 sm:max-w-[448px] data-[size=default]:sm:max-w-[448px]"
				onEscapeKeyDown={(e) => busy && e.preventDefault()}
				onCloseAutoFocus={(event) => {
					const target = returnFocus?.current;
					if (!target?.isConnected) return;
					event.preventDefault();
					target.focus();
				}}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle className="break-words">
						{t("catalog.removeDialog.title", { title, name: instanceName })}
					</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{t("catalog.removeDialog.description")}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus disabled={busy}>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button
						variant="destructive"
						aria-disabled={busy || undefined}
						onClick={() => !busy && void confirm()}
					>
						{busy && <LoaderCircle className="animate-spin" />}
						{t("common.remove")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
