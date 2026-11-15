import { useEffect } from "react";
import { openCreateInstanceDialog } from "@/lib/instances";

function typingTarget(target: EventTarget | null) {
	if (!(target instanceof HTMLElement)) return false;
	return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

export function overlayOpen() {
	return Boolean(document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"]'));
}

/** По `event.code`: в русской раскладке `key` другой. */
export function useCreateShortcut(enabled: boolean) {
	useEffect(() => {
		if (!enabled) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.code !== "KeyN" || !event.ctrlKey || event.altKey || event.shiftKey || event.metaKey)
				return;
			if (event.repeat || typingTarget(event.target) || overlayOpen()) return;
			event.preventDefault();
			openCreateInstanceDialog(document.activeElement as HTMLElement | null);
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [enabled]);
}

export { typingTarget };
