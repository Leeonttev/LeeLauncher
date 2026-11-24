import { Copy, FolderOpen, Image, LoaderCircle, OctagonX, RotateCcw, Trash2 } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CenteredState } from "@/components/centered-state";
import { ImageViewer, ViewerButton, type ViewerItem } from "@/components/image-viewer";
import { useScrollContainer } from "@/components/layout/scroll-container";
import { PixelGrid } from "@/components/pixel-grid";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { assetUrl, type Instance, type Screenshot } from "@/lib/api";
import { errorText } from "@/lib/errors";
import { formatDateTime } from "@/lib/format";
import { openInstanceFolder } from "@/lib/instances";
import {
	copyScreenshot,
	loadMoreScreenshots,
	loadScreenshots,
	removeScreenshot,
	useScreenshots,
} from "@/lib/screenshots";
import { cn } from "@/lib/utils";
import { isDeleteKey, isPrimaryMod } from "@/lib/platform";

export function ScreenshotsTab({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const shots = useScreenshots(instance.id, true);
	const loading = !shots || shots.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);
	const items = shots?.status === "ready" ? shots.items : [];
	const [viewer, setViewer] = useState<{ open: boolean; index: number }>({ open: false, index: 0 });
	const [opener, setOpener] = useState<HTMLElement | null>(null);
	const [removing, setRemoving] = useState<Screenshot | null>(null);
	const current = items[Math.min(viewer.index, items.length - 1)] ?? null;

	const viewerItems: ViewerItem[] = items.map((shot) => ({
		src: assetUrl(shot.path),
		title: shot.file,
		meta: [
			formatDateTime(shot.takenAt),
			shot.width && shot.height ? `${shot.width}×${shot.height}` : null,
		]
			.filter(Boolean)
			.join(" · "),
	}));

	// ← → и Esc обрабатывает сам просмотр.
	useEffect(() => {
		if (!viewer.open || !current || removing) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.code === "KeyC" && isPrimaryMod(event)) {
				event.preventDefault();
				void copyScreenshot(instance.id, current.file);
			} else if (isDeleteKey(event)) {
				event.preventDefault();
				setRemoving(current);
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [viewer.open, current, removing, instance.id]);

	const afterRemove = (file: string) => {
		const index = items.findIndex((s) => s.file === file);
		const left = items.length - 1;
		if (left <= 0) setViewer({ open: false, index: 0 });
		else if (index >= 0 && viewer.index >= left) setViewer((v) => ({ ...v, index: left - 1 }));
	};

	let body: ReactNode;
	if (shots?.status === "error") {
		body = (
			<CenteredState
				icon={OctagonX}
				tone="destructive"
				heading="h2"
				title={t("screenshots.error.title")}
				description={shots.error ? errorText(shots.error) : t("errors.fallback")}
				action={
					<Button variant="outline" onClick={() => void loadScreenshots(instance.id)}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				}
			/>
		);
	} else if (loading) {
		body = showSkeleton ? <ShotsSkeleton /> : null;
	} else if (!items.length) {
		body = (
			<CenteredState
				icon={Image}
				heading="h2"
				title={t("screenshots.empty.title")}
				description={t("screenshots.empty.description")}
			/>
		);
	} else {
		body = (
			<>
				<PixelGrid minColumn={240} className="mt-4">
					{items.map((shot, index) => (
						<Thumb
							key={shot.file}
							shot={shot}
							onOpen={(target) => {
								setOpener(target);
								setViewer({ open: true, index });
							}}
						/>
					))}
				</PixelGrid>
				<MoreSentinel instanceId={instance.id} />
			</>
		);
	}

	return (
		<div className="flex flex-1 flex-col">
			{items.length > 0 && shots && (
				<div className="flex items-center gap-3">
					<p className="min-w-0 flex-1 truncate text-[13px]/[18px] text-muted-foreground">
						{t("screenshots.count", { count: shots.total })}
					</p>
					<Button
						variant="outline"
						onClick={() => void openInstanceFolder(instance.id, "screenshots")}
					>
						<FolderOpen />
						{t("common.openFolder")}
					</Button>
				</div>
			)}
			{body}
			<ImageViewer
				items={viewerItems}
				index={Math.min(viewer.index, Math.max(0, items.length - 1))}
				onIndexChange={(index) => setViewer((v) => ({ ...v, index }))}
				open={viewer.open && items.length > 0}
				onOpenChange={(open) => setViewer((v) => ({ ...v, open }))}
				returnFocus={opener}
				actions={
					current && (
						<>
							<ViewerButton
								icon={Copy}
								label={t("viewer.copy")}
								onClick={() => void copyScreenshot(instance.id, current.file)}
							/>
							<ViewerButton
								icon={FolderOpen}
								label={t("viewer.showInFolder")}
								onClick={() =>
									void openInstanceFolder(instance.id, "screenshots", current.file)
								}
							/>
							<ViewerButton
								icon={Trash2}
								label={t("viewer.delete")}
								onClick={() => setRemoving(current)}
							/>
						</>
					)
				}
			/>
			<RemoveShotDialog
				instanceId={instance.id}
				shot={removing}
				onClose={() => setRemoving(null)}
				onRemoved={afterRemove}
			/>
		</div>
	);
}

function Thumb({ shot, onOpen }: { shot: Screenshot; onOpen: (target: HTMLElement) => void }) {
	const { t } = useTranslation();
	const [loaded, setLoaded] = useState(false);
	const [failed, setFailed] = useState(false);
	const date = formatDateTime(shot.takenAt);
	return (
		<div className="min-w-0">
			<button
				type="button"
				aria-label={t("screenshots.open", { date })}
				className="relative block aspect-video w-full overflow-clip rounded-xl border border-border bg-muted transition-colors duration-(--duration-base) ease-out hover:border-border-hover"
				onClick={(event) => onOpen(event.currentTarget)}
			>
				{!loaded && !failed && <span className="absolute inset-0 animate-pulse bg-foreground/10" />}
				{failed ? (
					<span className="absolute inset-0 flex items-center justify-center text-muted-foreground">
						<Image className="size-6" />
					</span>
				) : (
					<img
						src={assetUrl(shot.thumbPath ?? shot.path)}
						alt=""
						loading="lazy"
						decoding="async"
						draggable={false}
						className={cn(
							"size-full object-cover transition-opacity duration-(--duration-base)",
							loaded ? "opacity-100" : "opacity-0",
						)}
						onLoad={() => setLoaded(true)}
						onError={() => setFailed(true)}
					/>
				)}
			</button>
			<p className="mt-2 truncate text-[13px]/[18px] text-muted-foreground">{date}</p>
		</div>
	);
}

function MoreSentinel({ instanceId }: { instanceId: string }) {
	const { t } = useTranslation();
	const shots = useScreenshots(instanceId, false);
	const scrollRef = useScrollContainer();
	const ref = useRef<HTMLDivElement>(null);
	const hasMore = Boolean(shots && shots.items.length < shots.total);
	const canLoad = hasMore && shots?.status === "ready" && !shots.loadingMore && !shots.moreError;

	useEffect(() => {
		const node = ref.current;
		const root = scrollRef.current;
		if (!canLoad || !node || !root) return;
		const observer = new IntersectionObserver(
			([entry]) => {
				if (entry.isIntersecting) void loadMoreScreenshots(instanceId);
			},
			{ root, rootMargin: "0px 0px 600px 0px" },
		);
		observer.observe(node);
		return () => observer.disconnect();
	}, [canLoad, instanceId, scrollRef]);

	if (!hasMore) return null;
	return (
		<div ref={ref} className="flex min-h-12 items-center justify-center gap-3 py-4">
			{shots?.moreError ? (
				<>
					<p className="flex items-center gap-1.5 text-[13px]/[18px] text-destructive">
						<OctagonX className="shrink-0" />
						{errorText(shots.moreError)}
					</p>
					<Button variant="outline" size="sm" onClick={() => void loadMoreScreenshots(instanceId)}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				</>
			) : (
				<LoaderCircle className="animate-spin text-muted-foreground" />
			)}
		</div>
	);
}

function RemoveShotDialog({
	instanceId,
	shot,
	onClose,
	onRemoved,
}: {
	instanceId: string;
	shot: Screenshot | null;
	onClose: () => void;
	onRemoved: (file: string) => void;
}) {
	const { t } = useTranslation();
	const [busy, setBusy] = useState(false);

	const confirm = async () => {
		if (!shot || busy) return;
		setBusy(true);
		const file = shot.file;
		const ok = await removeScreenshot(instanceId, file);
		setBusy(false);
		if (ok) {
			onRemoved(file);
			onClose();
		}
	};

	return (
		<AlertDialog
			open={shot !== null}
			onOpenChange={(open) => {
				if (!open && !busy) onClose();
			}}
		>
			<AlertDialogContent className="max-w-[400px] gap-6 sm:max-w-[400px] data-[size=default]:sm:max-w-[400px]">
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle>{t("screenshots.remove.title")}</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{t("screenshots.remove.description")}
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel autoFocus disabled={busy}>
						{t("common.cancel")}
					</AlertDialogCancel>
					<Button variant="destructive" disabled={busy} onClick={() => void confirm()}>
						{busy && <LoaderCircle className="animate-spin" />}
						{t("common.remove")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function ShotsSkeleton() {
	return (
		<PixelGrid aria-busy minColumn={240} className="mt-4">
			{Array.from({ length: 6 }, (_, i) => (
				<div key={i}>
					<Skeleton className="aspect-video w-full rounded-xl" />
					<Skeleton className="mt-2 h-3 w-24" />
				</div>
			))}
		</PixelGrid>
	);
}
