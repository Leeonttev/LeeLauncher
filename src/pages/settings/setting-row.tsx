import type { ReactNode } from "react";
import { Label } from "@/components/ui/label";

type SettingRowProps = {
	title: string;
	description: string;
	htmlFor?: string;
	/** Для `aria-labelledby` групп. */
	titleId?: string;
	children: ReactNode;
	/** Управление справа, в одну строку с названием. */
	inline?: boolean;
	/** Под пояснением: состояние, предупреждение. */
	note?: ReactNode;
};

export function SettingRow({
	title,
	description,
	htmlFor,
	titleId,
	children,
	inline,
	note,
}: SettingRowProps) {
	const heading = (
		<div className="min-w-0 flex-1">
			{htmlFor ? (
				<Label htmlFor={htmlFor} className="text-sm font-medium">
					{title}
				</Label>
			) : (
				<h3 id={titleId} className="text-sm font-medium">
					{title}
				</h3>
			)}
			<p className="mt-0.5 max-w-prose text-[13px]/[18px] text-muted-foreground">{description}</p>
			{note}
		</div>
	);

	if (inline) {
		return (
			<div className="flex items-center gap-6 py-5">
				{heading}
				{children}
			</div>
		);
	}

	return (
		<div className="py-5">
			{heading}
			<div className="mt-4">{children}</div>
		</div>
	);
}
