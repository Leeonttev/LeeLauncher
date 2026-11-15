import { useSyncExternalStore } from "react";

export type SectionPath = "/" | "/instances" | "/catalog" | "/skins" | "/settings";
export type RoutePath = SectionPath | `/instances/${string}` | `/catalog/${string}`;

const HISTORY_LIMIT = 50;

let current: RoutePath = "/";
let history: RoutePath[] = [];
const listeners = new Set<() => void>();

function emit() {
	for (const listener of listeners) listener();
}

export function navigate(to: RoutePath, options?: { replace?: boolean }) {
	if (to === current) return;
	if (!options?.replace) history = [...history, current].slice(-HISTORY_LIMIT);
	current = to;
	emit();
}

export function goBack(fallback: RoutePath) {
	const previous = history.at(-1);
	if (previous && previous !== current) {
		history = history.slice(0, -1);
		current = previous;
		emit();
		return;
	}
	navigate(fallback, { replace: true });
}

export function instancePath(id: string): RoutePath {
	return `/instances/${encodeURIComponent(id)}`;
}

export function projectPath(source: string, id: string): RoutePath {
	return `/catalog/${encodeURIComponent(source)}/${encodeURIComponent(id)}`;
}

/** Страница сборки относится к «Сборкам», проекта — к «Каталогу». */
export function sectionOf(route: RoutePath): SectionPath {
	if (route.startsWith("/instances/")) return "/instances";
	if (route.startsWith("/catalog/")) return "/catalog";
	return route as SectionPath;
}

export function projectRouteOf(route: RoutePath): { source: string; id: string } | null {
	if (!route.startsWith("/catalog/")) return null;
	const [source, id] = route.slice("/catalog/".length).split("/");
	if (!source || !id) return null;
	return { source: decodeURIComponent(source), id: decodeURIComponent(id) };
}

export function instanceIdOf(route: RoutePath): string | null {
	if (!route.startsWith("/instances/")) return null;
	return decodeURIComponent(route.slice("/instances/".length));
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useRoute(): RoutePath {
	return useSyncExternalStore(subscribe, () => current);
}

export function getRoute(): RoutePath {
	return current;
}
