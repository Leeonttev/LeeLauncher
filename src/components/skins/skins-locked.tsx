import { Shirt } from "lucide-react";
import { useTranslation } from "react-i18next";
import { CenteredState } from "@/components/centered-state";
import { Button } from "@/components/ui/button";
import { openMicrosoftLoginDialog, setActiveAccount, useAccounts } from "@/lib/accounts";

export function SkinsLocked() {
	const { t } = useTranslation();
	const { accounts } = useAccounts();
	const microsoft = accounts.find((account) => account.kind === "microsoft");
	return (
		<div className="flex min-h-0 flex-1 flex-col">
			<CenteredState
				icon={Shirt}
				title={t("skins.locked.title")}
				description={t("skins.locked.text")}
				action={
					microsoft ? (
						<Button onClick={() => void setActiveAccount(microsoft.id)}>
							{t("skins.locked.switchTo", { nick: microsoft.username })}
						</Button>
					) : (
						<Button onClick={(event) => openMicrosoftLoginDialog(event.currentTarget)}>
							{t("skins.locked.add")}
						</Button>
					)
				}
			/>
		</div>
	);
}
