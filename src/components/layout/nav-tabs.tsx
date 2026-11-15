import { useId, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { mainNav, settingsNav, type NavItem } from "@/components/layout/nav";
import { SlidingMarker } from "@/components/sliding-marker";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { statusOf, useInstances } from "@/lib/instances";
import { navigate, sectionOf, useRoute } from "@/lib/router";
import { cn } from "@/lib/utils";

type Activity = { label: string } | null;

function useInstancesActivity(): Activity {
	const { t } = useTranslation();
	const store = useInstances();
	const playing = store.instances.find((i) => statusOf(i, store) === "running");
	return playing ? { label: t("nav.instancesPlaying", { name: playing.name }) } : null;
}

const planckTransition = { type: "spring", stiffness: 500, damping: 40, mass: 0.8 } as const;

const planckClass = "pointer-events-none absolute inset-x-3 bottom-0 z-20 h-0.5 rounded-t-[2px] bg-primary";

/** Пороги ужатия полосы заголовка; те же числа, что в container query `titlebar.tsx`: подсказки живут в портале, их решает JS. Расчёт — scripts/measure-titlebar.py. */
const TITLEBAR_BREAKPOINTS = { wordmark: 1080, compact: 940, iconsOnly: 880 } as const;

function subscribeResize(listener: () => void) {
	window.addEventListener("resize", listener);
	return () => window.removeEventListener("resize", listener);
}

function useNarrowerThan(width: number) {
	return useSyncExternalStore(subscribeResize, () => window.innerWidth < width);
}

function Tab({
	item,
	active,
	activity,
	group,
}: {
	item: NavItem;
	active: boolean;
	activity?: Activity;
	group: string;
}) {
	const { t } = useTranslation();
	const iconsOnly = useNarrowerThan(TITLEBAR_BREAKPOINTS.iconsOnly);
	const label = t(item.labelKey);
	// Подсказка нужна, только когда подпись спрятана.
	const hint = iconsOnly ? (activity?.label ?? label) : null;

	const button = (
		<button
			type="button"
			aria-current={active ? "page" : undefined}
			aria-label={activity ? activity.label : undefined}
			onClick={() => navigate(item.path)}
			className={cn(
				"relative flex h-9 items-center gap-1.5 rounded-full px-3 text-sm font-medium whitespace-nowrap transition-[color,background-color,scale] duration-(--duration-fast) ease-out hover:bg-sidebar-accent hover:text-foreground active:scale-[0.98] @max-[880px]/titlebar:px-2.5",
				active ? "text-foreground" : "text-muted-foreground",
			)}
		>
			<item.icon />
			<span className="@max-[880px]/titlebar:sr-only">{label}</span>
			{activity && <span aria-hidden className="size-2 rounded-full bg-primary" />}
		</button>
	);

	return (
		<div data-tauri-drag-region className="relative flex h-full shrink-0 items-center">
			<Tooltip>
				<TooltipTrigger asChild>{button}</TooltipTrigger>
				<TooltipContent side="bottom" sideOffset={6} hidden={!hint}>
					{hint}
				</TooltipContent>
			</Tooltip>
			{active && (
				<SlidingMarker
					group={group}
					value={item.path}
					className={planckClass}
					transition={planckTransition}
				/>
			)}
		</div>
	);
}

/** Если места не хватит, блок обрезается, а не налезает на правую часть полосы. */
export function NavTabs() {
	const { t } = useTranslation();
	const route = sectionOf(useRoute());
	const activity = useInstancesActivity();
	// После «Настроек» планка появляется сразу на месте, а не прилетает от прошлой вкладки: новый `layoutId`.
	const hasActive = mainNav.some((item) => item.path === route);
	const base = useId();
	const [shown, setShown] = useState(hasActive);
	const [generation, setGeneration] = useState(0);
	if (shown !== hasActive) {
		setShown(hasActive);
		if (hasActive) setGeneration((value) => value + 1);
	}
	const group = `${base}nav-${generation}`;
	return (
		<nav
			aria-label={t("nav.label")}
			data-tauri-drag-region
			className="ml-2 flex h-full min-w-0 gap-0.5 overflow-clip px-1.5"
		>
			{mainNav.map((item) => (
				<Tab
					key={item.path}
					item={item}
					active={route === item.path}
					activity={item.path === "/instances" ? activity : undefined}
					group={group}
				/>
			))}
		</nav>
	);
}

export function SettingsButton() {
	const { t } = useTranslation();
	const active = sectionOf(useRoute()) === settingsNav.path;
	const label = t(settingsNav.labelKey);
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={label}
					aria-current={active ? "page" : undefined}
					onClick={() => navigate(settingsNav.path)}
					className={cn(
						"relative flex size-9 items-center justify-center rounded-full transition-[color,background-color,scale] duration-(--duration-fast) ease-out active:scale-[0.98]",
						active
							? "bg-sidebar-accent text-foreground"
							: "text-muted-foreground hover:bg-sidebar-accent hover:text-foreground",
					)}
				>
					<settingsNav.icon />
				</button>
			</TooltipTrigger>
			<TooltipContent side="bottom" sideOffset={6}>
				{label}
			</TooltipContent>
		</Tooltip>
	);
}
