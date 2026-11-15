import { useTranslation } from "react-i18next";
import { Switch } from "@/components/ui/switch";
import { setReduceMotion, useAppearance } from "@/lib/appearance";
import { AboutSection } from "@/pages/settings/about-section";
import { DiscordSection } from "@/pages/settings/discord-section";
import { GameSection } from "@/pages/settings/game-section";
import { LanguagePicker } from "@/pages/settings/language-picker";
import { SettingRow } from "@/pages/settings/setting-row";
import { ThemePicker } from "@/pages/settings/theme-picker";

function InterfaceSection() {
	const { t } = useTranslation();
	const { reduceMotion } = useAppearance();
	return (
		<section aria-labelledby="settings-interface" className="mt-10">
			<h2 id="settings-interface" className="text-xl/[26px] font-semibold tracking-[-0.015em]">
				{t("settings.interface.title")}
			</h2>
			<div className="mt-2 divide-y divide-border">
				<SettingRow
					inline
					titleId="setting-language"
					title={t("settings.language.title")}
					description={t("settings.language.description")}
				>
					<LanguagePicker labelledBy="setting-language" />
				</SettingRow>
				<SettingRow
					titleId="setting-theme"
					title={t("settings.theme.title")}
					description={t("settings.theme.description")}
				>
					<ThemePicker labelledBy="setting-theme" />
				</SettingRow>
				<SettingRow
					inline
					htmlFor="reduce-motion"
					title={t("settings.reduceMotion.title")}
					description={t("settings.reduceMotion.description")}
				>
					<Switch id="reduce-motion" checked={reduceMotion} onCheckedChange={setReduceMotion} />
				</SettingRow>
			</div>
		</section>
	);
}

export function SettingsPage() {
	const { t } = useTranslation();
	return (
		<div>
			<h1 className="text-[28px]/[34px] font-semibold tracking-[-0.025em]">{t("settings.title")}</h1>
			<p className="mt-1 max-w-prose text-sm text-muted-foreground">{t("settings.description")}</p>
			<InterfaceSection />
			<GameSection />
			<DiscordSection />
			<AboutSection />
		</div>
	);
}
