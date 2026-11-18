import { Check, Copy, FolderOpen, LoaderCircle, OctagonX } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
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
import { GameAccount } from "@/components/instances/game-account";
import { useShownAccount } from "@/components/instances/instance-text";
import {
	closeInstanceDialog,
	gameAccountOf,
	installInstance,
	openInstanceFolder,
	removeInstance,
	restoreInstanceDialogFocus,
	stopInstance,
	updateInstance,
	useInstanceDialogs,
	useInstances,
	type InstanceDialogs as DialogsState,
} from "@/lib/instances";
import { navigate } from "@/lib/router";

/** Один экземпляр на приложение. */
export function InstanceDialogs() {
	const dialogs = useInstanceDialogs();
	return (
		<>
			<RenameDialog dialogs={dialogs} />
			<ConfirmDialog dialogs={dialogs} />
			<CrashDialog dialogs={dialogs} />
		</>
	);
}

function useDialogInstance(dialogs: DialogsState) {
	const store = useInstances();
	return store.instances.find((i) => i.id === dialogs.instanceId) ?? null;
}

function RenameDialog({ dialogs }: { dialogs: DialogsState }) {
	const { t } = useTranslation();
	const instance = useDialogInstance(dialogs);
	const open = dialogs.open && dialogs.kind === "rename" && Boolean(instance);
	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) closeInstanceDialog();
			}}
		>
			<DialogContent
				className="w-[400px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[400px]"
				onCloseAutoFocus={restoreInstanceDialogFocus}
			>
				<DialogTitle className="pr-10">{t("instance.renameDialog.title")}</DialogTitle>
				<DialogDescription className="sr-only">{instance?.name}</DialogDescription>
				{instance && dialogs.kind === "rename" && (
					<RenameForm key={dialogs.session} id={instance.id} current={instance.name} />
				)}
			</DialogContent>
		</Dialog>
	);
}

function RenameForm({ id, current }: { id: string; current: string }) {
	const { t } = useTranslation();
	const [value, setValue] = useState(current);
	const [busy, setBusy] = useState(false);
	const [touched, setTouched] = useState(false);
	const trimmed = value.trim();
	const empty = trimmed === "";

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		setTouched(true);
		if (empty || trimmed === current || busy) return;
		setBusy(true);
		const updated = await updateInstance(id, { name: trimmed });
		setBusy(false);
		if (updated) closeInstanceDialog();
	};

	return (
		<form onSubmit={(e) => void submit(e)} noValidate>
			<Input
				autoFocus
				value={value}
				maxLength={64}
				spellCheck={false}
				autoComplete="off"
				disabled={busy}
				aria-label={t("instance.settings.name.title")}
				aria-invalid={(touched && empty) || undefined}
				aria-describedby={touched && empty ? "rename-error" : undefined}
				className="mt-5 h-10"
				onFocus={(e) => e.currentTarget.select()}
				onChange={(e) => setValue(e.target.value)}
				onBlur={() => setTouched(true)}
			/>
			{touched && empty && (
				<p
					id="rename-error"
					className="mt-2 flex items-center gap-1.5 text-[13px]/[18px] text-destructive"
				>
					<OctagonX className="shrink-0" />
					{t("createInstance.nameRequired")}
				</p>
			)}
			<div className="mt-6 flex justify-end gap-2">
				<Button type="button" variant="ghost" disabled={busy} onClick={closeInstanceDialog}>
					{t("common.cancel")}
				</Button>
				<Button type="submit" disabled={busy || empty || trimmed === current}>
					{busy && <LoaderCircle className="animate-spin" />}
					{t("common.save")}
				</Button>
			</div>
		</form>
	);
}

