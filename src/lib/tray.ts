import { toast } from "sonner";
import { appApi } from "@/lib/api";
import { i18n } from "@/lib/i18n";
import { isMac } from "@/lib/platform";

let started = false;
let pending = false;

function showNotice() {
	if (!pending || document.visibilityState !== "visible") return;
	pending = false;
	toast(i18n.t(isMac ? "tray.hidden.titleMac" : "tray.hidden.title"), {
		description: i18n.t(isMac ? "tray.hidden.descriptionMac" : "tray.hidden.description"),
		duration: 10000,
	});
}

/** Окно прячется в трей уже скрытым, поэтому подсказку показываем, когда его снова откроют. */
export function initTrayNotice() {
	if (started) return;
	started = true;
	appApi
		.onHiddenToTray(({ first }) => {
			if (first) pending = true;
		})
		.catch((err: unknown) => console.warn("app-hidden-to-tray", err));
	window.addEventListener("focus", showNotice);
	document.addEventListener("visibilitychange", showNotice);
}
