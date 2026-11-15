import { LoaderCircle, OctagonX } from "lucide-react";
import { useId, useState, type ClipboardEvent, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ScreenHeader } from "@/components/accounts/screen-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAccounts } from "@/lib/accounts";
import { accountsApi, type Account } from "@/lib/api";
import { errorText, toAppError } from "@/lib/errors";
import { cn } from "@/lib/utils";

const MAX = 16;
const MIN = 3;
const ALLOWED = /^[A-Za-z0-9_]*$/;

type FieldError = "chars" | "duplicate" | "short";

export function OfflineForm({
	onBusyChange,
	onCancel,
	onAdded,
}: {
	onBusyChange: (busy: boolean) => void;
	onCancel: () => void;
	onAdded: (account: Account) => void;
}) {
	const { t } = useTranslation();
	const { accounts } = useAccounts();
	const inputId = useId();
	const messageId = useId();
	const [value, setValue] = useState("");
	const [touched, setTouched] = useState(false);
	const [busy, setBusy] = useState(false);
	const [serverError, setServerError] = useState<string | null>(null);

	const fieldError = ((): FieldError | null => {
		if (!ALLOWED.test(value)) return "chars";
		const lower = value.toLowerCase();
		if (value && accounts.some((a) => a.kind === "offline" && a.username.toLowerCase() === lower))
			return "duplicate";
		if (touched && value.length < MIN) return "short";
		return null;
	})();

	const errorMessage = fieldError
		? {
				chars: t("accounts.offline.errorChars"),
				duplicate: t("accounts.offline.errorDuplicate"),
				short: t("accounts.offline.errorShort"),
			}[fieldError]
		: serverError;

	const change = (next: string) => {
		setValue(next.slice(0, MAX));
		setServerError(null);
	};

	const paste = (e: ClipboardEvent<HTMLInputElement>) => {
		const text = e.clipboardData.getData("text");
		const trimmed = text.trim();
		if (trimmed === text) return;
		e.preventDefault();
		const input = e.currentTarget;
		const start = input.selectionStart ?? value.length;
		const end = input.selectionEnd ?? value.length;
		change(value.slice(0, start) + trimmed + value.slice(end));
	};

	const submit = async (e: FormEvent) => {
		e.preventDefault();
		setTouched(true);
		if (busy || !value || !ALLOWED.test(value) || value.length < MIN || fieldError) return;
		setBusy(true);
		onBusyChange(true);
		try {
			const account = await accountsApi.addOffline(value);
			onAdded(account);
			toast.success(t("accounts.toast.added", { username: account.username }));
		} catch (err) {
			const error = toAppError(err);
			if (error.code === "invalid_username" || error.code === "duplicate")
				setServerError(errorText(error));
			else toast.error(errorText(error));
			setBusy(false);
			onBusyChange(false);
		}
	};

	const invalid = Boolean(errorMessage);

	return (
		<form onSubmit={(e) => void submit(e)} noValidate>
			<ScreenHeader
				title={t("accounts.offline.title")}
				description={t("accounts.offline.description")}
			/>
			<div className="flex items-baseline justify-between gap-4">
				<Label htmlFor={inputId} className="text-[13px] font-medium">
					{t("accounts.offline.label")}
				</Label>
				<span className="text-xs text-muted-foreground tabular-nums">
					{value.length}/{MAX}
				</span>
			</div>
			<Input
				id={inputId}
				autoFocus
				value={value}
				maxLength={MAX}
				spellCheck={false}
				autoComplete="off"
				autoCapitalize="off"
				placeholder={t("accounts.offline.placeholder")}
				aria-invalid={invalid || undefined}
				aria-describedby={messageId}
				readOnly={busy}
				onChange={(e) => change(e.target.value)}
				onPaste={paste}
				onBlur={() => value && setTouched(true)}
				className="mt-2 h-10"
			/>
			<p
				id={messageId}
				className={cn(
					"mt-2 flex min-h-[18px] items-start gap-1.5 text-[13px]/[18px]",
					invalid ? "text-destructive" : "text-muted-foreground",
				)}
			>
				{invalid && <OctagonX className="mt-px shrink-0" />}
				{errorMessage ?? t("accounts.offline.hint")}
			</p>
			<div className="mt-6 flex justify-end gap-2">
				<Button type="button" variant="ghost" disabled={busy} onClick={onCancel}>
					{t("common.cancel")}
				</Button>
				<Button type="submit" disabled={!value || invalid || busy}>
					{busy && <LoaderCircle className="animate-spin" />}
					{busy ? t("accounts.offline.submitting") : t("accounts.offline.submit")}
				</Button>
			</div>
		</form>
	);
}
