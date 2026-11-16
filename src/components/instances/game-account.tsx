import { AccountAvatar } from "@/components/accounts/account-avatar";
import { useAccounts } from "@/lib/accounts";
import type { GameAccountInfo } from "@/lib/api";
import { cn } from "@/lib/utils";

/** Голова и ник аккаунта, с которого идёт игра. Текст для чтения с экрана даёт родитель. */
export function GameAccount({ account, className }: { account: GameAccountInfo; className?: string }) {
	const accounts = useAccounts();
	// Аккаунт могли удалить во время игры: тогда голова по снимку из процесса.
	const saved = accounts.accounts.find((a) => a.id === account.id);
	return (
		<span aria-hidden className={cn("inline-flex min-w-0 items-center gap-1.5", className)}>
			<AccountAvatar
				account={saved ?? { ...account, needsRelogin: false }}
				size={16}
				showStatus={false}
			/>
			<span className="truncate">{account.username}</span>
		</span>
	);
}
