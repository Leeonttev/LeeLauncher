import { ArrowLeft, ChevronRight, User, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { MicrosoftError, MicrosoftProgress, MicrosoftSuccess } from "@/components/accounts/microsoft-screens";
import { MicrosoftLogo } from "@/components/accounts/microsoft-logo";
import { OfflineForm } from "@/components/accounts/offline-form";
import { PixelBanner, type PixelEffect } from "@/components/accounts/pixel/pixel-banner";
import { ScreenHeader } from "@/components/accounts/screen-header";
import { AutoHeight } from "@/components/auto-height";
import { Dialog, DialogClose, DialogContent } from "@/components/ui/dialog";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
	applyAddedAccount,
	closeAddAccountDialog,
	getAccountDialogs,
	getAccounts,
	restoreDialogFocus,
	useAccountDialogs,
} from "@/lib/accounts";
import { accountsApi, LOGIN_STEPS, type Account, type LoginStep } from "@/lib/api";
import { toAppError, type AppError } from "@/lib/errors";
import { i18n } from "@/lib/i18n";
import { cn } from "@/lib/utils";

type Screen =
	| { name: "choose" }
	| { name: "offline" }
	| { name: "microsoft" }
	| { name: "success"; account: Account; refreshed: boolean }
	| { name: "error"; error: AppError };

const easeOut = [0.22, 1, 0.36, 1] as const;

const screenVariants = {
	enter: (direction: number) => ({ opacity: 0, x: 12 * direction }),
	center: { opacity: 1, x: 0, transition: { duration: 0.18, ease: easeOut } },
	exit: (direction: number) => ({
		opacity: 0,
		x: -12 * direction,
		transition: { duration: 0.12, ease: easeOut },
	}),
};

export function AddAccountDialog() {
	const { add } = useAccountDialogs();
	return (
		<AddAccountFlow
			key={add.session}
			open={add.open}
			relogin={add.mode === "relogin" ? add.account : null}
			microsoft={add.mode === "microsoft"}
		/>
	);
}

