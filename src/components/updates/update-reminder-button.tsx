import { CircleArrowUp } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { reopenUpdate, useUpdates } from "@/lib/updates";

export function UpdateReminderButton() {
	const { t } = useTranslation();
	const { info, skipped, visible } = useUpdates();
	if (!info || !skipped || visible) return null;
	const label = t("update.reminder", { version: info.version });

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={label}
					onClick={reopenUpdate}
					className="group/up relative flex size-9 items-center justify-center rounded-full text-muted-foreground transition-[color,background-color,scale] duration-(--duration-fast) ease-out hover:bg-sidebar-accent hover:text-foreground active:scale-[0.98]"
				>
					<CircleArrowUp />
					<span
						aria-hidden
						className="absolute top-[3px] right-[3px] flex size-3 items-center justify-center rounded-full bg-sidebar group-hover/up:bg-sidebar-accent"
					>
						<span className="size-2 rounded-full bg-warning" />
					</span>
				</button>
			</TooltipTrigger>
			<TooltipContent side="bottom" sideOffset={6}>
				{label}
			</TooltipContent>
		</Tooltip>
	);
}
