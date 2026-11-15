import { Clock, Play, Star, type LucideIcon } from "lucide-react";
import { motion } from "motion/react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { Instance } from "@/lib/api";
import { formatDuration, formatWhenCapitalized } from "@/lib/format";
import { useInstances, useNow } from "@/lib/instances";
import { instancePath, navigate } from "@/lib/router";
import { cn } from "@/lib/utils";

/** Раскладка по своей ширине: в колонке — столбиком, от 576px — в ряд. */
export function HomeStats({ instances, className }: { instances: Instance[]; className?: string }) {
	const { t } = useTranslation();
	const store = useInstances();
	const playing = instances.find((i) => store.processes[i.id] !== undefined) ?? null;
	// Раз в минуту: «Сегодня» сменится на «Вчера» и без новых данных.
	const now = useNow(true);
	const total = instances.reduce((sum, i) => sum + i.playtime, 0);
	const last = instances.reduce<string | null>(
		(latest, i) =>
			i.lastPlayedAt && (!latest || Date.parse(i.lastPlayedAt) > Date.parse(latest))
				? i.lastPlayedAt
				: latest,
		null,
	);
	const favorite =
		instances.length > 1
			? instances.reduce<Instance | null>(
					(best, i) => (i.playtime > (best?.playtime ?? 0) ? i : best),
					null,
				)
			: null;

	return (
		<motion.section
			aria-labelledby="home-your-game"
			initial={{ opacity: 0, y: 8 }}
			animate={{ opacity: 1, y: 0 }}
			transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1], delay: 0.04 }}
			className={cn("@container/stats min-w-0", className)}
		>
			{/* Высота как у шапки «Недавних», чтобы заголовки стояли на одной линии. */}
			<div className="flex h-8 items-center">
				<h2 id="home-your-game" className="text-xl/[26px] font-semibold tracking-[-0.015em]">
					{t("home.yourGame")}
				</h2>
			</div>
			<ul className="mt-4 grid gap-x-8 gap-y-1 @xl/stats:auto-cols-fr @xl/stats:grid-flow-col">
				<Fact icon={Clock} value={formatDuration(total)} hint={t("home.stats.playtimeHint")} />
				{playing ? (
					<Fact
						icon={Play}
						value={t("home.stats.playingNow")}
						hint={t("home.stats.playingHint", { name: playing.name })}
					/>
				) : (
					<Fact
						icon={Play}
						value={last ? formatWhenCapitalized(last, now) : t("time.never")}
						hint={t("home.stats.lastPlayedHint")}
					/>
				)}
				{favorite && (
					<li className="min-w-0">
						<button
							type="button"
							aria-label={t("home.stats.favoriteAria", { name: favorite.name })}
							className="relative -mx-2 flex h-14 w-[calc(100%+1rem)] items-center gap-3 rounded-lg border border-transparent px-2 text-left transition-[background-color] duration-(--duration-fast) ease-out hover:bg-accent"
							onClick={() => navigate(instancePath(favorite.id))}
						>
							<FactBody
								icon={Star}
								value={<span className="text-primary-text">{favorite.name}</span>}
								hint={t("home.stats.favoriteHint")}
							/>
						</button>
					</li>
				)}
			</ul>
		</motion.section>
	);
}

function Fact({ icon, value, hint }: { icon: LucideIcon; value: ReactNode; hint: string }) {
	return (
		<li className="flex h-14 min-w-0 items-center gap-3">
			<FactBody icon={icon} value={value} hint={hint} />
		</li>
	);
}

function FactBody({ icon: Icon, value, hint }: { icon: LucideIcon; value: ReactNode; hint: string }) {
	return (
		<>
			<span
				aria-hidden
				className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary-soft text-primary-text"
			>
				<Icon className="size-5" />
			</span>
			<span className="flex min-w-0 flex-col">
				<span className="truncate text-sm/5 font-semibold">{value}</span>
				<span className="truncate text-[13px]/[18px] text-muted-foreground">{hint}</span>
			</span>
		</>
	);
}
