import { type LucideIcon } from "lucide-react";
import { motion } from "motion/react";
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

type CenteredStateProps = {
	icon?: LucideIcon;
	art?: ReactNode;
	/** Картинка без плитки (например, обложка) вместо иконки. */
	media?: ReactNode;
	title: string;
	description?: string;
	tone?: "primary" | "destructive";
	badge?: ReactNode;
	action?: ReactNode;
	footer?: ReactNode;
	fill?: boolean;
	heading?: "h1" | "h2";
	className?: string;
};

export function CenteredState({
	icon: Icon,
	art,
	media,
	title,
	description,
	tone = "primary",
	badge,
	action,
	footer,
	fill = true,
	heading: Heading = "h1",
	className,
}: CenteredStateProps) {
	// У картинки центр округляется до целого пикселя: flex даёт полпикселя, и пиксель-арт размывается.
	const pixelCenter = fill && Boolean(media);
	return (
		<div
			className={cn(
				pixelCenter
					? "relative min-h-[400px] flex-1"
					: fill && "flex flex-1 items-center justify-center",
				className,
			)}
		>
			<motion.div
				initial={{ opacity: 0 }}
				animate={{ opacity: 1 }}
				transition={{ duration: 0.26, ease: [0.22, 1, 0.36, 1] }}
				className={cn(
					"flex flex-col items-center text-center",
					media ? "w-[400px] max-w-full" : "mx-auto max-w-sm",
					pixelCenter
						? "absolute top-[round(down,50%_-_40px,1px)] left-[round(down,50%,1px)] translate-x-[round(down,-50%,1px)] translate-y-[round(down,-50%,1px)]"
						: fill && "-mt-10",
					media && !pixelCenter && "mx-auto",
				)}
			>
				{media ?? (
					<div
						className={cn(
							"flex items-center justify-center",
							"rounded-2xl",
							art ? "size-24" : "size-14",
							tone === "primary"
								? "bg-primary-soft text-primary-text"
								: "bg-destructive/10 text-destructive",
						)}
					>
						{art ?? (Icon && <Icon className="size-6" />)}
					</div>
				)}
				{badge && <div className="mt-5">{badge}</div>}
				<Heading
					className={cn(
						"text-lg/6 font-semibold tracking-[-0.015em] text-balance",
						badge ? "mt-3" : "mt-5",
						media && "max-w-sm",
					)}
				>
					{title}
				</Heading>
				{description && (
					<p className={cn("mt-2 text-sm text-balance text-muted-foreground", media && "max-w-sm")}>
						{description}
					</p>
				)}
				{action && <div className="mt-6 flex flex-wrap justify-center gap-2">{action}</div>}
				{footer && <div className="mt-4 w-full">{footer}</div>}
			</motion.div>
		</div>
	);
}
