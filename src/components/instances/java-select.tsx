import { FolderOpen } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectSeparator,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { javaApi, type JavaInstall } from "@/lib/api";
import { errorText, toAppError } from "@/lib/errors";
import { cn } from "@/lib/utils";

const AUTO = "auto";
const BROWSE = "browse";

type JavaSelectProps = {
	id?: string;
	/** `null` — авто. */
	value: string | null;
	onChange: (path: string | null) => void;
	/** `null` — ещё ищем. */
	installs: JavaInstall[] | null;
	/** Нужная версия, остальные неактивны; `null` — подходят все. */
	major: number | null;
	autoLabel: string;
	autoNote?: string;
	/** Без подписи неподходящие скрыты. */
	incompatibleNote?: string;
	foundLabel?: string;
	ariaLabel?: string;
	className?: string;
	disabled?: boolean;
};

function javaName(java: JavaInstall) {
	return java.vendor ? `${java.version} · ${java.vendor}` : java.version;
}

export function JavaSelect({
	id,
	value,
	onChange,
	installs,
	major,
	autoLabel,
	autoNote,
	incompatibleNote,
	foundLabel,
	ariaLabel,
	className,
	disabled,
}: JavaSelectProps) {
	const { t, i18n } = useTranslation();
	const [picking, setPicking] = useState(false);
	const list = (installs ?? []).filter(
		(java) => incompatibleNote || major === null || java.major === major,
	);
	const known = value ? list.find((java) => java.path === value) : undefined;

	const selectedLabel = value
		? known
			? `Java ${javaName(known)}`
			: t("instance.settings.java.customPath")
		: autoLabel;

	const browse = async () => {
		if (picking) return;
		setPicking(true);
		try {
			const java = await javaApi.pickFile(i18n.language === "en" ? "en" : "ru");
			if (!java) return;
			if (major !== null && java.major !== major) {
				toast.error(t("settings.game.java.wrongMajor", { found: java.major, major }));
				return;
			}
			onChange(java.path);
		} catch (err) {
			toast.error(errorText(toAppError(err)));
		} finally {
			setPicking(false);
		}
	};

	return (
		<Select
			value={value ?? AUTO}
			disabled={disabled}
			onValueChange={(next) => {
				if (next === BROWSE) void browse();
				else onChange(next === AUTO ? null : next);
			}}
		>
			<SelectTrigger id={id} aria-label={ariaLabel} className={cn("h-9", className)}>
				<SelectValue>
					<span className="truncate">{selectedLabel}</span>
				</SelectValue>
			</SelectTrigger>
			<SelectContent
				position="popper"
				align="end"
				className="w-(--radix-select-trigger-width) min-w-80"
			>
				<SelectItem value={AUTO}>
					<span className="flex-1 truncate">{autoLabel}</span>
					{autoNote && <span className="text-xs text-muted-foreground">{autoNote}</span>}
				</SelectItem>
				{list.length > 0 && (
					<>
						<SelectSeparator />
						{/* `SelectLabel` у Radix падает вне `SelectGroup`, а вместе с ним вся страница. */}
						<SelectGroup>
							{foundLabel && (
								<SelectLabel className="text-xs/4 text-muted-foreground">
									{foundLabel}
								</SelectLabel>
							)}
							{list.map((java) => {
								const fits = major === null || java.major === major;
								return (
									<SelectItem
										key={java.path}
										value={java.path}
										disabled={!fits}
										className="py-2"
									>
										<span className="flex min-w-0 flex-col">
											<span className="truncate">{`Java ${javaName(java)}`}</span>
											<span className="truncate font-mono text-xs/4 text-muted-foreground">
												{fits ? java.path : incompatibleNote}
											</span>
										</span>
									</SelectItem>
								);
							})}
						</SelectGroup>
					</>
				)}
				{value && !known && (
					<SelectItem value={value} className="py-2">
						<span className="flex min-w-0 flex-col">
							<span className="truncate">{t("instance.settings.java.customPath")}</span>
							<span className="truncate font-mono text-xs/4 text-muted-foreground">
								{value}
							</span>
						</span>
					</SelectItem>
				)}
				<SelectSeparator />
				<SelectItem value={BROWSE}>
					<FolderOpen />
					{t("instance.settings.java.browse")}
				</SelectItem>
			</SelectContent>
		</Select>
	);
}
