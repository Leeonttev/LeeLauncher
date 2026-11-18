import { ChevronDown, Download, Package } from "lucide-react";
import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuShortcut,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { usePendingAction } from "@/hooks/use-pending-action";
import { pickModpackFile } from "@/lib/modpacks";

/** Действие — после закрытия меню. */
export function ImportMenu({ align = "end" }: { align?: "start" | "center" | "end" }) {
	const { t } = useTranslation();
	const pending = usePendingAction();
	const triggerRef = useRef<HTMLButtonElement>(null);
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button ref={triggerRef} variant="outline" className="aria-expanded:bg-accent">
					<Download />
					{t("instances.import.label")}
					<ChevronDown className="text-muted-foreground" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align={align} className="w-78" onCloseAutoFocus={pending.onCloseAutoFocus}>
				<DropdownMenuItem
					onSelect={() => pending.defer(() => void pickModpackFile(triggerRef.current))}
				>
					<Package />
					{t("instances.import.file")}
					<DropdownMenuShortcut className="text-[13px] tracking-normal">
						{t("instances.import.shortcut")}
					</DropdownMenuShortcut>
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
