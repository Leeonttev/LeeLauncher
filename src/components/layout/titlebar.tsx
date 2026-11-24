import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AccountSwitcher } from "@/components/accounts/account-switcher";
import { DownloadsButton } from "@/components/downloads/downloads-button";
import { NavTabs, SettingsButton } from "@/components/layout/nav-tabs";
import { WindowButton, WindowGlyph } from "@/components/layout/window-controls";
import { Lockup, Spark } from "@/components/brand/identity";
import { UpdateReminderButton } from "@/components/updates/update-reminder-button";
import { titlebarClasses } from "@/components/layout/titlebar-layout";
import { appWindow } from "@/lib/api";
import { isMac } from "@/lib/platform";
import { cn } from "@/lib/utils";

function useIsMaximized() {
	const [maximized, setMaximized] = useState(false);
	useEffect(() => {
		let disposed = false;
		let unlisten: (() => void) | undefined;
		const refresh = () => {
			appWindow
				.isMaximized()
				.then((value) => {
					if (!disposed) setMaximized(value);
				})
				.catch(() => {});
		};
		refresh();
		appWindow
			.onResized(refresh)
			.then((stop) => {
				if (disposed) stop();
				else unlisten = stop;
			})
			.catch(() => {});
		return () => {
			disposed = true;
			unlisten?.();
		};
	}, []);
	return maximized;
}

/** Таскает окно: у обёртки `data-tauri-drag-region`, у детей нет событий. */
function Brand() {
	return (
		<div
			data-tauri-drag-region
			className={cn("flex h-full shrink-0 items-center text-foreground", titlebarClasses.brand)}
		>
			<span data-tauri-drag-region className={cn("[&>*]:pointer-events-none", titlebarClasses.lockup)}>
				<Lockup size={22} />
			</span>
			<span data-tauri-drag-region className={cn("[&>*]:pointer-events-none", titlebarClasses.spark)}>
				<Spark accent size={22} />
			</span>
		</div>
	);
}

/**
 * Пустые места таскают окно. Линия снизу лежит поверх, чтобы планка активной вкладки стояла на ней.
 * На Mac кнопки окна системные (слева, поверх полосы), своих нет.
 */
export function Titlebar({
	variant = "full",
	brand = false,
}: {
	variant?: "full" | "minimal";
	brand?: boolean;
}) {
	return (
		<header
			data-tauri-drag-region
			className="@container/titlebar relative flex h-14 shrink-0 items-center bg-sidebar select-none after:pointer-events-none after:absolute after:inset-x-0 after:bottom-0 after:z-10 after:h-px after:bg-border after:content-['']"
		>
			<AnimatePresence initial={false}>
				{variant === "full" && (
					<motion.div
						key="nav"
						data-tauri-drag-region
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
						className="flex h-full min-w-0 flex-1 items-center"
					>
						<Brand />
						<NavTabs />
						<div data-tauri-drag-region className="h-full flex-1" />
						<div
							data-tauri-drag-region
							className={cn("flex shrink-0 items-center gap-1", isMac ? "pr-3" : "pr-2")}
						>
							<AccountSwitcher />
							<UpdateReminderButton />
							<DownloadsButton />
							<SettingsButton />
						</div>
						{!isMac && (
							<span data-tauri-drag-region aria-hidden className="mr-1 h-6 shrink-0 border-l" />
						)}
					</motion.div>
				)}
			</AnimatePresence>
			{variant === "minimal" && (
				<>
					{brand && <Brand />}
					<div data-tauri-drag-region className="h-full flex-1" />
				</>
			)}
			{!isMac && <WindowControls />}
		</header>
	);
}

function WindowControls() {
	const { t } = useTranslation();
	const maximized = useIsMaximized();
	return (
		<div className="flex h-full shrink-0">
			<WindowButton
				label={t("window.minimize")}
				onClick={() => void appWindow.minimize().catch(() => {})}
			>
				<WindowGlyph name="minimize" />
			</WindowButton>
			<WindowButton
				label={maximized ? t("window.restore") : t("window.maximize")}
				onClick={() => void appWindow.toggleMaximize().catch(() => {})}
			>
				<WindowGlyph name={maximized ? "restore" : "maximize"} />
			</WindowButton>
			<WindowButton
				label={t("window.close")}
				danger
				onClick={() => void appWindow.close().catch(() => {})}
			>
				<WindowGlyph name="close" />
			</WindowButton>
		</div>
	);
}
