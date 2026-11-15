import { Check } from "lucide-react";
import * as React from "react";
import { cn } from "cn";
import { Checkbox as CheckboxPrimitive } from "radix-ui";

/** Отмеченное — через `aria-checked:`: `TooltipTrigger` перезаписывает `data-state`. */
function Checkbox({ className, ...props }: React.ComponentProps<typeof CheckboxPrimitive.Root>) {
	return (
		<CheckboxPrimitive.Root
			data-slot="checkbox"
			className={cn(
				"peer relative flex size-[18px] shrink-0 items-center justify-center rounded-[5px] border border-input bg-field transition-[background-color,border-color] duration-(--duration-fast) ease-out disabled:cursor-not-allowed disabled:opacity-50 aria-checked:border-primary aria-checked:bg-primary aria-checked:text-primary-foreground aria-invalid:border-destructive",
				className,
			)}
			{...props}
		>
			<CheckboxPrimitive.Indicator data-slot="checkbox-indicator" className="flex">
				<Check className="size-3.5" strokeWidth={2.5} />
			</CheckboxPrimitive.Indicator>
		</CheckboxPrimitive.Root>
	);
}

export { Checkbox };
