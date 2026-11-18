import { ExternalLink, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ProjectIcon } from "@/components/catalog/project-icon";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import type { ManualFile, ProjectHit, ResolveView } from "@/lib/api";
import { SOURCE_NAMES } from "@/lib/catalog";
import { closeDepsDialog, confirmDeps, manualDependencies, useInstallUi } from "@/lib/content-install";

export function DepsDialog() {
	const { t } = useTranslation();
	const { dialog } = useInstallUi();
	const view = dialog.view;
	return (
		<Dialog open={dialog.open} onOpenChange={(open) => !open && closeDepsDialog()}>
			<DialogContent
				className="w-[448px] max-w-[calc(100vw-2rem)] gap-0 p-6 sm:max-w-[448px]"
				onCloseAutoFocus={(event) => {
					const target = dialog.returnFocus;
					if (!target?.isConnected) return;
					event.preventDefault();
					target.focus();
				}}
			>
				<DialogTitle className="pr-10">
					{t("catalog.deps.title", { title: view?.project.title ?? "" })}
				</DialogTitle>
				<DialogDescription className="mt-2">{t("catalog.deps.description")}</DialogDescription>
				{view && <DepsBody key={dialog.session} view={view} />}
			</DialogContent>
		</Dialog>
	);
}

function DepRow({
	hit,
	checked,
	disabled,
	note,
	siteOnly,
	onChange,
}: {
	hit: Pick<ProjectHit, "id" | "iconUrl" | "type" | "title">;
	checked: boolean;
	disabled?: boolean;
	note: string;
	siteOnly?: boolean;
	onChange?: (checked: boolean) => void;
}) {
	const { t } = useTranslation();
	const id = `dep-${hit.id}`;
	return (
		<label
			htmlFor={id}
			className="-mx-2.5 flex items-center gap-3 rounded-md px-2.5 py-2 has-[button:not(:disabled)]:hover:bg-accent"
		>
			<Checkbox
				id={id}
				checked={checked}
				disabled={disabled}
				onCheckedChange={(value) => onChange?.(value === true)}
			/>
			<ProjectIcon url={hit.iconUrl} type={hit.type} size={32} />
			<span className="min-w-0 flex-1">
				<span className="block truncate text-sm/5 font-semibold">{hit.title}</span>
				<span className="block truncate text-[13px]/[18px] text-muted-foreground">{note}</span>
			</span>
			{siteOnly && (
				<span className="flex shrink-0 items-center gap-1.5 text-[13px]/[18px] text-muted-foreground">
					<ExternalLink />
					{t("catalog.deps.siteOnly")}
				</span>
			)}
		</label>
	);
}

function DepsBody({ view }: { view: ResolveView }) {
	const { t } = useTranslation();
	const [optional, setOptional] = useState<string[]>([]);
	const manual = manualDependencies(view);
	const manualIds = new Set(manual.map((file) => file.projectId));
	// Уже стоит из другого каталога или файлом: не скачивается, строка только для сведения.
	const elsewhere = view.installedElsewhere ?? [];
	const elsewhereIds = new Set(elsewhere.map((item) => item.project.id));
	const required = view.required.filter((hit) => !elsewhereIds.has(hit.id));
	const optionalHits = view.optional.filter((hit) => !elsewhereIds.has(hit.id));
	const isManual = (hit: ProjectHit) => hit.siteOnly || manualIds.has(hit.id);
	// Ручные зависимости, которых нет среди обязательных проектов ответа, — отдельными строками.
	const listed = new Set(
		[...view.required, ...view.optional].map((hit) => hit.id).concat([...elsewhereIds]),
	);
	const manualOnly: ManualFile[] = manual.filter((file) => !listed.has(file.projectId));
	const count = 1 + required.length + manualOnly.length + optional.length;

	return (
		<>
			<div className="-mx-2.5 mt-5 flex max-h-80 flex-col overflow-x-hidden overflow-y-auto px-2.5">
				{required.map((hit) => (
					<DepRow
						key={hit.id}
						hit={hit}
						checked
						disabled
						note={isManual(hit) ? t("catalog.deps.manual") : t("catalog.deps.required")}
						siteOnly={isManual(hit)}
					/>
				))}
				{elsewhere.map((item) => (
					<DepRow
						key={`elsewhere:${item.project.id}`}
						hit={item.project}
						checked
						disabled
						note={
							item.installedFrom
								? t("catalog.deps.installedFrom", {
										source: SOURCE_NAMES[item.installedFrom],
									})
								: t("catalog.deps.installedFile")
						}
					/>
				))}
				{manualOnly.map((file) => (
					<DepRow
						key={`${file.source}:${file.projectId}`}
						hit={{
							id: file.projectId,
							iconUrl: file.iconUrl,
							type: file.type,
							title: file.title,
						}}
						checked
						disabled
						note={t("catalog.deps.manual")}
						siteOnly
					/>
				))}
				{optionalHits.map((hit) => (
					<DepRow
						key={hit.id}
						hit={hit}
						checked={optional.includes(hit.id)}
						note={t("catalog.deps.optional")}
						siteOnly={isManual(hit)}
						onChange={(value) =>
							setOptional((prev) =>
								value ? [...prev, hit.id] : prev.filter((id) => id !== hit.id),
							)
						}
					/>
				))}
			</div>
			{(view.conflicts.length > 0 || view.missing.length > 0) && (
				<ul className="mt-3 flex flex-col gap-1.5">
					{view.conflicts.map((item) => (
						<li
							key={`${item.world ?? ""}/${item.file}`}
							className="flex items-start gap-2 text-[13px]/[18px] text-warning"
						>
							<TriangleAlert className="mt-px" />
							{t("catalog.deps.conflict", { name: item.project?.title ?? item.file })}
						</li>
					))}
					{view.missing.map((dep) => (
						<li
							key={dep.projectId}
							className="flex items-start gap-2 text-[13px]/[18px] text-warning"
						>
							<TriangleAlert className="mt-px" />
							{dep.title
								? t("catalog.deps.missing", { name: dep.title })
								: t("catalog.deps.missingUnknown")}
						</li>
					))}
				</ul>
			)}
			<div className="mt-6 flex justify-end gap-2">
				<Button variant="ghost" onClick={closeDepsDialog}>
					{t("common.cancel")}
				</Button>
				<Button onClick={() => confirmDeps(optional)}>{t("catalog.deps.confirm", { count })}</Button>
			</div>
		</>
	);
}
