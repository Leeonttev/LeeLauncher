import { OctagonX } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { Instance } from "@/lib/api";
import { GameAccount } from "@/components/instances/game-account";
import { playedText, useSessionSeconds, useShownGameAccount } from "@/components/instances/instance-text";
import { formatDuration } from "@/lib/format";
import { progressValue, stageShortText, stageText, statusOf, useInstances } from "@/lib/instances";
import { versionLabel } from "@/lib/loaders";
import { cn } from "@/lib/utils";

/** `row` — без версии, `banner` — светлый текст на картинке. */
export type InstanceMetaVariant = "card" | "row" | "banner";

export function InstanceMeta({
	instance,
	variant,
	className,
}: {
	instance: Instance;
	variant: InstanceMetaVariant;
	className?: string;
}) {
	const { t } = useTranslation();
	const store = useInstances();
	const status = statusOf(instance, store);
	const session = useSessionSeconds(instance, store);
	const gameAccount = useShownGameAccount(instance.id, store);
	const banner = variant === "banner";

	const progressText = () => {
		const value = progressValue(instance, store);
		const stage = banner ? stageText(instance, store) : stageShortText(instance, store);
		return value === null ? stage : `${stage} · ${Math.round(value * 100)}%`;
	};

	let state: ReactNode;
	switch (status) {
		case "queued":
			state = t("install.stage.queued");
			break;
		case "installing":
			state = <span className="tabular-nums">{progressText()}</span>;
			break;
		case "launching":
			state = banner ? stageText(instance, store) : stageShortText(instance, store);
			break;
		case "running": {
			// На карточке «Играет» показывает чип на обложке.
			const duration = formatDuration(session ?? 0);
			state =
				variant === "card" ? null : (
					<span className="flex max-w-full min-w-0 items-center">
						<span
							aria-hidden={gameAccount ? true : undefined}
							className={cn(
								"inline-flex items-center gap-2",
								gameAccount ? "shrink-0" : "min-w-0",
								banner ? "text-on-cover" : "text-primary-text",
							)}
						>
							<span aria-hidden className="size-2 shrink-0 rounded-full bg-primary" />
							<span className="truncate">{t("instances.card.playing", { duration })}</span>
						</span>
						{gameAccount && (
							<>
								<span
									aria-hidden
									className={cn("shrink-0 whitespace-pre", banner && "text-on-cover/80")}
								>
									{" · "}
								</span>
								<GameAccount
									account={gameAccount}
									className={banner ? "text-on-cover/80" : "text-muted-foreground"}
								/>
								<span className="sr-only">
									{t("instances.card.playingAs", { nick: gameAccount.username, duration })}
								</span>
							</>
						)}
					</span>
				);
			break;
		}
		case "install_failed":
			state = (
				<span
					className={cn(
						"inline-flex max-w-full items-center gap-1.5",
						banner ? "text-on-cover" : "text-destructive",
					)}
				>
					{/* Обложка тёмная в обеих темах, поэтому цвет ошибки берём из тёмной. */}
					<OctagonX className={cn("shrink-0", banner && "dark text-destructive")} />
					<span className="truncate">{t("instances.card.installFailed")}</span>
				</span>
			);
			break;
		default:
			state = playedText(instance, banner);
	}

	const version = variant === "row" ? null : versionLabel(instance, banner);

	return (
		<div
			className={cn(
				"pointer-events-none relative min-w-0 truncate",
				banner ? "text-sm text-on-cover/80" : "text-[13px]/[18px] text-muted-foreground",
				className,
			)}
		>
			<AnimatePresence mode="popLayout" initial={false}>
				<motion.span
					key={status === "queued" ? "installing" : status}
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={{ duration: 0.12 }}
					className="flex min-w-0 items-center"
				>
					{version && <span className="shrink-0">{version}</span>}
					{version && state !== null && (
						<span aria-hidden className="shrink-0 whitespace-pre">
							{" · "}
						</span>
					)}
					{state !== null && <span className="min-w-0 truncate">{state}</span>}
				</motion.span>
			</AnimatePresence>
		</div>
	);
}
