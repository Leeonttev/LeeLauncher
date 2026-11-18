import { LoaderCircle } from "lucide-react";
import { useEffect, useState } from "react";
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
import { appApi } from "@/lib/api";
import { errorText, toAppError } from "@/lib/errors";
import { gameAccountOf, useInstances } from "@/lib/instances";

/** «Выйти» в трее, пока идут игры. Живёт в `AppFrame`: оболочки может не быть (экран обновления). */
export function QuitDialog() {
	const { t } = useTranslation();
	const [games, setGames] = useState<number | null>(null);
	const [busy, setBusy] = useState(false);
	const store = useInstances();

	useEffect(() => {
		let unlisten: (() => void) | null = null;
		let disposed = false;
		appApi
			.onQuitRequested((payload) => setGames(Math.max(1, payload.games)))
			.then((off) => {
				if (disposed) off();
				else unlisten = off;
			})
			.catch((err: unknown) => console.error("app-quit-requested", err));
		return () => {
			disposed = true;
			unlisten?.();
		};
	}, []);

	const running = Object.keys(store.processes);
	const single =
		games === 1 && running.length === 1 ? store.instances.find((i) => i.id === running[0]) : undefined;
	const account = single ? gameAccountOf(single.id, store) : null;
	const description = single
		? account
			? t("quit.descriptionNamedAs", { name: single.name, nick: account.username })
			: t("quit.descriptionNamed", { name: single.name })
		: t("quit.description", { count: games ?? 1 });

	const confirm = async () => {
		setBusy(true);
		try {
			await appApi.quit();
		} catch (err) {
			toast.error(errorText(toAppError(err)));
			setBusy(false);
		}
	};

	return (
		<AlertDialog open={games !== null} onOpenChange={(open) => !open && !busy && setGames(null)}>
			<AlertDialogContent
				className="max-w-[448px] gap-6 sm:max-w-[448px] data-[size=default]:sm:max-w-[448px]"
				onEscapeKeyDown={(e) => busy && e.preventDefault()}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle>{t("quit.title")}</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{description}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel
						autoFocus
						aria-disabled={busy || undefined}
						onClick={(event) => busy && event.preventDefault()}
					>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button
						variant="destructive"
						aria-disabled={busy || undefined}
						onClick={() => !busy && void confirm()}
					>
						{busy && <LoaderCircle className="animate-spin" />}
						{busy ? t("quit.closing") : t("quit.confirm")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
