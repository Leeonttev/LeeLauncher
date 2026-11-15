import { TriangleAlert } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Slider } from "@/components/ui/slider";
import { formatGbNumber, formatMemory } from "@/lib/format";

const STEP_MB = 512;
const MIN_MB = 1024;
/** Ядро не смогло узнать объём ОЗУ — шкала до 16 ГБ. */
const FALLBACK_TOTAL_MB = 17 * 1024;

function memoryRange(totalMb: number) {
	const total = totalMb > 0 ? totalMb : FALLBACK_TOTAL_MB;
	const max = Math.max(MIN_MB * 2, Math.floor((total - 1024) / STEP_MB) * STEP_MB);
	return { min: MIN_MB, max };
}

function clamp(mb: number, min: number, max: number) {
	return Math.min(max, Math.max(min, Math.round(mb / STEP_MB) * STEP_MB));
}

type MemoryFieldProps = {
	valueMb: number;
	onCommit: (mb: number) => void;
	/** `null` — ещё неизвестен, 0 — ядро не смогло определить. */
	totalMb: number | null;
	hint: string;
	labelledBy: string;
	disabled?: boolean;
};

export function MemoryField({ valueMb, onCommit, totalMb, hint, labelledBy, disabled }: MemoryFieldProps) {
	const { t } = useTranslation();
	const [draft, setDraft] = useState<number | null>(null);
	const [text, setText] = useState<string | null>(null);

	if (totalMb === null) {
		return (
			<div className="flex items-center gap-4">
				<Skeleton className="h-1.5 w-full max-w-[480px]" />
				<Input disabled className="h-9 w-20" aria-label={t("settings.game.memory.inputLabel")} />
			</div>
		);
	}

	const { min, max } = memoryRange(totalMb);
	const current = clamp(draft ?? valueMb, min, max);
	const tooMuch = totalMb > 0 && current > totalMb * 0.75;

	const commitText = () => {
		if (text === null) return;
		const parsed = Number.parseFloat(text.replace(",", ".").trim());
		setText(null);
		if (!Number.isFinite(parsed)) return;
		const mb = clamp(parsed * 1024, min, max);
		if (mb !== valueMb) onCommit(mb);
	};

	return (
		<div>
			<div className="flex items-center gap-4">
				<div className="max-w-[480px] flex-1">
					<Slider
						min={min}
						max={max}
						step={STEP_MB}
						value={[current]}
						disabled={disabled}
						thumbProps={{
							"aria-labelledby": labelledBy,
							"aria-valuetext": formatMemory(current),
						}}
						onValueChange={([mb]) => setDraft(mb)}
						onValueCommit={([mb]) => {
							setDraft(null);
							if (mb !== valueMb) onCommit(mb);
						}}
						onKeyDown={(e) => {
							if (e.key !== "PageUp" && e.key !== "PageDown") return;
							e.preventDefault();
							const mb = clamp(current + (e.key === "PageUp" ? 2048 : -2048), min, max);
							if (mb !== valueMb) onCommit(mb);
						}}
					/>
					<div className="mt-2 flex justify-between text-xs/4 text-muted-foreground tabular-nums">
						<span>{formatMemory(min)}</span>
						<span>{formatMemory(max)}</span>
					</div>
				</div>
				<div className="-mt-6 flex items-center gap-2">
					<Input
						inputMode="decimal"
						value={text ?? formatGbNumber(current)}
						disabled={disabled}
						aria-label={t("settings.game.memory.inputLabel")}
						className="h-9 w-20"
						inputClassName="text-right tabular-nums"
						onChange={(e) => setText(e.target.value)}
						onBlur={commitText}
						onKeyDown={(e) => {
							if (e.key === "Enter") {
								e.preventDefault();
								commitText();
							} else if (e.key === "Escape" && text !== null) {
								e.preventDefault();
								setText(null);
							}
						}}
					/>
					<span className="text-sm text-muted-foreground">{t("settings.game.memory.unit")}</span>
				</div>
			</div>
			{tooMuch ? (
				<p className="mt-3 flex items-center gap-1.5 text-[13px]/[18px] text-warning">
					<TriangleAlert className="shrink-0" />
					{t("settings.game.memory.tooMuch")}
				</p>
			) : (
				<p className="mt-3 text-[13px]/[18px] text-muted-foreground">{hint}</p>
			)}
		</div>
	);
}
