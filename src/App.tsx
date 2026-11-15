import { LucideProvider } from "lucide-react";
import { MotionConfig } from "motion/react";
import { useEffect } from "react";
import { ErrorBoundary } from "@/components/error-boundary";
import { AppFrame } from "@/components/layout/app-frame";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { appWindow } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import { watchWindowLayout } from "@/lib/layout-calm";

/** Окно создаётся скрытым (против белой вспышки) — показываем его, когда первый кадр готов. */
function ShowWindowOnMount() {
	useEffect(() => {
		const frame = requestAnimationFrame(() => void appWindow.show().catch(() => {}));
		return () => cancelAnimationFrame(frame);
	}, []);
	return null;
}

/** Пока меняется размер окна, раскладка встаёт на место без анимаций. */
function CalmLayoutOnResize() {
	useEffect(() => watchWindowLayout(), []);
	return null;
}

/** Стандартное меню браузера остаётся только там, где оно полезно: поля ввода и выделяемый текст. */
function allowsBrowserMenu(target: EventTarget | null) {
	const element = target instanceof Element ? target : target instanceof Node ? target.parentElement : null;
	if (!element) return false;
	if (element.closest("input, textarea, [contenteditable], .select-text")) return true;
	const selection = window.getSelection();
	return Boolean(selection && !selection.isCollapsed && selection.containsNode(element, true));
}

function BlockBrowserMenu() {
	useEffect(() => {
		const onContextMenu = (event: MouseEvent) => {
			if (!allowsBrowserMenu(event.target)) event.preventDefault();
		};
		document.addEventListener("contextmenu", onContextMenu);
		return () => document.removeEventListener("contextmenu", onContextMenu);
	}, []);
	return null;
}

export default function App() {
	const { reduceMotion } = useAppearance();
	return (
		<MotionConfig reducedMotion={reduceMotion ? "always" : "never"}>
			<LucideProvider size={16} strokeWidth={1.5}>
				<TooltipProvider delayDuration={300}>
					<ErrorBoundary root>
						<AppFrame />
					</ErrorBoundary>
					<Toaster position="bottom-right" />
					<ShowWindowOnMount />
					<BlockBrowserMenu />
					<CalmLayoutOnResize />
				</TooltipProvider>
			</LucideProvider>
		</MotionConfig>
	);
}
