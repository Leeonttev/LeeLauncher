import * as React from "react";
import { cn } from "cn";

/** Рамку и фокус рисует обёртка: `className` — ей, `inputClassName` — самому полю, `start`/`end` — внутри рамки. */
function Input({
	className,
	inputClassName,
	start,
	end,
	type,
	...props
}: React.ComponentProps<"input"> & {
	inputClassName?: string;
	start?: React.ReactNode;
	end?: React.ReactNode;
}) {
	return (
		<div
			data-slot="input-frame"
			className={cn(
				"relative flex h-9 w-full min-w-0 items-center rounded-lg border border-input bg-field transition-[border-color,outline-color] duration-(--duration-fast)",
				"has-[input:focus-visible]:border-ring has-[input:focus-visible]:outline-3 has-[input:focus-visible]:outline-offset-0 has-[input:focus-visible]:outline-primary-soft",
				"has-[input[aria-invalid=true]]:border-destructive has-[input[aria-invalid=true]]:outline-destructive/20",
				"has-[input:disabled]:opacity-50",
				className,
			)}
		>
			{start}
			<input
				type={type}
				data-slot="input"
				className={cn(
					"h-full w-full min-w-0 bg-transparent px-3 text-base outline-none placeholder:text-muted-foreground disabled:pointer-events-none disabled:cursor-not-allowed md:text-sm",
					inputClassName,
				)}
				{...props}
			/>
			{end}
		</div>
	);
}

export { Input };
