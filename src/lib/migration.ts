import { toast } from "sonner";

import { appApi } from "@/lib/api";
import { openAppFolder } from "@/lib/game-settings";
import { i18n } from "@/lib/i18n";

let started = false;

export async function initMigrationNotice() {
	if (started) return;
	started = true;
	try {
		const status = await appApi.migrationStatus();
		for (const { old, new: current } of status.conflicts) {
			toast.warning(i18n.t("migration.conflict"), {
				description: i18n.t("migration.conflictPaths", { old, current }),
				duration: Infinity,
				action: { label: i18n.t("migration.openData"), onClick: () => void openAppFolder("data") },
			});
		}
		if (status.unfinished) {
			toast.warning(i18n.t("migration.unfinished"), { duration: Infinity });
		}
	} catch (err) {
		console.warn("app_migration_status", err);
	}
}
