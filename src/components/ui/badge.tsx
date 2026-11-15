import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "cn";
import { Slot } from "radix-ui";

const badgeVariants = cva(
	"inline-flex h-[22px] w-fit shrink-0 items-center justify-center gap-1 overflow-clip rounded-md px-2 text-xs font-medium whitespace-nowrap transition-colors [&>svg]:pointer-events-none [&>svg]:size-3",
	{
		variants: {
			variant: {
				default: "bg-primary-soft text-primary-text",
				secondary: "bg-secondary text-secondary-foreground dark:bg-accent",
				warning: "bg-warning/14 text-warning",
				success: "bg-success/12 text-success",
				destructive: "bg-destructive/12 text-destructive",
				outline: "border border-border text-foreground",
				ghost: "[a&]:hover:bg-accent [a&]:hover:text-accent-foreground",
				link: "text-primary-text underline-offset-4 [a&]:hover:underline",
			},
		},
		defaultVariants: {
			variant: "default",
		},
	},
);

function Badge({
	className,
	variant = "default",
	asChild = false,
	...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
	const Comp = asChild ? Slot.Root : "span";

	return (
		<Comp
			data-slot="badge"
			data-variant={variant}
			className={cn(badgeVariants({ variant }), className)}
			{...props}
		/>
	);
}

export { Badge, badgeVariants };
