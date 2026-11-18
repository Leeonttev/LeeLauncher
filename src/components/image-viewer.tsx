import { ChevronLeft, ChevronRight, Image, LoaderCircle, X, type LucideIcon } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { Dialog as DialogPrimitive } from "radix-ui";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { onCoverButton } from "@/components/instances/play-button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useAppearance } from "@/lib/appearance";
import { cn } from "@/lib/utils";

export type ViewerItem = {
	src: string;
	title: string;
	meta?: string;
};

export function ViewerButton({
	icon: Icon,
	label,
	onClick,
	className,
}: {
	icon: LucideIcon;
	label: string;
	onClick: () => void;
	className?: string;
}) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={label}
					className={cn(onCoverButton, "size-9", className)}
					onClick={onClick}
				>
					<Icon />
				</button>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}

const VIEWER_DEADLINE = 20_000;

export function ImageViewer({
	items,
	index,
	onIndexChange,
	open,
	onOpenChange,
	actions,
	returnFocus,
}: {
	items: ViewerItem[];
	index: number;
	onIndexChange: (index: number) => void;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	actions?: ReactNode;
	returnFocus?: HTMLElement | null;
}) {
	const { t } = useTranslation();
	const { reduceMotion } = useAppearance();
	const item = items[index];
	/** Уже загруженные адреса: при листании не мигает заглушка. */
	const [loaded, setLoaded] = useState<Record<string, "ready" | "error">>({});
	const count = items.length;

	const go = (delta: number) => {
		if (count < 2) return;
		onIndexChange((index + delta + count) % count);
	};

	// Крупная картинка с недоступного сервера иначе крутилась бы бесконечно.
	const src = item?.src;
	const pending = open && src !== undefined && !loaded[src];
	useEffect(() => {
		if (!pending || src === undefined) return;
		const timer = setTimeout(
			() => setLoaded((prev) => (prev[src] ? prev : { ...prev, [src]: "error" })),
			VIEWER_DEADLINE,
		);
		return () => clearTimeout(timer);
	}, [pending, src]);

	useEffect(() => {
		if (!open) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key === "ArrowLeft") go(-1);
			if (event.key === "ArrowRight") go(1);
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	});

	if (!item) return null;
	const meta = [item.meta, count > 1 ? t("viewer.position", { index: index + 1, total: count }) : null]
		.filter(Boolean)
		.join(" · ");

	return (
		<DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
			<DialogPrimitive.Portal>
				<DialogPrimitive.Content
					aria-describedby={undefined}
					className="fixed inset-x-0 top-14 bottom-0 z-50 bg-cover-scrim/92 text-on-cover outline-none select-none data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0"
					onOpenAutoFocus={(event) => {
						// Иначе фокус на первой кнопке откроет подсказку, и первый Esc закроет только её.
						event.preventDefault();
						(event.currentTarget as HTMLElement | null)?.focus();
					}}
					onCloseAutoFocus={(event) => {
						if (!returnFocus?.isConnected) return;
						event.preventDefault();
						returnFocus.focus();
					}}
				>
					<div className="absolute inset-x-0 top-0 flex h-14 items-center gap-4 px-6">
						<div className="min-w-0 flex-1">
							<DialogPrimitive.Title className="truncate text-sm/5 font-semibold">
								{item.title}
							</DialogPrimitive.Title>
							{meta && <p className="truncate text-[13px]/[18px] text-on-cover/75">{meta}</p>}
						</div>
						<div className="flex shrink-0 items-center gap-2">
							{actions}
							<ViewerButton
								icon={X}
								label={t("viewer.close")}
								onClick={() => onOpenChange(false)}
							/>
						</div>
					</div>
					<div className="absolute inset-x-20 top-14 bottom-6">
						<AnimatePresence initial={false}>
							<motion.img
								key={item.src}
								src={item.src}
								alt={item.title}
								draggable={false}
								referrerPolicy="no-referrer"
								className="absolute inset-0 size-full object-contain"
								initial={{ opacity: 0 }}
								animate={{ opacity: loaded[item.src] === "ready" ? 1 : 0.001 }}
								exit={{ opacity: 0 }}
								transition={{ duration: reduceMotion ? 0 : 0.12 }}
								onLoad={() => setLoaded((prev) => ({ ...prev, [item.src]: "ready" }))}
								onError={() => setLoaded((prev) => ({ ...prev, [item.src]: "error" }))}
							/>
						</AnimatePresence>
						{!loaded[item.src] && (
							<div className="pointer-events-none absolute inset-0 flex items-center justify-center text-on-cover/75">
								<LoaderCircle className="size-6 animate-spin" />
							</div>
						)}
						{loaded[item.src] === "error" && (
							<div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-3 text-sm text-on-cover/75">
								<Image className="size-6" />
								{t("viewer.loadFailed")}
							</div>
						)}
					</div>
					{count > 1 && (
						<>
							<div className="absolute top-[round(down,50%_-_20px,1px)] left-5 flex">
								<ViewerButton
									icon={ChevronLeft}
									label={t("viewer.prev")}
									className="size-10"
									onClick={() => go(-1)}
								/>
							</div>
							<div className="absolute top-[round(down,50%_-_20px,1px)] right-5 flex">
								<ViewerButton
									icon={ChevronRight}
									label={t("viewer.next")}
									className="size-10"
									onClick={() => go(1)}
								/>
							</div>
						</>
					)}
				</DialogPrimitive.Content>
			</DialogPrimitive.Portal>
		</DialogPrimitive.Root>
	);
}
