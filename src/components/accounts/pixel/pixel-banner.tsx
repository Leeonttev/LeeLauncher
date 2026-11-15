import { AnimatePresence, motion } from "motion/react";
import type { ReactNode } from "react";
import cloudsDay from "@/assets/pixel/clouds-day.png";
import cloudsNight from "@/assets/pixel/clouds-night.png";
import rainDayA from "@/assets/pixel/rain-day-a.png";
import rainDayB from "@/assets/pixel/rain-day-b.png";
import rainNightA from "@/assets/pixel/rain-night-a.png";
import rainNightB from "@/assets/pixel/rain-night-b.png";
import sceneDay from "@/assets/pixel/scene-day.png";
import sceneNight from "@/assets/pixel/scene-night.png";
import sparkleDayA from "@/assets/pixel/sparkle-day-a.png";
import sparkleDayB from "@/assets/pixel/sparkle-day-b.png";
import sparkleNightA from "@/assets/pixel/sparkle-night-a.png";
import sparkleNightB from "@/assets/pixel/sparkle-night-b.png";
import spriteChoose from "@/assets/pixel/sprite-choose.png";
import spriteError from "@/assets/pixel/sprite-error.png";
import spriteMicrosoftB from "@/assets/pixel/sprite-microsoft-b.png";
import spriteMicrosoft from "@/assets/pixel/sprite-microsoft.png";
import spriteOfflineB from "@/assets/pixel/sprite-offline-b.png";
import spriteOffline from "@/assets/pixel/sprite-offline.png";
import spriteSuccessB from "@/assets/pixel/sprite-success-b.png";
import spriteSuccess from "@/assets/pixel/sprite-success.png";
import starsNight from "@/assets/pixel/stars-night.png";
import stormDay from "@/assets/pixel/storm-day.png";
import stormNight from "@/assets/pixel/storm-night.png";
import { cn } from "@/lib/utils";

export type PixelSprite = "choose" | "offline" | "microsoft" | "success" | "error";
export type PixelEffect = "sparkles" | "storm";

const SCALE = 4;
const SCENE = { width: 110 * SCALE, height: 42 * SCALE };
const SPRITE_SIZE = 16 * SCALE;

const SPRITES: Record<PixelSprite, { a: string; b?: string; motion?: string; swap?: string }> = {
	choose: { a: spriteChoose, motion: "px-bob [--px-bob:1.2s]" },
	offline: { a: spriteOffline, b: spriteOfflineB, swap: "[--px-swap:1s]" },
	microsoft: {
		a: spriteMicrosoft,
		b: spriteMicrosoftB,
		motion: "px-bob [--px-bob:1.6s]",
		swap: "[--px-swap:0.8s]",
	},
	success: { a: spriteSuccess, b: spriteSuccessB, motion: "px-jump", swap: "[--px-swap:0.8s]" },
	error: { a: spriteError, motion: "px-shake" },
};

const easeOut = [0.22, 1, 0.36, 1] as const;

function PixelImg({
	src,
	width,
	height,
	className,
}: {
	src: string;
	width: number;
	height: number;
	className?: string;
}) {
	return (
		<img
			src={src}
			alt=""
			draggable={false}
			width={width}
			height={height}
			className={cn(
				"pointer-events-none max-w-none select-none [image-rendering:pixelated]",
				className,
			)}
		/>
	);
}

function SceneLayer({ day, night, className }: { day: string; night: string; className?: string }) {
	return (
		<>
			<PixelImg src={day} {...SCENE} className={cn("absolute inset-0 dark:hidden", className)} />
			<PixelImg
				src={night}
				{...SCENE}
				className={cn("absolute inset-0 hidden dark:block", className)}
			/>
		</>
	);
}

/** Кадры строго чередуются; без анимации виден только `a`. */
function SceneFrames({ day, night }: { day: [string, string]; night: [string, string] }) {
	return (
		<>
			<SceneLayer day={day[0]} night={night[0]} className="px-swap" />
			<SceneLayer day={day[1]} night={night[1]} className="px-swap px-swap-b opacity-0" />
		</>
	);
}

function EffectLayer({
	show,
	className,
	children,
}: {
	show: boolean;
	className?: string;
	children: ReactNode;
}) {
	return (
		<AnimatePresence initial={false}>
			{show && (
				<motion.div
					className={cn("absolute inset-0", className)}
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					exit={{ opacity: 0 }}
					transition={{ duration: 0.2, ease: easeOut }}
				>
					{children}
				</motion.div>
			)}
		</AnimatePresence>
	);
}

/** Сцена и облака не перемонтируются при смене шага — меняются только спрайт и эффекты. */
export function PixelBanner({ sprite, effect }: { sprite: PixelSprite; effect: PixelEffect | null }) {
	const frames = SPRITES[sprite];
	return (
		<div aria-hidden className="relative h-[168px] w-[440px] shrink-0 overflow-hidden select-none">
			<SceneLayer day={sceneDay} night={sceneNight} />
			<PixelImg src={starsNight} {...SCENE} className="px-stars absolute inset-0 hidden dark:block" />
			<div className="px-clouds absolute inset-y-0 left-0 flex w-[880px]">
				<PixelImg src={cloudsDay} {...SCENE} className="shrink-0 dark:hidden" />
				<PixelImg src={cloudsDay} {...SCENE} className="shrink-0 dark:hidden" />
				<PixelImg src={cloudsNight} {...SCENE} className="hidden shrink-0 dark:block" />
				<PixelImg src={cloudsNight} {...SCENE} className="hidden shrink-0 dark:block" />
			</div>
			<EffectLayer show={effect === "sparkles"} className="[--px-swap:0.8s]">
				<SceneFrames day={[sparkleDayA, sparkleDayB]} night={[sparkleNightA, sparkleNightB]} />
			</EffectLayer>
			<EffectLayer show={effect === "storm"}>
				<SceneLayer day={stormDay} night={stormNight} />
				<div className="absolute inset-0 [--px-swap:0.4s]">
					<SceneFrames day={[rainDayA, rainDayB]} night={[rainNightA, rainNightB]} />
				</div>
			</EffectLayer>
			<AnimatePresence mode="popLayout" initial={false}>
				<motion.div
					key={sprite}
					className="absolute top-[80px] left-[188px] size-16"
					initial={{ opacity: 0, y: 4 }}
					animate={{ opacity: 1, y: 0, transition: { duration: 0.16, ease: easeOut } }}
					exit={{ opacity: 0, transition: { duration: 0.1, ease: easeOut } }}
				>
					<div className={cn("absolute inset-0", frames.motion, frames.swap)}>
						<PixelImg
							src={frames.a}
							width={SPRITE_SIZE}
							height={SPRITE_SIZE}
							className={cn("absolute inset-0", frames.b && "px-swap")}
						/>
						{frames.b && (
							<PixelImg
								src={frames.b}
								width={SPRITE_SIZE}
								height={SPRITE_SIZE}
								className="px-swap px-swap-b absolute inset-0 opacity-0"
							/>
						)}
					</div>
				</motion.div>
			</AnimatePresence>
		</div>
	);
}
