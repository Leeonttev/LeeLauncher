import * as React from "react";
import { cn } from "cn";

/** Рамку и фокус рисует обёртка; `className` — самому полю. */
function Textarea({
	className,
	frameClassName,
	...props
}: React.ComponentProps<"textarea"> & { frameClassName?: string }) {
	return (
		<div
			data-slot="textarea-frame"
			className={cn(
				"relative flex w-full rounded-lg border border-input bg-field transition-[border-color,outline-color] duration-(--duration-fast)",
				"has-[textarea:focus-visible]:border-ring has-[textarea:focus-visible]:outline-3 has-[textarea:focus-visible]:outline-offset-0 has-[textarea:focus-visible]:outline-primary-soft",
				"has-[textarea[aria-invalid=true]]:border-destructive has-[textarea[aria-invalid=true]]:outline-destructive/20",
				"has-[textarea:disabled]:opacity-50",
				frameClassName,
			)}
		>
			<textarea
				data-slot="textarea"
				className={cn(
					"flex field-sizing-content min-h-16 w-full bg-transparent px-3 py-2 text-base outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed md:text-sm",
					className,
				)}
				{...props}
			/>
		</div>
	);
}

export { Textarea };
