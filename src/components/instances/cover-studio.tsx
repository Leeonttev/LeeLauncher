import { AnimatePresence, motion } from "motion/react";
import { useId, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CoverRadioGroup } from "@/components/instances/cover-picker";
import { InstanceCover as CoverImage } from "@/components/instances/instance-cover";
import { useAppearance } from "@/lib/appearance";
import type { InstanceCover } from "@/lib/covers";

/** Закреплён верх окна, а не центр: когда форма меняет высоту, поля выше изменения стоят на месте. */
export const STUDIO_DIALOG_CLASS =
	"top-[max(1rem,round(down,50%_-_288px,1px))] block h-fit w-[803px] max-w-[calc(100vw-2rem)] translate-y-0 gap-0 overflow-clip p-0 sm:max-w-[803px]";
export const STUDIO_SCROLL_CLASS =
	"max-h-[calc(100vh_-_max(1rem,50vh_-_288px)_-_1rem_-_2px)] overflow-y-auto";
export const STUDIO_GRID_CLASS = "grid grid-cols-[401px_400px]";

export function CoverStudio({
	cover,
	onCoverChange,
	eyebrow,
	title,
	media,
}: {
	cover: InstanceCover;
	onCoverChange: (cover: InstanceCover) => void;
	eyebrow: string;
	title: string;
	media?: ReactNode;
}) {
	const { t } = useTranslation();
	const { reduceMotion } = useAppearance();
	const labelId = useId();
	return (
		<div className="border-r">
			<div className="relative h-40 w-[400px] overflow-hidden bg-muted">
				<AnimatePresence initial={false}>
					<motion.div
						key={cover}
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: reduceMotion ? 0 : 0.2, ease: [0.22, 1, 0.36, 1] }}
						className="absolute inset-0"
					>
						<CoverImage scene={cover} className="size-full" />
					</motion.div>
				</AnimatePresence>
				<div
					aria-hidden
					className="pointer-events-none absolute inset-0 bg-linear-to-t from-cover-scrim/85 via-cover-scrim/60 via-32% to-transparent to-64%"
				/>
				<div aria-hidden className="absolute inset-x-5 bottom-3.5 flex items-end gap-3 text-on-cover">
					{media}
					<div className="min-w-0 flex-1">
						<p className="text-[13px]/[18px] font-medium text-on-cover/75">{eyebrow}</p>
						<p className="min-h-6 truncate text-lg/6 font-semibold">{title}</p>
					</div>
				</div>
			</div>
			<div className="px-6 pt-5 pb-6">
				<div className="flex items-center justify-between gap-3 text-[13px]/[18px]">
					<span id={labelId} className="font-medium">
						{t("createInstance.cover")}
					</span>
					<span className="truncate text-muted-foreground">{t(`covers.${cover}`)}</span>
				</div>
				<div className="mt-2">
					<CoverRadioGroup
						value={cover}
						labelledBy={labelId}
						layout="dialog"
						onChange={onCoverChange}
					/>
				</div>
			</div>
		</div>
	);
}
