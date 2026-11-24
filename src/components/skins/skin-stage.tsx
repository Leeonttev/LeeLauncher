import { Footprints, Pause, RotateCcw } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { IdleAnimation, SkinViewer, WalkingAnimation, WaveAnimation } from "skinview3d";
import cloudsDay from "@/assets/pixel/clouds-day.png";
import cloudsNight from "@/assets/pixel/clouds-night.png";
import mannequin from "@/assets/pixel/mannequin.png";
import sparkleDayA from "@/assets/pixel/sparkle-day-a.png";
import sparkleDayB from "@/assets/pixel/sparkle-day-b.png";
import sparkleNightA from "@/assets/pixel/sparkle-night-a.png";
import sparkleNightB from "@/assets/pixel/sparkle-night-b.png";
import stageDay from "@/assets/pixel/stage-day.png";
import stageNight from "@/assets/pixel/stage-night.png";
import { onCoverButton } from "@/components/instances/play-button";
import { SkinFlat } from "@/components/skins/skin-flat";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { SkinModel } from "@/lib/api";
import { useAppearance } from "@/lib/appearance";
import { useInstances } from "@/lib/instances";
import { setSkinWalking, useSkins } from "@/lib/skins";
import { cn } from "@/lib/utils";

const START_ROTATION = -0.45;
const YAW_STEP = (15 * Math.PI) / 180;
const PITCH_STEP = (10 * Math.PI) / 180;
const PITCH_LIMIT = (20 * Math.PI) / 180;
const ZOOM_STEP = 0.88;
const MIN_DISTANCE = 28;
/** При приближении фигура опускается, чтобы в кадре оказалась голова. */
const HEAD_HEIGHT = 12;
const RESET_MS = 240;
const CONTEXT_WAIT_MS = 5000;
const WALK_SPEED = 0.7;
/** В покое хватит 30 кадров; на мониторе 180 Гц иначе было бы 180. */
const IDLE_FPS = 30;
const MOTION_FPS = 60;
const MAX_FRAME_MS = 100;

function PixelLayer({
	src,
	className,
	width = 440,
	height = 168,
}: {
	src: string;
	className?: string;
	width?: number;
	height?: number;
}) {
	return (
		<img
			src={src}
			alt=""
			draggable={false}
			width={width}
			height={height}
			className={cn(
				"pointer-events-none max-w-none select-none [image-rendering:pixelated]",
				className,
			)}
		/>
	);
}

/**
 * `loadSkin(url)` у skinview3d применяет картинку, даже если её уже сменили, поэтому грузим сами и отдаём готовую.
 * Ждём `decode()`, а не `onload`: WebKit после `onload` может ещё не раскодировать картинку, и первый
 * `drawImage` в холст skinview3d рисует пустоту — прозрачная текстура на непрозрачном слое выходит чёрной.
 */
async function loadTexture(src: string) {
	const image = new Image();
	image.src = src;
	await image.decode();
	return image;
}

function easeOut(t: number) {
	return 1 - (1 - t) ** 3;
}

function StageButton({
	label,
	onClick,
	children,
}: {
	label: string;
	onClick: () => void;
	children: ReactNode;
}) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={label}
					className={cn(onCoverButton, "size-8")}
					onClick={onClick}
				>
					{children}
				</button>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}

