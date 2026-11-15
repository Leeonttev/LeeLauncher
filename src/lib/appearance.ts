import { useSyncExternalStore } from "react";
import { flushSync } from "react-dom";
import { readStorage, writeStorage } from "@/lib/storage";

export const THEMES = ["light", "dark", "system"] as const;
export type Theme = (typeof THEMES)[number];

export type Appearance = {
	theme: Theme;
	/** `null` — пользователь не трогал переключатель, следуем системе. */
	reduceMotionSetting: boolean | null;
	resolvedTheme: "light" | "dark";
	reduceMotion: boolean;
};

const KEYS = {
	theme: "theme",
	reduceMotion: "reduce-motion",
} as const;

function readBoolean(key: string): boolean | null {
	const value = readStorage(key);
	if (value === "true") return true;
	if (value === "false") return false;
	return null;
}

function mediaQuery(query: string): MediaQueryList | null {
	try {
		return window.matchMedia(query);
	} catch {
		return null;
	}
}

const darkQuery = mediaQuery("(prefers-color-scheme: dark)");
const motionQuery = mediaQuery("(prefers-reduced-motion: reduce)");

type Stored = Pick<Appearance, "theme" | "reduceMotionSetting">;

function loadStored(): Stored {
	const theme = readStorage(KEYS.theme);
	return {
		theme: THEMES.includes(theme as Theme) ? (theme as Theme) : "system",
		reduceMotionSetting: readBoolean(KEYS.reduceMotion),
	};
}

function resolve(stored: Stored): Appearance {
	const systemDark = darkQuery?.matches ?? false;
	const systemReduce = motionQuery?.matches ?? false;
	return {
		...stored,
		resolvedTheme: stored.theme === "system" ? (systemDark ? "dark" : "light") : stored.theme,
		reduceMotion: stored.reduceMotionSetting ?? systemReduce,
	};
}

// Ключи прошлых версий.
writeStorage("accent", null);
writeStorage("sidebar-collapsed", null);

let state = resolve(loadStored());
const listeners = new Set<() => void>();

function applyToDocument(next: Appearance) {
	const root = document.documentElement;
	root.classList.toggle("dark", next.resolvedTheme === "dark");
	root.classList.toggle("light", next.resolvedTheme === "light");
	root.style.colorScheme = next.resolvedTheme;
	if (next.reduceMotion) root.dataset.reduceMotion = "";
	else delete root.dataset.reduceMotion;
}

function update(patch: Partial<Stored>) {
	state = resolve({ ...state, ...patch });
	applyToDocument(state);
	for (const listener of listeners) listener();
}

export function withViewTransition(apply: () => void) {
	if (state.reduceMotion || typeof document.startViewTransition !== "function") {
		apply();
		return;
	}
	document.startViewTransition(() => flushSync(apply));
}

function onSystemThemeChange() {
	if (state.theme === "system") withViewTransition(() => update({}));
}

function onSystemMotionChange() {
	update({});
}

applyToDocument(state);
darkQuery?.addEventListener("change", onSystemThemeChange);
motionQuery?.addEventListener("change", onSystemMotionChange);
import.meta.hot?.dispose(() => {
	darkQuery?.removeEventListener("change", onSystemThemeChange);
	motionQuery?.removeEventListener("change", onSystemMotionChange);
});

export function setTheme(theme: Theme) {
	if (theme === state.theme) return;
	writeStorage(KEYS.theme, theme);
	withViewTransition(() => update({ theme }));
}

export function setReduceMotion(reduceMotion: boolean) {
	writeStorage(KEYS.reduceMotion, String(reduceMotion));
	update({ reduceMotionSetting: reduceMotion });
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useAppearance(): Appearance {
	return useSyncExternalStore(subscribe, () => state);
}
