import { DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

export function ScreenHeader({
	title,
	description,
	flush,
}: {
	title: string;
	description: string;
	flush?: boolean;
}) {
	return (
		<div className={cn("flex flex-col items-center text-center", !flush && "mb-6")}>
			<DialogTitle>{title}</DialogTitle>
			<DialogDescription className="mt-1 max-w-[340px] text-sm text-balance text-muted-foreground">
				{description}
			</DialogDescription>
		</div>
	);
}
