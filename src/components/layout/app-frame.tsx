import { AnimatePresence, motion } from "motion/react";
import { useEffect } from "react";
import { AppShell } from "@/components/layout/app-shell";
import { Titlebar } from "@/components/layout/titlebar";
import { LanguageOnboarding } from "@/components/onboarding/language-onboarding";
import { QuitDialog } from "@/components/quit-dialog";
import { UpdateGate } from "@/components/updates/update-gate";
import { UpdateNotesDialog } from "@/components/updates/update-notes-dialog";
import { useLanguage } from "@/lib/i18n";
import { initUpdates, useUpdates } from "@/lib/updates";

const easeInOut = [0.65, 0, 0.35, 1] as const;
const easeOut = [0.22, 1, 0.36, 1] as const;

/** Полоса заголовка вне `AnimatePresence`, чтобы не мигала при смене экрана. */
export function AppFrame() {
	const { language } = useLanguage();
	const onboarding = language === null;
	const updating = useUpdates().visible && !onboarding;
	const screen = onboarding ? "onboarding" : updating ? "update" : "shell";

	useEffect(() => initUpdates(), []);

	return (
		<div className="flex h-svh min-h-0 flex-col overflow-hidden bg-background">
			<Titlebar variant={screen === "shell" ? "full" : "minimal"} brand={updating} />
			<AnimatePresence mode="wait" initial={false}>
				{screen === "onboarding" ? (
					<motion.div
						key="onboarding"
						exit={{ opacity: 0, transition: { duration: 0.18, ease: easeInOut } }}
						className="flex min-h-0 flex-1 flex-col"
					>
						<LanguageOnboarding />
					</motion.div>
				) : screen === "update" ? (
					<motion.div
						key="update"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1, transition: { duration: 0.22, ease: easeOut } }}
						exit={{ opacity: 0, transition: { duration: 0.18, ease: easeInOut } }}
						className="flex min-h-0 flex-1 flex-col"
					>
						<UpdateGate />
					</motion.div>
				) : (
					<motion.div
						key="shell"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1, transition: { duration: 0.22, ease: easeOut } }}
						className="flex min-h-0 flex-1 flex-col"
					>
						<AppShell />
					</motion.div>
				)}
			</AnimatePresence>
			<UpdateNotesDialog />
			<QuitDialog />
		</div>
	);
}
