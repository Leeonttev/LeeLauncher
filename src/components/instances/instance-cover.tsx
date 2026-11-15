import type { CSSProperties } from "react";
import { useAppearance } from "@/lib/appearance";
import { COVER_SIZE, coverUrls, type CoverVariant, type InstanceCover as Scene } from "@/lib/covers";
import { cn } from "@/lib/utils";

export const coverFrame = "overflow-clip rounded-xl";

type InstanceCoverProps = {
	scene: Scene;
	variant?: CoverVariant;
	/** Только целый масштаб, иначе пиксели станут разной ширины. */
	scale?: 2 | 4;
	/** Параллакс при наведении на ближайший `group`. Только при ×4. */
	parallax?: boolean;
	className?: string;
};

/** Центр округляется до целого пикселя: колонки бывают дробной ширины, а `translate(-50%)` размыл бы края. */
export function InstanceCover({
	scene,
	variant = "card",
	scale = 4,
	parallax = false,
	className,
}: InstanceCoverProps) {
	const { reduceMotion } = useAppearance();
	const { back, front } = coverUrls(scene, variant);
	const width = COVER_SIZE[variant].width * scale;
	const height = COVER_SIZE[variant].height * scale;
	const shift = parallax && scale === 4 && !reduceMotion;
	const style: CSSProperties = {
		width,
		height,
		left: `round(down, 50% - ${width / 2}px, 1px)`,
		top: `round(down, 50% - ${height / 2}px, 1px)`,
	};
	const layer = cn(
		"pointer-events-none absolute max-w-none select-none [image-rendering:pixelated]",
		shift && "transition-transform duration-(--duration-slow) ease-out",
	);

	return (
		<div aria-hidden className={cn("relative overflow-clip bg-muted", className)}>
			<img
				src={back}
				alt=""
				draggable={false}
				style={style}
				className={cn(
					layer,
					shift && "group-hover:-translate-x-1 group-has-[:focus-visible]:-translate-x-1",
				)}
			/>
			<img
				src={front}
				alt=""
				draggable={false}
				style={style}
				className={cn(
					layer,
					shift && "group-hover:translate-x-1 group-has-[:focus-visible]:translate-x-1",
				)}
			/>
		</div>
	);
}

const THUMB_CLASS = {
	24: "size-6 rounded-sm",
	32: "size-8 rounded-sm",
	40: "size-10 rounded-md",
	48: "size-12 rounded-md",
} as const;

/** Квадратный центр обложки ×2 для списков и поповеров. */
export function InstanceThumb({
	scene,
	size,
	className,
}: {
	scene: Scene;
	size: 24 | 32 | 40 | 48;
	className?: string;
}) {
	return <InstanceCover scene={scene} scale={2} className={cn("shrink-0", THUMB_CLASS[size], className)} />;
}
