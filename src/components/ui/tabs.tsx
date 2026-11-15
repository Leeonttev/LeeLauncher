import * as React from "react";
import { cn } from "cn";
import { Tabs as TabsPrimitive } from "radix-ui";

function Tabs({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.Root>) {
	return <TabsPrimitive.Root data-slot="tabs" className={cn("flex flex-col", className)} {...props} />;
}

function TabsList({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.List>) {
	return (
		<TabsPrimitive.List
			data-slot="tabs-list"
			className={cn(
				"relative flex h-10 items-stretch gap-6 after:pointer-events-none after:absolute after:inset-x-0 after:bottom-0 after:h-px after:bg-border after:content-['']",
				className,
			)}
			{...props}
		/>
	);
}

function TabsTrigger({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.Trigger>) {
	return (
		<TabsPrimitive.Trigger
			data-slot="tabs-trigger"
			className={cn(
				"relative flex h-10 items-center gap-2 rounded-md text-sm font-medium whitespace-nowrap text-muted-foreground transition-colors duration-(--duration-fast) ease-out not-aria-selected:hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-0 disabled:pointer-events-none disabled:opacity-50 aria-selected:text-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0",
				className,
			)}
			{...props}
		/>
	);
}

/** Не останавливает Tab, но принимает фокус программно. */
function TabsContent({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.Content>) {
	return (
		<TabsPrimitive.Content
			data-slot="tabs-content"
			tabIndex={-1}
			className={cn("outline-none", className)}
			{...props}
		/>
	);
}

export { Tabs, TabsList, TabsTrigger, TabsContent };
