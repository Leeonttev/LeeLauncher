import { motion } from "motion/react";
import { useCallback, useRef, useState, type ReactNode } from "react";
import { useAppearance } from "@/lib/appearance";

const easeOut = [0.22, 1, 0.36, 1] as const;

/** Высота плавно подстраивается под содержимое; при «Меньше анимаций» — сразу. */
export function AutoHeight({ children, className }: { children: ReactNode; className?: string }) {
	const { reduceMotion } = useAppearance();
	const [height, setHeight] = useState<number | null>(null);
	const observerRef = useRef<ResizeObserver | null>(null);

	const measure = useCallback((node: HTMLDivElement | null) => {
		observerRef.current?.disconnect();
		observerRef.current = null;
		if (!node) return;
		const observer = new ResizeObserver(() => setHeight(node.offsetHeight));
		observer.observe(node);
		observerRef.current = observer;
	}, []);

	return (
		<motion.div
			className="overflow-hidden"
			initial={false}
			animate={{ height: height ?? "auto" }}
			transition={{ duration: reduceMotion ? 0 : 0.2, ease: easeOut }}
		>
			<div ref={measure} className={className}>
				{children}
			</div>
		</motion.div>
	);
}
