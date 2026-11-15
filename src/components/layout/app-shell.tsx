import { AnimatePresence, motion } from "motion/react";
import { lazy, Suspense, useEffect, useRef, type ComponentType } from "react";
import { AddAccountDialog } from "@/components/accounts/add-account-dialog";
import { RemoveAccountDialog } from "@/components/accounts/remove-account-dialog";
import { DepsDialog } from "@/components/catalog/deps-dialog";
import { DropOverlay } from "@/components/drop-overlay";
import { ErrorBoundary } from "@/components/error-boundary";
import { ScrollContainerContext } from "@/components/layout/scroll-container";
import { ManualDialog } from "@/components/manual/manual-dialog";
import { SkinDialogs } from "@/components/skins/skin-dialogs";
import { SkinsLocked } from "@/components/skins/skins-locked";
import { SkinsPageSkeleton } from "@/components/skins/skins-skeleton";
import { FailedFilesDialog } from "@/components/modpacks/failed-files-dialog";
import { InstallModpackDialog } from "@/components/modpacks/install-modpack-dialog";
import { UnlinkModpackDialog } from "@/components/modpacks/unlink-modpack-dialog";
import { UpdateModpackDialog } from "@/components/modpacks/update-modpack-dialog";
import { activeAccount, initAccounts, useAccounts } from "@/lib/accounts";
import { initCatalogSources } from "@/lib/catalog";
import { initContent } from "@/lib/content";
import { initGameLog } from "@/lib/game-log";
import { initScreenshots } from "@/lib/screenshots";
import { initWorlds } from "@/lib/worlds";
import { initDownloads } from "@/lib/downloads";
import { useAppearance } from "@/lib/appearance";
import { initInstances } from "@/lib/instances";
import { initManual } from "@/lib/manual";
import { initMigrationNotice } from "@/lib/migration";
import { useRoute, type SectionPath } from "@/lib/router";
import { initSkins } from "@/lib/skins";
import { initTrayNotice } from "@/lib/tray";
import { cn } from "@/lib/utils";
import { CatalogPage } from "@/pages/catalog/catalog-page";
import { ProjectPage } from "@/pages/catalog/project-page";
import { HomePage } from "@/pages/home";
import { CreateInstanceDialog } from "@/pages/instances/create-instance-dialog";
import { GroupDialogs } from "@/pages/instances/group-dialogs";
import { InstanceDialogs } from "@/pages/instances/instance-dialogs";
import { InstancePage } from "@/pages/instances/instance-page";
import { InstancesPage } from "@/pages/instances/instances-page";
import { SettingsPage } from "@/pages/settings/settings-page";

/** Просмотр в 3D тяжёлый: грузится при первом входе на страницу. */
const LazySkinsPage = lazy(() =>
	import("@/pages/skins/skins-page").then((module) => ({ default: module.SkinsPage })),
);

/** У офлайн-аккаунта раздел закрыт, код страницы не грузится. */
function SkinsRoute() {
	const accounts = useAccounts();
	if (accounts.status === "idle" || accounts.status === "loading") return <SkinsPageSkeleton />;
	if (activeAccount(accounts)?.kind === "offline") return <SkinsLocked />;
	return (
		<Suspense fallback={<SkinsPageSkeleton />}>
			<LazySkinsPage />
		</Suspense>
	);
}

const pages: Record<SectionPath, ComponentType> = {
	"/": HomePage,
	"/instances": InstancesPage,
	"/catalog": CatalogPage,
	"/skins": SkinsRoute,
	"/settings": SettingsPage,
};

const easeOut = [0.22, 1, 0.36, 1] as const;

export function AppShell() {
	const { reduceMotion } = useAppearance();
	const route = useRoute();
	const scrollRef = useRef<HTMLElement>(null);
	const Page = route.startsWith("/instances/")
		? InstancePage
		: route.startsWith("/catalog/")
			? ProjectPage
			: pages[route as SectionPath];
	// Эти страницы рисуют отступы сами.
	const fullBleed = route === "/" || route === "/skins" || route.startsWith("/instances/");
	// Прокручиваются части страницы, а не холст.
	const fullHeight = route === "/catalog" || route === "/skins";

	useEffect(() => {
		initAccounts();
		initInstances();
		initDownloads();
		initContent();
		initWorlds();
		initScreenshots();
		initGameLog();
		initCatalogSources();
		initManual();
		initSkins();
		void initMigrationNotice();
		initTrayNotice();
	}, []);

	return (
		<ScrollContainerContext value={scrollRef}>
			<div className="flex min-h-0 flex-1 flex-col">
				<main
					ref={scrollRef}
					className={cn(
						// Место под полосу прокрутки занято всегда, чтобы ширина холста не прыгала между страницами.
						"min-h-0 flex-1 [scrollbar-gutter:stable] bg-background",
						fullHeight ? "overflow-hidden" : "overflow-y-auto",
					)}
				>
					{/* Сдвиг страницы при появлении не должен давать полосу прокрутки. */}
					<div className={cn("flex flex-col overflow-clip", fullHeight ? "h-full" : "min-h-full")}>
						<AnimatePresence mode="wait" onExitComplete={() => scrollRef.current?.scrollTo(0, 0)}>
							<motion.div
								key={route}
								initial={{ opacity: 0, y: 8 }}
								animate={{
									opacity: 1,
									y: 0,
									transition: { duration: reduceMotion ? 0.12 : 0.22, ease: easeOut },
								}}
								exit={{ opacity: 0, transition: { duration: 0.12, ease: easeOut } }}
								className={cn(
									"flex min-h-0 flex-1 flex-col",
									!fullBleed && (fullHeight ? "px-8 pt-7" : "px-8 pt-7 pb-10"),
								)}
							>
								<ErrorBoundary>
									<Page />
								</ErrorBoundary>
							</motion.div>
						</AnimatePresence>
					</div>
				</main>
				<AddAccountDialog />
				<RemoveAccountDialog />
				<CreateInstanceDialog />
				<InstanceDialogs />
				<GroupDialogs />
				<DepsDialog />
				<InstallModpackDialog />
				<UpdateModpackDialog />
				<UnlinkModpackDialog />
				<FailedFilesDialog />
				<ManualDialog />
				<SkinDialogs />
				<DropOverlay />
			</div>
		</ScrollContainerContext>
	);
}
