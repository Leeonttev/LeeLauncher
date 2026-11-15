import { Check, ChevronsUpDown } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import type { ContentVersion, VersionChannel } from "@/lib/api";
import { formatAgo } from "@/lib/format";
import { versionTarget } from "@/lib/modpacks";
import { cn } from "@/lib/utils";

export function ChannelChip({ channel }: { channel: VersionChannel }) {
	const { t } = useTranslation();
	return (
		<span
			className={cn(
				"inline-flex h-[22px] shrink-0 items-center rounded-md px-[7px] text-[13px] font-medium",
				channel === "release" ? "bg-primary-soft text-primary-text" : "bg-warning/18 text-warning",
			)}
		>
			{t(`project.versions.chip.${channel}`)}
		</span>
	);
}

function searchText(version: ContentVersion) {
	return `${version.number} ${version.name}`;
}

export function ModpackVersionPicker({
	versions,
	value,
	onChange,
	labelledBy,
	loading,
	disabled,
	className,
}: {
	versions: ContentVersion[];
	value: string | null;
	onChange: (id: string) => void;
	labelledBy: string;
	loading?: boolean;
	disabled?: boolean;
	className?: string;
}) {
	const { t } = useTranslation();
	const [open, setOpen] = useState(false);
	const hasRelease = versions.some((v) => v.channel === "release");
	const [unstable, setUnstable] = useState<boolean | null>(null);
	const listRef = useRef<HTMLDivElement>(null);
	const selected = versions.find((v) => v.id === value) ?? null;
	// Переключатель выключен, если есть релизы; выбранная бета видна всегда.
	const showUnstable = unstable ?? !hasRelease;
	const shown = versions.filter((v) => showUnstable || v.channel === "release" || v.id === value);

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
					className={cn(
						"relative flex h-10 w-full items-center gap-2 rounded-lg border border-input bg-field px-3 text-left text-sm transition-[color,border-color] duration-(--duration-fast) focus-visible:border-ring focus-visible:outline-3 focus-visible:outline-offset-0 focus-visible:outline-primary-soft disabled:pointer-events-none disabled:opacity-50 aria-expanded:border-ring",
						className,
					)}
				>
					{loading ? (
						<Skeleton className="h-4 w-40" />
					) : selected ? (
						<span className="flex min-w-0 flex-1 items-center gap-2">
							<span className="truncate font-semibold">{selected.number}</span>
							<ChannelChip channel={selected.channel} />
							<span className="min-w-0 truncate text-[13px]/[18px] text-muted-foreground">
								{versionTarget(selected)}
							</span>
						</span>
					) : (
						<span className="flex-1" />
					)}
					<ChevronsUpDown className="ml-auto shrink-0 text-muted-foreground" />
				</button>
			</PopoverTrigger>
			<PopoverContent align="start" sideOffset={6} fit className="w-100 overflow-clip p-0">
				<Command
					filter={(item, search) =>
						item.toLowerCase().includes(search.trim().toLowerCase()) ? 1 : 0
					}
					defaultValue={selected ? searchText(selected) : undefined}
					className="bg-transparent"
				>
					<CommandInput placeholder={t("modpack.dialog.findVersion")} />
					<CommandList ref={listRef} className="max-h-[288px] p-1.5">
						<CommandEmpty className="py-6 text-center text-sm text-muted-foreground">
							{t("modpack.dialog.versionNotFound")}
						</CommandEmpty>
						{shown.map((version) => {
							const active = version.id === value;
							return (
								<CommandItem
									key={version.id}
									value={searchText(version)}
									className="h-auto gap-3 px-2.5 py-1.5"
									onSelect={() => {
										onChange(version.id);
										setOpen(false);
									}}
								>
									<span className="min-w-0 flex-1">
										<span className="flex min-w-0 items-center gap-2">
											<span
												className={cn(
													"truncate text-sm/5",
													active && "font-semibold",
												)}
											>
												{version.number}
											</span>
											<ChannelChip channel={version.channel} />
										</span>
										<span className="block truncate text-[13px]/[18px] text-muted-foreground">
											{[versionTarget(version), formatAgo(version.publishedAt)]
												.filter(Boolean)
												.join(" · ")}
										</span>
									</span>
									<Check
										className={cn(
											"size-3.5",
											"shrink-0 text-primary-text",
											!active && "invisible",
										)}
									/>
								</CommandItem>
							);
						})}
					</CommandList>
				</Command>
				<label className="flex h-11 shrink-0 cursor-pointer items-center justify-between gap-3 border-t px-3 text-sm">
					{t("modpack.dialog.showUnstable")}
					<Switch checked={showUnstable} onCheckedChange={setUnstable} />
				</label>
			</PopoverContent>
		</Popover>
	);
}
