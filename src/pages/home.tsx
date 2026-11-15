import { ChevronRight, OctagonX, RotateCcw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AccountCta } from "@/components/accounts/account-cta";
import { CenteredState } from "@/components/centered-state";
import { HomeHero, HomeHeroEmpty } from "@/components/home/home-hero";
import { HomePopular } from "@/components/home/home-popular";
import { HomeStats } from "@/components/home/home-stats";
import { HomeVersions } from "@/components/home/home-versions";
import { InstanceCardSkeleton } from "@/components/instances/instance-card";
import { GRID_CLASS, InstanceGrid } from "@/components/instances/instance-grid";
import { PixelGrid } from "@/components/pixel-grid";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useCreateShortcut } from "@/hooks/use-create-shortcut";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { activeAccount, useAccounts } from "@/lib/accounts";
import type { Instance } from "@/lib/api";
import { errorText } from "@/lib/errors";
import { hasPlayed, useHomeVersions } from "@/lib/home";
import { compareRecent, reloadInstances, useInstances } from "@/lib/instances";
import { navigate } from "@/lib/router";
import { readStorage, writeStorage } from "@/lib/storage";

/** Ширина карточки 260 + промежуток 16. */
const CARD_STEP = 276;
const RECENT_MAX = 3;
const HOME_INSTANCE_KEY = "home-instance";

type HomePick = { id: string; at: number };

function loadPick(): HomePick | null {
	const raw = readStorage(HOME_INSTANCE_KEY);
	if (!raw) return null;
	try {
		const value: unknown = JSON.parse(raw);
		if (
			value &&
			typeof value === "object" &&
			"id" in value &&
			"at" in value &&
			typeof value.id === "string" &&
			typeof value.at === "number"
		)
			return { id: value.id, at: value.at };
	} catch {
		// Повреждённая запись — как будто выбора нет.
	}
	return null;
}

function activityAt(instance: Instance): number {
	const played = instance.lastPlayedAt ? Date.parse(instance.lastPlayedAt) : 0;
	return Math.max(played, Date.parse(instance.createdAt) || 0);
}

/** Выбор действует, пока после него не запускали и не создавали другую сборку. */
function pickValid(pick: HomePick | null, instances: Instance[]): pick is HomePick {
	if (!pick || !instances.some((i) => i.id === pick.id)) return false;
	return !instances.some((i) => i.id !== pick.id && activityAt(i) > pick.at);
}

