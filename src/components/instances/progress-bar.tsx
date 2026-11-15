import { cn } from "@/lib/utils";

/* На обложке полоса непрозрачная: тёмная подпись на белом и голубом читается на любой сцене. */
const COVER_TRACK = "bg-on-cover";
const COVER_FILL = "bg-primary";

type ProgressBarProps = {
	/** 0..1; `null` — неизвестен. */
	value: number | null;
	variant: "compact" | "thin" | "edge";
	tone?: "default" | "cover";
	/** Без текста — проценты. */
	label?: string;
	ariaLabel: string;
	className?: string;
};

export function ProgressBar({
	value,
	variant,
	tone = "default",
	label,
	ariaLabel,
	className,
}: ProgressBarProps) {
	const percent = value === null ? null : Math.round(value * 100);
	const compact = variant === "compact";
	const onCover = tone === "cover" || variant === "edge";
	return (
		<div
			role="progressbar"
			aria-label={ariaLabel}
			aria-valuemin={0}
			aria-valuemax={100}
			aria-valuenow={percent ?? undefined}
			aria-valuetext={label}
			className={cn(
				"relative overflow-clip",
				variant === "edge"
					? "bg-cover-scrim/45"
					: onCover
						? COVER_TRACK
						: compact
							? "bg-primary-soft"
							: "bg-border",
				variant === "compact" && "h-8 rounded-full",
				variant === "thin" && "h-1 rounded-full",
				variant === "edge" && "h-1",
				className,
			)}
		>
			{value === null ? (
				<div
					className={cn(
						"progress-indeterminate absolute inset-y-0 left-0 w-[30%]",
						compact ? (onCover ? COVER_FILL : "bg-primary/40") : "rounded-full bg-primary",
					)}
				/>
			) : (
				<div
					className={cn(
						"absolute inset-0 origin-left transition-transform duration-200 ease-linear",
						compact ? (onCover ? COVER_FILL : "bg-primary/40") : "bg-primary",
					)}
					style={{ transform: `scaleX(${value})` }}
				/>
			)}
			{compact && (
				<span
					className={cn(
						"absolute inset-0 flex items-center justify-center text-[13px]/[18px] font-medium tabular-nums",
						onCover ? "text-primary-foreground" : "text-foreground",
					)}
				>
					{label ?? (percent === null ? null : `${percent}%`)}
				</span>
			)}
		</div>
	);
}
