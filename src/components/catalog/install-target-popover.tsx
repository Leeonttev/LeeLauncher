import { ArrowLeft, Check, Globe, LoaderCircle, OctagonX, Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { InstanceThumb } from "@/components/instances/instance-cover";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { catalogApi, contentApi, LOADER_KINDS, type Instance, type LoaderKind, type World } from "@/lib/api";
import { ensureContent, findProjectItem, useContentCache } from "@/lib/content";
import {
	incompatibilityOf,
	installProject,
	type Incompatibility,
	type ProjectRef,
} from "@/lib/content-install";
import { coverOf } from "@/lib/covers";
import { errorText, toAppError, type AppError } from "@/lib/errors";
import { formatAgo } from "@/lib/format";
import { compareRecent, openCreateInstanceDialog, useInstances } from "@/lib/instances";
import { compareGameVersions, versionLabel } from "@/lib/loaders";
import { cn } from "@/lib/utils";

type Row = { instance: Instance; installed: boolean; problem: Incompatibility | null };

const row =
	"rounded-md flex w-full items-center gap-3 px-2.5 py-2 text-left outline-none transition-colors duration-(--duration-fast) ease-out hover:bg-accent focus-visible:bg-accent";

/** Сборки: подходящие по последнему запуску, потом установленные, потом неподходящие. У датапака второй шаг — мир. */
export function InstallTargetContent({
	project,
	instanceId,
	onPick,
}: {
	project: ProjectRef;
	instanceId?: string;
	onPick: (instanceId: string, world?: string) => void;
}) {
	const [worldsFor, setWorldsFor] = useState<string | null>(instanceId ?? null);
	const datapack = project.type === "datapack";

	if (datapack && worldsFor) {
		return (
			<WorldStep
				instanceId={worldsFor}
				onBack={instanceId ? undefined : () => setWorldsFor(null)}
				onPick={(world) => onPick(worldsFor, world)}
			/>
		);
	}
	return <InstanceStep project={project} onPick={(id) => (datapack ? setWorldsFor(id) : onPick(id))} />;
}

function InstanceStep({ project, onPick }: { project: ProjectRef; onPick: (instanceId: string) => void }) {
	const { t } = useTranslation();
	const store = useInstances();
	const content = useContentCache();
	const ids = store.instances.map((i) => i.id).join(",");

	useEffect(() => {
		for (const id of ids.split(",")) if (id) ensureContent(id);
	}, [ids]);

	const rows: Row[] = [...store.instances].sort(compareRecent).map((instance) => ({
		instance,
		installed: Boolean(findProjectItem(content[instance.id], project.id)),
		problem: incompatibilityOf(project, instance),
	}));
	const rank = (r: Row) => (r.installed ? 1 : r.problem ? 2 : 0);
	rows.sort((a, b) => rank(a) - rank(b));

	return (
		<div className="flex min-h-0 flex-col">
			<div className="shrink-0 px-2.5 pt-1.5 pb-2">
				<p className="truncate text-sm/5 font-semibold">
					{t("catalog.where.title", { title: project.title })}
				</p>
				<p className="text-[13px]/[18px] text-muted-foreground">
					{rows.length ? t("catalog.where.hint") : t("catalog.where.noInstances")}
				</p>
			</div>
			{rows.length > 0 && (
				<div className="max-h-80 min-h-0 overflow-y-auto">
					{rows.map(({ instance, installed, problem }) => {
						const disabled = installed || Boolean(problem);
						return (
							<button
								key={instance.id}
								type="button"
								aria-disabled={disabled || undefined}
								className={cn(
									row,
									disabled && "pointer-events-none",
									problem && "opacity-55",
								)}
								onClick={() => {
									if (!disabled) onPick(instance.id);
								}}
							>
								<InstanceThumb scene={coverOf(instance)} size={40} />
								<span className="min-w-0 flex-1">
									<span className="block truncate text-sm/5 font-semibold">
										{instance.name}
									</span>
									<span className="block truncate text-[13px]/[18px] text-muted-foreground">
										{versionLabel(instance)}
									</span>
								</span>
								{installed ? (
									<span className="flex shrink-0 items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
										<Check />
										{t("catalog.where.already")}
									</span>
								) : problem ? (
									<span className="max-w-36 shrink-0 text-right text-[13px]/[18px] text-muted-foreground">
										{problem.kind === "needs_loader"
											? t("catalog.where.needsLoader")
											: project.type === "world"
												? t("catalog.where.noGameVersion", {
														version: problem.target,
													})
												: t("catalog.where.noVersion", { loader: problem.target })}
									</span>
								) : null}
							</button>
						);
					})}
				</div>
			)}
			<div className="-mx-1.5 my-1.5 h-px shrink-0 bg-border" />
			<NewInstanceRow project={project} />
		</div>
	);
}

async function presetFor(project: ProjectRef): Promise<{ gameVersion?: string; loader: LoaderKind }> {
	const fallbackLoader: LoaderKind =
		project.type === "mod" ? "fabric" : project.type === "shader" ? "fabric" : "vanilla";
	try {
		const versions = await catalogApi.versions({ projectId: project.id, source: project.source });
		const latest = versions.find((v) => v.channel === "release") ?? versions[0];
		if (!latest) return { loader: fallbackLoader };
		const gameVersion = [...latest.gameVersions].sort(compareGameVersions)[0];
		const loader =
			project.type === "mod"
				? (LOADER_KINDS.find((k) => k !== "vanilla" && latest.loaders.includes(k)) ?? fallbackLoader)
				: fallbackLoader;
		return { gameVersion, loader };
	} catch {
		return { loader: fallbackLoader };
	}
}

function NewInstanceRow({ project }: { project: ProjectRef }) {
	const { t } = useTranslation();
	const [busy, setBusy] = useState(false);
	return (
		<button
			type="button"
			aria-disabled={busy || undefined}
			className={cn(row, "shrink-0 text-sm/5 font-medium")}
			onClick={async (event) => {
				if (busy) return;
				const returnFocus = event.currentTarget;
				setBusy(true);
				const preset = await presetFor(project);
				setBusy(false);
				openCreateInstanceDialog(returnFocus, {
					...preset,
					afterCreate: (instance) => void installProject({ instanceId: instance.id, project }),
				});
			}}
		>
			<span className="flex size-8 shrink-0 items-center justify-center text-primary-text">
				{busy ? <LoaderCircle className="animate-spin" /> : <Plus />}
			</span>
			{t("catalog.where.newInstance")}
		</button>
	);
}

function WorldStep({
	instanceId,
	onBack,
	onPick,
}: {
	instanceId: string;
	onBack?: () => void;
	onPick: (world: string) => void;
}) {
	const { t } = useTranslation();
	const [state, setState] = useState<
		{ status: "loading" } | { status: "ready"; worlds: World[] } | { status: "error"; error: AppError }
	>({ status: "loading" });

	useEffect(() => {
		let alive = true;
		contentApi
			.worlds(instanceId)
			.then((worlds) => {
				if (alive) setState({ status: "ready", worlds: worlds.filter((w) => !w.damaged) });
			})
			.catch((err: unknown) => {
				if (alive) setState({ status: "error", error: toAppError(err) });
			});
		return () => {
			alive = false;
		};
	}, [instanceId]);

	let body;
	if (state.status === "loading") {
		body = (
			<div aria-busy className="flex flex-col gap-1 px-2.5 py-1">
				{[0, 1].map((i) => (
					<div key={i} className="flex items-center gap-3 py-1.5">
						<Skeleton className="size-8 rounded-lg" />
						<Skeleton className="h-3.5 w-36" />
					</div>
				))}
			</div>
		);
	} else if (state.status === "error") {
		body = (
			<p className="flex items-start gap-1.5 px-2.5 py-2 text-[13px]/[18px] text-destructive">
				<OctagonX className="mt-px shrink-0" />
				<span className="min-w-0">
					{t("catalog.where.worldsFailed")} · {errorText(state.error)}
				</span>
			</p>
		);
	} else if (!state.worlds.length) {
		body = (
			<p className="px-2.5 py-2 text-[13px]/[18px] text-muted-foreground">
				{t("catalog.where.noWorlds")}
			</p>
		);
	} else {
		body = (
			<div className="max-h-80 min-h-0 overflow-y-auto">
				{state.worlds.map((world) => (
					<button
						key={world.folder}
						type="button"
						className={row}
						onClick={() => onPick(world.folder)}
					>
						{world.iconUrl ? (
							<img
								src={world.iconUrl}
								alt=""
								className="size-8 shrink-0 rounded-lg bg-muted [image-rendering:pixelated]"
							/>
						) : (
							<span
								aria-hidden
								className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"
							>
								<Globe />
							</span>
						)}
						<span className="min-w-0 flex-1">
							<span className="block truncate text-sm/5 font-semibold">{world.name}</span>
							{world.lastPlayedAt !== null && (
								<span className="block truncate text-[13px]/[18px] text-muted-foreground">
									{formatAgo(world.lastPlayedAt)}
								</span>
							)}
						</span>
					</button>
				))}
			</div>
		);
	}

	return (
		<div className="flex min-h-0 flex-col">
			<div className="flex shrink-0 items-start gap-1 px-1 pt-0.5 pb-2">
				{onBack && (
					<Button
						variant="ghost"
						size="icon-sm"
						aria-label={t("catalog.where.back")}
						className="shrink-0"
						onClick={onBack}
					>
						<ArrowLeft />
					</Button>
				)}
				<div className={cn("min-w-0 flex-1", !onBack && "px-1.5", onBack && "pt-1")}>
					<p className="truncate text-sm/5 font-semibold">{t("catalog.where.world")}</p>
					<p className="text-[13px]/[18px] text-muted-foreground">{t("catalog.where.worldHint")}</p>
				</div>
			</div>
			{body}
		</div>
	);
}
