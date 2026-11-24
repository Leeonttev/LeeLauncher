import { ArrowRight, ChevronsUpDown, Plus } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { stretchedLink } from "@/components/instances/instance-link";
import { InstanceCover, InstanceThumb } from "@/components/instances/instance-cover";
import { InstanceMeta } from "@/components/instances/instance-meta";
import { onCoverButton, PlayButton } from "@/components/instances/play-button";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { Instance } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import { coverOf } from "@/lib/covers";
import { openCreateInstanceDialog } from "@/lib/instances";
import { versionLabel } from "@/lib/loaders";
import { instancePath, navigate } from "@/lib/router";
import { cn } from "@/lib/utils";
import { ariaShortcut, shortcutLabel } from "@/lib/platform";

const bannerFocusRing =
	"has-[button[data-link]:focus-visible]:outline-2 has-[button[data-link]:focus-visible]:outline-offset-2 has-[button[data-link]:focus-visible]:outline-ring";

const scrim =
	"pointer-events-none absolute inset-0 bg-linear-to-r from-cover-scrim/80 via-cover-scrim/35 to-transparent";

export function HomeHero({
	greeting,
	instance,
	instances,
	onPick,
}: {
	greeting: string;
	instance: Instance;
	instances: Instance[];
	onPick: (id: string) => void;
}) {
	const { reduceMotion } = useAppearance();

	const fade = { duration: reduceMotion ? 0 : 0.2 };

	// Текст не позиционирован, чтобы `::after` ссылки растягивался на весь баннер; обложка лежит под ним (`-z-10` в `isolate`).
	return (
		<section
			aria-label={instance.name}
			className={cn(
				"group relative isolate flex h-60 shrink-0 items-end gap-3 overflow-hidden rounded-2xl bg-muted px-9 pb-9",
				bannerFocusRing,
			)}
		>
			<AnimatePresence initial={false}>
				<motion.div
					key={instance.id}
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={fade}
					className="absolute inset-0 -z-10"
				>
					<InstanceCover
						scene={coverOf(instance)}
						variant="wide"
						parallax
						className="absolute inset-0"
					/>
					<div aria-hidden className={scrim} />
				</motion.div>
			</AnimatePresence>

			<motion.div
				key={instance.id}
				initial={{ opacity: 0 }}
				animate={{ opacity: 1 }}
				transition={fade}
				className="min-w-0 flex-1"
			>
				<h1 className="truncate text-sm font-medium text-on-cover/80">{greeting}</h1>
				<button
					type="button"
					data-link
					// Без позиционирования, `filter` и `transform`: иначе `::after` ссылки сожмётся до кнопки.
					className={cn("block max-w-full text-on-cover", stretchedLink)}
					onClick={() => navigate(instancePath(instance.id))}
				>
					<span className="block truncate text-[28px]/[34px] font-semibold tracking-[-0.025em]">
						{instance.name}
					</span>
				</button>
				<InstanceMeta instance={instance} variant="banner" className="mt-1" />
			</motion.div>

			{instances.length > 1 && (
				<OtherInstance current={instance} instances={instances} onPick={onPick} />
			)}
			<PlayButton instance={instance} size="banner" />
		</section>
	);
}

function OtherInstance({
	current,
	instances,
	onPick,
}: {
	current: Instance;
	instances: Instance[];
	onPick: (id: string) => void;
}) {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	const triggerRef = useRef<HTMLButtonElement>(null);

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					ref={triggerRef}
					type="button"
					className={cn(
						onCoverButton,
						"relative z-10 h-11 px-5 text-[15px] aria-expanded:bg-on-cover/26",
					)}
				>
					<ChevronsUpDown />
					{t("home.hero.otherInstance")}
				</button>
			</PopoverTrigger>
			<PopoverContent align="end" sideOffset={8} className="w-72 p-1.5">
				<div className="-mx-1.5 -mt-1.5 max-h-72 overflow-y-auto p-1.5">
					{instances.map((instance) => {
						const selected = instance.id === current.id;
						return (
							<button
								key={instance.id}
								type="button"
								aria-current={selected || undefined}
								className="relative flex h-12 w-full items-center gap-3 rounded-lg border border-transparent px-2 text-left transition-colors duration-(--duration-fast) ease-out hover:bg-accent aria-[current=true]:bg-accent"
								onClick={() => {
									onPick(instance.id);
									setOpen(false);
								}}
							>
								<InstanceThumb scene={coverOf(instance)} size={32} />
								<span className="min-w-0 flex-1">
									<span className="block truncate text-sm/5 font-semibold">
										{instance.name}
									</span>
									<span className="block truncate text-[13px]/[18px] text-muted-foreground">
										{versionLabel(instance)}
									</span>
								</span>
							</button>
						);
					})}
				</div>
				<div className="border-t pt-1.5">
					<Button
						variant="ghost"
						size="sm"
						className="w-full justify-between"
						onClick={() => {
							setOpen(false);
							navigate("/instances");
						}}
					>
						{t("home.allInstances")}
						<ArrowRight />
					</Button>
				</div>
			</PopoverContent>
		</Popover>
	);
}

/** Если виден призыв добавить аккаунт, кнопка здесь не главная. */
export function HomeHeroEmpty({ greeting, secondary }: { greeting: string; secondary: boolean }) {
	const { t } = useTranslation();
	const create = (target: HTMLElement) => openCreateInstanceDialog(target);
	const label = (
		<>
			<Plus />
			{t("instances.create")}
		</>
	);

	return (
		<section
			aria-labelledby="home-hero-empty"
			className="relative isolate flex h-60 shrink-0 items-end gap-6 overflow-hidden rounded-2xl bg-muted px-9 pb-9"
		>
			<InstanceCover scene="meadow" variant="wide" className="absolute inset-0 -z-10" />
			<div aria-hidden className={cn(scrim, "-z-10")} />
			<div className="min-w-0 flex-1">
				<h1 className="truncate text-sm font-medium text-on-cover/80">{greeting}</h1>
				<h2
					id="home-hero-empty"
					className="truncate text-[28px]/[34px] font-semibold tracking-[-0.025em] text-on-cover"
				>
					{t("home.hero.emptyTitle")}
				</h2>
				<p className="mt-1 text-sm text-on-cover/80">{t("home.hero.emptyDescription")}</p>
			</div>
			<Tooltip>
				<TooltipTrigger asChild>
					{secondary ? (
						<button
							type="button"
							aria-keyshortcuts={ariaShortcut("N")}
							className={cn(onCoverButton, "h-11 px-6 text-[15px]")}
							onClick={(e) => create(e.currentTarget)}
						>
							{label}
						</button>
					) : (
						<Button
							aria-keyshortcuts={ariaShortcut("N")}
							size="xl"
							onClick={(e) => create(e.currentTarget)}
						>
							{label}
						</Button>
					)}
				</TooltipTrigger>
				<TooltipContent>{t("instances.createShortcut", { keys: shortcutLabel("N") })}</TooltipContent>
			</Tooltip>
		</section>
	);
}
