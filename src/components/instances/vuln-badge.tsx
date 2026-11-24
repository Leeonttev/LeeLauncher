import { ShieldAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { Instance } from "@/lib/api";
import { instancePath, navigate } from "@/lib/router";
import { cn } from "@/lib/utils";

/**
 * Значок уязвимой версии в мете сборки. Лежит поверх растянутой ссылки ради подсказки,
 * поэтому щелчок по нему тоже открывает сборку.
 */
export function VulnBadge({ instance, onCover = false }: { instance: Instance; onCover?: boolean }) {
	const { t } = useTranslation();
	if (instance.fontVulnerable !== true) return null;
	const label = t("security.fontVuln.badge");
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span
					role="img"
					aria-label={label}
					className={cn(
						"pointer-events-auto relative z-10 inline-flex shrink-0 cursor-pointer items-center text-warning",
						// Обложка тёмная в обеих темах, поэтому цвет берём из тёмной.
						onCover && "dark",
					)}
					onClick={() => navigate(instancePath(instance.id))}
				>
					<ShieldAlert aria-hidden className="size-3.5" />
				</span>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}
