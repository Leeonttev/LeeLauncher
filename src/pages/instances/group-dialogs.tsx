import { Info, LoaderCircle, OctagonX } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
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
import { InstanceThumb } from "@/components/instances/instance-cover";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { toAppError } from "@/lib/errors";
import {
	closeGroupDialog,
	findGroup,
	GROUP_NAME_MAX,
	groupErrorText,
	groupKey,
	groupNameLength,
	normalizeGroupName,
	restoreGroupDialogFocus,
	useGroupDialogs,
	type GroupDialogs as DialogsState,
} from "@/lib/groups";
import { coverOf } from "@/lib/covers";
import {
	moveInstanceToGroup,
	moveInstancesToGroup,
	removeGroup,
	renameGroup,
	useInstances,
} from "@/lib/instances";
import { cn } from "@/lib/utils";

const COUNTER_FROM = 25;

export function GroupDialogs() {
	const dialogs = useGroupDialogs();
	return (
		<>
			<NameDialog dialogs={dialogs} />
			<DeleteDialog dialogs={dialogs} />
		</>
	);
}

function NameDialog({ dialogs }: { dialogs: DialogsState }) {
	const { t } = useTranslation();
	const store = useInstances();
	const kind = dialogs.kind === "new" || dialogs.kind === "rename" ? dialogs.kind : null;
	const instance =
		kind === "new" ? (store.instances.find((i) => i.id === dialogs.instanceId) ?? null) : null;
	const pick = kind === "new" && dialogs.instanceId === null;
	const open =
		dialogs.open &&
		kind !== null &&
		(kind === "rename" ? Boolean(dialogs.group) : pick ? store.instances.length > 0 : Boolean(instance));
	const [busy, setBusy] = useState<{ session: number } | null>(null);
	const saving = busy?.session === dialogs.session;

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next && !saving) closeGroupDialog();
			}}
		>
			<DialogContent
				className="w-[448px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[448px]"
				onCloseAutoFocus={restoreGroupDialogFocus}
				onEscapeKeyDown={(e) => saving && e.preventDefault()}
				onInteractOutside={(e) => saving && e.preventDefault()}
			>
				<DialogTitle className="pr-10">
					{kind === "rename" ? t("groups.renameDialog.title") : t("groups.newDialog.title")}
				</DialogTitle>
				{kind === "new" ? (
					<DialogDescription className="mt-2 text-sm text-muted-foreground">
						{pick
							? t("groups.newDialog.pickDescription")
							: t("groups.newDialog.description", { name: instance?.name ?? "" })}
					</DialogDescription>
				) : (
					<DialogDescription className="sr-only">{dialogs.group}</DialogDescription>
				)}
				{kind && (
					<NameForm
						key={dialogs.session}
						kind={kind}
						instanceId={instance?.id ?? null}
						pick={pick}
						current={kind === "rename" ? (dialogs.group ?? "") : ""}
						onBusyChange={(value) =>
							setBusy((prev) =>
								value
									? prev?.session === dialogs.session
										? prev
										: { session: dialogs.session }
									: null,
							)
						}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}

function NameForm({
	kind,
	instanceId,
	pick,
	current,
	onBusyChange,
}: {
	kind: "new" | "rename";
	instanceId: string | null;
	/** Пустых папок не бывает. */
	pick: boolean;
	current: string;
	onBusyChange: (busy: boolean) => void;
}) {
	const { t } = useTranslation();
	const store = useInstances();
	const [value, setValue] = useState(current);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const name = normalizeGroupName(value);
	const length = groupNameLength(name);
	const existing = name ? findGroup(store.instances, name) : null;
	// Та же группа в другом регистре — не «другая».
	const merge = kind === "rename" && existing !== null && groupKey(existing) !== groupKey(current);
	const unchanged = kind === "rename" && name === current;
	const valid = length >= 1 && length <= GROUP_NAME_MAX;
	const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set());
	const canSubmit = valid && !unchanged && !busy && (!pick || selected.size > 0);

	useEffect(() => onBusyChange(busy), [busy, onBusyChange]);

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		if (!canSubmit) return;
		setBusy(true);
		setError(null);
		try {
			if (kind === "new") {
				if (pick) {
					const ids = store.instances.filter((i) => selected.has(i.id)).map((i) => i.id);
					await moveInstancesToGroup(ids, existing ?? name);
				} else if (instanceId) await moveInstanceToGroup(instanceId, existing ?? name);
			} else {
				await renameGroup(current, name);
			}
			closeGroupDialog();
		} catch (err) {
			setError(groupErrorText(toAppError(err)));
			setBusy(false);
		}
	};

	const submitLabel =
		kind === "new"
			? existing
				? t("groups.newDialog.move")
				: t("groups.newDialog.create")
			: merge
				? t("groups.renameDialog.mergeButton")
				: t("groups.renameDialog.save");

	return (
		<form onSubmit={(e) => void submit(e)} noValidate>
			<Label htmlFor="group-name" className="mt-5 text-[13px]/[18px] font-medium">
				{t("groups.newDialog.label")}
			</Label>
			<Input
				id="group-name"
				autoFocus
				value={value}
				maxLength={GROUP_NAME_MAX}
				spellCheck={false}
				autoComplete="off"
				disabled={busy}
				placeholder={kind === "new" ? t("groups.newDialog.placeholder") : undefined}
				aria-invalid={Boolean(error) || undefined}
				aria-describedby={error ? "group-name-error" : undefined}
				className="mt-2 h-10"
				onFocus={(e) => e.currentTarget.select()}
				onChange={(e) => {
					setValue(e.target.value);
					setError(null);
				}}
			/>
			<div className="mt-2 flex min-h-[18px] items-start gap-3 text-[13px]/[18px]">
				<div className="min-w-0 flex-1">
					{error ? (
						<p id="group-name-error" className="flex items-start gap-1.5 text-destructive">
							<OctagonX className="mt-px shrink-0" />
							{error}
						</p>
					) : (
						(kind === "new" ? existing : merge) && (
							<p className="flex items-start gap-1.5 text-muted-foreground">
								<Info className="mt-px shrink-0" />
								{kind === "new"
									? pick
										? t("groups.newDialog.existsMany")
										: t("groups.newDialog.exists")
									: t("groups.renameDialog.merge", { name: existing })}
							</p>
						)
					)}
				</div>
				{length >= COUNTER_FROM && (
					<span className="shrink-0 text-muted-foreground tabular-nums">
						{t("groups.newDialog.counter", { count: length })}
					</span>
				)}
			</div>
			{pick && (
				<InstancePicker
					selected={selected}
					disabled={busy}
					onChange={(id, checked) =>
						setSelected((prev) => {
							const next = new Set(prev);
							if (checked) next.add(id);
							else next.delete(id);
							return next;
						})
					}
				/>
			)}
			<div className="mt-6 flex justify-end gap-2">
				<Button type="button" variant="ghost" disabled={busy} onClick={closeGroupDialog}>
					{t("common.cancel")}
				</Button>
				<Button type="submit" disabled={!canSubmit}>
					{busy && <LoaderCircle className="animate-spin" />}
					{submitLabel}
				</Button>
			</div>
		</form>
	);
}

