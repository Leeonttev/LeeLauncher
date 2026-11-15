import { useRef } from "react";

/** Действие ждёт `onCloseAutoFocus` меню: иначе меню и диалог спорят за фокус. */
export function usePendingAction() {
	const pending = useRef<(() => void) | null>(null);
	return {
		defer: (action: () => void) => {
			pending.current = action;
		},
		onCloseAutoFocus: (event: Event) => {
			const action = pending.current;
			if (!action) return;
			pending.current = null;
			event.preventDefault();
			action();
		},
	};
}
