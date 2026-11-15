import { ArrowRight, Plus } from "lucide-react";
import { motion } from "motion/react";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import type { Instance } from "@/lib/api";
import { formatDate } from "@/lib/format";
import type { HomeVersion, HomeVersionsState } from "@/lib/home";
import { compareRecent, openCreateInstanceDialog } from "@/lib/instances";
import { instancePath, navigate } from "@/lib/router";

const ROW = "flex h-18 items-center gap-4 border-b px-2 last:border-b-0";

export function HomeVersions({ state, instances }: { state: HomeVersionsState; instances: Instance[] }) {
	const { t } = useTranslation();
	const loading = state.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);

	if (state.status === "hidden") return null;
	if (loading && !showSkeleton) return null;

	return (
		<section aria-labelledby="home-versions" aria-busy={loading || undefined}>
			<h2 id="home-versions" className="text-xl/[26px] font-semibold tracking-[-0.015em]">
				{t("home.versions.title")}
			</h2>
			<ul className="mt-2">
				{loading
					? Array.from({ length: 2 }, (_, i) => (
							<li key={i} className={ROW}>
								<Skeleton className="h-[22px] w-20 shrink-0" />
								<div className="flex min-w-0 flex-1 flex-col gap-2">
									<Skeleton className="h-3.5 w-32" />
									<Skeleton className="h-3 w-56 max-w-full" />
								</div>
								<Skeleton className="h-9 w-42 shrink-0 rounded-full" />
							</li>
						))
					: state.items.map((item, index) => (
							<VersionRow
								key={item.version.id}
								item={item}
								index={index}
								instances={instances}
							/>
						))}
			</ul>
		</section>
	);
}

function VersionRow({ item, index, instances }: { item: HomeVersion; index: number; instances: Instance[] }) {
	const { t } = useTranslation();
	const titleId = useId();
	const { version, kind } = item;
	const existing = instances.filter((i) => i.gameVersion === version.id).sort(compareRecent)[0] ?? null;
	const release = kind === "release";
	const date = formatDate(version.releaseTime);

	return (
		<motion.li
			initial={{ opacity: 0, y: 8 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1], delay: index * 0.04 }}
			className={ROW}
		>
			<span className="flex h-[22px] w-20 shrink-0 items-center justify-center rounded-md bg-secondary text-xs font-medium dark:bg-accent">
				{release ? t("home.versions.release") : t("home.versions.snapshot")}
			</span>
			<div className="flex min-w-0 flex-1 flex-col">
				<span id={titleId} className="truncate text-sm/5 font-semibold">
					{release ? t("home.versions.releaseName", { version: version.id }) : version.id}
				</span>
				<span className="truncate text-[13px]/[18px] text-muted-foreground">
					{release
						? t("home.versions.latest", { date })
						: t("home.versions.snapshotHint", { date })}
				</span>
			</div>
			<span className="hidden max-w-60 min-w-0 truncate text-[13px]/[18px] text-muted-foreground @2xl:block">
				{existing ? t("home.versions.exists", { name: existing.name }) : t("home.versions.none")}
			</span>
			<Button
				variant="outline"
				aria-describedby={titleId}
				className="w-42 shrink-0"
				onClick={(e) => {
					if (existing) navigate(instancePath(existing.id));
					else
						openCreateInstanceDialog(e.currentTarget, {
							gameVersion: version.id,
							loader: "vanilla",
							showSnapshots: !release,
						});
				}}
			>
				{existing ? (
					<>
						{t("home.versions.open")}
						<ArrowRight />
					</>
				) : (
					<>
						<Plus />
						{release ? t("home.versions.create") : t("home.versions.try")}
					</>
				)}
			</Button>
		</motion.li>
	);
}
