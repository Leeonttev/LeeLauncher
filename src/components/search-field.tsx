import { Search, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { KeyboardEvent, ReactNode, Ref } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

export function SearchField({
	value,
	onChange,
	placeholder,
	className,
	inputRef,
	extra,
	onKeyDown,
	keyShortcuts,
}: {
	value: string;
	onChange: (value: string) => void;
	placeholder: string;
	className?: string;
	inputRef?: Ref<HTMLInputElement>;
	extra?: ReactNode;
	onKeyDown?: (event: KeyboardEvent<HTMLInputElement>) => void;
	keyShortcuts?: string;
}) {
	const { t } = useTranslation();
	return (
		<Input
			ref={inputRef}
			type="search"
			value={value}
			placeholder={placeholder}
			aria-label={placeholder}
			aria-keyshortcuts={keyShortcuts}
			spellCheck={false}
			autoComplete="off"
			className={cn("h-9", className)}
			inputClassName="px-2 [&::-webkit-search-cancel-button]:hidden"
			start={<Search className="pointer-events-none ml-2.5 text-muted-foreground" />}
			end={
				<>
					{extra}
					{value && (
						<Button
							variant="ghost"
							size="icon-xs"
							aria-label={t("common.clear")}
							className="mr-0.5 size-7 text-muted-foreground focus-visible:bg-accent"
							onClick={(event) => {
								onChange("");
								(
									event.currentTarget.parentElement?.querySelector(
										"input",
									) as HTMLInputElement | null
								)?.focus();
							}}
						>
							<X />
						</Button>
					)}
				</>
			}
			onChange={(event) => onChange(event.target.value)}
			onKeyDown={(event) => {
				onKeyDown?.(event);
				if (event.defaultPrevented) return;
				if (event.key === "Escape" && value) {
					event.preventDefault();
					event.stopPropagation();
					onChange("");
				}
			}}
		/>
	);
}