function AddAccountFlow({
	open,
	relogin,
	microsoft,
}: {
	open: boolean;
	relogin: Account | null;
	microsoft: boolean;
}) {
	const [screen, setScreen] = useState<Screen>(
		relogin || microsoft ? { name: "microsoft" } : { name: "choose" },
	);
	const [direction, setDirection] = useState(1);
	const [step, setStep] = useState<LoginStep>("waiting_browser");
	const [cancelling, setCancelling] = useState(false);
	const [offlineBusy, setOfflineBusy] = useState(false);
	const contentRef = useRef<HTMLDivElement>(null);
	/** Номер текущей попытки входа: ответы прошлых попыток игнорируются. */
	const attemptRef = useRef(0);
	const runningRef = useRef(false);
	/** Отмену запросили крестиком или Esc: после `cancelled` закрыть диалог. */
	const closeAfterCancelRef = useRef(false);

	const go = (next: Screen, dir: 1 | -1) => {
		setDirection(dir);
		setScreen(next);
	};

	const startLogin = async () => {
		const attempt = ++attemptRef.current;
		closeAfterCancelRef.current = false;
		setCancelling(false);
		setStep("waiting_browser");
		go({ name: "microsoft" }, 1);
		const known = new Set(getAccounts().accounts.map((a) => a.id));

		const unlisten = await accountsApi
			.onLoginProgress((next) => {
				if (attemptRef.current !== attempt) return;
				setStep((prev) => (LOGIN_STEPS.indexOf(next) > LOGIN_STEPS.indexOf(prev) ? next : prev));
			})
			.catch(() => null);
		if (attemptRef.current !== attempt) {
			unlisten?.();
			return;
		}

		runningRef.current = true;
		try {
			const lang = i18n.language === "en" ? "en" : "ru";
			const account = await accountsApi.loginMicrosoft(lang);
			if (attemptRef.current !== attempt) return;
			applyAddedAccount(account);
			const refreshed = relogin ? account.id === relogin.id : known.has(account.id);
			go({ name: "success", account, refreshed }, 1);
		} catch (err) {
			if (attemptRef.current !== attempt) return;
			const error = toAppError(err);
			if (error.code === "cancelled") {
				if (closeAfterCancelRef.current || relogin) closeAddAccountDialog();
				else go({ name: "choose" }, -1);
			} else {
				go({ name: "error", error }, 1);
			}
		} finally {
			if (attemptRef.current === attempt) runningRef.current = false;
			setCancelling(false);
			unlisten?.();
		}
	};

	const cancelLogin = () => {
		if (cancelling) return;
		if (!runningRef.current) {
			attemptRef.current++;
			if (closeAfterCancelRef.current || relogin) closeAddAccountDialog();
			else go({ name: "choose" }, -1);
			return;
		}
		setCancelling(true);
		void accountsApi.cancelMicrosoftLogin().catch(() => {});
	};

	const startLoginRef = useRef(startLogin);
	useLayoutEffect(() => {
		startLoginRef.current = startLogin;
	});

	useEffect(() => {
		const attempts = attemptRef;
		const running = runningRef;
		if (relogin || microsoft) void startLoginRef.current();
		// Новое открытие с «Войти снова» само отменит прошлый вход в ядре; поздняя отмена отсюда оборвала бы уже новый.
		return () => {
			attempts.current++;
			if (!running.current) return;
			running.current = false;
			const next = getAccountDialogs().add;
			if (next.open && next.mode !== "add") return;
			void accountsApi.cancelMicrosoftLogin().catch(() => {});
		};
	}, [relogin, microsoft]);

	const requestClose = () => {
		if (screen.name === "microsoft") {
			closeAfterCancelRef.current = true;
			cancelLogin();
			return;
		}
		closeAddAccountDialog();
	};

	const backFromError = () => {
		if (relogin) closeAddAccountDialog();
		else go({ name: "choose" }, -1);
	};

	const goBack = () => {
		if (screen.name === "microsoft") cancelLogin();
		else go({ name: "choose" }, -1);
	};

	let body: ReactNode;
	let effect: PixelEffect | null = null;
	let showBack = false;
	let backDisabled = false;
	switch (screen.name) {
		case "choose":
			body = (
				<ChooseMethod
					onMicrosoft={() => void startLogin()}
					onOffline={() => {
						setOfflineBusy(false);
						go({ name: "offline" }, 1);
					}}
				/>
			);
			break;
		case "offline":
			showBack = true;
			backDisabled = offlineBusy;
			body = (
				<OfflineForm
					onBusyChange={setOfflineBusy}
					onCancel={closeAddAccountDialog}
					onAdded={(account) => {
						applyAddedAccount(account);
						closeAddAccountDialog();
					}}
				/>
			);
			break;
		case "microsoft":
			showBack = !relogin;
			backDisabled = cancelling;
			body = (
				<MicrosoftProgress
					step={step}
					cancelling={cancelling}
					relogin={relogin}
					onCancel={cancelLogin}
				/>
			);
			break;
		case "success":
			effect = "sparkles";
			body = (
				<MicrosoftSuccess
					account={screen.account}
					refreshed={screen.refreshed}
					containerRef={contentRef}
					onDone={closeAddAccountDialog}
				/>
			);
			break;
		case "error":
			effect = "storm";
			body = (
				<MicrosoftError
					error={screen.error}
					onBack={backFromError}
					onRetry={() => void startLogin()}
				/>
			);
			break;
	}

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) requestClose();
			}}
		>
			<DialogContent
				ref={contentRef}
				showCloseButton={false}
				zoom={false}
				className="w-[442px] max-w-[calc(100vw-2rem)] gap-0 overflow-clip p-0 sm:max-w-[442px]"
				onCloseAutoFocus={restoreDialogFocus}
				onInteractOutside={(e) => {
					if (screen.name === "microsoft") e.preventDefault();
				}}
			>
				<PixelBanner sprite={screen.name} effect={effect} />
				<AnimatePresence initial={false}>
					{showBack && (
						<motion.div
							key="back"
							className="absolute top-4 left-4"
							initial={{ opacity: 0 }}
							animate={{ opacity: 1 }}
							exit={{ opacity: 0 }}
							transition={{ duration: 0.12, ease: easeOut }}
						>
							<BackButton disabled={backDisabled} onClick={goBack} />
						</motion.div>
					)}
				</AnimatePresence>
				<AutoHeight className="px-6 pt-5 pb-6">
					<AnimatePresence mode="wait" initial={false} custom={direction}>
						<motion.div
							key={screen.name}
							custom={direction}
							variants={screenVariants}
							initial="enter"
							animate="center"
							exit="exit"
						>
							{body}
						</motion.div>
					</AnimatePresence>
				</AutoHeight>
				<CloseButton />
			</DialogContent>
		</Dialog>
	);
}

