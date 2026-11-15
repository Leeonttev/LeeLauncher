import { ExternalLink } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Lockup } from "@/components/brand/identity";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { appInfo, external } from "@/lib/api";
import { APP_AUTHOR, APP_LICENSE, SOURCE_URL } from "@/lib/app-info";

type VersionState = { status: "loading" } | { status: "ready"; version: string } | { status: "error" };

function useAppVersion(): VersionState {
	const [state, setState] = useState<VersionState>({ status: "loading" });
	useEffect(() => {
		let cancelled = false;
		appInfo
			.version()
			.then((version) => {
				if (!cancelled) setState({ status: "ready", version });
			})
			.catch(() => {
				if (!cancelled) setState({ status: "error" });
			});
		return () => {
			cancelled = true;
		};
	}, []);
	return state;
}

export function AboutSection() {
	const { t } = useTranslation();
	const version = useAppVersion();

	const openSource = () => {
		external.open(SOURCE_URL).catch(() => toast.error(t("common.openLinkFailed")));
	};

	return (
		<section aria-labelledby="settings-about" className="mt-10">
			<h2 id="settings-about" className="text-xl/[26px] font-semibold tracking-[-0.015em]">
				{t("settings.about.title")}
			</h2>
			<div className="mt-5 flex flex-wrap items-center gap-x-5 gap-y-2">
				<Lockup size={28} className="text-foreground" />
				<div className="flex flex-col">
					<div className="flex h-[18px] items-center text-[13px]/[18px] text-muted-foreground">
						{version.status === "loading" && <Skeleton className="h-4 w-24" />}
						{version.status === "ready" && (
							<span>{t("settings.about.version", { version: version.version })}</span>
						)}
						{version.status === "error" && <span>{t("settings.about.versionUnknown")}</span>}
					</div>
				</div>
			</div>
			<p className="mt-5 max-w-prose text-sm">{t("settings.about.tagline")}</p>
			<dl className="mt-4 grid grid-cols-[120px_1fr] gap-y-2 text-[13px]/[18px]">
				<dt className="text-muted-foreground">{t("settings.about.license")}</dt>
				<dd className="font-medium">{APP_LICENSE}</dd>
				<dt className="text-muted-foreground">{t("settings.about.author")}</dt>
				<dd className="font-medium">{APP_AUTHOR}</dd>
			</dl>
			<Button variant="outline" size="sm" className="mt-5" onClick={openSource}>
				{t("settings.about.sourceCode")}
				<ExternalLink />
			</Button>
		</section>
	);
}
