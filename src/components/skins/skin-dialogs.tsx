import { Check, Copy, LoaderCircle, OctagonX } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
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
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { getAccounts } from "@/lib/accounts";
import { errorText, toAppError } from "@/lib/errors";
import {
	closeSkinDialog,
	confirmReplaceSkin,
	currentSkin,
	isHistorySkin,
	removeSkin,
	renameSkin,
	SKIN_NAME_MAX,
	skinDisplayName,
	takeOffSkin,
	useSkins,
	type SkinsStore,
} from "@/lib/skins";

/** Пока диалог закрывается, его содержимое не должно опустеть. */
function useLast<T>(value: T | null): T | null {
	const [last, setLast] = useState(value);
	if (value !== null && value !== last) setLast(value);
	return value ?? last;
}

function restoreFocus(event: Event, target: HTMLElement | null) {
	if (!target?.isConnected) return;
	event.preventDefault();
	target.focus();
}

/** Один экземпляр на приложение, чтобы «Подробнее» из тоста работало на любой странице. */
export function SkinDialogs() {
	const store = useSkins();
	return (
		<>
			<RenameDialog store={store} />
			<RemoveDialog store={store} />
			<TakeOffDialog store={store} />
			<ReplaceDialog store={store} />
			<DetailsDialog store={store} />
		</>
	);
}