export function HomePage() {
	const { t } = useTranslation();
	const store = useInstances();
	const accounts = useAccounts();
	const account = activeAccount(accounts);
	const loading = store.status === "idle" || store.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);
	const ctaVisible =
		(accounts.status === "ready" || accounts.status === "error") && accounts.accounts.length === 0;
	const [picked, setPicked] = useState(loadPick);

	useCreateShortcut(store.status === "ready");

	const sorted = [...store.instances].sort(compareRecent);
	const valid = pickValid(picked, store.instances);
	const hero = (valid && sorted.find((i) => i.id === picked.id)) || sorted[0] || null;
	const stale = store.status === "ready" && picked !== null && !valid;

	// Выбор устарел — убираем и ключ, чтобы он не вернулся после перезапуска.
	useEffect(() => {
		if (stale) writeStorage(HOME_INSTANCE_KEY, null);
	}, [stale]);
	const versions = useHomeVersions();
	const recent = hero ? sorted.filter((i) => i.id !== hero.id).slice(0, RECENT_MAX) : [];

	const pick = (id: string) => {
		const next = { id, at: Date.now() };
		setPicked(next);
		writeStorage(HOME_INSTANCE_KEY, JSON.stringify(next));
	};

	const greeting = account
		? t("home.greeting", { username: account.username })
		: t("home.greetingAnonymous");

	let banner = null;
	let content;
	if (store.status === "error") {
		content = (
			<>
				<h1 className="truncate text-[28px]/[34px] font-semibold tracking-[-0.025em]">{greeting}</h1>
				<AccountCta />
				<CenteredState
					icon={OctagonX}
					tone="destructive"
					heading="h2"
					fill={false}
					title={t("instances.loadFailed")}
					description={store.error ? errorText(store.error) : t("errors.fallback")}
					action={
						<Button variant="outline" onClick={() => void reloadInstances()}>
							<RotateCcw />
							{t("common.retry")}
						</Button>
					}
				/>
			</>
		);
	} else if (loading) {
		banner = showSkeleton ? (
			<div aria-busy className="h-60 shrink-0 animate-pulse rounded-2xl bg-foreground/8" />
		) : null;
		content = showSkeleton ? <HomeSkeleton /> : null;
	} else if (!hero) {
		banner = <HomeHeroEmpty greeting={greeting} secondary={ctaVisible} />;
		content = (
			<>
				<AccountCta />
				<HomeVersions state={versions} instances={store.instances} />
				<HomePopular hero={null} />
			</>
		);
	} else {
		banner = <HomeHero greeting={greeting} instance={hero} instances={sorted} onPick={pick} />;
		content = (
			<>
				<AccountCta />
				<RecentAndStats recent={recent} instances={store.instances} />
				<HomeVersions state={versions} instances={store.instances} />
				<HomePopular hero={hero} />
			</>
		);
	}

	return (
		<div className="@container flex flex-1 flex-col gap-10 px-8 pt-7 pb-10">
			{banner}
			{content}
		</div>
	);
}

function RecentHeader() {
	const { t } = useTranslation();
	return (
		<div className="flex items-center justify-between gap-4">
			<h2 id="home-recent" className="text-xl/[26px] font-semibold tracking-[-0.015em]">
				{t("home.recent")}
			</h2>
			<Button variant="more" size="sm" onClick={() => navigate("/instances")}>
				{t("home.allInstances")}
				<ChevronRight />
			</Button>
		</div>
	);
}

function RecentInstances({ instances, className }: { instances: Instance[]; className?: string }) {
	const [columns, setColumns] = useState(RECENT_MAX);
	const observerRef = useRef<ResizeObserver | null>(null);

	const measure = useCallback((node: HTMLDivElement | null) => {
		observerRef.current?.disconnect();
		observerRef.current = null;
		if (!node) return;
		const observer = new ResizeObserver(([entry]) => {
			const width = entry.contentRect.width;
			setColumns(Math.max(1, Math.min(RECENT_MAX, Math.floor((width + 16) / CARD_STEP))));
		});
		observer.observe(node);
		observerRef.current = observer;
	}, []);

	return (
		<section aria-labelledby="home-recent" className={className}>
			<RecentHeader />
			<div ref={measure} className="mt-4">
				<InstanceGrid instances={instances.slice(0, columns)} />
			</div>
		</section>
	);
}

function HomeSkeleton() {
	return (
		<div aria-busy>
			<Skeleton className="h-6 w-28" />
			<PixelGrid className={`mt-4 ${GRID_CLASS}`}>
				{Array.from({ length: RECENT_MAX }, (_, i) => (
					<InstanceCardSkeleton key={i} />
				))}
			</PixelGrid>
		</div>
	);
}

function RecentAndStats({ recent, instances }: { recent: Instance[]; instances: Instance[] }) {
	const played = hasPlayed(instances);
	if (!played) return recent.length > 0 ? <RecentInstances instances={recent} /> : null;
	if (recent.length === 0) return <HomeStats instances={instances} />;
	return (
		<div className="flex flex-col gap-10 @3xl:flex-row">
			<RecentInstances instances={recent} className="min-w-0 flex-1" />
			<HomeStats
				instances={instances}
				className="@3xl:w-[292px] @3xl:shrink-0 @3xl:border-l @3xl:pl-8"
			/>
		</div>
	);
}
