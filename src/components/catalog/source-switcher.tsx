import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { segmentGroup, segmentItem, segmentThumb } from "@/components/segment";
import { SlidingMarker } from "@/components/sliding-marker";
import {
	setCatalogSource,
	SOURCE_NAMES,
	sourceList,
	useSources,
	viewInfo,
	type CatalogSource,
} from "@/lib/catalog";
import { cn } from "@/lib/utils";

/** Ширины сегментов целые и заполняют группу без остатка: 232 − 2×3 − 2×2 = 48 + 79 + 95. Меняя колонку или подписи — пересчитать. */
export function SourceSwitcher({ value, labelledBy }: { value: CatalogSource; labelledBy: string }) {
	const { t } = useTranslation();
	const noteId = useId();
	const group = useId();
	const state = useSources();
	const sources = sourceList(state.items);
	const disabled = sources.filter((source) => !source.enabled);
	const segments: { id: CatalogSource; label: string; enabled: boolean }[] = [
		{ id: "all", label: t("catalog.source.all"), enabled: viewInfo("all", state).enabled },
		...sources.map((source) => ({
			id: source.id,
			label: SOURCE_NAMES[source.id],
			enabled: source.enabled,
		})),
	];
	const columns = sources.length === 2 ? "grid-cols-[48px_79px_95px]" : "auto-cols-fr grid-flow-col";
	return (
		<>
			<RadioGroupPrimitive.Root
				aria-labelledby={labelledBy}
				aria-describedby={disabled.length ? noteId : undefined}
				value={value}
				onValueChange={(next) => setCatalogSource(next as CatalogSource)}
				orientation="horizontal"
				className={cn(segmentGroup, "grid w-full", columns)}
			>
				{segments.map((segment) => (
					<RadioGroupPrimitive.Item
						key={segment.id}
						value={segment.id}
						disabled={!segment.enabled}
						className={cn(
							segmentItem,
							!segment.enabled && "cursor-default opacity-50 hover:text-muted-foreground",
						)}
					>
						{segment.id === value && (
							<SlidingMarker group={group} value={value} className={segmentThumb} />
						)}
						<span className="relative truncate">{segment.label}</span>
					</RadioGroupPrimitive.Item>
				))}
			</RadioGroupPrimitive.Root>
			{disabled.length > 0 && (
				<div
					id={noteId}
					className="mt-2 flex flex-col gap-1 text-[13px]/[18px] text-muted-foreground"
				>
					{disabled.map((source) => (
						<p key={source.id}>
							{t("catalog.source.disabled", { source: SOURCE_NAMES[source.id] })}
						</p>
					))}
				</div>
			)}
		</>
	);
}
