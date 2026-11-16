import { FolderOpen, LoaderCircle, OctagonX, RefreshCw } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { JavaSelect } from "@/components/instances/java-select";
import { MemoryField } from "@/components/instances/memory-field";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import type { JavaInstall } from "@/lib/api";
import { errorText } from "@/lib/errors";
import { formatMemory } from "@/lib/format";
import {
	defaultMemory,
	initGameSettings,
	openAppFolder,
	reloadSettings,
	rescanJava,
	updateSettings,
	useGameSettings,
} from "@/lib/game-settings";
import { setSameAccountWarning, useSameAccountWarning } from "@/lib/instances";
import { cn } from "@/lib/utils";
import { SettingRow } from "@/pages/settings/setting-row";

const JAVA_MAJORS = [21, 17, 8] as const;
type JavaMajor = (typeof JAVA_MAJORS)[number];

export function GameSection() {
	const { t } = useTranslation();
	const game = useGameSettings();

	useEffect(() => initGameSettings(), []);

	return (
		<section aria-labelledby="settings-game" className="mt-10">
			<h2 id="settings-game" className="text-xl/[26px] font-semibold tracking-[-0.015em]">
				{t("settings.game.title")}
			</h2>
			{game.settings.status === "error" && (
				<div className="mt-4 flex items-center gap-2">
					<OctagonX className="shrink-0 text-destructive" />
					<span className="text-[13px]/[18px]">{t("settings.game.loadFailed")}</span>
					<Button variant="ghost" size="sm" onClick={reloadSettings}>
						{t("common.retry")}
					</Button>
				</div>
			)}
			<div className="mt-2 divide-y divide-border">
				<MemoryRow />
				<JavaRows />
				<SameAccountRow />
				<DataFolderRow />
			</div>
		</section>
	);
}

function MemoryRow() {
	const { t } = useTranslation();
	const game = useGameSettings();
	const memory = defaultMemory(game);
	const loaded =
		game.settings.status === "ready" && game.memory.status !== "loading" && game.memory.status !== "idle";
	const totalMb = loaded ? (game.memory.value?.totalMb ?? 0) : null;

	return (
		<SettingRow
			titleId="settings-memory"
			title={t("settings.game.memory.title")}
			description={t("settings.game.memory.description")}
		>
			<MemoryField
				valueMb={memory?.maxMb ?? 2048}
				totalMb={memory ? totalMb : null}
				labelledBy="settings-memory"
				hint={totalMb ? t("settings.game.memory.total", { total: formatMemory(totalMb) }) : ""}
				onCommit={(mb) =>
					void updateSettings({
						defaultMemory: { minMb: Math.min(memory?.minMb ?? 512, mb), maxMb: mb },
					})
				}
			/>
		</SettingRow>
	);
}

function JavaRows() {
	const { t } = useTranslation();
	const game = useGameSettings();
	const scanning = game.java.status === "loading" || game.java.status === "idle";
	const settingsLoading = game.settings.status !== "ready";

	return (
		<div className="py-5">
			<div className="flex items-start gap-6">
				<div className="min-w-0 flex-1">
					<h3 className="text-sm font-medium">{t("settings.game.java.title")}</h3>
					<p className="mt-0.5 max-w-prose text-[13px]/[18px] text-muted-foreground">
						{t("settings.game.java.description")}
					</p>
				</div>
				<Button variant="outline" size="sm" disabled={scanning} onClick={() => void rescanJava()}>
					{scanning ? <LoaderCircle className="animate-spin" /> : <RefreshCw />}
					{scanning ? t("settings.game.java.scanning") : t("settings.game.java.rescan")}
				</Button>
			</div>
			<div className="mt-4 divide-y divide-border">
				{JAVA_MAJORS.map((major) =>
					scanning || settingsLoading ? (
						<div key={major} className="py-3">
							<Skeleton className="h-9 w-full" />
						</div>
					) : (
						<JavaMajorRow
							key={major}
							major={major}
							path={game.settings.value?.javaPaths[String(major)] ?? null}
							installs={game.java.value ?? []}
						/>
					),
				)}
			</div>
		</div>
	);
}

function JavaMajorRow({
	major,
	path,
	installs,
}: {
	major: JavaMajor;
	path: string | null;
	installs: JavaInstall[];
}) {
	const { t } = useTranslation();
	const label = `Java ${major}`;
	let status: string;
	let missing = false;
	if (path) {
		const found = installs.find((java) => java.path === path);
		missing = !found;
		status = found
			? t("settings.game.java.statusFound", { version: found.version })
			: t("settings.game.java.statusMissing");
	} else {
		const managed = installs.find((java) => java.managed && java.major === major);
		status = managed
			? t("settings.game.java.statusManaged", { version: managed.version })
			: t("settings.game.java.statusPending");
	}

	return (
		<div className="flex items-center gap-4 py-3">
			<div className="w-44 shrink-0">
				<p className="text-sm/5 font-medium">{label}</p>
				<p className="text-xs/4 text-muted-foreground">{t(`settings.game.java.for${major}`)}</p>
			</div>
			<p
				className={cn(
					"flex min-w-0 flex-1 items-center gap-1.5 text-[13px]/[18px]",
					missing ? "text-destructive" : "text-muted-foreground",
				)}
			>
				{missing && <OctagonX className="shrink-0" />}
				<span className="truncate">{status}</span>
			</p>
			<JavaSelect
				value={path}
				installs={installs}
				major={major}
				autoLabel={t("settings.game.java.auto")}
				autoNote={t("settings.game.java.recommended")}
				ariaLabel={label}
				className="w-72"
				onChange={(next) => void updateSettings({ javaPaths: { [String(major)]: next } })}
			/>
		</div>
	);
}

function SameAccountRow() {
	const { t } = useTranslation();
	const warn = useSameAccountWarning();
	return (
		<SettingRow
			inline
			htmlFor="settings-same-account"
			title={t("settings.game.sameAccountWarning.title")}
			description={t("settings.game.sameAccountWarning.description")}
		>
			<Switch id="settings-same-account" checked={warn} onCheckedChange={setSameAccountWarning} />
		</SettingRow>
	);
}

function DataFolderRow() {
	const { t } = useTranslation();
	const paths = useGameSettings().paths;

	return (
		<div className="flex items-center gap-6 py-5">
			<div className="min-w-0 flex-1">
				<h3 className="text-sm font-medium">{t("settings.game.dataFolder.title")}</h3>
				{paths.value ? (
					<p
						title={paths.value.data}
						className="mt-0.5 truncate font-mono text-[13px]/[18px] text-muted-foreground select-text"
					>
						{paths.value.data}
					</p>
				) : paths.status === "error" && paths.error ? (
					<p className="mt-0.5 flex items-start gap-1.5 text-[13px]/[18px] text-destructive">
						<OctagonX className="mt-px shrink-0" />
						<span className="min-w-0">{errorText(paths.error)}</span>
					</p>
				) : (
					<Skeleton className="mt-0.5 h-[18px] w-72" />
				)}
			</div>
			<Button variant="outline" size="sm" onClick={() => void openAppFolder("data")}>
				<FolderOpen />
				{t("common.openFolder")}
			</Button>
		</div>
	);
}