function InstancePicker({
	selected,
	disabled,
	onChange,
}: {
	selected: ReadonlySet<string>;
	disabled: boolean;
	onChange: (id: string, checked: boolean) => void;
}) {
	const { t, i18n } = useTranslation();
	const store = useInstances();
	const instances = [...store.instances].sort((a, b) =>
		a.name.localeCompare(b.name, i18n.language, { numeric: true, sensitivity: "base" }),
	);
	return (
		<div className="mt-4">
			<div className="flex items-center justify-between gap-3 text-[13px]/[18px]">
				<span id="group-pick-label" className="font-medium">
					{t("groups.newDialog.instances")}
				</span>
				<span className="text-muted-foreground tabular-nums">
					{t("groups.newDialog.selected", { count: selected.size })}
				</span>
			</div>
			<div
				role="group"
				aria-labelledby="group-pick-label"
				className="-mx-2.5 mt-1.5 flex max-h-64 flex-col gap-0.5 overflow-y-auto"
			>
				{instances.map((instance) => {
					const id = `group-pick-${instance.id}`;
					return (
						<label
							key={instance.id}
							htmlFor={id}
							className={cn(
								"flex h-12 shrink-0 cursor-pointer items-center gap-3 rounded-md px-2.5 transition-[background-color] duration-(--duration-fast) ease-out hover:bg-accent",
								disabled && "pointer-events-none opacity-50",
							)}
						>
							<Checkbox
								id={id}
								checked={selected.has(instance.id)}
								disabled={disabled}
								onCheckedChange={(checked) => onChange(instance.id, checked === true)}
							/>
							<InstanceThumb scene={coverOf(instance)} size={32} />
							<span className="min-w-0 flex-1">
								<span className="block truncate text-sm/5 font-medium">{instance.name}</span>
								{instance.group && (
									<span className="block truncate text-[13px]/[18px] text-muted-foreground">
										{t("groups.newDialog.inFolder", { group: instance.group })}
									</span>
								)}
							</span>
						</label>
					);
				})}
			</div>
		</div>
	);
}

function DeleteDialog({ dialogs }: { dialogs: DialogsState }) {
	const { t } = useTranslation();
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<{ session: number; text: string } | null>(null);
	const open = dialogs.open && dialogs.kind === "delete" && Boolean(dialogs.group);
	const name = dialogs.group ?? "";
	const errorText = error?.session === dialogs.session ? error.text : null;

	const confirm = async () => {
		setBusy(true);
		setError(null);
		try {
			await removeGroup(name);
			closeGroupDialog();
		} catch (err) {
			setError({ session: dialogs.session, text: groupErrorText(toAppError(err)) });
		}
		setBusy(false);
	};

	return (
		<AlertDialog
			open={open}
			onOpenChange={(next) => {
				if (!next && !busy) closeGroupDialog();
			}}
		>
			<AlertDialogContent
				className="max-w-[448px] gap-6 sm:max-w-[448px] data-[size=default]:sm:max-w-[448px]"
				onCloseAutoFocus={restoreGroupDialogFocus}
				onEscapeKeyDown={(e) => busy && e.preventDefault()}
			>
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle className="break-words">
						{t("groups.deleteDialog.title", { name })}
					</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{t("groups.deleteDialog.description")}
					</AlertDialogDescription>
					{errorText && (
						<p className="flex items-start gap-1.5 text-[13px]/[18px] text-destructive">
							<OctagonX className="mt-px shrink-0" />
							{errorText}
						</p>
					)}
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus disabled={busy}>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button disabled={busy} onClick={() => void confirm()}>
						{busy && <LoaderCircle className="animate-spin" />}
						{t("groups.deleteDialog.confirm")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
