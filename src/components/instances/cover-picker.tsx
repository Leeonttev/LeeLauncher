import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useTranslation } from "react-i18next";
import { InstanceCover } from "@/components/instances/instance-cover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { INSTANCE_COVERS, isCover, type InstanceCover as Scene } from "@/lib/covers";
import { cn } from "@/lib/utils";

type CoverRadioGroupProps = {
	value: Scene;
	/** Любая смена, в том числе стрелкой: Radix выбирает вариант уже при переходе. */
	onChange: (cover: Scene) => void;
	label?: string;
	labelledBy?: string;
	layout: "dialog" | "row";
};

/** Выбор — через `aria-checked:`: `TooltipTrigger` перезаписывает `data-state`. */
export function CoverRadioGroup({ value, onChange, label, labelledBy, layout }: CoverRadioGroupProps) {
	const { t } = useTranslation();
	const dialog = layout === "dialog";
	return (
		<RadioGroupPrimitive.Root
			value={value}
			onValueChange={(next) => isCover(next) && onChange(next)}
			aria-label={label}
			aria-labelledby={labelledBy}
			className={dialog ? "grid grid-cols-[repeat(3,112px)] gap-2" : "flex flex-wrap gap-3"}
		>
			{INSTANCE_COVERS.map((cover) => (
				<Tooltip key={cover}>
					<TooltipTrigger asChild>
						<RadioGroupPrimitive.Item
							value={cover}
							aria-label={t(`covers.${cover}`)}
							className={cn(
								"relative cursor-pointer overflow-clip rounded-lg border border-transparent",
								"transition-[border-color] duration-(--duration-fast) ease-out hover:border-foreground/25",
								"focus-visible:outline-foreground aria-checked:outline-2 aria-checked:outline-offset-2 aria-checked:outline-primary aria-checked:focus-visible:outline-foreground",
								dialog ? "h-12 w-28" : "h-16 w-40",
							)}
						>
							<InstanceCover scene={cover} scale={2} className="absolute -inset-0.5" />
						</RadioGroupPrimitive.Item>
					</TooltipTrigger>
					<TooltipContent>{t(`covers.${cover}`)}</TooltipContent>
				</Tooltip>
			))}
		</RadioGroupPrimitive.Root>
	);
}
