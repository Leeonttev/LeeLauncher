import { useEffect, type RefObject } from "react";

export const IMAGE_DEADLINE = 8_000;

/**
 * Картинка с недоступного сервера может грузиться бесконечно. Отсчёт идёт, только пока элемент
 * на экране: `loading="lazy"` вне экрана и не начинает загрузку.
 */
export function useImageDeadline(
	ref: RefObject<Element | null>,
	pending: boolean,
	onTimeout: () => void,
	ms = IMAGE_DEADLINE,
) {
	useEffect(() => {
		const node = ref.current;
		if (!pending || !node) return;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const observer = new IntersectionObserver(([entry]) => {
			if (!entry.isIntersecting || timer !== undefined) return;
			timer = setTimeout(onTimeout, ms);
			observer.disconnect();
		});
		observer.observe(node);
		return () => {
			observer.disconnect();
			if (timer !== undefined) clearTimeout(timer);
		};
	}, [ref, pending, onTimeout, ms]);
}
