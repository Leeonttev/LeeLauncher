import { cn } from "@/lib/utils";
import type { ReactNode } from "react";
const glyphs = {
	minimize: <path d="M0 5.5h10" />,
	maximize: <rect x="0.5" y="0.5" width="9" height="9" rx="1.5" />,
	restore: (
		<>
			<rect x="0.5" y="2.5" width="7" height="7" rx="1.5" />
			<path d="M2.5 2.5V2a1.5 1.5 0 0 1 1.5-1.5h4A1.5 1.5 0 0 1 9.5 2v4A1.5 1.5 0 0 1 8 7.5h-.5" />
		</>
	),
	close: <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" />,
};

/** Свои глифы 10px с линией 1px, как в Windows 11: lucide для них слишком крупный. */
export function WindowGlyph({ name }: { name: keyof typeof glyphs }) {
	return (
		<svg
			aria-hidden
			width="10"
			height="10"
			viewBox="0 0 10 10"
			fill="none"
			stroke="currentColor"
			strokeWidth="1"
			className="shrink-0"
		>
			{glyphs[name]}
		</svg>
	);
}

export function WindowButton({
	label,
	onClick,
	danger,
	children,
}: {
	label: string;
	onClick: () => void;
	danger?: boolean;
	children: ReactNode;
}) {
	return (
		<button
			type="button"
			aria-label={label}
			onClick={onClick}
			className={cn(
				"flex h-full w-[46px] items-center justify-center text-foreground/80 transition-colors duration-(--duration-fast) ease-out focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
				danger
					? "hover:bg-titlebar-close hover:text-white active:bg-titlebar-close/90 active:text-white"
					: "hover:bg-foreground/6 hover:text-foreground active:bg-foreground/10",
			)}
		>
			{children}
		</button>
	);
}
