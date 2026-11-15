import type { CSSProperties } from "react";
import { cn } from "@/lib/utils";
import { APP_NAME } from "@/lib/app-info";
import { ICON_MARK, ICON_TILE, LOCKUP, SPARK } from "@/components/brand/identity-data";

/** Часть [0] с `accent` красится в `--brand`, остальное — `currentColor`. */
export function Spark({
	accent = false,
	size = 22,
	className,
	style,
}: {
	accent?: boolean;
	size?: number;
	className?: string;
	style?: CSSProperties;
}) {
	return (
		<svg
			viewBox={`0 0 ${SPARK.width} ${SPARK.height}`}
			height={size}
			width={(size * SPARK.width) / SPARK.height}
			fill="currentColor"
			aria-hidden
			className={cn("shrink-0", className)}
			style={style}
		>
			{SPARK.paths.map((d, i) => (
				<path key={i} d={d} fill={i === 0 && accent ? "var(--brand)" : undefined} />
			))}
		</svg>
	);
}

/** «Искра + leelauncher» в кривых, без шрифта. `size` — высота. */
export function Lockup({
	size = 22,
	className,
	style,
}: {
	size?: number;
	className?: string;
	style?: CSSProperties;
}) {
	return (
		<svg
			viewBox={`0 0 ${LOCKUP.width} ${LOCKUP.height}`}
			height={size}
			width={(size * LOCKUP.width) / LOCKUP.height}
			fill="currentColor"
			role="img"
			aria-label={APP_NAME}
			className={cn("shrink-0", className)}
			style={style}
		>
			{LOCKUP.spark.map((d, i) => (
				<path key={`s${i}`} d={d} fill={i === 0 ? "var(--brand)" : undefined} />
			))}
			{LOCKUP.word.map((d, i) => (
				<path key={`w${i}`} d={d} fill={i === 1 ? "var(--brand)" : undefined} />
			))}
		</svg>
	);
}

export function AppIcon({ size = 64, className }: { size?: number; className?: string }) {
	const share = size <= 48 ? ICON_MARK.small : ICON_MARK.large;
	const h = 100 * share;
	const w = (h * SPARK.width) / SPARK.height;
	const k = h / SPARK.height;
	return (
		<svg
			viewBox="0 0 100 100"
			width={size}
			height={size}
			aria-hidden
			className={cn("shrink-0", className)}
		>
			<path d={ICON_TILE} fill="var(--brand-tile)" />
			<g
				transform={`translate(${(100 - w) / 2} ${(100 - h) / 2}) scale(${k})`}
				fill="var(--brand-mark)"
			>
				{SPARK.paths.map((d, i) => (
					<path key={i} d={d} fill={i === 0 ? "var(--brand-icon)" : undefined} />
				))}
			</g>
		</svg>
	);
}