function ConfirmDialog({ dialogs }: { dialogs: DialogsState }) {
	const { t } = useTranslation();
	const instance = useDialogInstance(dialogs);
	const store = useInstances();
	const gameAccount = useShownAccount(dialogs.instanceId ? gameAccountOf(dialogs.instanceId, store) : null);
	const [busy, setBusy] = useState(false);
	const kind =
		dialogs.kind === "stop" || dialogs.kind === "reinstall" || dialogs.kind === "remove"
			? dialogs.kind
			: null;
	const open = dialogs.open && kind !== null && Boolean(instance);
	if (!kind) return null;
	const name = instance?.name ?? "";

	const text = {
		stop: {
			title: t("instance.stopDialog.titleNamed", { name }),
			description: gameAccount
				? t("instance.stopDialog.descriptionAs", { nick: gameAccount.username })
				: t("instance.stopDialog.description"),
			confirm: t("instance.stopDialog.confirm"),
		},
		reinstall: {
			title: t("instance.reinstallDialog.title", { name }),
			description: t("instance.reinstallDialog.description"),
			confirm: t("instance.reinstallDialog.confirm"),
		},
		remove: {
			title: t("instance.deleteDialog.title", { name }),
			description: t("instance.deleteDialog.description"),
			confirm: t("instance.deleteDialog.confirm"),
		},
	}[kind];

	const confirm = async () => {
		if (!instance) return;
		setBusy(true);
		if (kind === "stop") await stopInstance(instance.id);
		else if (kind === "reinstall") void installInstance(instance.id, "reinstall");
		else if ((await removeInstance(instance.id)) && dialogs.leaveAfterRemove) {
			navigate("/instances", { replace: true });
		}
		setBusy(false);
		closeInstanceDialog();
	};

	return (
		<AlertDialog
			open={open}
			onOpenChange={(next) => {
				if (!next && !busy) closeInstanceDialog();
			}}
		>
			<AlertDialogContent
				className="max-w-[400px] gap-6 sm:max-w-[400px] data-[size=default]:sm:max-w-[400px]"
				onCloseAutoFocus={restoreInstanceDialogFocus}
				onEscapeKeyDown={(e) => busy && e.preventDefault()}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle className="break-words">{text.title}</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{text.description}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus disabled={busy}>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button
						variant={kind === "reinstall" ? "default" : "destructive"}
						disabled={busy}
						onClick={() => void confirm()}
					>
						{busy && <LoaderCircle className="animate-spin" />}
						{text.confirm}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function CrashDialog({ dialogs }: { dialogs: DialogsState }) {
	const { t } = useTranslation();
	const open = dialogs.open && dialogs.kind === "crash";
	const crash = dialogs.crash;
	const store = useInstances();
	const crashAccount = useShownAccount(crash?.account ?? null);
	const crashName = store.instances.find((i) => i.id === dialogs.instanceId)?.name ?? null;
	const [copied, setCopied] = useState(false);
	const log = crash?.lines.join("\n") ?? "";

	useEffect(() => {
		if (!copied) return;
		const timer = setTimeout(() => setCopied(false), 1500);
		return () => clearTimeout(timer);
	}, [copied]);

	const copy = () => {
		navigator.clipboard.writeText(log).then(
			() => setCopied(true),
			() => toast.error(t("errors.fallback")),
		);
	};

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) closeInstanceDialog();
			}}
		>
			<DialogContent className="w-[520px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[520px]">
				<DialogTitle className="pr-10">{t("instance.crash.title")}</DialogTitle>
				{crashAccount && (
					<p className="mt-1 flex min-w-0 items-center text-sm text-muted-foreground">
						<GameAccount account={crashAccount} />
						{crashName && (
							<span aria-hidden className="min-w-0 truncate whitespace-pre">
								{` · ${crashName}`}
							</span>
						)}
						<span className="sr-only">
							{crashName
								? t("toasts.sessionAs", { name: crashName, nick: crashAccount.username })
								: crashAccount.username}
						</span>
					</p>
				)}
				<DialogDescription className="mt-2 text-sm text-muted-foreground">
					{crash?.exitCode != null
						? t("instance.crash.description", { code: crash.exitCode })
						: t("instance.crash.descriptionNoCode")}
				</DialogDescription>
				{log ? (
					<pre className="mt-4 max-h-60 overflow-auto rounded-lg bg-muted p-3 font-mono text-xs/[18px] break-all whitespace-pre-wrap select-text">
						{log}
					</pre>
				) : (
					<p className="mt-4 text-[13px]/[18px] text-muted-foreground">
						{crash?.recovered ? t("log.recovered") : t("instance.crash.emptyLog")}
					</p>
				)}
				<div className="mt-6 flex items-center gap-2">
					{log && (
						<Button variant="ghost" onClick={copy}>
							{copied ? <Check /> : <Copy />}
							{copied ? t("common.copied") : t("common.copy")}
						</Button>
					)}
					{dialogs.instanceId && (
						<Button
							variant="outline"
							className="ml-auto"
							onClick={() => {
								if (dialogs.instanceId) void openInstanceFolder(dialogs.instanceId);
							}}
						>
							<FolderOpen />
							{t("instance.crash.logsFolder")}
						</Button>
					)}
					<Button
						autoFocus
						className={dialogs.instanceId ? undefined : "ml-auto"}
						onClick={closeInstanceDialog}
					>
						{t("common.done")}
					</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
