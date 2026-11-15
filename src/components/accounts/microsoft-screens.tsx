import { Check, ChevronRight, Copy, ExternalLink, LoaderCircle } from "lucide-react";
import { motion } from "motion/react";
import { useEffect, useRef, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { AccountAvatar } from "@/components/accounts/account-avatar";
import { ScreenHeader } from "@/components/accounts/screen-header";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { external, LOGIN_STEPS, type Account, type LoginStep } from "@/lib/api";
import { errorText, type AppError } from "@/lib/errors";
import { cn } from "@/lib/utils";

export function MicrosoftProgress({
	step,
	cancelling,
	relogin,
	onCancel,
}: {
	step: LoginStep;
	cancelling: boolean;
	relogin: Account | null;
	onCancel: () => void;
}) {
	const { t } = useTranslation();
	const current = LOGIN_STEPS.indexOf(step);
	return (
		<div>
			<ScreenHeader
				title={relogin ? t("accounts.relogin.title") : t("accounts.microsoft.title")}
				description={
					relogin
						? t("accounts.relogin.description", { username: relogin.username })
						: t("accounts.microsoft.description")
				}
			/>
			<ol aria-live="polite" className="mx-auto flex w-fit flex-col">
				{LOGIN_STEPS.map((item, index) => {
					const state = index < current ? "done" : index === current ? "current" : "next";
					const last = index === LOGIN_STEPS.length - 1;
					return (
						<li
							key={item}
							aria-current={state === "current" ? "step" : undefined}
							className={cn("relative flex items-start gap-3", !last && "pb-3.5")}
						>
							<StepIcon state={state} />
							{!last && (
								<span
									aria-hidden
									className="absolute top-[22px] bottom-0.5 left-[9px] w-0.5 overflow-clip bg-border"
								>
									<motion.span
										className="absolute inset-0 origin-top bg-primary"
										initial={false}
										animate={{ scaleY: state === "done" ? 1 : 0 }}
										transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
									/>
								</span>
							)}
							<span
								className={cn(
									"text-sm/5",
									state === "current" && "font-medium text-foreground",
									state === "done" && "text-foreground",
									state === "next" && "text-muted-foreground",
								)}
							>
								{t(`accounts.microsoft.step.${item}`)}
							</span>
						</li>
					);
				})}
			</ol>
			{step === "waiting_browser" && (
				<p className="mt-5 text-center text-[13px]/[18px] text-muted-foreground">
					{t("accounts.microsoft.behindHint")}
				</p>
			)}
			<div className="mt-6 flex justify-end gap-2">
				<Button variant="outline" disabled={cancelling} onClick={onCancel}>
					{cancelling && <LoaderCircle className="animate-spin" />}
					{t("common.cancel")}
				</Button>
			</div>
		</div>
	);
}

function StepIcon({ state }: { state: "done" | "current" | "next" }) {
	if (state === "done") {
		return (
			<motion.span
				initial={{ opacity: 0 }}
				animate={{ opacity: 1 }}
				transition={{ duration: 0.15 }}
				className="flex size-5 shrink-0 items-center justify-center rounded-full bg-primary text-primary-foreground"
			>
				<Check className="size-3.5" strokeWidth={2.5} />
			</motion.span>
		);
	}
	if (state === "current") {
		return (
			<span className="flex size-5 shrink-0 items-center justify-center">
				<LoaderCircle className="animate-spin text-primary-text" />
			</span>
		);
	}
	return <span className="size-5 shrink-0 rounded-full border border-input" />;
}

const AUTO_CLOSE_MS = 1500;
const TICK_MS = 100;

export function MicrosoftSuccess({
	account,
	refreshed,
	containerRef,
	onDone,
}: {
	account: Account;
	refreshed: boolean;
	/** Пока курсор над диалогом или им пользуются с клавиатуры, автозакрытие на паузе. */
	containerRef: RefObject<HTMLElement | null>;
	onDone: () => void;
}) {
	const { t } = useTranslation();
	const onDoneRef = useRef(onDone);
	useEffect(() => {
		onDoneRef.current = onDone;
	});

	useEffect(() => {
		const container = containerRef.current;
		let keyboard = false;
		const onKey = () => (keyboard = true);
		container?.addEventListener("keydown", onKey);
		let elapsed = 0;
		const timer = setInterval(() => {
			if (keyboard || container?.matches(":hover")) return;
			elapsed += TICK_MS;
			if (elapsed >= AUTO_CLOSE_MS) {
				clearInterval(timer);
				onDoneRef.current();
			}
		}, TICK_MS);
		return () => {
			clearInterval(timer);
			container?.removeEventListener("keydown", onKey);
		};
	}, [containerRef]);

	return (
		<div className="flex flex-col items-center text-center">
			<div className="flex max-w-full items-center justify-center gap-3">
				<AccountAvatar account={account} size={40} surface="popover" />
				<DialogTitle className="max-w-[260px] truncate">{account.username}</DialogTitle>
			</div>
			<p className="mt-1 text-[13px]/[18px] text-muted-foreground">{t("accounts.kind.microsoft")}</p>
			<DialogDescription className="mt-1 text-sm text-foreground">
				{refreshed
					? t("accounts.microsoft.success.refreshed")
					: t("accounts.microsoft.success.added")}
			</DialogDescription>
			<Button autoFocus className="mt-6 w-full" onClick={onDone}>
				{t("common.done")}
			</Button>
		</div>
	);
}

const ERROR_LINKS: Partial<Record<string, { url: string; label: "createXbox" | "buyMinecraft" }>> = {
	no_xbox_account: { url: "https://www.xbox.com/live", label: "createXbox" },
	no_minecraft: {
		url: "https://www.minecraft.net/store/minecraft-java-bedrock-edition-pc",
		label: "buyMinecraft",
	},
};

export function MicrosoftError({
	error,
	onBack,
	onRetry,
}: {
	error: AppError;
	onBack: () => void;
	onRetry: () => void;
}) {
	const { t } = useTranslation();
	const link = ERROR_LINKS[error.code];
	return (
		<div className="flex flex-col items-center">
			<ScreenHeader
				title={t("accounts.microsoft.error.title")}
				description={
					// Общий текст `rate_limited` — про запросы вообще; при входе — свой.
					error.code === "rate_limited"
						? t("accounts.microsoft.error.rateLimited")
						: errorText(error)
				}
				flush
			/>
			{link && (
				<Button
					variant="outline"
					size="sm"
					className="mt-4"
					onClick={() =>
						void external.open(link.url).catch(() => toast.error(t("common.openLinkFailed")))
					}
				>
					{t(`accounts.microsoft.error.${link.label}`)}
					<ExternalLink />
				</Button>
			)}
			{error.details && <ErrorDetails details={error.details} />}
			<div className="mt-6 flex w-full items-center justify-end gap-2">
				<Button variant="ghost" onClick={onBack}>
					{t("common.back")}
				</Button>
				<Button autoFocus onClick={onRetry}>
					{t("common.tryAgain")}
				</Button>
			</div>
		</div>
	);
}

function ErrorDetails({ details }: { details: string }) {
	const { t } = useTranslation();
	const [copied, setCopied] = useState(false);

	useEffect(() => {
		if (!copied) return;
		const timer = setTimeout(() => setCopied(false), 1500);
		return () => clearTimeout(timer);
	}, [copied]);

	const copy = () => {
		navigator.clipboard.writeText(details).then(
			() => setCopied(true),
			() => toast.error(t("errors.fallback")),
		);
	};

	return (
		<Collapsible className="group/details mt-3 flex w-full flex-col items-center">
			<CollapsibleTrigger asChild>
				<Button variant="ghost" size="sm" className="h-7 gap-1 px-2 text-muted-foreground">
					<ChevronRight className="transition-transform duration-(--duration-base) ease-out group-data-[state=open]/details:rotate-90" />
					{t("common.details")}
				</Button>
			</CollapsibleTrigger>
			<CollapsibleContent className="-mx-2 -mb-1.5 w-[calc(100%+1rem)] overflow-clip px-2 pb-1.5 data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down">
				<pre className="mt-2 max-h-32 overflow-auto rounded-lg bg-muted p-3 font-mono text-xs/[18px] break-all whitespace-pre-wrap select-text">
					{details}
				</pre>
				<Button variant="ghost" size="sm" className="mt-2 -ml-2" onClick={copy}>
					{copied ? <Check /> : <Copy />}
					{copied ? t("common.copied") : t("common.copy")}
				</Button>
			</CollapsibleContent>
		</Collapsible>
	);
}