function RenameDialog({ store }: { store: SkinsStore }) {
	const { t } = useTranslation();
	const dialog = store.dialogs.kind === "rename" ? store.dialogs : null;
	const shown = useLast(dialog);
	const lastFocus = useRef<HTMLElement | null>(null);
	useEffect(() => {
		if (dialog) lastFocus.current = dialog.returnFocus;
	}, [dialog]);
	const [busy, setBusy] = useState(false);
	return (
		<Dialog
			open={Boolean(dialog)}
			onOpenChange={(next) => {
				if (!next && !busy) closeSkinDialog();
			}}
		>
			<DialogContent
				className="w-[448px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[448px]"
				onCloseAutoFocus={(event) => restoreFocus(event, lastFocus.current)}
			>
				<DialogTitle className="pr-10">{t("skins.rename.title")}</DialogTitle>
				<DialogDescription className="sr-only">
					{shown ? skinDisplayName(shown.skin, getAccounts().activeId) : null}
				</DialogDescription>
				{shown && (
					<RenameForm
						key={shown.session}
						id={shown.skin.id}
						current={skinDisplayName(shown.skin, getAccounts().activeId)}
						busy={busy}
						setBusy={setBusy}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}

function RenameForm({
	id,
	current,
	busy,
	setBusy,
}: {
	id: string;
	current: string;
	busy: boolean;
	setBusy: (busy: boolean) => void;
}) {
	const { t } = useTranslation();
	const [value, setValue] = useState(current);
	const [error, setError] = useState<string | null>(null);
	const name = value.trim();
	const canSubmit = name.length >= 1 && name.length <= SKIN_NAME_MAX && name !== current && !busy;

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		if (!canSubmit) return;
		setBusy(true);
		setError(null);
		try {
			await renameSkin(id, name);
			closeSkinDialog();
		} catch (err) {
			setError(errorText(toAppError(err)));
		} finally {
			setBusy(false);
		}
	};

	return (
		<form onSubmit={(event) => void submit(event)} className="mt-5">
			<Label htmlFor="skin-name">{t("skins.rename.label")}</Label>
			<Input
				id="skin-name"
				autoFocus
				value={value}
				maxLength={SKIN_NAME_MAX}
				spellCheck={false}
				autoComplete="off"
				aria-invalid={Boolean(error)}
				className="mt-2"
				onFocus={(event) => event.currentTarget.select()}
				onChange={(event) => {
					setValue(event.target.value);
					setError(null);
				}}
			/>
			{error && (
				<p role="alert" className="mt-2 flex items-start gap-1.5 text-[13px]/[18px] text-destructive">
					<OctagonX className="mt-px shrink-0" />
					<span className="min-w-0">{error}</span>
				</p>
			)}
			<div className="mt-6 flex justify-end gap-2">
				<Button type="button" variant="ghost" disabled={busy} onClick={closeSkinDialog}>
					{t("common.cancel")}
				</Button>
				<Button type="submit" disabled={!canSubmit}>
					{busy && <LoaderCircle className="animate-spin" />}
					{t("common.save")}
				</Button>
			</div>
		</form>
	);
}

function RemoveDialog({ store }: { store: SkinsStore }) {
	const { t } = useTranslation();
	const dialog = store.dialogs.kind === "remove" ? store.dialogs : null;
	const shown = useLast(dialog);
	const [busy, setBusy] = useState(false);
	const lastFocus = useRef<HTMLElement | null>(null);
	const removedNext = useRef<{ next: string | null } | null>(null);
	useEffect(() => {
		if (dialog) lastFocus.current = dialog.returnFocus;
	}, [dialog]);

	const history = shown ? isHistorySkin(shown.skin) : false;
	const wornSomewhere =
		dialog !== null &&
		getAccounts().accounts.some((a) => currentSkin(store, a.id)?.skinId === dialog.skin.id);
	// Запись стирается раньше, чем диалог закроется: текст на это время не меняем.
	const text = useLast(
		dialog && !busy
			? history
				? t("skins.removeHistory.text")
				: wornSomewhere
					? t("skins.remove.textWorn")
					: t("skins.remove.text")
			: null,
	);

	const confirm = async () => {
		if (!dialog || busy) return;
		const index = store.items.findIndex((skin) => skin.id === dialog.skin.id);
		const next = store.items[index + 1]?.id ?? store.items[index - 1]?.id ?? null;
		setBusy(true);
		try {
			await removeSkin(dialog.skin.id);
			removedNext.current = { next };
		} catch (err) {
			toast.error(errorText(toAppError(err)));
		} finally {
			setBusy(false);
			closeSkinDialog();
		}
	};

	return (
		<AlertDialog
			open={Boolean(dialog)}
			onOpenChange={(next) => {
				if (!next && !busy) closeSkinDialog();
			}}
		>
			<AlertDialogContent
				onCloseAutoFocus={(event) => {
					const removed = removedNext.current;
					removedNext.current = null;
					if (!removed) {
						restoreFocus(event, lastFocus.current);
						return;
					}
					event.preventDefault();
					const card = removed.next
						? document.querySelector<HTMLElement>(`[data-skin-card="${removed.next}"]`)
						: null;
					(card ?? document.querySelector<HTMLElement>("[data-skins-add]"))?.focus();
				}}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle className="break-words">
						{history
							? t("skins.removeHistory.title")
							: t("skins.remove.title", {
									name: shown ? skinDisplayName(shown.skin, getAccounts().activeId) : "",
								})}
					</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{text}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus disabled={busy}>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button variant="destructive" disabled={busy} onClick={() => void confirm()}>
						{busy && <LoaderCircle className="animate-spin" />}
						{history ? t("skins.removeHistory.confirm") : t("common.remove")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function TakeOffDialog({ store }: { store: SkinsStore }) {
	const { t } = useTranslation();
	const dialog = store.dialogs.kind === "takeOff" ? store.dialogs : null;
	const lastFocus = useRef<HTMLElement | null>(null);
	useEffect(() => {
		if (dialog) lastFocus.current = dialog.returnFocus;
	}, [dialog]);

	const confirm = () => {
		const account = getAccounts().accounts.find((a) => a.id === dialog?.accountId);
		closeSkinDialog();
		if (account) void takeOffSkin(account);
	};

	return (
		<AlertDialog
			open={Boolean(dialog)}
			onOpenChange={(next) => {
				if (!next) closeSkinDialog();
			}}
		>
			<AlertDialogContent
				className="max-w-[400px] gap-6 sm:max-w-[400px] data-[size=default]:sm:max-w-[400px]"
				onCloseAutoFocus={(event) => restoreFocus(event, lastFocus.current)}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle>{t("skins.takeOff.title")}</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{t("skins.takeOff.text")}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus>{t("common.cancel")}</AlertDialogCancel>
					<Button variant="destructive" onClick={confirm}>
						{t("skins.takeOff.confirm")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

/** Текущий скин лаунчер не сохранил: замену подтверждает пользователь. */
function ReplaceDialog({ store }: { store: SkinsStore }) {
	const { t } = useTranslation();
	const dialog = store.dialogs.kind === "replace" ? store.dialogs : null;
	const lastFocus = useRef<HTMLElement | null>(null);
	useEffect(() => {
		if (dialog) lastFocus.current = dialog.returnFocus;
	}, [dialog]);

	return (
		<AlertDialog
			open={Boolean(dialog)}
			onOpenChange={(next) => {
				if (!next) closeSkinDialog();
			}}
		>
			<AlertDialogContent
				className="max-w-[400px] gap-6 sm:max-w-[400px] data-[size=default]:sm:max-w-[400px]"
				onCloseAutoFocus={(event) => restoreFocus(event, lastFocus.current)}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle>{t("skins.replace.title")}</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{t("skins.replace.text")}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus>{t("common.cancel")}</AlertDialogCancel>
					<Button
						variant="destructive"
						onClick={() => {
							closeSkinDialog();
							if (dialog) confirmReplaceSkin(dialog);
						}}
					>
						{t("skins.replace.confirm")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function DetailsDialog({ store }: { store: SkinsStore }) {
	const { t } = useTranslation();
	const dialog = useLast(store.dialogs.kind === "details" ? store.dialogs : null);
	const open = store.dialogs.kind === "details";
	const [copied, setCopied] = useState(false);
	useEffect(() => {
		if (!copied) return;
		const timer = setTimeout(() => setCopied(false), 1500);
		return () => clearTimeout(timer);
	}, [copied]);

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) closeSkinDialog();
			}}
		>
			<DialogContent className="w-[448px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[448px]">
				<DialogTitle className="pr-10">{dialog?.title}</DialogTitle>
				<DialogDescription className="sr-only">{t("common.details")}</DialogDescription>
				<pre className="mt-4 max-h-60 overflow-auto rounded-lg bg-muted p-3 font-mono text-xs/[18px] break-all whitespace-pre-wrap select-text">
					{dialog?.details}
				</pre>
				<div className="mt-4 flex justify-end">
					<Button
						variant="outline"
						onClick={() => {
							navigator.clipboard.writeText(dialog?.details ?? "").then(
								() => setCopied(true),
								() => toast.error(t("errors.fallback")),
							);
						}}
					>
						{copied ? <Check /> : <Copy />}
						{copied ? t("common.copied") : t("common.copy")}
					</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
