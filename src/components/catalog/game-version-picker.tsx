import { Check, ChevronsUpDown } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { readStorage } from "@/lib/storage";
import { cn } from "@/lib/utils";
import { useGameVersions } from "@/lib/versions";

const ANY = "__any__";

function substring(value: string, search: string) {
	if (value === ANY) return search.trim() ? 0 : 1;
	return value.toLowerCase().includes(search.trim().toLowerCase()) ? 1 : 0;
}

export function GameVersionPicker({
	value,
	onChange,
	labelledBy,
	disabled,
	className,
}: {
	value: string | null;
	onChange: (value: string | null) => void;
	labelledBy: string;
	disabled?: boolean;
	className?: string;
}) {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	const versions = useGameVersions();
	const listRef = useRef<HTMLDivElement>(null);
	const snapshots = readStorage("show-snapshots") === "true";
	const options =
		versions?.status === "ready"
			? versions.value.versions.filter(
					(v) => v.kind === "release" || (snapshots && v.kind === "snapshot"),
				)
			: [];

	useEffect(() => {
		if (!open) return;
		const frame = requestAnimationFrame(() => {
			listRef.current?.querySelector('[data-selected="true"]')?.scrollIntoView({ block: "center" });
		});
		return () => cancelAnimationFrame(frame);
	}, [open]);

	const select = (next: string | null) => {
		onChange(next);
		setOpen(false);
	};

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					role="combobox"
					aria-expanded={open}
					aria-labelledby={labelledBy}
					disabled={disabled}
					className={cn(
						"relative flex h-9 w-full items-center justify-between gap-3 rounded-lg border border-input bg-field px-3 text-sm transition-[color,border-color] duration-(--duration-fast) focus-visible:border-ring focus-visible:outline-3 focus-visible:outline-offset-0 focus-visible:outline-primary-soft disabled:pointer-events-none aria-expanded:border-ring",
						className,
					)}
				>
					<span className="truncate font-medium">{value ?? t("catalog.filter.anyVersion")}</span>
					<ChevronsUpDown className="shrink-0 text-muted-foreground" />
				</button>
			</PopoverTrigger>
			<PopoverContent align="start" sideOffset={6} fit className="w-60 overflow-clip p-0">
				<Command filter={substring} defaultValue={value ?? ANY} className="bg-transparent">
					<CommandInput placeholder={t("createInstance.searchVersion")} />
					<CommandList ref={listRef} className="max-h-[264px] p-1.5">
						<CommandEmpty className="py-6 text-center text-sm text-muted-foreground">
							{t("createInstance.versionNotFound")}
						</CommandEmpty>
						<CommandItem value={ANY} className="h-9 gap-3 px-2.5" onSelect={() => select(null)}>
							<span className={cn("flex-1 text-sm/5", value === null && "font-semibold")}>
								{t("catalog.filter.anyVersion")}
							</span>
							<Check className={cn("text-primary-text", value !== null && "invisible")} />
						</CommandItem>
						{options.map((option) => (
							<CommandItem
								key={option.id}
								value={option.id}
								className="h-9 gap-3 px-2.5"
								onSelect={() => select(option.id)}
							>
								<span
									className={cn(
										"flex-1 truncate text-sm/5",
										option.id === value && "font-semibold",
									)}
								>
									{option.id}
								</span>
								<Check
									className={cn("text-primary-text", option.id !== value && "invisible")}
								/>
							</CommandItem>
						))}
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}
