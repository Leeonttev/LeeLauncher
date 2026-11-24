function detectMac(): boolean {
	if (typeof navigator === "undefined") return false;
	const platform =
		(navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform ??
		navigator.platform ??
		"";
	return /mac/i.test(platform) || /Macintosh|Mac OS X/.test(navigator.userAgent);
}

export const isMac = detectMac();

/** Место слева под системные кнопки окна macOS (titleBarStyle Overlay). */
export const MAC_TRAFFIC_LIGHTS = 92;

type Modifiers = Pick<KeyboardEvent, "ctrlKey" | "metaKey" | "altKey" | "shiftKey">;

/** Главный модификатор (⌘ на Mac, Ctrl иначе) без остальных. */
export function isPrimaryMod(event: Modifiers, { shift = false }: { shift?: boolean } = {}): boolean {
	const primary = isMac ? event.metaKey : event.ctrlKey;
	const other = isMac ? event.ctrlKey : event.metaKey;
	return primary && !other && !event.altKey && event.shiftKey === shift;
}

/** Нет ни главного модификатора, ни Alt: для одиночных клавиш вроде «/». */
export function noModifiers(event: Modifiers): boolean {
	return !event.ctrlKey && !event.metaKey && !event.altKey;
}

/** Удаление выбранного: Delete, а на Mac ещё ⌫ (на клавиатурах Mac нет отдельной Delete). */
export function isDeleteKey(event: Pick<KeyboardEvent, "key" | "code">): boolean {
	if (event.key === "Delete" || event.code === "Delete") return true;
	return isMac && (event.key === "Backspace" || event.code === "Backspace");
}

/** Подпись сочетания с главным модификатором: «⌘N» на Mac, «Ctrl+N» иначе. */
export function shortcutLabel(key: string): string {
	return isMac ? `⌘${key}` : `Ctrl+${key}`;
}

/** Значение для `aria-keyshortcuts`: «Meta+N» на Mac, «Control+N» иначе. */
export function ariaShortcut(key: string): string {
	return `${isMac ? "Meta" : "Control"}+${key}`;
}

/** `aria-keyshortcuts` для клавиши удаления. */
export const ariaDeleteKey = isMac ? "Delete Backspace" : "Delete";

/** «Назад»: Alt+←, на Mac ещё ⌘[ (как в Safari и Finder). */
export function isBackShortcut(
	event: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">,
): boolean {
	if (event.key === "ArrowLeft" && event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey)
		return true;
	return isMac && event.code === "BracketLeft" && isPrimaryMod(event);
}

/** `aria-keyshortcuts` для «Назад». */
export const ariaBackShortcut = isMac ? "Alt+ArrowLeft Meta+BracketLeft" : "Alt+ArrowLeft";
