import { useTranslation } from "react-i18next";
import { MarkdownView } from "@/components/markdown";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { localizedNotes, setNotesOpen, useUpdates } from "@/lib/updates";

export function UpdateNotesDialog() {
	const { t, i18n } = useTranslation();
	const { lastUpdate, notesOpen } = useUpdates();
	const notes = localizedNotes(lastUpdate?.notes ?? null, i18n.language);

	return (
		<Dialog open={notesOpen && notes !== null} onOpenChange={setNotesOpen}>
			<DialogContent className="w-[560px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[560px]">
				<DialogTitle className="pr-10">
					{t("update.notesDialog", { version: lastUpdate?.version ?? "" })}
				</DialogTitle>
				<DialogDescription className="sr-only">{t("update.notes")}</DialogDescription>
				<div className="mt-4 max-h-[min(480px,70vh)] overflow-y-auto select-text">
					{notes && <MarkdownView source={notes} className="md-compact" />}
				</div>
				<div className="mt-6 flex justify-end">
					<Button variant="outline" onClick={() => setNotesOpen(false)}>
						{t("common.close")}
					</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
