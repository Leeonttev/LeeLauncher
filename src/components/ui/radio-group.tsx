import * as React from "react";
import { cn } from "cn";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";

function RadioGroup({ className, ...props }: React.ComponentProps<typeof RadioGroupPrimitive.Root>) {
	return (
		<RadioGroupPrimitive.Root
			data-slot="radio-group"
			className={cn("grid gap-3", className)}
			{...props}
		/>
	);
}

function RadioGroupItem({ className, ...props }: React.ComponentProps<typeof RadioGroupPrimitive.Item>) {
	return (
		<RadioGroupPrimitive.Item
			data-slot="radio-group-item"
			className={cn(
				"relative aspect-square size-[18px] shrink-0 rounded-full border border-input bg-field transition-[border-color,border-width] duration-(--duration-fast) disabled:cursor-not-allowed disabled:opacity-50 aria-checked:border-[5px] aria-checked:border-primary aria-invalid:border-destructive",
				className,
			)}
			{...props}
		></RadioGroupPrimitive.Item>
	);
}

export { RadioGroup, RadioGroupItem };
