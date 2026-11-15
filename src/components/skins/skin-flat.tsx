import { Bookmark, Shirt } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Skeleton } from "@/components/ui/skeleton";
import type { SkinModel } from "@/lib/api";
import { drawCapeFront, drawSkinFigure } from "@/lib/skin-texture";
import { cn } from "@/lib/utils";

const WIDTH = 16;
const HEIGHT = 32;

/** Без WebGL: браузер ограничивает число контекстов. Масштаб только целый. */
export function SkinFlat({
	texture,
	model,
	scale,
	side = "front",
	fallback,
	className,
}: {
	texture: string;
	fallback?: string;
	model: SkinModel;
	scale: 4 | 8;
	side?: "front" | "back";
	className?: string;
}) {
	const ref = useRef<HTMLCanvasElement>(null);
	const key = `${model}:${side}:${texture}`;
	const [result, setResult] = useState<{ key: string; ok: boolean } | null>(null);

	useEffect(() => {
		const canvas = ref.current;
		if (!canvas) return;
		let stale = false;
		drawSkinFigure(canvas, texture, model, side).then(
			() => {
				if (!stale) setResult({ key, ok: true });
			},
			(err: unknown) => {
				console.error("Не удалось нарисовать скин", err);
				if (stale) return;
				if (!fallback || fallback === texture) return setResult({ key, ok: false });
				drawSkinFigure(canvas, fallback, "classic", side).then(
					() => {
						if (!stale) setResult({ key, ok: true });
					},
					() => {
						if (!stale) setResult({ key, ok: false });
					},
				);
			},
		);
		return () => {
			stale = true;
		};
	}, [key, texture, model, side, fallback]);

	const done = result?.key === key;
	const width = WIDTH * scale;
	const height = HEIGHT * scale;
	return (
		<span aria-hidden className={cn("relative block shrink-0", className)} style={{ width, height }}>
			<canvas
				ref={ref}
				width={WIDTH}
				height={HEIGHT}
				className="absolute inset-0 size-full [image-rendering:pixelated]"
			/>
			{!done && <Skeleton className="absolute inset-0 rounded-lg" />}
			{done && !result.ok && (
				<span className="absolute inset-0 flex items-center justify-center text-muted-foreground">
					<Shirt />
				</span>
			)}
		</span>
	);
}

export function CapeFlat({ texture }: { texture: string | null }) {
	const ref = useRef<HTMLCanvasElement>(null);
	const [failed, setFailed] = useState<string | null>(null);

	useEffect(() => {
		const canvas = ref.current;
		if (!canvas || !texture) return;
		let stale = false;
		drawCapeFront(canvas, texture).catch((err: unknown) => {
			console.error("Не удалось нарисовать плащ", err);
			if (!stale) setFailed(texture);
		});
		return () => {
			stale = true;
		};
	}, [texture]);

	if (!texture || failed === texture)
		return (
			<span aria-hidden className="flex h-8 w-5 shrink-0 items-center justify-center">
				<Bookmark />
			</span>
		);
	return (
		<canvas
			ref={ref}
			aria-hidden
			width={10}
			height={16}
			className="h-8 w-5 shrink-0 [image-rendering:pixelated]"
		/>
	);
}
