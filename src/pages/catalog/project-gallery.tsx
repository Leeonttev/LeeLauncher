import { ExternalLink, ImageOff } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ImageViewer, ViewerButton } from "@/components/image-viewer";
import { PixelGrid } from "@/components/pixel-grid";
import { useImageDeadline } from "@/hooks/use-image-deadline";
import { external, type Project } from "@/lib/api";
import { i18n } from "@/lib/i18n";
import { cn } from "@/lib/utils";

const PREVIEW_ROWS = 2;

const thumbClass =
	"rounded-xl border relative block aspect-video w-full overflow-clip border-border bg-muted transition-[border-color] duration-(--duration-fast) ease-out hover:border-border-hover";

type ViewerState = { index: number; open: boolean; returnFocus: HTMLElement | null };

/** Индекс — по всей галерее, фокус вернуть на `target`. */
function useViewer() {
	const [state, setState] = useState<ViewerState>({ index: 0, open: false, returnFocus: null });
	return {
		state,
		open: (index: number, target: HTMLElement) => setState({ index, open: true, returnFocus: target }),
		setIndex: (index: number) => setState((s) => ({ ...s, index })),
		setOpen: (open: boolean) => setState((s) => ({ ...s, open })),
	};
}

function GalleryViewer({ project, viewer }: { project: Project; viewer: ReturnType<typeof useViewer> }) {
	const { t } = useTranslation();
	const items = project.gallery.map((image, i) => ({
		src: image.url,
		title: image.title || t("project.gallery.untitled", { index: i + 1 }),
		meta: image.description ?? undefined,
	}));
	return (
		<ImageViewer
			items={items}
			index={viewer.state.index}
			onIndexChange={viewer.setIndex}
			open={viewer.state.open}
			onOpenChange={viewer.setOpen}
			returnFocus={viewer.state.returnFocus}
			actions={
				<ViewerButton
					icon={ExternalLink}
					label={t("viewer.openInBrowser")}
					onClick={() => {
						const url = project.gallery[viewer.state.index]?.url;
						if (url) external.open(url).catch(() => toast.error(i18n.t("common.openLinkFailed")));
					}}
				/>
			}
		/>
	);
}

function Thumb({
	image,
	label,
	onOpen,
}: {
	image: Project["gallery"][number];
	label: string;
	onOpen: (target: HTMLElement) => void;
}) {
	const [state, setState] = useState<"loading" | "ready" | "error">("loading");
	const ref = useRef<HTMLButtonElement>(null);
	const giveUp = useCallback(() => setState((s) => (s === "loading" ? "error" : s)), []);
	useImageDeadline(ref, state === "loading", giveUp);
	return (
		<button
			ref={ref}
			type="button"
			aria-label={label}
			className={cn(thumbClass)}
			onClick={(e) => onOpen(e.currentTarget)}
		>
			{state === "error" ? (
				<span className="flex size-full items-center justify-center text-muted-foreground">
					<ImageOff className="size-6" />
				</span>
			) : (
				<img
					src={image.thumbUrl || image.url}
					alt=""
					loading="lazy"
					decoding="async"
					referrerPolicy="no-referrer"
					draggable={false}
					className={cn(
						"size-full object-cover transition-opacity duration-(--duration-base)",
						state === "loading" && "opacity-0",
					)}
					onLoad={() => setState("ready")}
					onError={() => setState("error")}
				/>
			)}
		</button>
	);
}

export function ProjectGallery({ project }: { project: Project }) {
	const { t } = useTranslation();
	const viewer = useViewer();
	const titleOf = (i: number) =>
		project.gallery[i]?.title || t("project.gallery.untitled", { index: i + 1 });

	return (
		<>
			<PixelGrid minColumn={320}>
				{project.gallery.map((image, i) => (
					<div key={image.url} className="min-w-0">
						<Thumb
							image={image}
							label={t("project.gallery.open", { title: titleOf(i) })}
							onOpen={(target) => viewer.open(i, target)}
						/>
						{image.title && (
							<p className="mt-2 truncate text-sm/5 font-semibold">{image.title}</p>
						)}
						{image.description && (
							<p className="mt-0.5 line-clamp-2 text-[13px]/[18px] text-muted-foreground">
								{image.description}
							</p>
						)}
					</div>
				))}
			</PixelGrid>
			<GalleryViewer project={project} viewer={viewer} />
		</>
	);
}

export function GalleryPreview({ project }: { project: Project }) {
	const { t } = useTranslation();
	const viewer = useViewer();
	const [columns, setColumns] = useState(3);
	const order = project.gallery
		.map((image, index) => ({ image, index }))
		.sort((a, b) => Number(b.image.featured) - Number(a.image.featured));
	const shown = order.slice(0, columns * PREVIEW_ROWS);
	return (
		<>
			<PixelGrid
				minColumn={200}
				minColumns={2}
				maxColumns={3}
				gap={12}
				className="gap-y-3"
				onColumnsChange={setColumns}
			>
				{shown.map(({ image, index }) => (
					<Thumb
						key={image.url}
						image={image}
						label={t("project.gallery.open", {
							title: image.title || t("project.gallery.untitled", { index: index + 1 }),
						})}
						onOpen={(target) => viewer.open(index, target)}
					/>
				))}
			</PixelGrid>
			<GalleryViewer project={project} viewer={viewer} />
		</>
	);
}
