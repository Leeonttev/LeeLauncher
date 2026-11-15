import {
	Braces,
	Ellipsis,
	FolderOpen,
	Globe,
	LoaderCircle,
	OctagonX,
	Play,
	RotateCcw,
	Trash2,
} from "lucide-react";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CenteredState } from "@/components/centered-state";
import { SearchField } from "@/components/search-field";
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
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";
import { usePendingAction } from "@/hooks/use-pending-action";
import type { Instance, World } from "@/lib/api";
import { SOURCE_NAMES } from "@/lib/catalog";
import { errorText } from "@/lib/errors";
import { formatAgo } from "@/lib/format";
import { i18n } from "@/lib/i18n";
import { launchInstance, openInstanceFolder, statusOf, useInstances } from "@/lib/instances";
import { cn } from "@/lib/utils";
import { loadWorlds, removeWorld, useWorlds } from "@/lib/worlds";

const SEARCH_FROM = 6;

export function WorldsTab({ instance }: { instance: Instance }) {
	const { t } = useTranslation();
	const worlds = useWorlds(instance.id);
	const store = useInstances();
	const status = statusOf(instance, store);
	const running = status === "running" || status === "launching";
	const [query, setQuery] = useState("");
	const [removing, setRemoving] = useState<World | null>(null);
	const loading = !worlds || worlds.status === "loading";
	const showSkeleton = useDelayedFlag(loading, 150);
	const items = worlds?.status === "ready" ? worlds.items : [];

	const openSaves = () => void openInstanceFolder(instance.id, "saves");

	let body: ReactNode;
	if (worlds?.status === "error") {
		body = (
			<CenteredState
				icon={OctagonX}
				tone="destructive"
				heading="h2"
				title={t("worlds.error.title")}
				description={worlds.error ? errorText(worlds.error) : t("errors.fallback")}
				action={
					<Button variant="outline" onClick={() => void loadWorlds(instance.id)}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				}
			/>
		);
	} else if (loading) {
		body = showSkeleton ? <WorldsSkeleton /> : null;
	} else if (!items.length) {
		body = (
			<CenteredState
				icon={Globe}
				heading="h2"
				title={t("worlds.empty.title")}
				description={t("worlds.empty.description")}
				action={
					status === "installed" && (
						<Button
							variant="outline"
							onClick={(e) => void launchInstance(instance.id, e.currentTarget)}
						>
							<Play fill="currentColor" />
							{t("instance.play")}
						</Button>
					)
				}
			/>
		);
	} else {
		const needle = query.trim().toLocaleLowerCase();
		const shown = needle
			? items.filter(
					(w) =>
						w.name.toLocaleLowerCase().includes(needle) ||
						w.folder.toLocaleLowerCase().includes(needle),
				)
			: items;
		body = shown.length ? (
			<div className="mt-3">
				{shown.map((world) => (
					<WorldRow
						key={world.folder}
						instance={instance}
						world={world}
						canPlay={status === "installed"}
						running={running}
						onRemove={setRemoving}
					/>
				))}
			</div>
		) : (
			<p className="py-10 text-center text-[13px]/[18px] text-muted-foreground">
				{t("content.notFound")}
			</p>
		);
	}

	return (
		<div className="flex flex-1 flex-col">
			{items.length > 0 && (
				<div className="flex items-center gap-3">
					{items.length >= SEARCH_FROM && (
						<SearchField
							value={query}
							onChange={setQuery}
							placeholder={t("worlds.search")}
							className="w-65"
						/>
					)}
					<Button variant="outline" className="ml-auto" onClick={openSaves}>
						<FolderOpen />
						{t("common.openFolder")}
					</Button>
				</div>
			)}
			{body}
			<RemoveWorldDialog instanceId={instance.id} world={removing} onClose={() => setRemoving(null)} />
		</div>
	);
}

/** Вызывать вместе с `useTranslation()`. */
function modeText(world: World) {
	const parts = [world.hardcore ? i18n.t("worlds.mode.hardcore") : i18n.t(`worlds.mode.${world.mode}`)];
	if (world.cheats) parts.push(i18n.t("worlds.cheats"));
	if (world.source) parts.push(i18n.t("worlds.fromSource", { source: SOURCE_NAMES[world.source.source] }));
	if (world.name !== world.folder) parts.push(i18n.t("worlds.folder", { folder: world.folder }));
	return parts.join(" · ");
}

