import { useEffect, useState, type ComponentProps } from "react";
import { cn } from "@/lib/utils";

const MIN_COLUMN = 260;
const GAP = 16;

/** Колонки целой ширины: у `1fr` она дробная, и пиксельные обложки размываются. Остаток в несколько px — справа. */
export function PixelGrid({
	columns,
	maxColumn,
	minColumn = MIN_COLUMN,
	maxColumns,
	minColumns,
	gap = GAP,
	onColumnsChange,
	className,
	style,
	...props
}: ComponentProps<"div"> & {
	columns?: number;
	maxColumn?: number;
	minColumn?: number;
	maxColumns?: number;
	/** Даже если колонка выйдет уже `minColumn`. */
	minColumns?: number;
	gap?: number;
	onColumnsChange?: (count: number) => void;
}) {
	const [node, setNode] = useState<HTMLDivElement | null>(null);
	const [template, setTemplate] = useState<string | null>(null);

	useEffect(() => {
		if (!node) return;
		const observer = new ResizeObserver(([entry]) => {
			const width = entry.contentRect.width;
			let count = columns ?? Math.max(1, Math.floor((width + gap) / (minColumn + gap)));
			if (maxColumns) count = Math.min(count, maxColumns);
			if (minColumns) count = Math.max(count, minColumns);
			let column = Math.floor((width - gap * (count - 1)) / count);
			if (maxColumn) column = Math.min(column, maxColumn);
			setTemplate(`repeat(${count}, ${Math.max(0, column)}px)`);
			onColumnsChange?.(count);
		});
		observer.observe(node);
		return () => observer.disconnect();
	}, [node, columns, maxColumn, minColumn, maxColumns, minColumns, gap, onColumnsChange]);

	return (
		<div
			ref={setNode}
			className={cn("grid gap-x-4 gap-y-5", className)}
			style={{
				...style,
				...(gap !== GAP && { columnGap: gap }),
				...(template && { gridTemplateColumns: template }),
			}}
			{...props}
		/>
	);
}
