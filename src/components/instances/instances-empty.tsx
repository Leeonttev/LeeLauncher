import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
import emptySprite from "@/assets/pixel/sprite-instances-empty.png";
import { CenteredState } from "@/components/centered-state";
import { InstanceCover } from "@/components/instances/instance-cover";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { openCreateInstanceDialog } from "@/lib/instances";

/** Ящик стоит от внутреннего края рамки, без `translate`: координаты целые. */
function EmptyArt() {
	return (
		<div className="relative h-40 w-[400px] shrink-0 overflow-clip rounded-xl border border-border">
			<InstanceCover scene="meadow" className="absolute inset-0" />
			<img
				src={emptySprite}
				alt=""
				width={64}
				height={64}
				draggable={false}
				className="px-bob pointer-events-none absolute bottom-2 left-[164px] size-16 select-none [--px-bob:1.6s] [image-rendering:pixelated]"
			/>
		</div>
	);
}

export function CreateInstanceButton({
	variant = "default",
	disabled,
	className,
}: {
	variant?: "default" | "outline";
	disabled?: boolean;
	className?: string;
}) {
	const { t } = useTranslation();
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant={variant}
					disabled={disabled}
					aria-keyshortcuts="Control+N"
					className={className}
					onClick={(e) => openCreateInstanceDialog(e.currentTarget)}
				>
					<Plus />
					{t("instances.create")}
				</Button>
			</TooltipTrigger>
			<TooltipContent>{t("instances.createShortcut")}</TooltipContent>
		</Tooltip>
	);
}

export function InstancesEmpty() {
	const { t } = useTranslation();
	return (
		<CenteredState
			media={<EmptyArt />}
			title={t("instances.empty.title")}
			description={t("instances.empty.description")}
			heading="h2"
			action={<CreateInstanceButton />}
		/>
	);
}
