import { Check, ChevronsUpDown, Plus } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Command, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { GROUP_NAME_MAX, groupKey, listGroups, normalizeGroupName } from "@/lib/groups";
import { useInstances } from "@/lib/instances";
import { cn } from "@/lib/utils";

type GroupComboboxProps = {
	value: string | null;
	onChange: (group: string | null) => void;
	id?: string;
	labelledBy?: string;
	disabled?: boolean;
	className?: string;
	/** В окнах с кнопками под полем — `top`, иначе список закроет кнопки. */
	side?: "top" | "bottom";
};

const CREATE = "__create__";
const NONE = "__none__";

function filter(value: string, search: string, keywords?: string[]) {
	if (value === CREATE) return 1;
	const query = search.trim().toLocaleLowerCase();
	if (!query) return 1;
	return (keywords ?? []).join(" ").toLocaleLowerCase().includes(query) ? 1 : 0;
}

export function GroupCombobox({
	value,
	onChange,
	id,
	labelledBy,
	disabled,
	className,
	side = "bottom",
}: GroupComboboxProps) {
	const { t, i18n } = useTranslation();
	const store = useInstances();
	const [open, setOpen] = useState(false);
	const [query, setQuery] = useState("");
	const groups = useMemo(() => {
		const list = listGroups(store.instances, i18n.language);
		// Выбранная, но ещё не созданная группа тоже в списке.
		if (value && !list.some((g) => groupKey(g) === groupKey(value))) return [value, ...list];
		return list;
	}, [store.instances, i18n.language, value]);
	const typed = normalizeGroupName(query);
	const canCreate = typed !== "" && !groups.some((g) => groupKey(g) === groupKey(typed));
	const currentKey = groupKey(value);
	const listRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (!open) return;
		const frame = requestAnimationFrame(() => {
			listRef.current?.querySelector('[data-selected="true"]')?.scrollIntoView({ block: "center" });
		});
		return () => cancelAnimationFrame(frame);
	}, [open]);

	const pick = (group: string | null) => {
		setOpen(false);
		if (group !== value) onChange(group);
	};

	return (
		<Popover
			open={open}
			onOpenChange={(next) => {
				setOpen(next);
				if (next) setQuery("");
			}}
		>
			<PopoverTrigger asChild>
				<button
					id={id}
					type="button"
					role="combobox"
					aria-expanded={open}
					aria-labelledby={labelledBy}
					disabled={disabled}
					className={cn(
						"relative flex h-10 items-center justify-between gap-3 rounded-lg border border-input bg-field px-3 text-sm transition-[color,border-color] duration-(--duration-fast) focus-visible:border-ring focus-visible:outline-3 focus-visible:outline-offset-0 focus-visible:outline-primary-soft disabled:pointer-events-none disabled:opacity-50 aria-expanded:border-ring",
						className,
					)}
				>
					<span className={cn("min-w-0 truncate", value ? "font-medium" : "text-muted-foreground")}>
						{value ?? t("groups.none")}
					</span>
					<ChevronsUpDown className="ml-auto shrink-0 text-muted-foreground" />
				</button>
			</PopoverTrigger>
			<PopoverContent
				side={side}
				align="start"
				sideOffset={6}
				fit
				className="w-(--radix-popover-trigger-width) min-w-64 overflow-clip p-0"
			>
				<Command
					filter={filter}
					defaultValue={value ? `g:${currentKey}` : NONE}
					className="bg-transparent"
				>
					<CommandInput
						value={query}
						maxLength={GROUP_NAME_MAX}
						placeholder={t("groups.field.search")}
						onValueChange={setQuery}
					/>
					<CommandList ref={listRef} className="max-h-[264px] p-1.5">
						{canCreate && (
							<CommandItem
								value={CREATE}
								forceMount
								className="h-9 gap-2.5 px-2.5"
								onSelect={() => pick(typed)}
							>
								<Plus className="text-muted-foreground" />
								<span className="flex-1 truncate">
									{t("groups.field.create", { name: typed })}
								</span>
							</CommandItem>
						)}
						<CommandItem
							value={NONE}
							keywords={[t("groups.none")]}
							className="h-9 gap-2.5 px-2.5"
							onSelect={() => pick(null)}
						>
							<span className="flex-1 truncate text-muted-foreground">{t("groups.none")}</span>
							<Check className={cn("text-primary-text", value !== null && "invisible")} />
						</CommandItem>
						{groups.map((group) => {
							const active = groupKey(group) === currentKey;
							return (
								<CommandItem
									key={groupKey(group)}
									value={`g:${groupKey(group)}`}
									keywords={[group]}
									className="h-9 gap-2.5 px-2.5"
									onSelect={() => pick(group)}
								>
									<span className={cn("flex-1 truncate", active && "font-semibold")}>
										{group}
									</span>
									<Check className={cn("text-primary-text", !active && "invisible")} />
								</CommandItem>
							);
						})}
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}
