import { MotionGlobalConfig } from "motion/react";

/** Пока меняется размер окна, анимаций нет: раскладка меняется не от действия пользователя. Атрибут на `<html>` ставим напрямую — стор оформления чужие атрибуты не трогает. */
const SETTLE_MS = 150;

let timer: number | undefined;

function end() {
	timer = undefined;
	delete document.documentElement.dataset.layoutCalm;
	MotionGlobalConfig.instantAnimations = false;
}

export function holdLayoutCalm() {
	if (timer === undefined) {
		document.documentElement.dataset.layoutCalm = "";
		MotionGlobalConfig.instantAnimations = true;
	} else {
		window.clearTimeout(timer);
	}
	timer = window.setTimeout(end, SETTLE_MS);
}

export function watchWindowLayout() {
	let width = window.innerWidth;
	let height = window.innerHeight;
	const onResize = () => {
		if (window.innerWidth === width && window.innerHeight === height) return;
		width = window.innerWidth;
		height = window.innerHeight;
		holdLayoutCalm();
	};
	window.addEventListener("resize", onResize);
	return () => {
		window.removeEventListener("resize", onResize);
		if (timer !== undefined) {
			window.clearTimeout(timer);
			end();
		}
	};
}
