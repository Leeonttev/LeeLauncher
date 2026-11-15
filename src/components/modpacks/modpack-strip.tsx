import {
	ArrowUpRight,
	Check,
	ChevronsUpDown,
	Ellipsis,
	FolderOpen,
	LoaderCircle,
	RefreshCw,
	Unlink,
	WifiOff,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { usePendingAction } from "@/hooks/use-pending-action";
import type { Instance, ModpackLink } from "@/lib/api";
import { openProject } from "@/lib/catalog";
import { useDownloads } from "@/lib/downloads";
import { errorText } from "@/lib/errors";
import { formatAgo } from "@/lib/format";
import { statusOf, useInstances } from "@/lib/instances";
import {
	activeModpackTask,
	catalogLink,
	checkModpackUpdate,
	openUnlinkModpack,
	openUpdateModpack,
	updateModpackFromFile,
	useModpackCheck,
} from "@/lib/modpacks";
import { cn } from "@/lib/utils";

/** Проверка обновления — при открытии, тихо, не чаще раза в 10 минут. */
export function ModpackStrip({ instance, link }: { instance: Instance; link: ModpackLink }) {
	const { t } = useTranslation();
	const catalog = catalogLink(instance);
	const check = useModpackCheck(instance.id);
	const downloads = useDownloads();
	const store = useInstances();
	const pending = usePendingAction();
	const menuRef = useRef<HTMLButtonElement>(null);
	const [menuOpen, setMenuOpen] = useState(false);
	const status = statusOf(instance, store);
	const running = status === "running" || status === "launching";
	const task = activeModpackTask(downloads, instance.id);
	const fromFile = !catalog;

	useEffect(() => {
		void checkModpackUpdate(instance.id);
	}, [instance.id, link.versionId]);

	const openPage = () => {
		if (catalog) openProject({ source: catalog.source, id: catalog.projectId }, "modpack");
	};

	const guarded = (button: ReactNode) =>
		running ? (
			<Tooltip>
				<TooltipTrigger asChild>{button}</TooltipTrigger>
				<TooltipContent>{t("modpack.strip.gameRunning")}</TooltipContent>
			</Tooltip>
		) : (
			button
		);

	let right: ReactNode;
	if (task && task.mode !== "update") {
		// Ход первой установки показывает блок под вкладками страницы сборки.
		right = null;
	} else if (task) {
		const percent = task.state === "running" && task.percent !== null ? Math.round(task.percent) : null;
		right = (
			<span className="flex items-center gap-2 text-[13px]/[18px] text-primary-text">
				<LoaderCircle className="animate-spin" />
				{percent !== null
					? t("modpack.strip.updating", { percent })
					: t("modpack.strip.updatingShort")}
			</span>
		);
	} else if (fromFile) {
		right = guarded(
			<Button
				variant="outline"
				aria-disabled={running || undefined}
				className="aria-disabled:opacity-60"
				onClick={(e) => {
					if (!running) void updateModpackFromFile(instance.id, e.currentTarget);
				}}
			>
				<FolderOpen />
				{t("modpack.strip.fromFile")}
			</Button>,
		);
	} else if (!check) {
		right = null;
	} else if (check.status === "checking") {
		right = (
			<span className="flex items-center gap-2 text-[13px]/[18px] text-muted-foreground">
				<LoaderCircle className="animate-spin" />
				{t("modpack.strip.checking")}
			</span>
		);
	} else if (check.status === "ready" && check.result?.available && check.result.latest) {
		const latest = check.result.latest;
		right = (
			<>
				<span className="truncate text-[13px]/[18px] text-muted-foreground">
					{t("modpack.strip.released", {
						version: latest.number,
						when: formatAgo(latest.publishedAt),
					})}
				</span>
				{guarded(
					<Button
						variant="secondary"
						aria-disabled={running || undefined}
						className="shrink-0 border-transparent bg-primary-soft text-primary-text hover:bg-primary hover:text-primary-foreground aria-disabled:opacity-60 aria-disabled:hover:bg-primary-soft aria-disabled:hover:text-primary-text"
						onClick={(e) => {
							if (!running)
								openUpdateModpack(
									instance.id,
									{ kind: "version", versionId: latest.id, pick: false },
									e.currentTarget,
								);
						}}
					>
						<RefreshCw />
						{t("modpack.strip.updateTo", { version: latest.number })}
					</Button>,
				)}
			</>
		);
	} else {
		const failed = check.status === "error";
		const status = (
			<span className="flex min-w-0 items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
				{failed ? <WifiOff className="shrink-0" /> : <Check className="shrink-0" />}
				<span className="truncate">
					{failed
						? t("modpack.strip.checkFailed")
						: t("modpack.strip.latest", {
								when: formatAgo(check.checkedAt ?? check.result?.checkedAt ?? 0),
							})}
				</span>
			</span>
		);
		right = (
			<>
				{failed && check.error ? (
					<Tooltip>
						<TooltipTrigger asChild>{status}</TooltipTrigger>
						<TooltipContent className="max-w-72">{errorText(check.error)}</TooltipContent>
					</Tooltip>
				) : (
					status
				)}
				<Button
					variant="outline"
					className="shrink-0"
					onClick={() => void checkModpackUpdate(instance.id, true)}
				>
					{t("modpack.strip.check")}
				</Button>
			</>
		);
	}

	return (
		<div className="mb-5 flex items-center gap-4 border-b pb-5">
			<ProjectIcon url={link.iconUrl} type="modpack" size={40} />
			<div className="min-w-0 flex-1">
				<div className="flex min-w-0 items-center gap-2">
					<span className="inline-flex h-[22px] shrink-0 items-center rounded-md bg-muted px-[7px] text-[13px] font-medium text-muted-foreground">
						{t("modpack.strip.label")}
					</span>
					{catalog ? (
						<button
							type="button"
							className="relative min-w-0 truncate rounded-md border border-transparent text-sm/5 font-semibold hover:underline"
							aria-keyshortcuts="Shift+F10"
							onClick={openPage}
							onKeyDown={(event) => {
								if (event.key === "F10" && event.shiftKey) {
									event.preventDefault();
									setMenuOpen(true);
								}
							}}
						>
							{link.title}
						</button>
					) : (
						<span className="min-w-0 truncate text-sm/5 font-semibold">{link.title}</span>
					)}
					<span className="shrink-0 text-[13px]/[18px] text-muted-foreground">
						{link.versionNumber}
					</span>
				</div>
				<p className="mt-0.5 truncate text-[13px]/[18px] text-muted-foreground">
					{fromFile ? t("modpack.strip.hintFile") : t("modpack.strip.hint")}
				</p>
			</div>
			<div className={cn("flex min-w-0 shrink items-center justify-end gap-3")}>{right}</div>
			<DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
				<DropdownMenuTrigger asChild>
					<Button
						ref={menuRef}
						variant="ghost"
						size="icon"
						aria-label={t("modpack.strip.menuLabel", { title: link.title })}
						className="shrink-0 aria-expanded:bg-accent"
					>
						<Ellipsis />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end" className="w-64" onCloseAutoFocus={pending.onCloseAutoFocus}>
					{catalog ? (
						<>
							<DropdownMenuItem
								disabled={running || Boolean(task)}
								onSelect={() =>
									pending.defer(() =>
										openUpdateModpack(
											instance.id,
											{ kind: "version", versionId: null, pick: true },
											menuRef.current,
										),
									)
								}
							>
								<ChevronsUpDown />
								{t("modpack.menu.changeVersion")}
							</DropdownMenuItem>
							<DropdownMenuItem onSelect={() => void checkModpackUpdate(instance.id, true)}>
								<RefreshCw />
								{t("modpack.menu.check")}
							</DropdownMenuItem>
							<DropdownMenuItem onSelect={() => pending.defer(openPage)}>
								<ArrowUpRight />
								{t("modpack.menu.openPage")}
							</DropdownMenuItem>
						</>
					) : (
						<DropdownMenuItem
							disabled={running || Boolean(task)}
							onSelect={() => void updateModpackFromFile(instance.id, menuRef.current)}
						>
							<FolderOpen />
							{t("modpack.strip.fromFile")}
						</DropdownMenuItem>
					)}
					<DropdownMenuSeparator />
					<DropdownMenuItem
						disabled={Boolean(task)}
						onSelect={() => pending.defer(() => openUnlinkModpack(instance.id, menuRef.current))}
					>
						<Unlink />
						{t("modpack.menu.unlink")}
					</DropdownMenuItem>
				</DropdownMenuContent>
			</DropdownMenu>
		</div>
	);
}
