import { ArrowRight } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { RadioGroup as RadioGroupPrimitive } from "radix-ui";
import { useEffect, useRef, type FormEvent, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { AppIcon } from "@/components/brand/identity";
import { Button } from "@/components/ui/button";
import { RadioGroup } from "@/components/ui/radio-group";
import { useAppearance } from "@/lib/appearance";
import {
	LANGUAGE_INFO,
	LANGUAGES,
	previewLanguage,
	setLanguage,
	useLanguage,
	type Language,
} from "@/lib/i18n";
import { cn } from "@/lib/utils";

const easeOut = [0.22, 1, 0.36, 1] as const;

function SwapText({ children, className }: { children: string; className?: string }) {
	const { i18n } = useTranslation();
	return (
		<span className={cn("grid overflow-clip", className)}>
			<AnimatePresence mode="popLayout" initial={false}>
				<motion.span
					key={i18n.language}
					className="col-start-1 row-start-1"
					initial={{ opacity: 0, y: 6 }}
					animate={{ opacity: 1, y: 0, transition: { duration: 0.18, ease: easeOut } }}
					exit={{ opacity: 0, y: -6, transition: { duration: 0.12, ease: easeOut } }}
				>
					{children}
				</motion.span>
			</AnimatePresence>
		</span>
	);
}

function LanguageOption({ language, selected }: { language: Language; selected: boolean }) {
	const info = LANGUAGE_INFO[language];
	return (
		<RadioGroupPrimitive.Item
			value={language}
			className="relative flex h-14 w-full cursor-pointer items-center gap-3 rounded-xl border border-border bg-card px-4 text-left transition-[border-color,background-color,scale] duration-(--duration-fast) ease-out hover:bg-accent active:scale-[0.98] aria-checked:border-primary aria-checked:bg-primary-soft"
		>
			<span
				aria-hidden
				className={cn(
					"size-[18px] shrink-0 rounded-full border transition-[border-color,border-width] duration-(--duration-fast)",
					selected ? "border-[5px] border-primary bg-card" : "border-muted-foreground/60 bg-field",
				)}
			/>
			<span lang={language} className="text-sm font-medium">
				{info.name}
			</span>
			<span lang={language} className="ml-auto truncate text-[13px]/[18px] text-muted-foreground">
				{info.description}
			</span>
		</RadioGroupPrimitive.Item>
	);
}

/** Выбор сразу меняет язык экрана, ключ пишется по «Продолжить». */
export function LanguageOnboarding() {
	const { t } = useTranslation();
	const { resolvedLanguage } = useLanguage();
	const { reduceMotion } = useAppearance();
	const groupRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		groupRef.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
	}, []);

	const appear = (delay: number) => ({
		initial: { opacity: 0, y: reduceMotion ? 0 : 8 },
		animate: { opacity: 1, y: 0 },
		transition: reduceMotion ? { duration: 0.12 } : { duration: 0.22, delay, ease: easeOut },
	});

	const submit = (event: FormEvent) => {
		event.preventDefault();
		setLanguage(resolvedLanguage);
	};

	const onKeyDown = (event: KeyboardEvent<HTMLFormElement>) => {
		// Radix не обрабатывает Enter на радиокнопках.
		if (event.key !== "Enter" || (event.target as HTMLElement).closest('[type="submit"]')) return;
		event.preventDefault();
		event.currentTarget.requestSubmit();
	};

	return (
		<div className="flex min-h-0 flex-1 items-center justify-center overflow-hidden bg-background">
			<form
				onSubmit={submit}
				onKeyDown={onKeyDown}
				className="-mt-10 flex w-full max-w-[400px] flex-col items-center text-center"
			>
				<motion.div
					initial={{ opacity: 0 }}
					animate={{ opacity: 1 }}
					transition={reduceMotion ? { duration: 0.12 } : { duration: 0.26, ease: easeOut }}
				>
					<AppIcon size={72} />
				</motion.div>
				<motion.div {...appear(0.06)} className="flex flex-col items-center">
					<h1 className="mt-6 text-[28px]/[34px] font-semibold tracking-[-0.025em]">
						<SwapText>{t("onboarding.language.title")}</SwapText>
					</h1>
					<p className="mt-2 text-sm text-muted-foreground">
						<SwapText>{t("onboarding.language.description")}</SwapText>
					</p>
				</motion.div>
				<RadioGroup
					ref={groupRef}
					aria-label={t("onboarding.language.groupLabel")}
					value={resolvedLanguage}
					onValueChange={(value) => previewLanguage(value as Language)}
					className="mt-8 flex w-full flex-col gap-3"
				>
					{LANGUAGES.map((language, index) => (
						<motion.div key={language} {...appear(0.12 + index * 0.04)}>
							<LanguageOption language={language} selected={language === resolvedLanguage} />
						</motion.div>
					))}
				</RadioGroup>
				<motion.div {...appear(0.2)} className="mt-6 w-full">
					<Button type="submit" className="h-10 w-full">
						<SwapText>{t("onboarding.language.continue")}</SwapText>
						<ArrowRight className="ml-1" />
					</Button>
				</motion.div>
			</form>
		</div>
	);
}
