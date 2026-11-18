import { useEffect, useState } from "react";
import { catalogApi, type ContentSource, type ContentVersion, type Project } from "@/lib/api";
import { withDeadline } from "@/lib/catalog";

export const DEPS_LIMIT = 6;

export type DepsState = { status: "loading" } | { status: "ready"; value: Project[] } | { status: "error" };

/** Их читают и подпись под кнопкой, и блок «Зависимости» — один запрос на проект. Ошибки не кэшируются. */
const cache = new Map<string, Promise<Project | null>>();

function loadProject(id: string, source: ContentSource): Promise<Project | null> {
	const key = `${source}:${id}`;
	let promise = cache.get(key);
	if (!promise) {
		promise = withDeadline(catalogApi.project({ id, source })).catch(() => {
			cache.delete(key);
			return null;
		});
		cache.set(key, promise);
	}
	return promise;
}

export function requiredDepIds(version: ContentVersion | undefined): string[] {
	if (!version) return [];
	return version.dependencies
		.filter((d) => d.kind === "required" && d.projectId)
		.map((d) => d.projectId as string)
		.slice(0, DEPS_LIMIT);
}

export function useRequiredDeps(version: ContentVersion | undefined, source: ContentSource): DepsState {
	const key = requiredDepIds(version).join(",");
	const [state, setState] = useState<{ key: string; deps: DepsState }>({
		key: "",
		deps: { status: "ready", value: [] },
	});

	useEffect(() => {
		if (!key) return;
		let alive = true;
		Promise.all(key.split(",").map((id) => loadProject(id, source)))
			.then((list) => {
				if (!alive) return;
				setState({
					key,
					deps: { status: "ready", value: list.filter((p): p is Project => p !== null) },
				});
			})
			.catch(() => alive && setState({ key, deps: { status: "error" } }));
		return () => {
			alive = false;
		};
	}, [key, source]);

	if (!key) return { status: "ready", value: [] };
	return state.key === key ? state.deps : { status: "loading" };
}
