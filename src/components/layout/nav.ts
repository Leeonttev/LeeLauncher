import { Boxes, Compass, House, Settings, Shirt, type LucideIcon } from "lucide-react";
import type { RoutePath } from "@/lib/router";
import type { Dictionary } from "@/locales/ru";

export type NavItem = { path: RoutePath; labelKey: `nav.${keyof Dictionary["nav"]}`; icon: LucideIcon };

export const mainNav: NavItem[] = [
	{ path: "/", labelKey: "nav.home", icon: House },
	{ path: "/instances", labelKey: "nav.instances", icon: Boxes },
	{ path: "/catalog", labelKey: "nav.catalog", icon: Compass },
	{ path: "/skins", labelKey: "nav.skins", icon: Shirt },
];

export const settingsNav: NavItem = { path: "/settings", labelKey: "nav.settings", icon: Settings };
