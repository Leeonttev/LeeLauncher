import { useTranslation } from "react-i18next";
import type { Account } from "@/lib/api";

export function useAccountStatus() {
	const { t } = useTranslation();
	return (account: Account) =>
		account.needsRelogin ? t("accounts.needsRelogin") : t(`accounts.kind.${account.kind}`);
}