const overBannerButton = cn(
	"relative inline-flex size-8 items-center justify-center rounded-full bg-popover/80 text-foreground backdrop-blur-md transition-[background-color,scale] duration-(--duration-fast) hover:bg-popover active:scale-[0.98]",
	"transition-[background-color] duration-(--duration-fast) ease-out hover:bg-popover",
	"disabled:pointer-events-none disabled:opacity-50",
);

function BackButton({ disabled, onClick }: { disabled: boolean; onClick: () => void }) {
	const { t } = useTranslation();
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					className={overBannerButton}
					aria-label={t("common.back")}
					disabled={disabled}
					onClick={onClick}
				>
					<ArrowLeft />
				</button>
			</TooltipTrigger>
			<TooltipContent>{t("common.back")}</TooltipContent>
		</Tooltip>
	);
}

function CloseButton() {
	const { t } = useTranslation();
	return (
		<DialogClose className={cn(overBannerButton, "absolute top-4 right-4")}>
			<X />
			<span className="sr-only">{t("common.close")}</span>
		</DialogClose>
	);
}

function ChooseMethod({ onMicrosoft, onOffline }: { onMicrosoft: () => void; onOffline: () => void }) {
	const { t } = useTranslation();
	return (
		<div>
			<ScreenHeader title={t("accounts.add.title")} description={t("accounts.add.description")} />
			<div className="-mx-3 flex flex-col gap-1">
				<MethodButton
					autoFocus
					icon={<MicrosoftLogo className="size-[18px]" />}
					title={t("accounts.add.microsoft.title")}
					description={t("accounts.add.microsoft.description")}
					onClick={onMicrosoft}
				/>
				<MethodButton
					icon={<User />}
					title={t("accounts.add.offline.title")}
					description={t("accounts.add.offline.description")}
					onClick={onOffline}
				/>
			</div>
		</div>
	);
}

function MethodButton({
	icon,
	title,
	description,
	autoFocus,
	onClick,
}: {
	icon: ReactNode;
	title: string;
	description: string;
	autoFocus?: boolean;
	onClick: () => void;
}) {
	return (
		<button
			type="button"
			autoFocus={autoFocus}
			onClick={onClick}
			className={cn(
				"group relative flex w-full items-center gap-4 rounded-xl px-3 py-3 text-left",
				"transition-[background-color] duration-(--duration-fast) ease-out",
				"hover:bg-accent active:scale-[0.98]",
			)}
		>
			<span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary-soft text-primary-text [&_svg]:size-5">
				{icon}
			</span>
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="text-sm/5 font-medium">{title}</span>
				<span className="text-[13px]/[18px] text-muted-foreground">{description}</span>
			</span>
			<ChevronRight className="shrink-0 text-muted-foreground transition-[translate,color] duration-(--duration-fast) ease-out group-hover:translate-x-0.5 group-hover:text-foreground" />
		</button>
	);
}
