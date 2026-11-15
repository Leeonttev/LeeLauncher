import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { InstanceCard } from "@/components/instances/instance-card";
import { PixelGrid } from "@/components/pixel-grid";
import type { Instance } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import { cn } from "@/lib/utils";

const easeOut = [0.22, 1, 0.36, 1] as const;

/** Колонка не шире картинки обложки (400). Запасной вариант до первого замера `PixelGrid`. */
export const GRID_CLASS = "grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-x-4 gap-y-5";

/** Сдвиг карточек — только при смене самого списка: при новом числе колонок они встают на место сразу. */
export function InstanceGrid({ instances, className }: { instances: Instance[]; className?: string }) {
	const { reduceMotion } = useAppearance();
	const [initialIds] = useState(() => new Set(instances.map((i) => i.id)));
	const order = instances.map((i) => i.id).join("|");

	return (
		// Сдвиг при появлении не должен давать полосу прокрутки; отступ 4px — под кольцо фокуса.
		<div className={cn("-m-1 overflow-clip p-1", className)}>
			<PixelGrid className={GRID_CLASS}>
				<AnimatePresence initial mode="popLayout">
					{instances.map((instance, index) => {
						const first = initialIds.has(instance.id);
						const hidden = reduceMotion
							? { opacity: 0 }
							: first
								? { opacity: 0, y: 8 }
								: { opacity: 0, y: 4 };
						return (
							<motion.div
								key={instance.id}
								layout={reduceMotion ? false : "position"}
								layoutDependency={order}
								initial={hidden}
								animate={{
									opacity: 1,
									y: 0,
									transition: {
										duration: first ? 0.22 : 0.2,
										ease: easeOut,
										delay: first && !reduceMotion ? Math.min(index, 8) * 0.03 : 0,
									},
								}}
								exit={{
									opacity: 0,
									transition: { duration: 0.15, ease: easeOut },
								}}
								transition={{ layout: { duration: 0.22, ease: easeOut } }}
							>
								<InstanceCard instance={instance} />
							</motion.div>
						);
					})}
				</AnimatePresence>
			</PixelGrid>
		</div>
	);
}
