import { Download, type LucideIcon, OctagonX, RotateCcw } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { DownloadRow, type DownloadRowKind } from "@/components/downloads/download-row";
import { Button } from "@/components/ui/button";
import { PopoverContent } from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { useAppearance } from "@/lib/appearance";
import { clearDownloads, downloadSections, reloadDownloads, useDownloads } from "@/lib/downloads";
import { errorText } from "@/lib/errors";
import { useInstances, useNow } from "@/lib/instances";

const easeOut = [0.22, 1, 0.36, 1] as const;

type Entry =
	| { type: "label"; key: string; text: string }
	| { type: "row"; key: string; kind: DownloadRowKind; node: ReactNode };

export function DownloadsPanel() {
	const { t } = useTranslation();
	const store = useDownloads();
	const instances = useInstances();
	const { reduceMotion } = useAppearance();
	const now = useNow(store.open, 30_000);
	const { active, queued, done } = downloadSections(store.items);

	const instanceOf = (id: string | undefined) =>
		id ? (instances.instances.find((i) => i.id === id) ?? null) : null;

	const entries: Entry[] = [];
	const addSection = (kind: DownloadRowKind, text: string, items: typeof active) => {
		const rows = items.flatMap((download) => {
			// Сборки может ещё не быть в сторе — строка с запасным названием, без ссылки.
			const instance = instanceOf(download.instanceId);
			return [
				{
					type: "row" as const,
					key: download.id,
					kind,
					node: <DownloadRow download={download} kind={kind} instance={instance} now={now} />,
				},
			];
		});
		if (!rows.length) return;
		entries.push({ type: "label", key: `label-${kind}`, text }, ...rows);
	};
	addSection("active", t("downloads.section.active"), active);
	addSection("queued", t("downloads.section.queued", { count: queued.length }), queued);
	addSection("done", t("downloads.section.done"), done);

	let body: ReactNode;
	if (store.status === "error") {
		body = (
			<PanelState
				icon={OctagonX}
				destructive
				title={t("downloads.error.title")}
				description={store.error ? errorText(store.error) : t("errors.fallback")}
				action={
					<Button variant="outline" size="sm" onClick={() => void reloadDownloads()}>
						<RotateCcw />
						{t("common.retry")}
					</Button>
				}
			/>
		);
	} else if (store.status !== "ready") {
		body = (
			<div aria-busy className="flex flex-col gap-1 px-4 py-3">
				{[0, 1].map((i) => (
					<div key={i} className="flex gap-3 py-2">
						<Skeleton className="size-10 shrink-0 rounded-lg" />
						<div>
							<Skeleton className="h-3.5 w-40" />
							<Skeleton className="mt-2 h-3 w-56" />
						</div>
					</div>
				))}
			</div>
		);
	} else if (!entries.length) {
		body = (
			<PanelState
				icon={Download}
				title={t("downloads.empty.title")}
				description={t("downloads.empty.description")}
			/>
		);
	} else {
		const motionOff = reduceMotion;
		// Сдвиг строк — только при смене состава или порядка, а не высоты строки.
		const order = entries.map((entry) => entry.key).join("|");
		body = (
			<div className="pb-2">
				<AnimatePresence initial={false}>
					{entries.map((entry) => (
						<motion.div
							key={entry.key}
							layout={motionOff ? false : "position"}
							layoutDependency={order}
							initial={motionOff ? false : { opacity: 0, y: 4 }}
							animate={{ opacity: 1, y: 0, transition: { duration: 0.2, ease: easeOut } }}
							exit={{ opacity: 0, transition: { duration: motionOff ? 0 : 0.12 } }}
							transition={{ layout: { duration: motionOff ? 0 : 0.2, ease: easeOut } }}
						>
							{entry.type === "label" ? (
								<p className="px-4 pt-3 pb-1 text-[13px]/[18px] font-semibold text-muted-foreground">
									{entry.text}
								</p>
							) : (
								entry.node
							)}
						</motion.div>
					))}
				</AnimatePresence>
			</div>
		);
	}

	return (
		<PopoverContent
			align="end"
			sideOffset={8}
			data-downloads-panel
			aria-label={t("downloads.title")}
			className="w-[400px] p-0"
		>
			<div className="flex h-12 items-center gap-3 border-b pr-2.5 pl-4">
				<h2 className="min-w-0 flex-1 truncate text-sm/5 font-semibold">{t("downloads.title")}</h2>
				{store.status === "ready" && done.length > 0 && (
					<Button
						variant="ghost"
						size="sm"
						className="text-[13px] font-semibold text-muted-foreground hover:text-foreground"
						onClick={() => void clearDownloads()}
					>
						{t("downloads.clear")}
					</Button>
				)}
			</div>
			<div className="max-h-[min(560px,calc(100svh-96px))] overflow-y-auto">{body}</div>
		</PopoverContent>
	);
}

function PanelState({
	icon: Icon,
	destructive,
	title,
	description,
	action,
}: {
	icon: LucideIcon;
	destructive?: boolean;
	title: string;
	description: string;
	action?: ReactNode;
}) {
	return (
		<div className="flex flex-col items-center px-8 py-8 text-center">
			<span
				aria-hidden
				className={
					destructive
						? "flex size-14 items-center justify-center rounded-2xl bg-destructive/10 text-destructive"
						: "flex size-14 items-center justify-center rounded-2xl bg-primary-soft text-primary-text"
				}
			>
				<Icon className="size-6" />
			</span>
			<p className="mt-4 text-sm/5 font-semibold">{title}</p>
			<p className="mt-1 max-w-[260px] text-[13px]/[18px] text-muted-foreground">{description}</p>
			{action && <div className="mt-4">{action}</div>}
		</div>
	);
}
