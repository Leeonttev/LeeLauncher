import { cn } from "@/lib/utils";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useId } from "react";
import { segmentGroup, segmentItem, segmentThumb } from "@/components/segment";
import { SlidingMarker } from "@/components/sliding-marker";
import { RadioGroup } from "@/components/ui/radio-group";
import { withViewTransition } from "@/lib/appearance";
import { LANGUAGE_INFO, LANGUAGES, setLanguage, useLanguage, type Language } from "@/lib/i18n";

export function LanguagePicker({ labelledBy }: { labelledBy: string }) {
	const { resolvedLanguage } = useLanguage();
	const group = useId();
	return (
		<RadioGroup
			aria-labelledby={labelledBy}
			value={resolvedLanguage}
			onValueChange={(value) => withViewTransition(() => setLanguage(value as Language))}
			orientation="horizontal"
			className={segmentGroup}
		>
			{LANGUAGES.map((option) => (
				<RadioGroupPrimitive.Item
					key={option}
					value={option}
					lang={option}
					className={cn(segmentItem, "w-24")}
				>
					{option === resolvedLanguage && (
						<SlidingMarker group={group} value={resolvedLanguage} className={segmentThumb} />
					)}
					<span className="relative">{LANGUAGE_INFO[option].name}</span>
				</RadioGroupPrimitive.Item>
			))}
		</RadioGroup>
	);
}