function WorldRow({
	instance,
	world,
	canPlay,
	running,
	onRemove,
}: {
	instance: Instance;
	world: World;
	canPlay: boolean;
	running: boolean;
	onRemove: (world: World) => void;
}) {
	const { t } = useTranslation();
	const pending = usePendingAction();
	const openFolder = () => void openInstanceFolder(instance.id, `saves/${world.folder}`);

	return (
		<div
			className="relative -mx-3 flex h-16 items-center gap-4 rounded-lg px-3 transition-colors duration-(--duration-base) ease-out hover:bg-accent"
			onKeyDown={(event) => {
				if (event.key !== "Delete" || running) return;
				if ((event.target as HTMLElement).closest("[role=menu]")) return;
				event.preventDefault();
				onRemove(world);
			}}
		>
			{world.iconUrl ? (
				<img
					src={world.iconUrl}
					alt=""
					draggable={false}
					className="size-12 shrink-0 rounded-lg bg-muted object-cover [image-rendering:pixelated]"
				/>
			) : (
				<span
					aria-hidden
					className="flex size-12 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
				>
					<Globe />
				</span>
			)}
			<div className="min-w-0 flex-1">
				<p className="truncate text-sm/5 font-semibold">{world.name}</p>
				<p
					className={cn(
						"flex items-center gap-1.5 text-[13px]/[18px]",
						world.damaged ? "text-destructive" : "text-muted-foreground",
					)}
				>
					{world.damaged && <OctagonX className="shrink-0" />}
					<span className="truncate">{world.damaged ? t("worlds.damaged") : modeText(world)}</span>
				</p>
			</div>
			<span className="w-45 shrink-0 truncate text-right text-[13px]/[18px] text-muted-foreground">
				{world.lastPlayedAt !== null ? formatAgo(world.lastPlayedAt) : ""}
			</span>
			{world.quickPlay && !world.damaged && (
				<Button
					variant="outline"
					size="sm"
					aria-label={t("worlds.playAria", { name: world.name })}
					aria-disabled={!canPlay || undefined}
					className="shrink-0 aria-disabled:opacity-50"
					onClick={(e) => {
						if (canPlay)
							void launchInstance(instance.id, e.currentTarget, { world: world.folder });
					}}
				>
					<Play fill="currentColor" />
					{t("worlds.play")}
				</Button>
			)}
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon-sm"
						aria-label={t("common.openFolder")}
						className="shrink-0"
						onClick={openFolder}
					>
						<FolderOpen />
					</Button>
				</TooltipTrigger>
				<TooltipContent>{t("common.openFolder")}</TooltipContent>
			</Tooltip>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<Button
						variant="ghost"
						size="icon-sm"
						aria-label={t("worlds.menuLabel", { name: world.name })}
						className="shrink-0 aria-expanded:bg-accent"
					>
						<Ellipsis />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end" onCloseAutoFocus={pending.onCloseAutoFocus}>
					<DropdownMenuItem onSelect={openFolder}>
						<FolderOpen />
						{t("common.openFolder")}
					</DropdownMenuItem>
					<DropdownMenuItem
						onSelect={() =>
							void openInstanceFolder(instance.id, `saves/${world.folder}/datapacks`)
						}
					>
						<Braces />
						{t("worlds.datapacks")}
					</DropdownMenuItem>
					<DropdownMenuSeparator />
					<DropdownMenuItem
						variant="destructive"
						disabled={running}
						onSelect={() => pending.defer(() => onRemove(world))}
					>
						<Trash2 />
						{t("common.remove")}
					</DropdownMenuItem>
				</DropdownMenuContent>
			</DropdownMenu>
		</div>
	);
}

function RemoveWorldDialog({
	instanceId,
	world,
	onClose,
}: {
	instanceId: string;
	world: World | null;
	onClose: () => void;
}) {
	const { t } = useTranslation();
	const [busy, setBusy] = useState(false);
	const [shown, setShown] = useState<World | null>(world);
	if (world && world !== shown) setShown(world);

	const confirm = async () => {
		if (!shown || busy) return;
		setBusy(true);
		const ok = await removeWorld(instanceId, shown.folder);
		setBusy(false);
		if (ok) onClose();
	};

	return (
		<AlertDialog
			open={world !== null}
			onOpenChange={(open) => {
				if (!open && !busy) onClose();
			}}
		>
			<AlertDialogContent className="max-w-[400px] gap-6 sm:max-w-[400px] data-[size=default]:sm:max-w-[400px]">
				<AlertDialogHeader className="gap-2">
					<AlertDialogTitle className="break-words">
						{t("worlds.remove.title", { name: shown?.name ?? "" })}
					</AlertDialogTitle>
					<AlertDialogDescription className="text-sm text-muted-foreground">
						{t("worlds.remove.description")}
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

function WorldsSkeleton() {
	return (
		<div aria-busy className="mt-3">
			{Array.from({ length: 4 }, (_, i) => (
				<div key={i} className="flex h-16 items-center gap-4">
					<Skeleton className="size-12 rounded-lg" />
					<div className="flex flex-1 flex-col gap-1.5">
						<Skeleton className="h-4 w-44" />
						<Skeleton className="h-3 w-28" />
					</div>
					<Skeleton className="h-8 w-24 rounded-lg" />
				</div>
			))}
		</div>
	);
}
