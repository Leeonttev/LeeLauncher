import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useId, type ReactNode } from "react";
import { segmentGroup, segmentItem, segmentThumb } from "@/components/segment";
import { SlidingMarker } from "@/components/sliding-marker";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export type SegmentOption = {
	value: string;
	label?: string;
	content: ReactNode;
};

export function Segments({
	label,
	value,
	onChange,
	options,
	layoutId,
	disabled,
}: {
	label: string;
	value: string;
	onChange: (value: string) => void;
	options: SegmentOption[];
	layoutId: string;
	disabled?: boolean;
}) {
	const group = useId() + layoutId;
	return (
		<RadioGroupPrimitive.Root
			aria-label={label}
			disabled={disabled}
			value={value}
			onValueChange={onChange}
			orientation="horizontal"
			className={segmentGroup}
		>
			{options.map((option) => {
				const item = (
					<RadioGroupPrimitive.Item
						key={option.value}
						value={option.value}
						aria-label={option.label}
						className={cn(segmentItem, option.label ? "w-8" : "px-3")}
					>
						{option.value === value && (
							<SlidingMarker group={group} value={value} className={segmentThumb} />
						)}
						<span className="relative flex">{option.content}</span>
					</RadioGroupPrimitive.Item>
				);
				return option.label ? (
					<Tooltip key={option.value}>
						<TooltipTrigger asChild>{item}</TooltipTrigger>
						<TooltipContent>{option.label}</TooltipContent>
					</Tooltip>
				) : (
					item
				);
			})}
		</RadioGroupPrimitive.Root>
	);
}
