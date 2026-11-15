import { Check } from "lucide-react";
import { motion } from "motion/react";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useTranslation } from "react-i18next";
import { RadioGroup } from "@/components/ui/radio-group";
import { setTheme, THEMES, useAppearance, type Theme } from "@/lib/appearance";
import { cn } from "@/lib/utils";

function MiniWindow({ mode, className }: { mode: "light" | "dark"; className?: string }) {
	return (
		<div className={cn(mode, "absolute -inset-1 flex flex-col bg-background", className)}>
			<div className="relative flex h-[22%] shrink-0 items-center gap-1.5 border-b bg-sidebar px-2">
				<span className="size-2 rounded-full bg-brand" />
				<span className="relative ml-1 h-1.5 w-5 rounded-full bg-foreground/60">
					<span className="absolute inset-x-0.5 -bottom-[5px] h-0.5 rounded-t-[1px] bg-primary" />
				</span>
				<span className="h-1.5 w-5 rounded-full bg-sidebar-foreground/30" />
				<span className="h-1.5 w-5 rounded-full bg-sidebar-foreground/30" />
			</div>
			<span className="mx-2 mt-2 h-[26%] shrink-0 rounded-md bg-primary/40" />
			<div className="flex flex-1 flex-col gap-1.5 p-2">
				<span className="h-1.5 w-3/4 rounded-full bg-foreground/20" />
				<span className="h-1.5 w-1/2 rounded-full bg-foreground/10" />
			</div>
		</div>
	);
}

function ThemePreview({ theme }: { theme: Theme }) {
	if (theme !== "system") return <MiniWindow mode={theme} />;
	return (
		<>
			<MiniWindow mode="dark" />
			<MiniWindow mode="light" className="[clip-path:polygon(0_0,100%_0,0_100%)]" />
		</>
	);
}

export function ThemePicker({ labelledBy }: { labelledBy: string }) {
	const { t } = useTranslation();
	const { theme } = useAppearance();
	return (
		<RadioGroup
			aria-labelledby={labelledBy}
			value={theme}
			onValueChange={(value) => setTheme(value as Theme)}
			className="flex flex-wrap gap-4"
		>
			{THEMES.map((option) => {
				const selected = option === theme;
				return (
					<RadioGroupPrimitive.Item
						key={option}
						value={option}
						className="group flex w-44 cursor-pointer flex-col gap-2 text-left outline-none"
					>
						<div
							className={cn(
								"relative aspect-[16/10] overflow-clip rounded-lg border border-border",
								"transition-[border-color] duration-(--duration-fast) ease-out",
								"group-hover:group-aria-[checked=false]:border-foreground/25",
								"group-aria-checked:outline-2 group-aria-checked:outline-offset-2 group-aria-checked:outline-primary",
								"group-focus-visible:outline-2 group-focus-visible:outline-offset-2 group-focus-visible:outline-foreground group-focus-visible:group-aria-checked:outline-foreground",
							)}
						>
							<ThemePreview theme={option} />
							{selected && (
								<motion.span
									initial={{ opacity: 0 }}
									animate={{ opacity: 1 }}
									transition={{ duration: 0.15 }}
									className="absolute top-2 right-2 z-[1] flex size-5 items-center justify-center rounded-full bg-primary text-primary-foreground"
								>
									<Check className="size-3.5" strokeWidth={2.5} />
								</motion.span>
							)}
						</div>
						<span className="text-center text-[13px]/[18px] font-medium text-muted-foreground transition-colors duration-(--duration-base) ease-out group-data-[state=checked]:text-foreground">
							{t(`settings.theme.${option}`)}
						</span>
					</RadioGroupPrimitive.Item>
				);
			})}
		</RadioGroup>
	);
}
