import { motion, type Transition } from "motion/react";
import { segmentSpring } from "@/components/segment";
import { useAppearance } from "@/lib/appearance";

/** Переезжает только при смене выбора: своя рамка (язык, размер окна) встаёт сразу, а `layoutId` у каждого монтирования свой, чтобы отметка не прилетала со старого места. */
export function SlidingMarker({
	group,
	value,
	className,
	transition = segmentSpring,
}: {
	group: string;
	value: string;
	className: string;
	transition?: Transition;
}) {
	const { reduceMotion } = useAppearance();
	return (
		<motion.span
			layoutId={group}
			layoutDependency={value}
			aria-hidden
			className={className}
			transition={reduceMotion ? { duration: 0 } : transition}
		/>
	);
}
