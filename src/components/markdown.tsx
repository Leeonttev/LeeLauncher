import { ExternalLink } from "lucide-react";
import { useState, type ComponentProps, type MouseEvent } from "react";
import Markdown, { type Components } from "react-markdown";
import { useTranslation } from "react-i18next";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema, type Options as SanitizeSchema } from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { external, type CatalogProjectType } from "@/lib/api";
import { isSourceEnabled, openProject } from "@/lib/catalog";
import { i18n } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/** Только разметка текста, ссылки и картинки. `iframe` остаётся ради ссылки на YouTube, сам он не рисуется. */
const SCHEMA: SanitizeSchema = {
	...defaultSchema,
	tagNames: [
		"p",
		"h1",
		"h2",
		"h3",
		"h4",
		"h5",
		"h6",
		"ul",
		"ol",
		"li",
		"a",
		"img",
		"strong",
		"b",
		"em",
		"i",
		"del",
		"s",
		"code",
		"pre",
		"blockquote",
		"table",
		"thead",
		"tbody",
		"tr",
		"th",
		"td",
		"details",
		"summary",
		"br",
		"hr",
		"center",
		"div",
		"span",
		"sup",
		"sub",
		"iframe",
	],
	attributes: {
		a: ["href", "title"],
		img: ["src", "alt", "title", "width", "height", "align"],
		iframe: ["src"],
		code: [["className", /^language-./]],
		th: ["align"],
		td: ["align"],
		p: ["align"],
		div: ["align"],
		h1: ["align"],
		h2: ["align"],
		h3: ["align"],
		h4: ["align"],
		h5: ["align"],
		h6: ["align"],
		blockquote: ["align"],
		details: ["open"],
	},
	// Opener открывает только `https://`, остальные ссылки остаются текстом.
	protocols: { href: ["http", "https"], src: ["https"] },
	strip: ["script", "style", "form", "input", "textarea", "select", "button", "object", "embed"],
};

type HastNode = {
	type: string;
	tagName?: string;
	value?: string;
	properties?: Record<string, unknown>;
	children?: HastNode[];
};

/** До санитайзера, который вырезает `style`: так центрирует CurseForge. */
function rehypeStyleAlign() {
	const walk = (node: HastNode) => {
		if (node.type === "element" && node.tagName !== "img") {
			const style = node.properties?.style;
			if (typeof style === "string" && /text-align\s*:\s*center/i.test(style))
				(node.properties ??= {}).align = "center";
		}
		node.children?.forEach(walk);
	};
	return (tree: HastNode) => walk(tree);
}

function isEmptyParagraph(node: HastNode) {
	if (node.type !== "element" || node.tagName !== "p") return false;
	return (node.children ?? []).every(
		(child) => child.type === "text" && !(child.value ?? "").replace(/[\s\u00a0]/g, ""),
	);
}

/** CurseForge ставит пустые абзацы вместо отступов — схлопываем подряд идущие. */
function rehypeCollapseEmpty() {
	const walk = (node: HastNode) => {
		if (node.children) {
			let previousEmpty = false;
			node.children = node.children.filter((child) => {
				if (child.type === "text" && !(child.value ?? "").trim()) return true;
				const empty = isEmptyParagraph(child);
				const keep = !(empty && previousEmpty);
				previousEmpty = empty;
				return keep;
			});
			node.children.forEach(walk);
		}
	};
	return (tree: HastNode) => walk(tree);
}

function rehypeCenter() {
	const walk = (node: HastNode) => {
		if (node.type === "element") {
			const props = (node.properties ??= {});
			if (node.tagName === "center") {
				node.tagName = "div";
				props.className = ["md-center"];
			} else if (props.align === "center" && node.tagName !== "img") {
				props.className = ["md-center"];
			}
			delete props.align;
		}
		node.children?.forEach(walk);
	};
	return (tree: HastNode) => walk(tree);
}

