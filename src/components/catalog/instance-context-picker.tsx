import { Check, ChevronsUpDown } from "lucide-react";
import { motion } from "motion/react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { InstanceThumb } from "@/components/instances/instance-cover";
import {
	Command,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
} from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import type { Instance } from "@/lib/api";
import { coverOf } from "@/lib/covers";
import { compareRecent } from "@/lib/instances";
import { versionLabel } from "@/lib/loaders";
import { cn } from "@/lib/utils";

const NONE = "__none__";

function substring(value: string, search: string) {
	return value.toLowerCase().includes(search.trim().toLowerCase()) ? 1 : 0;
}

export type ContextPickerSize = "default" | "page" | "compact";

const TRIGGER_CLASS: Record<ContextPickerSize, string> = {
	default: "h-11 max-w-80 min-w-65 gap-2.5 pr-3.5 pl-1.5",
	page: "h-10 w-65 gap-2.5 pr-3.5 pl-1",
	compact: "h-8 w-50 gap-2 pr-3 pl-1",
};

/** `flash` растёт, когда сборку сменили не отсюда: фон кнопки ненадолго подсвечивается. */
export function InstanceContextPicker({
	instances,
	value,
	onChange,
	flash = 0,
	size = "default",
}: {
	instances: Instance[];
	value: Instance | null;
	onChange: (id: string | null) => void;
	flash?: number;
	size?: ContextPickerSize;
}) {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	// Значение при монтировании — старая смена: мигаем только на новую.
	const [initialFlash] = useState(flash);

	const sorted = [...instances].sort(compareRecent);
	const modded = sorted.filter((i) => i.loader.kind !== "vanilla");
	const vanilla = sorted.filter((i) => i.loader.kind === "vanilla");

	const item = (instance: Instance) => {
		const active = instance.id === value?.id;
		return (
			<CommandItem
				key={instance.id}
				value={`${instance.name} ${instance.id}`}
				className="gap-3 px-2.5 py-2"
				onSelect={() => {
					onChange(instance.id);
					setOpen(false);
				}}
			>
				<InstanceThumb scene={coverOf(instance)} size={40} />
				<span className="min-w-0 flex-1">
					<span
						className={cn("block truncate text-sm/5", active ? "font-semibold" : "font-medium")}
					>
						{instance.name}
					</span>
					<span className="block truncate text-[13px]/[18px] text-muted-foreground">
						{versionLabel(instance)}
					</span>
				</span>
				<Check className={cn("text-primary-text", !active && "invisible")} />
			</CommandItem>
		);
	};

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					aria-label={t("catalog.context.aria", { name: value?.name ?? t("catalog.context.none") })}
					className={cn(
						"relative isolate flex shrink-0 items-center overflow-clip rounded-full border border-border bg-card text-left transition-colors duration-(--duration-base) ease-out hover:bg-accent aria-expanded:bg-accent",
						TRIGGER_CLASS[size],
					)}
				>
					{flash > initialFlash && (
						<motion.span
							key={flash}
							aria-hidden
							className="absolute inset-0 -z-10 bg-primary-soft"
							initial={{ opacity: 1 }}
							animate={{ opacity: 0 }}
							transition={{ delay: 0.4, duration: 0.2 }}
						/>
					)}
					{value && (
						<InstanceThumb
							scene={coverOf(value)}
							size={size === "compact" ? 24 : size === "page" ? 32 : 40}
							className="rounded-full"
						/>
					)}
					{size === "compact" ? (
						<span
							className={cn(
								"min-w-0 flex-1 truncate text-[13px]/[18px] font-semibold",
								!value && "pl-1.5 font-medium text-muted-foreground",
							)}
						>
							{value?.name ?? t("catalog.context.none")}
						</span>
					) : (
						<span className={cn("min-w-0 flex-1", !value && "pl-1.5")}>
							<span
								className={cn(
									"block truncate text-[13px] text-muted-foreground",
									size === "page" ? "leading-4" : "leading-[18px]",
								)}
							>
								{t("catalog.context.label")}
								{value && ` · ${versionLabel(value)}`}
							</span>
							<span
								className={cn(
									"block max-w-56 truncate text-sm font-semibold",
									size === "page" ? "leading-[18px]" : "leading-5",
								)}
							>
								{value?.name ?? t("catalog.context.none")}
							</span>
						</span>
					)}
					<ChevronsUpDown className="shrink-0 text-muted-foreground" />
				</button>
			</PopoverTrigger>
			<PopoverContent align="end" sideOffset={6} fit className="w-100 overflow-clip p-0">
				<Command filter={substring} className="bg-transparent">
					<CommandInput placeholder={t("catalog.context.find")} />
					<CommandList className="max-h-[360px] p-1.5">
						<CommandEmpty className="py-6 text-center text-sm text-muted-foreground">
							{t("catalog.context.notFound")}
						</CommandEmpty>
						<CommandItem
							value={NONE}
							className="h-9 gap-3 px-2.5 text-muted-foreground"
							onSelect={() => {
								onChange(null);
								setOpen(false);
							}}
						>
							<span className="flex-1 text-sm/5">{t("catalog.context.noInstance")}</span>
							<Check className={cn("text-primary-text", value && "invisible")} />
						</CommandItem>
						{modded.map(item)}
						{vanilla.length > 0 && (
							<CommandGroup
								heading={t("catalog.context.vanilla")}
								className="p-0 [&_[cmdk-group-heading]]:px-2.5 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:text-[13px] [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:text-muted-foreground"
							>
								{vanilla.map(item)}
							</CommandGroup>
						)}
					</CommandList>
				</Command>
			</PopoverContent>
		</Popover>
	);
}
