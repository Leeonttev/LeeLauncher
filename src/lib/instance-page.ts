import { useSyncExternalStore } from "react";
import type { ContentType } from "@/lib/api";
import { instancePath, navigate } from "@/lib/router";

export const INSTANCE_TABS = ["content", "worlds", "screenshots", "log", "settings"] as const;
export type InstanceTab = (typeof INSTANCE_TABS)[number];

export type ContentFilter = "all" | ContentType;

/** На время работы лаунчера, по сборкам. */
type PageState = { tabs: Record<string, InstanceTab>; filters: Record<string, ContentFilter> };

let state: PageState = { tabs: {}, filters: {} };
const listeners = new Set<() => void>();

function set(next: PageState) {
	state = next;
	for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

export function useInstancePageState(): PageState {
	return useSyncExternalStore(subscribe, () => state);
}

export function setInstanceTab(id: string, tab: InstanceTab) {
	if (state.tabs[id] === tab) return;
	set({ ...state, tabs: { ...state.tabs, [id]: tab } });
}

export function setContentFilter(id: string, filter: ContentFilter) {
	if (state.filters[id] === filter) return;
	set({ ...state, filters: { ...state.filters, [id]: filter } });
}

export function openInstancePage(id: string, tab?: InstanceTab) {
	if (tab) setInstanceTab(id, tab);
	navigate(instancePath(id));
}
