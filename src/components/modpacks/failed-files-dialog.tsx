import { Check, Copy, LoaderCircle, RotateCcw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { useDownloads } from "@/lib/downloads";
import { errorText } from "@/lib/errors";
import { useInstances } from "@/lib/instances";
import { closeModpackDialog, restoreModpackFocus, retryModpackTask, useModpackDialogs } from "@/lib/modpacks";

export function FailedFilesDialog() {
	const { t } = useTranslation();
	const dialog = useModpackDialogs().failed;
	const downloads = useDownloads();
	const store = useInstances();
	const download = downloads.items.find((d) => d.id === dialog.downloadId) ?? null;
	const failed = download?.warnings?.failed ?? [];
	const instance = store.instances.find((i) => i.id === download?.instanceId) ?? null;
	const name = instance?.name ?? download?.title ?? "";
	const [copied, setCopied] = useState(false);
	const [retrying, setRetrying] = useState(false);

	const copy = () => {
		const text = failed.map((f) => `${f.path} — ${errorText(f.error)}`).join("\n");
		void navigator.clipboard.writeText(text).then(() => {
			setCopied(true);
			setTimeout(() => setCopied(false), 1500);
		});
	};

	const retry = async () => {
		if (!download || retrying) return;
		setRetrying(true);
		const ok = await retryModpackTask(download.id);
		setRetrying(false);
		if (ok) closeModpackDialog("failed");
	};

	return (
		<Dialog
			open={dialog.open && Boolean(download)}
			onOpenChange={(next) => {
				if (!next) closeModpackDialog("failed");
			}}
		>
			<DialogContent
				className="w-[520px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[520px]"
				onCloseAutoFocus={(e) => restoreModpackFocus("failed", e)}
			>
				<DialogTitle className="pr-10">
					{t("modpack.failed.title", { count: failed.length })}
				</DialogTitle>
				<DialogDescription className="mt-2 text-sm text-muted-foreground">
					{t("modpack.failed.description", { name })}
				</DialogDescription>
				<ul className="mt-5 max-h-60 overflow-y-auto rounded-lg bg-muted px-3 py-2 select-text">
					{failed.map((file) => (
						<li key={file.path} className="flex min-w-0 gap-2 text-[13px]/6">
							<span className="min-w-0 truncate font-mono">{file.path}</span>
							<span className="shrink-0 text-muted-foreground">
								{"— "}
								{errorText(file.error)}
							</span>
						</li>
					))}
				</ul>
				<div className="mt-6 flex items-center gap-2">
					<Button variant="ghost" onClick={copy}>
						{copied ? <Check /> : <Copy />}
						{copied ? t("common.copied") : t("common.copy")}
					</Button>
					<div className="ml-auto flex gap-2">
						<Button variant="outline" onClick={() => closeModpackDialog("failed")}>
							{t("common.close")}
						</Button>
						<Button
							aria-disabled={retrying || undefined}
							className="aria-disabled:opacity-60"
							onClick={() => void retry()}
						>
							{retrying ? <LoaderCircle className="animate-spin" /> : <RotateCcw />}
							{t("common.retry")}
						</Button>
					</div>
				</div>
			</DialogContent>
		</Dialog>
	);
}
