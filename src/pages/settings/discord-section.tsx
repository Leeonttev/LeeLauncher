import { Info } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Switch } from "@/components/ui/switch";
import { discordApi, type DiscordStatus } from "@/lib/api";
import { initGameSettings, updateSettings, useGameSettings } from "@/lib/game-settings";
import { SettingRow } from "@/pages/settings/setting-row";

type StatusState = { status: "loading" } | { status: "ready"; value: DiscordStatus } | { status: "error" };

export function DiscordSection() {
	const { t } = useTranslation();
	const settings = useGameSettings().settings;
	const [discord, setDiscord] = useState<StatusState>({ status: "loading" });
	const [saving, setSaving] = useState(false);
	const [checking, setChecking] = useState(false);
	const mounted = useRef(true);

	useEffect(() => initGameSettings(), []);

	// Один раз при входе на страницу: событий о смене связи ядро не шлёт.
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);

	useEffect(() => {
		let alive = true;
		discordApi
			.status()
			.then((value) => alive && setDiscord({ status: "ready", value }))
			.catch((err: unknown) => {
				console.warn("discord_status", err);
				if (alive) setDiscord({ status: "error" });
			});
		return () => {
			alive = false;
		};
	}, []);

	const value = settings.value;
	const unavailable = discord.status === "ready" && !discord.value.configured;
	const ready =
		settings.status === "ready" && value !== null && discord.status !== "loading" && !unavailable;
	const presence = value?.discordPresence ?? true;
	const showInstance = value?.discordShowInstance ?? true;
	const notRunning =
		!checking &&
		discord.status === "ready" &&
		discord.value.configured &&
		!discord.value.connected &&
		presence;

	// Не отключаем переключатель на время записи: с `disabled` пропал бы фокус.
	async function save(patch: { discordPresence?: boolean; discordShowInstance?: boolean }) {
		if (saving) return;
		setSaving(true);
		const enabling = patch.discordPresence === true;
		if (enabling) setChecking(true);
		const ok = await updateSettings(patch);
		setSaving(false);
		if (!enabling) return;
		if (!ok) {
			setChecking(false);
			return;
		}
		// Сразу после включения ядро ещё только подключается к Discord.
		setTimeout(() => {
			discordApi
				.status()
				.then((value) => mounted.current && setDiscord({ status: "ready", value }))
				.catch((err: unknown) => console.warn("discord_status", err))
				.finally(() => mounted.current && setChecking(false));
		}, 2000);
	}

	return (
		<section aria-labelledby="settings-discord" className="mt-10">
			<h2 id="settings-discord" className="text-xl/[26px] font-semibold tracking-[-0.015em]">
				{t("settings.discord.title")}
			</h2>
			{unavailable && (
				<p className="mt-2 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
					<Info className="shrink-0" />
					{t("settings.discord.unavailable")}
				</p>
			)}
			<div className="mt-2 divide-y divide-border">
				<SettingRow
					inline
					htmlFor="discord-presence"
					title={t("settings.discord.presence.title")}
					description={t("settings.discord.presence.description")}
					note={
						notRunning && (
							<p className="mt-1.5 flex items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
								<Info className="shrink-0" />
								{t("settings.discord.notRunning")}
							</p>
						)
					}
				>
					<Switch
						id="discord-presence"
						checked={ready && presence}
						disabled={!ready}
						onCheckedChange={(checked) => void save({ discordPresence: checked })}
					/>
				</SettingRow>
				<SettingRow
					inline
					htmlFor="discord-show-instance"
					title={t("settings.discord.showInstance.title")}
					description={t("settings.discord.showInstance.description")}
				>
					<Switch
						id="discord-show-instance"
						checked={ready && presence && showInstance}
						disabled={!ready || !presence}
						onCheckedChange={(checked) => void save({ discordShowInstance: checked })}
					/>
				</SettingRow>
			</div>
		</section>
	);
}
