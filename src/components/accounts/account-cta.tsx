import { User } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { openAddAccountDialog, useAccounts } from "@/lib/accounts";

const easeOut = [0.22, 1, 0.36, 1] as const;

export function AccountCta() {
	const { t } = useTranslation();
	const { status, accounts } = useAccounts();
	const visible = (status === "ready" || status === "error") && accounts.length === 0;

	return (
		<AnimatePresence initial={false}>
			{visible && (
				<motion.div
					key="cta"
					initial={{ opacity: 0, height: 0 }}
					animate={{ opacity: 1, height: "auto", transition: { duration: 0.2, ease: easeOut } }}
					exit={{ opacity: 0, height: 0, transition: { duration: 0.2, ease: easeOut } }}
					className="w-full shrink-0 overflow-hidden"
				>
					<section className="relative flex items-center gap-4 rounded-xl border border-border bg-card p-5">
						<div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary-soft text-primary-text">
							<User />
						</div>
						<div className="min-w-0">
							<h2 className="text-sm/5 font-medium">{t("accounts.cta.title")}</h2>
							<p className="text-[13px]/[18px] text-muted-foreground">
								{t("accounts.cta.description")}
							</p>
						</div>
						<Button className="ml-auto" onClick={(e) => openAddAccountDialog(e.currentTarget)}>
							{t("accounts.cta.action")}
						</Button>
					</section>
				</motion.div>
			)}
		</AnimatePresence>
	);
}
