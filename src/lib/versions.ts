import { useEffect, useSyncExternalStore } from "react";
import {
	LOADER_KINDS,
	versionsApi,
	type GameVersions,
	type Listed,
	type LoaderKind,
	type LoaderVersion,
} from "@/lib/api";
import { toAppError, type AppError } from "@/lib/errors";

/* Удачные ответы кэшируются на время работы; ошибки и ответы из старого кэша ядра перезапрашиваются при следующем открытии. */

export type Resource<T> =
	| { status: "loading"; value: null; error: null }
	| { status: "ready"; value: T; error: null }
	| { status: "error"; value: null; error: AppError };

const LOADING = { status: "loading", value: null, error: null } as const;

const cache = new Map<string, Resource<unknown>>();
const stale = new Set<string>();
const generations = new Map<string, number>();
const listeners = new Set<() => void>();

function emit() {
	for (const listener of listeners) listener();
}

function nextGeneration(key: string) {
	const generation = (generations.get(key) ?? 0) + 1;
	generations.set(key, generation);
	return generation;
}

function fetchInto<T>(key: string, fetcher: () => Promise<T>) {
	const generation = nextGeneration(key);
	stale.delete(key);
	cache.set(key, LOADING);
	emit();
	fetcher().then(
		(value) => {
			if (generations.get(key) !== generation) return;
			cache.set(key, { status: "ready", value, error: null });
			emit();
		},
		(err: unknown) => {
			if (generations.get(key) !== generation) return;
			cache.set(key, { status: "error", value: null, error: toAppError(err) });
			emit();
		},
	);
}

/** Список остаётся на экране, при ошибке — старый. */
function refreshInto<T>(key: string, fetcher: () => Promise<T>) {
	const generation = nextGeneration(key);
	stale.delete(key);
	fetcher().then(
		(value) => {
			if (generations.get(key) !== generation) return;
			cache.set(key, { status: "ready", value, error: null });
			emit();
		},
		(err: unknown) => {
			if (generations.get(key) !== generation) return;
			console.error(`versions ${key}`, err);
		},
	);
}

function ensure<T>(key: string, fetcher: () => Promise<T>) {
	const current = cache.get(key);
	if (!current || current.status === "error") fetchInto(key, fetcher);
	else if (current.status === "ready" && stale.has(key)) refreshInto(key, fetcher);
}

function isOffline(value: unknown) {
	return typeof value === "object" && value !== null && "offline" in value && value.offline === true;
}

/** Ошибки забываются, ответы из старого кэша ядра помечаются на обновление. */
export function revalidateVersions() {
	for (const [key, resource] of cache) {
		if (resource.status === "error") cache.delete(key);
		else if (resource.status === "ready" && isOffline(resource.value)) stale.add(key);
	}
}

/** У списка версий игры поменялись отметки «Скачана». */
export function invalidateGameVersions() {
	if (cache.get(GAME_VERSIONS_KEY)?.status !== "ready") return;
	// Кто-то подписан — обновить сразу, иначе при следующем использовании.
	if (listeners.size) refreshInto(GAME_VERSIONS_KEY, fetchGameVersions);
	else stale.add(GAME_VERSIONS_KEY);
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

function useResource<T>(key: string | null, fetcher: () => Promise<T>): Resource<T> | null {
	useEffect(() => {
		if (key) ensure(key, fetcher);
		// fetcher определяется ключом
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [key]);
	const resource = useSyncExternalStore(subscribe, () => (key ? cache.get(key) : undefined));
	if (!key) return null;
	return (resource as Resource<T> | undefined) ?? LOADING;
}

export function retryResource(key: string, fetcher: () => Promise<unknown>) {
	fetchInto(key, fetcher);
}

const GAME_VERSIONS_KEY = "minecraft";
const fetchGameVersions = () => versionsApi.minecraft(["release", "snapshot"]);

export function useGameVersions() {
	return useResource<GameVersions>(GAME_VERSIONS_KEY, fetchGameVersions);
}

export function useLoaderGameVersions(loader: LoaderKind) {
	return useResource<Listed<string>>(`loader-games:${loader}`, () =>
		versionsApi.loaderGameVersions(loader),
	);
}

/** Заранее, при открытии «Новой сборки». */
export function prefetchLoaderGameVersions() {
	for (const loader of LOADER_KINDS)
		ensure(`loader-games:${loader}`, () => versionsApi.loaderGameVersions(loader));
}

export function useLoaderVersions(loader: LoaderKind, gameVersion: string | null) {
	const key = loader !== "vanilla" && gameVersion ? `loader:${loader}:${gameVersion}` : null;
	return useResource<Listed<LoaderVersion>>(key, () =>
		versionsApi.loaderVersions(loader, gameVersion ?? ""),
	);
}

export function retryVersions(loader: LoaderKind, gameVersion: string | null) {
	if (cache.get(GAME_VERSIONS_KEY)?.status === "error") retryResource(GAME_VERSIONS_KEY, fetchGameVersions);
	const gamesKey = `loader-games:${loader}`;
	if (cache.get(gamesKey)?.status === "error")
		retryResource(gamesKey, () => versionsApi.loaderGameVersions(loader));
	if (loader !== "vanilla" && gameVersion) {
		const key = `loader:${loader}:${gameVersion}`;
		if (cache.get(key)?.status === "error")
			retryResource(key, () => versionsApi.loaderVersions(loader, gameVersion));
	}
}
