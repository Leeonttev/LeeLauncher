import { Spark } from "@/components/brand/identity";
import { OctagonX, RotateCcw } from "lucide-react";
import { Component, type ErrorInfo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CenteredState } from "@/components/centered-state";
import { ErrorDetails } from "@/components/error-details";
import { WindowButton, WindowGlyph } from "@/components/layout/window-controls";
import { Button } from "@/components/ui/button";
import { appWindow } from "@/lib/api";
import { en } from "@/locales/en";
import { ru, type Dictionary } from "@/locales/ru";

type ErrorBoundaryProps = {
	children: ReactNode;
	/** У запасного экрана своя полоса заголовка: окно без рамки иначе не сдвинуть и не закрыть. */
	root?: boolean;
};

type ErrorBoundaryState = { failed: boolean; details: string | null };

function describeError(error: unknown, componentStack?: string | null): string {
	const lines = (text: string) => text.split("\n").filter((line) => line.trim());
	const parts: string[] = [];
	if (error instanceof Error) {
		const stack = lines(error.stack ?? "");
		const head = `${error.name}: ${error.message}`;
		parts.push((stack[0]?.includes(error.message) ? stack : [head, ...stack]).slice(0, 9).join("\n"));
	} else {
		parts.push(String(error));
	}
	const components = lines(componentStack ?? "").slice(0, 6);
	if (components.length) parts.push(components.map((line) => line.trim()).join("\n"));
	return parts.join("\n\n");
}

function ErrorMessage({ text, details }: { text: Dictionary["errorBoundary"]; details?: string | null }) {
	return (
		<CenteredState
			icon={OctagonX}
			tone="destructive"
			title={text.title}
			description={text.description}
			action={
				<Button onClick={() => location.reload()}>
					<RotateCcw />
					{text.reload}
				</Button>
			}
			footer={details ? <ErrorDetails details={details} className="mt-0 text-center" /> : undefined}
		/>
	);
}

function PageFallback({ details }: { details: string | null }) {
	const { t } = useTranslation();
	return (
		<div className="flex h-full min-h-0 flex-1 flex-col">
			<ErrorMessage
				text={{
					title: t("errorBoundary.title"),
					description: t("errorBoundary.description"),
					reload: t("errorBoundary.reload"),
				}}
				details={details}
			/>
		</div>
	);
}

/** Без react-i18next: экран может сработать, когда упали сами переводы. */
function RootFallback() {
	const dict = document.documentElement.lang === "en" ? en : ru;
	return (
		<div className="flex h-svh flex-col bg-background">
			<header
				data-tauri-drag-region
				className="flex h-14 shrink-0 items-center border-b bg-sidebar select-none"
			>
				<span
					data-tauri-drag-region
					className="flex h-full items-center pl-5 text-foreground [&>*]:pointer-events-none"
				>
					<Spark accent size={22} />
				</span>
				<div data-tauri-drag-region className="h-full flex-1" />
				<WindowButton
					label={dict.window.minimize}
					onClick={() => void appWindow.minimize().catch(() => {})}
				>
					<WindowGlyph name="minimize" />
				</WindowButton>
				<WindowButton
					label={dict.window.close}
					danger
					onClick={() => void appWindow.close().catch(() => {})}
				>
					<WindowGlyph name="close" />
				</WindowButton>
			</header>
			<div className="flex min-h-0 flex-1 flex-col">
				<ErrorMessage text={dict.errorBoundary} />
			</div>
		</div>
	);
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
	state: ErrorBoundaryState = { failed: false, details: null };

	static getDerivedStateFromError(error: unknown): ErrorBoundaryState {
		return { failed: true, details: describeError(error) };
	}

	componentDidCatch(error: Error, info: ErrorInfo) {
		console.error("ErrorBoundary", error, info.componentStack);
		this.setState({ details: describeError(error, info.componentStack) });
	}

	render() {
		if (!this.state.failed) return this.props.children;
		if (this.props.root) return <RootFallback />;
		return <PageFallback details={this.state.details} />;
	}
}