const MODRINTH_PROJECT =
	/^https?:\/\/(?:www\.)?modrinth\.com\/(mod|shader|resourcepack|datapack|plugin)\/([^/?#]+)\/?(?:[?#].*)?$/i;
const CURSEFORGE_PROJECT =
	/^https?:\/\/(?:www\.)?curseforge\.com\/minecraft\/(mc-mods|texture-packs|shaders|data-packs|worlds|modpacks)\/([^/?#]+)\/?(?:[?#].*)?$/i;
const CURSEFORGE_TYPES: Record<string, CatalogProjectType> = {
	"mc-mods": "mod",
	"texture-packs": "resourcepack",
	shaders: "shader",
	"data-packs": "datapack",
	worlds: "world",
	modpacks: "modpack",
};
const LINKOUT = /^https?:\/\/(?:www\.)?curseforge\.com\/linkout\?(?:.*&)?remoteUrl=([^&#]+)/i;

/** `curseforge.com/linkout?remoteUrl=…` → настоящий адрес (бывает закодирован дважды). */
function unwrapLinkout(href: string): string {
	const match = LINKOUT.exec(href);
	if (!match) return href;
	let url = match[1];
	for (let i = 0; i < 2 && /^https?%3a/i.test(url); i++) {
		try {
			url = decodeURIComponent(url);
		} catch {
			return href;
		}
	}
	return /^https?:\/\//i.test(url) ? url : href;
}
const YOUTUBE_EMBED = /^https:\/\/(?:www\.)?youtube(?:-nocookie)?\.com\/embed\/([\w-]{6,})/i;

function openLink(raw: string) {
	const href = unwrapLinkout(raw);
	const project = MODRINTH_PROJECT.exec(href);
	if (project && isSourceEnabled("modrinth")) {
		const kind = project[1].toLowerCase();
		const type: CatalogProjectType | undefined =
			kind === "shader" || kind === "resourcepack" || kind === "datapack"
				? kind
				: kind === "mod"
					? "mod"
					: undefined;
		openProject({ source: "modrinth", id: decodeURIComponent(project[2]) }, type);
		return;
	}
	// Проект CurseForge открываем своей страницей; не нашёлся — она предложит сайт.
	const curseforge = CURSEFORGE_PROJECT.exec(href);
	if (curseforge && isSourceEnabled("curseforge")) {
		openProject(
			{ source: "curseforge", id: decodeURIComponent(curseforge[2]) },
			CURSEFORGE_TYPES[curseforge[1].toLowerCase()],
			{ url: href },
		);
		return;
	}
	if (!/^https?:\/\//i.test(href)) return;
	external.open(href.replace(/^http:/i, "https:")).catch((err: unknown) => {
		console.error("open-url", err);
		toast.error(i18n.t("common.openLinkFailed"));
	});
}

function MarkdownLink({ href, children, title }: ComponentProps<"a">) {
	if (!href || !/^https?:\/\//i.test(href)) return <span title={title}>{children}</span>;
	return (
		<a
			href={href}
			title={title}
			className="text-primary-text underline underline-offset-2 hover:no-underline"
			onClick={(event: MouseEvent<HTMLAnchorElement>) => {
				event.preventDefault();
				if (href) openLink(href);
			}}
		>
			{children}
		</a>
	);
}

function MarkdownImage({ src, alt, title, width, height }: ComponentProps<"img">) {
	const [state, setState] = useState<"loading" | "ready" | "error">("loading");
	if (!src || typeof src !== "string" || state === "error") return null;
	return (
		<img
			src={src}
			alt={alt ?? ""}
			title={title}
			width={width}
			height={height}
			loading="lazy"
			decoding="async"
			referrerPolicy="no-referrer"
			className={cn(
				"inline-block h-auto max-w-full align-middle",
				state === "loading" && "min-h-8 min-w-16 animate-pulse bg-foreground/10",
			)}
			onLoad={() => setState("ready")}
			onError={() => setState("error")}
		/>
	);
}

function YoutubeButton({ src }: ComponentProps<"iframe">) {
	const { t } = useTranslation();
	const match = typeof src === "string" ? YOUTUBE_EMBED.exec(src) : null;
	if (!match) return null;
	return (
		<span className="my-3 block">
			<Button
				variant="outline"
				size="sm"
				onClick={() => openLink(`https://www.youtube.com/watch?v=${match[1]}`)}
			>
				<ExternalLink />
				{t("project.video")}
			</Button>
		</span>
	);
}

const COMPONENTS: Components = {
	a: MarkdownLink,
	img: MarkdownImage,
	iframe: YoutubeButton,
	pre: ({ children }) => <pre className="overflow-x-auto rounded-lg bg-muted p-3">{children}</pre>,
};

export function MarkdownView({ source, className }: { source: string; className?: string }) {
	return (
		<div className={cn("md", className)}>
			<Markdown
				remarkPlugins={[remarkGfm]}
				rehypePlugins={[
					rehypeRaw,
					rehypeStyleAlign,
					[rehypeSanitize, SCHEMA],
					rehypeCollapseEmpty,
					rehypeCenter,
				]}
				components={COMPONENTS}
			>
				{source}
			</Markdown>
		</div>
	);
}
