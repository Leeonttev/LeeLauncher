import { Check, ChevronsUpDown } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

export type VersionBadge = { label: string; accent?: boolean };
export type VersionOption = { id: string; badges: VersionBadge[] };

type VersionComboboxProps = {
	options: VersionOption[];
	value: string | null;
	onChange: (id: string) => void;
	labelledBy: string;
	loading?: boolean;
	disabled?: boolean;
	footer?: ReactNode;
};

function substring(value: string, search: string) {
	return value.toLowerCase().includes(search.trim().toLowerCase()) ? 1 : 0;
}

function Badges({ badges }: { badges: VersionBadge[] }) {
	return (
		<>
			{badges.map((badge) => (
				<span
					key={badge.label}
					className={cn(
						"shrink-0 text-xs/4 font-medium",
						badge.accent ? "text-primary-text" : "text-muted-foreground",
					)}
				>
					{badge.label}
				</span>
			))}
		</>
	);
}

export function VersionCombobox({
	options,
	value,
	onChange,
	labelledBy,
	loading,
	disabled,
	footer,
}: VersionComboboxProps) {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	const listRef = useRef<HTMLDivElement>(null);
	const selected = options.find((o) => o.id === value) ?? null;

	useEffect(() => {
		if (!open) return;
		const frame = requestAnimationFrame(() => {
			listRef.current?.querySelector('[data-selected="true"]')?.scrollIntoView({ block: "center" });
		});
		return () => cancelAnimationFrame(frame);
	}, [open]);

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					role="combobox"
					aria-expanded={open}
					aria-labelledby={labelledBy}
					disabled={disabled || loading}
					className="relative mt-2 flex h-10 w-full items-center justify-between gap-3 rounded-lg border border-input bg-field px-3 text-sm transition-[color,border-color] duration-(--duration-fast) focus-visible:border-ring focus-visible:outline-3 focus-visible:outline-offset-0 focus-visible:outline-primary-soft disabled:pointer-events-none disabled:opacity-50 aria-expanded:border-ring"
				>
					{loading ? (
						<Skeleton className="h-4 w-24" />
					) : (
						// В поле только первый бейдж, сразу за номером.
						<span className="flex min-w-0 items-center gap-1.5">
							<span className="truncate font-medium">{selected?.id ?? value ?? ""}</span>
							{selected?.badges[0] && (
								<>
									<span aria-hidden className="shrink-0 text-muted-foreground">
										·
									</span>
									<span
										className={cn(
											"shrink-0 text-[13px]/[18px]",
											selected.badges[0].accent
												? "text-primary-text"
												: "text-muted-foreground",
										)}
									>
										{selected.badges[0].label}
									</span>
								</>
							)}
						</span>
					)}
					<ChevronsUpDown className="ml-auto shrink-0 text-muted-foreground" />
				</button>
			</PopoverTrigger>
			<PopoverContent
				align="start"
				sideOffset={6}
				fit
				className="w-(--radix-popover-trigger-width) overflow-clip p-0"
			>
				<Command filter={substring} defaultValue={value ?? undefined} className="bg-transparent">
					<CommandInput placeholder={t("createInstance.searchVersion")} />
					<CommandList ref={listRef} className="max-h-[264px] p-1.5">
						<CommandEmpty className="py-6 text-center text-sm text-muted-foreground">
							{t("createInstance.versionNotFound")}
						</CommandEmpty>
						{options.map((option) => {
							const active = option.id === value;
							return (
								<CommandItem
									key={option.id}
									value={option.id}
									className="h-9 gap-3 px-2.5"
									onSelect={() => {
										onChange(option.id);
										setOpen(false);
									}}
								>
									<span
										className={cn("flex-1 truncate text-sm/5", active && "font-semibold")}
									>
										{option.id}
									</span>
									<Badges badges={option.badges} />
									<Check className={cn("text-primary-text", !active && "invisible")} />
								</CommandItem>
							);
						})}
					</CommandList>
				</Command>
				{footer}
			</PopoverContent>
		</Popover>
	);
}