/** Единственное место со `skinview3d`. Цикл кадров свой: у пакета он идёт с частотой монитора. */
export function SkinStage({
	texture,
	model,
	ariaLabel,
	overlay,
	center,
	bottom,
	onEscape,
	celebration,
	cape,
	capeTurn,
}: {
	/** `null` — манекен. */
	texture: string | null;
	model: SkinModel;
	cape: string | null;
	/** Растёт при выборе плаща: фигура один раз поворачивается спиной. */
	capeTurn: number;
	ariaLabel: string;
	/** Слева сверху: плашка статуса. */
	overlay?: ReactNode;
	/** По центру сцены: загрузка или ошибка без скина. */
	center?: ReactNode;
	/** Снизу по центру: «Вернуть» во время примерки. */
	bottom?: ReactNode;
	/** Esc на сцене; без обработчика клавиша не перехватывается. */
	onEscape?: () => void;
	celebration: { seq: number } | null;
}) {
	const { t } = useTranslation();
	const { reduceMotion } = useAppearance();
	const { walking } = useSkins();
	const { processes } = useInstances();
	const gameRunning = Object.keys(processes).length > 0;
	const [hidden, setHidden] = useState(() => document.hidden);
	const [flat, setFlat] = useState(false);
	const [side, setSide] = useState<"front" | "back">("front");
	/** «Вернуть вид» видна, только когда вид меняли. */
	const [viewChanged, setViewChanged] = useState(false);
	const hostRef = useRef<HTMLDivElement>(null);
	const viewerRef = useRef<SkinViewer | null>(null);
	const tweenRef = useRef<number | null>(null);
	const lastCapeTurn = useRef(capeTurn);
	const source = texture ?? mannequin;
	const paused = reduceMotion || gameRunning || hidden;

	useEffect(() => {
		const onChange = () => setHidden(document.hidden);
		document.addEventListener("visibilitychange", onChange);
		return () => document.removeEventListener("visibilitychange", onChange);
	}, []);

	/** В паузе кадров нет: всё, что меняет картинку, просит кадр само. */
	const requestRender = () => {
		const viewer = viewerRef.current;
		if (viewer?.renderPaused) viewer.render();
	};

	// Каждому входу свой canvas: на старом контекст потерян нарочно, а новый на нём не получить.
	useEffect(() => {
		const host = hostRef.current;
		if (!host || flat) return;
		const canvas = document.createElement("canvas");
		canvas.className = "absolute bottom-0 left-0 block";
		host.append(canvas);
		const listeners = new AbortController();
		let viewer: SkinViewer;
		try {
			viewer = new SkinViewer({
				canvas,
				width: Math.max(1, Math.floor(host.clientWidth)),
				height: Math.max(1, Math.floor(host.clientHeight)),
				fov: 35,
				zoom: 0.82,
				pixelRatio: "match-device",
				renderPaused: true,
			});
		} catch (err) {
			console.error("Не удалось запустить 3D", err);
			canvas.remove();
			queueMicrotask(() => setFlat(true));
			return;
		}
		viewerRef.current = viewer;
		viewer.globalLight.intensity = 3;
		viewer.cameraLight.intensity = 0.6;
		viewer.playerWrapper.rotation.y = START_ROTATION;

		const controls = viewer.controls;
		controls.minPolarAngle = 1.2;
		controls.maxPolarAngle = 1.75;
		const maxDistance = viewer.camera.position.length();
		controls.maxDistance = maxDistance;
		controls.minDistance = MIN_DISTANCE;
		controls.saveState();
		// `start` — только от мыши и колеса, не от `reset()`.
		controls.addEventListener("start", () => setViewChanged(true));
		controls.addEventListener("change", () => {
			const closeness = (maxDistance - viewer.camera.position.length()) / (maxDistance - MIN_DISTANCE);
			viewer.playerWrapper.position.y = -HEAD_HEIGHT * Math.min(1, Math.max(0, closeness));
			if (viewer.renderPaused && !viewer.disposed) viewer.render();
		});

		const observer = new ResizeObserver(() => {
			const width = Math.max(1, Math.floor(host.clientWidth));
			const height = Math.max(1, Math.floor(host.clientHeight));
			if (width === viewer.width && height === viewer.height) return;
			viewer.setSize(width, height);
			if (viewer.renderPaused) viewer.render();
		});
		observer.observe(host);

		let contextTimer: ReturnType<typeof setTimeout> | null = null;
		canvas.addEventListener(
			"webglcontextlost",
			() => {
				contextTimer = setTimeout(() => setFlat(true), CONTEXT_WAIT_MS);
			},
			{ signal: listeners.signal },
		);
		canvas.addEventListener(
			"webglcontextrestored",
			() => {
				if (contextTimer !== null) clearTimeout(contextTimer);
				contextTimer = null;
				if (viewer.renderPaused) viewer.render();
			},
			{ signal: listeners.signal },
		);

		return () => {
			observer.disconnect();
			listeners.abort();
			if (contextTimer !== null) clearTimeout(contextTimer);
			if (tweenRef.current !== null) cancelAnimationFrame(tweenRef.current);
			tweenRef.current = null;
			viewerRef.current = null;
			const renderer = viewer.renderer;
			viewer.dispose();
			// Контекст отдаём сразу, не дожидаясь сборки мусора: их число ограничено.
			renderer.forceContextLoss();
			canvas.remove();
		};
	}, [flat]);

	useEffect(() => {
		const viewer = viewerRef.current;
		if (!viewer) return;
		let stale = false;
		loadTexture(source)
			.then((image) => {
				if (stale || viewer.disposed) return;
				viewer.loadSkin(image, { model: model === "slim" ? "slim" : "default" });
				if (viewer.renderPaused) viewer.render();
				if (!reduceMotion)
					viewer.canvas.animate([{ opacity: 0.6 }, { opacity: 1 }], {
						duration: 120,
						easing: "cubic-bezier(0.22, 1, 0.36, 1)",
					});
			})
			.catch((err: unknown) => {
				if (stale) return;
				console.error("Не удалось загрузить скин на подиум", err);
				if (source === mannequin) return;
				void loadTexture(mannequin).then((image) => {
					if (stale || viewer.disposed) return;
					viewer.loadSkin(image, { model: "default" });
					if (viewer.renderPaused) viewer.render();
				});
			});
		return () => {
			stale = true;
		};
		// `reduceMotion` нужен только для вспышки и не должен перезагружать текстуру.
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [source, model, flat]);

	useEffect(() => {
		const viewer = viewerRef.current;
		if (!viewer) return;
		if (!cape) {
			viewer.loadCape(null);
			viewer.render();
			return;
		}
		let stale = false;
		loadTexture(cape)
			.then((image) => {
				if (stale || viewer.disposed) return;
				viewer.loadCape(image);
				viewer.render();
			})
			.catch((err: unknown) => {
				if (!stale) console.error("Не удалось загрузить плащ на подиум", err);
			});
		return () => {
			stale = true;
		};
	}, [cape, flat]);

	const waving = Boolean(celebration) && !reduceMotion;

	useEffect(() => {
		const viewer = viewerRef.current;
		if (!viewer) return;
		if (reduceMotion) {
			viewer.animation = null;
		} else if (waving) {
			viewer.animation = new WaveAnimation();
		} else if (walking) {
			const walk = new WalkingAnimation();
			walk.speed = WALK_SPEED;
			viewer.animation = walk;
		} else {
			viewer.animation = new IdleAnimation();
		}
		if (viewer.renderPaused) viewer.render();
	}, [reduceMotion, waving, walking, flat, celebration?.seq]);

	const fps = waving || walking ? MOTION_FPS : IDLE_FPS;

	useEffect(() => {
		const viewer = viewerRef.current;
		if (!viewer) return;
		if (paused) {
			viewer.render();
			return;
		}
		const interval = 1000 / fps;
		let last = performance.now();
		const step = (now: number) => {
			frame = requestAnimationFrame(step);
			const elapsed = now - last;
			// Запас в 1 мс: кадр монитора приходит чуть раньше ровного интервала.
			if (elapsed < interval - 1) return;
			last = now;
			viewer.animation?.update(viewer.playerObject, Math.min(elapsed, MAX_FRAME_MS) / 1000);
			viewer.controls.update();
			viewer.render();
		};
		let frame = requestAnimationFrame(step);
		return () => cancelAnimationFrame(frame);
	}, [paused, flat, fps]);

	/** Лишние полные обороты не докручиваем. */
	const turnTo = (viewer: SkinViewer, targetY: number) => {
		if (tweenRef.current !== null) cancelAnimationFrame(tweenRef.current);
		tweenRef.current = null;
		const rotation = viewer.playerWrapper.rotation;
		const from = {
			y: targetY + Math.atan2(Math.sin(rotation.y - targetY), Math.cos(rotation.y - targetY)),
			x: rotation.x,
		};
		if (reduceMotion || (from.y === targetY && from.x === 0)) {
			rotation.set(0, targetY, 0);
			viewer.render();
			return;
		}
		const start = performance.now();
		const step = (now: number) => {
			const progress = Math.min(1, (now - start) / RESET_MS);
			const eased = easeOut(progress);
			rotation.y = from.y + (targetY - from.y) * eased;
			rotation.x = from.x * (1 - eased);
			viewer.render();
			tweenRef.current = progress < 1 ? requestAnimationFrame(step) : null;
		};
		tweenRef.current = requestAnimationFrame(step);
	};

	useEffect(() => {
		if (capeTurn === lastCapeTurn.current) return;
		lastCapeTurn.current = capeTurn;
		const viewer = viewerRef.current;
		if (!viewer) return;
		turnTo(viewer, START_ROTATION + Math.PI);
		setViewChanged(true);
		// Поворот — только на новый выбор плаща.
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [capeTurn]);

	const resetView = () => {
		const viewer = viewerRef.current;
		if (!viewer) return;
		viewer.controls.reset();
		viewer.playerWrapper.position.y = 0;
		turnTo(viewer, START_ROTATION);
		setViewChanged(false);
	};

	const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
		if (event.code === "Escape" && onEscape) {
			event.preventDefault();
			onEscape();
			return;
		}
		const viewer = viewerRef.current;
		if (!viewer || event.ctrlKey || event.altKey || event.metaKey) return;
		const rotation = viewer.playerWrapper.rotation;
		const zoom = (factor: number) => {
			viewer.camera.position.multiplyScalar(factor);
			viewer.controls.update();
		};
		switch (event.code) {
			case "ArrowLeft":
				rotation.y -= YAW_STEP;
				break;
			case "ArrowRight":
				rotation.y += YAW_STEP;
				break;
			case "ArrowUp":
				rotation.x = Math.max(-PITCH_LIMIT, rotation.x - PITCH_STEP);
				break;
			case "ArrowDown":
				rotation.x = Math.min(PITCH_LIMIT, rotation.x + PITCH_STEP);
				break;
			case "Equal":
			case "NumpadAdd":
				zoom(ZOOM_STEP);
				break;
			case "Minus":
			case "NumpadSubtract":
				zoom(1 / ZOOM_STEP);
				break;
			case "Digit0":
			case "Numpad0":
			case "Home":
				resetView();
				event.preventDefault();
				requestRender();
				return;
			case "Space":
				if (reduceMotion) return;
				setSkinWalking(!walking);
				break;
			default:
				return;
		}
		if (event.code !== "Space") setViewChanged(true);
		event.preventDefault();
		requestRender();
	};

	return (
		<div className="relative min-h-[280px] flex-1 overflow-clip rounded-2xl select-none">
			<PixelLayer
				src={stageDay}
				width={440}
				height={1600}
				className="absolute bottom-0 left-[round(down,50%_-_220px,1px)] dark:hidden"
			/>
			<PixelLayer
				src={stageNight}
				width={440}
				height={1600}
				className="absolute bottom-0 left-[round(down,50%_-_220px,1px)] hidden dark:block"
			/>
			<div className="pointer-events-none absolute bottom-[240px] left-[round(down,50%_-_220px,1px)] h-[168px] w-[440px] overflow-clip">
				<div className="px-clouds absolute inset-y-0 left-0 flex w-[880px]">
					<PixelLayer src={cloudsDay} className="shrink-0 dark:hidden" />
					<PixelLayer src={cloudsDay} className="shrink-0 dark:hidden" />
					<PixelLayer src={cloudsNight} className="hidden shrink-0 dark:block" />
					<PixelLayer src={cloudsNight} className="hidden shrink-0 dark:block" />
				</div>
			</div>
			{celebration && (
				<div
					aria-hidden
					className="pointer-events-none absolute bottom-[160px] left-[round(down,50%_-_220px,1px)] h-[168px] w-[440px] [--px-swap:0.8s]"
				>
					<PixelLayer src={sparkleDayA} className="px-swap absolute inset-0 dark:hidden" />
					<PixelLayer
						src={sparkleDayB}
						className="px-swap px-swap-b absolute inset-0 opacity-0 dark:hidden"
					/>
					<PixelLayer src={sparkleNightA} className="px-swap absolute inset-0 hidden dark:block" />
					<PixelLayer
						src={sparkleNightB}
						className="px-swap px-swap-b absolute inset-0 hidden opacity-0 dark:block"
					/>
				</div>
			)}

			{flat ? (
				<>
					<SkinFlat
						texture={source}
						fallback={mannequin}
						model={model}
						scale={8}
						side={side}
						className="absolute bottom-16 left-[round(down,50%_-_64px,1px)]"
					/>
					<span role="img" aria-label={ariaLabel} />
				</>
			) : (
				<div
					ref={hostRef}
					role="img"
					tabIndex={0}
					aria-label={ariaLabel}
					data-skin-stage
					className="peer absolute inset-x-0 top-0 bottom-8 mt-auto max-h-[800px] cursor-grab outline-none active:cursor-grabbing"
					onKeyDown={onKeyDown}
					onDoubleClick={resetView}
				/>
			)}
			<span
				aria-hidden
				className="pointer-events-none absolute inset-0 rounded-2xl ring-ring ring-inset peer-focus-visible:ring-2"
			/>

			{overlay && <div className="absolute top-3 left-3 max-w-[calc(100%-108px)]">{overlay}</div>}
			<div className="absolute top-3 right-3 flex gap-1">
				{flat ? (
					<button
						type="button"
						className={cn(onCoverButton, "h-8 px-3 text-[13px]")}
						onClick={() => setSide(side === "front" ? "back" : "front")}
					>
						{side === "front" ? t("skins.stage.back") : t("skins.stage.front")}
					</button>
				) : (
					<>
						{!reduceMotion && (
							<StageButton
								label={walking ? t("skins.stage.stop") : t("skins.stage.walk")}
								onClick={() => setSkinWalking(!walking)}
							>
								{walking ? <Pause /> : <Footprints />}
							</StageButton>
						)}
						{viewChanged && (
							<StageButton label={t("skins.stage.resetView")} onClick={resetView}>
								<RotateCcw />
							</StageButton>
						)}
					</>
				)}
			</div>

			{flat && (
				<p className="pointer-events-none absolute inset-x-3 bottom-14 rounded-md bg-cover-scrim/70 px-2.5 py-1.5 text-center text-[13px]/[18px] text-on-cover">
					{t("skins.stage.noWebgl")}
				</p>
			)}
			{center && (
				<div className="pointer-events-none absolute inset-0 flex items-center justify-center p-4">
					<div className="pointer-events-auto flex max-w-full flex-col items-center gap-2">
						{center}
					</div>
				</div>
			)}
			{bottom && (
				<div className="absolute bottom-3 left-[round(down,50%,1px)] max-w-[calc(100%-24px)] translate-x-[round(down,-50%,1px)]">
					{bottom}
				</div>
			)}
		</div>
	);
}
