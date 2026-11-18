import { Compass, Layers, LoaderCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import {
	otherSource,
	setCatalogSource,
	SLOW_AFTER,
	SOURCE_NAMES,
	useSources,
	viewInfo,
	type CatalogSource,
} from "@/lib/catalog";
import { cn } from "@/lib/utils";

/** Появляется, если ответа нет дольше `SLOW_AFTER`, и пропадает сразу с ответом. `switchable` — кнопки смены источника (страница каталога). */
export function SlowNotice({
	loading,
	source,
	switchable = false,
	className,
}: {
	loading: boolean;
	source: CatalogSource;
	switchable?: boolean;
	className?: string;
}) {
	const { t } = useTranslation();
	const sources = useSources();
	const slow = useDelayedFlag(loading, SLOW_AFTER);
	const single = source === "all" ? null : source;
	const other = switchable && single ? otherSource(single, sources.items) : null;
	const all = switchable && single !== null && viewInfo("all", sources).enabled;
	return (
		<div role="status">
			{slow && (
				<div
					className={cn(
						"flex flex-wrap items-center gap-x-3 gap-y-2 text-[13px]/[18px] text-muted-foreground",
						className,
					)}
				>
					<span className="flex items-center gap-1.5">
						<LoaderCircle className="shrink-0 animate-spin" />
						{single
							? t("catalog.slow.one", { source: SOURCE_NAMES[single] })
							: t("catalog.slow.all")}
					</span>
					{(other || all) && (
						<span className="flex flex-wrap gap-2">
							{other && (
								<Button variant="outline" size="sm" onClick={() => setCatalogSource(other)}>
									<Compass />
									{t("catalog.error.openSource", { source: SOURCE_NAMES[other] })}
								</Button>
							)}
							{all && (
								<Button variant="outline" size="sm" onClick={() => setCatalogSource("all")}>
									<Layers />
									{t("catalog.slow.openAll")}
								</Button>
							)}
						</span>
					)}
				</div>
			)}
		</div>
	);
}
