import { type LucideIcon } from "lucide-react";
import { useId, type ReactNode } from "react";
import { SlidingMarker } from "@/components/sliding-marker";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

export type PageTab<T extends string> = {
	value: T;
	label: string;
	icon?: LucideIcon;
	count?: number | null;
	extra?: ReactNode;
};

const planckTransition = { type: "spring", stiffness: 500, damping: 40, mass: 0.8 } as const;

const planckClass = "pointer-events-none absolute inset-x-0 bottom-0 z-20 h-0.5 rounded-t-[2px] bg-primary";

/** Планка лежит рядом с кнопкой, а не в ней, чтобы её не обрезал фон кнопки. `aside` — справа в той же полосе, вне `tablist`. */
export function PageTabs<T extends string>({
	value,
	onValueChange,
	tabs,
	layoutId,
	label,
	listClassName,
	barClassName,
	aside,
	className,
	children,
}: {
	value: T;
	onValueChange: (value: T) => void;
	tabs: PageTab<T>[];
	layoutId: string;
	label: string;
	listClassName?: string;
	barClassName?: string;
	aside?: ReactNode;
	className?: string;
	children?: ReactNode;
}) {
	const group = useId() + layoutId;
	const list = (
		<TabsList aria-label={label} className={listClassName}>
			{tabs.map((tab) => (
				<div key={tab.value} className="relative flex shrink-0 items-center">
					<TabsTrigger value={tab.value}>
						{tab.icon && <tab.icon />}
						{tab.label}
						{tab.count !== undefined && tab.count !== null && (
							<span className="font-medium tabular-nums">{tab.count}</span>
						)}
						{tab.extra}
					</TabsTrigger>
					{tab.value === value && (
						<SlidingMarker
							group={group}
							value={value}
							className={planckClass}
							transition={planckTransition}
						/>
					)}
				</div>
			))}
		</TabsList>
	);
	return (
		<Tabs value={value} onValueChange={(next) => onValueChange(next as T)} className={className}>
			{barClassName !== undefined || aside !== undefined ? (
				<div className={cn("relative", barClassName)}>
					{list}
					{aside}
				</div>
			) : (
				list
			)}
			{children}
		</Tabs>
	);
}
