import type { KeyboardEvent } from "react";
import type { Instance } from "@/lib/api";
import { isBusy, openInstanceDialog, type InstanceStatus } from "@/lib/instances";

/** `::after` названия накрывает весь блок (ближайший `relative`); то, что должно нажиматься, — `z-10`. */
export const stretchedLink =
	"text-left outline-none after:absolute after:inset-0 after:z-[1] after:content-['']";

export const linkFocusRing =
	"transition-[border-color,background-color] duration-(--duration-fast) ease-out has-[button[data-link]:focus-visible]:outline-2 has-[button[data-link]:focus-visible]:outline-offset-2 has-[button[data-link]:focus-visible]:outline-ring";

/** Ссылка лежит не в обложке, а рядом, внутри общего `group`. */
export const coverFocusRing =
	"group-has-[button[data-link]:focus-visible]:outline-2 group-has-[button[data-link]:focus-visible]:outline-offset-2 group-has-[button[data-link]:focus-visible]:outline-ring";

export function onLinkKeyDown(instance: Instance, status: InstanceStatus) {
	return (event: KeyboardEvent<HTMLButtonElement>) => {
		if (event.key === "Delete" && !isBusy(status)) {
			event.preventDefault();
			openInstanceDialog("remove", instance.id, { returnFocus: event.currentTarget });
		}
	};
}
