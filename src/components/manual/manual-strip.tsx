import { ExternalLink, TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";

export function ManualStrip({ count, onOpen }: { count: number; onOpen: (target: HTMLElement) => void }) {
	const { t } = useTranslation();
	return (
		<div className="mb-4 flex items-center gap-3 border-b pb-4">
			<TriangleAlert className="shrink-0 text-warning" />
			<div className="min-w-0 flex-1">
				<p className="truncate text-sm/5 font-semibold">{t("manual.strip.title", { count })}</p>
				<p className="truncate text-[13px]/[18px] text-muted-foreground">{t("manual.strip.hint")}</p>
			</div>
			<Button variant="outline" size="sm" className="shrink-0" onClick={(e) => onOpen(e.currentTarget)}>
				<ExternalLink />
				{t("manual.strip.action")}
			</Button>
		</div>
	);
}
