import { isMac } from "@/lib/platform";

/**
 * Пороги ужатия полосы заголовка; расчёт — scripts/measure-titlebar.py (с `--mac` для macOS).
 * На Mac слева место под системные кнопки окна, а своих кнопок справа нет, поэтому пороги другие.
 * Классы с container query — целыми строками: Tailwind ищет их в исходнике текстом.
 */
export const TITLEBAR_BREAKPOINTS = isMac
	? ({ wordmark: 1016, compact: 876, iconsOnly: 816 } as const)
	: ({ wordmark: 1080, compact: 940, iconsOnly: 880 } as const);

export const titlebarClasses = isMac
	? {
			// pl-[92px] = MAC_TRAFFIC_LIGHTS.
			brand: "w-[249px] pl-[92px] @max-[1016px]/titlebar:w-[113px]",
			lockup: "flex @max-[1016px]/titlebar:hidden",
			spark: "hidden @max-[1016px]/titlebar:flex",
			tab: "@max-[816px]/titlebar:px-2.5",
			tabLabel: "@max-[816px]/titlebar:sr-only",
			nick: "@max-[876px]/titlebar:max-w-20",
			addAccount: "@max-[876px]/titlebar:max-w-24",
		}
	: {
			brand: "w-[177px] pl-5 @max-[1080px]/titlebar:w-[41px]",
			lockup: "flex @max-[1080px]/titlebar:hidden",
			spark: "hidden @max-[1080px]/titlebar:flex",
			tab: "@max-[880px]/titlebar:px-2.5",
			tabLabel: "@max-[880px]/titlebar:sr-only",
			nick: "@max-[940px]/titlebar:max-w-20",
			addAccount: "@max-[940px]/titlebar:max-w-24",
		};
