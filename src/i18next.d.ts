import "i18next";
import type { Dictionary } from "@/locales/ru";

declare module "i18next" {
	interface CustomTypeOptions {
		defaultNS: "translation";
		resources: { translation: Dictionary };
		returnNull: false;
		interpolationPrefix: "{";
		interpolationSuffix: "}";
	}
}
