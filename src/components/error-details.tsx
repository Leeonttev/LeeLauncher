import { Check, ChevronRight, Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

export function ErrorDetails({ details, className }: { details: string; className?: string }) {
	const { t } = useTranslation();
	const [copied, setCopied] = useState(false);

	useEffect(() => {
		if (!copied) return;
		const timer = setTimeout(() => setCopied(false), 1500);
		return () => clearTimeout(timer);
	}, [copied]);

	const copy = () => {
		navigator.clipboard.writeText(details).then(
			() => setCopied(true),
			() => toast.error(t("errors.fallback")),
		);
	};

	return (
		<Collapsible className={cn("group/details mt-3", className)}>
			<CollapsibleTrigger asChild>
				<Button variant="ghost" size="sm" className="-ml-2 h-7 gap-1 px-2 text-muted-foreground">
					<ChevronRight className="transition-transform duration-(--duration-base) ease-out group-data-[state=open]/details:rotate-90" />
					{t("common.details")}
				</Button>
			</CollapsibleTrigger>
			<CollapsibleContent className="-mx-2 -mb-1.5 overflow-hidden px-2 pb-1.5 data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down">
				<pre className="mt-2 max-h-40 overflow-auto rounded-lg bg-muted p-3 text-left font-mono text-xs/[18px] break-all whitespace-pre-wrap select-text">
					{details}
				</pre>
				<Button variant="ghost" size="sm" className="mt-2 -ml-2" onClick={copy}>
					{copied ? <Check /> : <Copy />}
					{copied ? t("common.copied") : t("common.copy")}
				</Button>
			</CollapsibleContent>
		</Collapsible>
	);
}
