import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "cn";
import { Slot } from "radix-ui";

const buttonVariants = cva(
	"relative inline-flex shrink-0 items-center justify-center gap-2 rounded-full border border-transparent text-sm font-medium whitespace-nowrap transition-[color,background-color,border-color,scale] duration-(--duration-fast) ease-out select-none focus-visible:outline-2 focus-visible:outline-offset-2 active:scale-[0.98] disabled:pointer-events-none disabled:opacity-50 aria-disabled:opacity-60 aria-invalid:border-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
	{
		variants: {
			variant: {
				default: "bg-primary text-primary-foreground hover:bg-primary/90",
				destructive: "border-destructive/40 text-destructive hover:bg-destructive/10",
				outline: "border-border bg-card hover:bg-accent aria-expanded:bg-accent",
				secondary: "border-border bg-card hover:bg-accent aria-expanded:bg-accent",
				ghost: "text-muted-foreground hover:bg-accent hover:text-foreground aria-expanded:bg-accent aria-expanded:text-foreground",
				link: "text-primary-text underline underline-offset-4 hover:no-underline",
				more: "gap-0.5 rounded-md px-1 text-foreground hover:underline hover:underline-offset-4 [&_svg]:transition-transform hover:[&_svg]:translate-x-0.5",
			},
			size: {
				default: "h-9 px-4",
				xs: "h-7 gap-1.5 px-2.5 text-[13px]",
				sm: "h-8 gap-1.5 px-3 text-[13px]",
				lg: "h-10 px-5",
				xl: "h-11 px-6 text-[15px]",
				icon: "size-9",
				"icon-xs": "size-7",
				"icon-sm": "size-8",
				"icon-lg": "size-10",
			},
		},
		compoundVariants: [{ variant: "more", class: "h-auto px-1" }],
		defaultVariants: {
			variant: "default",
			size: "default",
		},
	},
);

function Button({
	className,
	variant = "default",
	size = "default",
	asChild = false,
	...props
}: React.ComponentProps<"button"> &
	VariantProps<typeof buttonVariants> & {
		asChild?: boolean;
	}) {
	const Comp = asChild ? Slot.Root : "button";

	return (
		<Comp
			data-slot="button"
			data-variant={variant}
			data-size={size}
			className={cn(buttonVariants({ variant, size, className }))}
			{...props}
		/>
	);
}

export { Button, buttonVariants };
