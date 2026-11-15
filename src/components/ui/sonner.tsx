import { CircleCheck, Info, LoaderCircle, OctagonX, TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Toaster as Sonner, type ToasterProps } from "sonner";
import { useAppearance } from "@/lib/appearance";

const Toaster = ({ ...props }: ToasterProps) => {
	const { t } = useTranslation();
	const { resolvedTheme } = useAppearance();

	return (
		<Sonner
			theme={resolvedTheme}
			className="toaster group"
			containerAriaLabel={t("toasts.region")}
			toastOptions={{
				closeButtonAriaLabel: t("toasts.close"),
			}}
			icons={{
				success: <CircleCheck className="size-5 text-success" />,
				info: <Info className="size-5" />,
				warning: <TriangleAlert className="size-5 text-warning" />,
				error: <OctagonX className="size-5 text-destructive" />,
				loading: <LoaderCircle className="size-5 animate-spin" />,
			}}
			style={
				{
					"--normal-bg": "var(--popover)",
					"--normal-text": "var(--popover-foreground)",
					"--normal-border": "var(--popover-border)",
					"--border-radius": "16px",
				} as React.CSSProperties
			}
			{...props}
		/>
	);
};

export { Toaster };
