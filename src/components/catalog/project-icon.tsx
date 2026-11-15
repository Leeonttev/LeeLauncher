import { Braces, Globe, Package, Palette, Puzzle, Sparkles, type LucideIcon } from "lucide-react";
import { useState } from "react";
import type { CatalogProjectType } from "@/lib/api";
import { cn } from "@/lib/utils";

export type ProjectIconSize = 24 | 32 | 40 | 48 | 64 | 96;

const SIZE_CLASS: Record<ProjectIconSize, string> = {
	24: "size-6 rounded-md",
	32: "size-8 rounded-lg",
	40: "size-10 rounded-lg",
	48: "size-12 rounded-lg",
	64: "size-16 rounded-lg",
	96: "size-24 rounded-xl",
};

const TYPE_ICONS: Record<CatalogProjectType, LucideIcon> = {
	mod: Puzzle,
	resourcepack: Palette,
	shader: Sparkles,
	datapack: Braces,
	world: Globe,
	modpack: Package,
};

/** Подложка только пока грузится: прозрачные иконки не должны стоять на плашке. */
export function ProjectIcon({
	url,
	type,
	size,
	className,
}: {
	url: string | null;
	type: CatalogProjectType;
	size: ProjectIconSize;
	className?: string;
}) {
	return url ? (
		<Loaded key={url} url={url} type={type} size={size} className={className} />
	) : (
		<Fallback type={type} size={size} className={className} />
	);
}

function Loaded({
	url,
	type,
	size,
	className,
}: {
	url: string;
	type: CatalogProjectType;
	size: ProjectIconSize;
	className?: string;
}) {
	const [state, setState] = useState<"loading" | "ready" | "error">("loading");
	const [pixelated, setPixelated] = useState(false);
	if (state === "error") return <Fallback type={type} size={size} className={className} />;
	return (
		<span
			aria-hidden
			className={cn(
				"relative block shrink-0",
				state === "loading" && "bg-muted",
				SIZE_CLASS[size],
				className,
			)}
		>
			{state === "loading" && <span className="absolute inset-0 animate-pulse bg-foreground/10" />}
			<img
				src={url}
				alt=""
				loading="lazy"
				decoding="async"
				referrerPolicy="no-referrer"
				draggable={false}
				className={cn(
					"size-full object-cover transition-opacity duration-(--duration-base)",
					state === "loading" ? "opacity-0" : "opacity-100",
					pixelated && "[image-rendering:pixelated]",
				)}
				onLoad={(e) => {
					setPixelated(e.currentTarget.naturalWidth > 0 && e.currentTarget.naturalWidth < size);
					setState("ready");
				}}
				onError={() => setState("error")}
			/>
		</span>
	);
}

function Fallback({
	type,
	size,
	className,
}: {
	type: CatalogProjectType;
	size: ProjectIconSize;
	className?: string;
}) {
	const TypeIcon = TYPE_ICONS[type];
	return (
		<span
			aria-hidden
			className={cn(
				"flex shrink-0 items-center justify-center bg-muted text-muted-foreground",
				SIZE_CLASS[size],
				className,
			)}
		>
			<TypeIcon className={size >= 96 ? "size-11" : size >= 64 ? "size-6" : "size-4"} />
		</span>
	);
}
