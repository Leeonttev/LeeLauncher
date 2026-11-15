import type { SkinModel } from "@/lib/api";

type SkinImage = { image: HTMLImageElement; helm: boolean };

const images = new Map<string, Promise<SkinImage>>();

/** У старых скинов без прозрачности верхний слой головы чёрный. Правило как в skinview3d, чтобы плоский вид совпадал с 3D. */
function hasHelm(image: HTMLImageElement) {
	const canvas = document.createElement("canvas");
	canvas.width = image.naturalWidth;
	canvas.height = image.naturalHeight;
	const context = canvas.getContext("2d", { willReadFrequently: true });
	if (!context) return true;
	context.drawImage(image, 0, 0);
	const { data } = context.getImageData(0, 0, canvas.width, canvas.height);
	for (let i = 3; i < data.length; i += 4) if (data[i] !== 255) return true;
	return false;
}

function loadImage(texture: string): Promise<SkinImage> {
	const known = images.get(texture);
	if (known) return known;
	const image = new Image();
	image.src = texture;
	const loaded = image.decode().then(() => ({ image, helm: hasHelm(image) }));
	loaded.catch(() => images.delete(texture));
	images.set(texture, loaded);
	return loaded;
}

type Rect = readonly [x: number, y: number, w: number, h: number];

/** `mirror` — зеркальная копия для 64×32. */
type Part = { from: Rect; overlay?: Rect; x: number; y: number; mirror?: boolean };

/** Тонкая рука — 3 пикселя шириной. */
function figureParts(legacy: boolean, slim: boolean, side: "front" | "back"): Part[] {
	const arm = slim ? 3 : 4;
	// Слева на холсте у вида спереди — правая рука персонажа, у вида сзади — левая.
	const onLeft = side === "front" ? "right" : "left";
	const armX = (which: "right" | "left") => (which === onLeft ? 4 - arm : 12);
	const legX = (which: "right" | "left") => (which === onLeft ? 4 : 8);
	const front = side === "front";

	const parts: Part[] = [
		{
			from: front ? [8, 8, 8, 8] : [24, 8, 8, 8],
			overlay: front ? [40, 8, 8, 8] : [56, 8, 8, 8],
			x: 4,
			y: 0,
		},
		{
			from: front ? [20, 20, 8, 12] : [32, 20, 8, 12],
			overlay: front ? [20, 36, 8, 12] : [32, 36, 8, 12],
			x: 4,
			y: 8,
		},
		{
			from: front ? [44, 20, arm, 12] : [48 + arm, 20, arm, 12],
			overlay: front ? [44, 36, arm, 12] : [48 + arm, 36, arm, 12],
			x: armX("right"),
			y: 8,
		},
		{
			from: front ? [4, 20, 4, 12] : [12, 20, 4, 12],
			overlay: front ? [4, 36, 4, 12] : [12, 36, 4, 12],
			x: legX("right"),
			y: 20,
		},
	];
	if (legacy) {
		// В 64×32 нет левых рук и ног и верхних слоёв тела: левые — зеркало правых.
		return [
			{ from: parts[0].from, overlay: parts[0].overlay, x: 4, y: 0 },
			{ from: parts[1].from, x: 4, y: 8 },
			{ from: parts[2].from, x: armX("right"), y: 8 },
			{ from: parts[2].from, x: armX("left"), y: 8, mirror: true },
			{ from: parts[3].from, x: legX("right"), y: 20 },
			{ from: parts[3].from, x: legX("left"), y: 20, mirror: true },
		];
	}
	return [
		...parts,
		{
			from: front ? [36, 52, arm, 12] : [40 + arm, 52, arm, 12],
			overlay: front ? [52, 52, arm, 12] : [56 + arm, 52, arm, 12],
			x: armX("left"),
			y: 8,
		},
		{
			from: front ? [20, 52, 4, 12] : [28, 52, 4, 12],
			overlay: front ? [4, 52, 4, 12] : [12, 52, 4, 12],
			x: legX("left"),
			y: 20,
		},
	];
}

function drawRect(
	context: CanvasRenderingContext2D,
	image: HTMLImageElement,
	[sx, sy, w, h]: Rect,
	x: number,
	y: number,
	mirror = false,
) {
	if (!mirror) {
		context.drawImage(image, sx, sy, w, h, x, y, w, h);
		return;
	}
	context.save();
	context.translate(x + w, y);
	context.scale(-1, 1);
	context.drawImage(image, sx, sy, w, h, 0, 0, w, h);
	context.restore();
}

export async function drawSkinFigure(
	canvas: HTMLCanvasElement,
	texture: string,
	model: SkinModel,
	side: "front" | "back" = "front",
) {
	const { image, helm } = await loadImage(texture);
	const context = canvas.getContext("2d");
	if (!context) throw new Error("canvas 2d недоступен");
	context.imageSmoothingEnabled = false;
	context.clearRect(0, 0, canvas.width, canvas.height);
	const legacy = image.naturalHeight === 32;
	for (const part of figureParts(legacy, model === "slim" && !legacy, side)) {
		drawRect(context, image, part.from, part.x, part.y, part.mirror);
		// Голова — единственная часть с `y === 0`.
		if (part.overlay && (helm || part.y !== 0))
			drawRect(context, image, part.overlay, part.x, part.y, part.mirror);
	}
}

/** Область 1,1,10,16 при 64×32, у других размеров — пропорционально. */
export async function drawCapeFront(canvas: HTMLCanvasElement, texture: string) {
	const { image } = await loadImage(texture);
	const context = canvas.getContext("2d");
	if (!context) throw new Error("canvas 2d недоступен");
	const scale = image.naturalWidth / 64;
	context.imageSmoothingEnabled = false;
	context.clearRect(0, 0, canvas.width, canvas.height);
	context.drawImage(image, scale, scale, 10 * scale, 16 * scale, 0, 0, canvas.width, canvas.height);
}

const heads = new Map<string, string>();

/** Чтобы не мигал скелетон при каждом монтировании аватара. */
export function cachedSkinHead(key: string): string | null {
	return heads.get(key) ?? null;
}

/** Лицо принудительно непрозрачное. Кэш по ключу текстуры: у несохранённого скина нет `skinId`. */
export async function skinHead(key: string, texture: string): Promise<string> {
	const known = heads.get(key);
	if (known) return known;
	const { image, helm } = await loadImage(texture);
	const canvas = document.createElement("canvas");
	canvas.width = 8;
	canvas.height = 8;
	const context = canvas.getContext("2d", { willReadFrequently: true });
	if (!context) throw new Error("canvas 2d недоступен");
	context.imageSmoothingEnabled = false;
	context.drawImage(image, 8, 8, 8, 8, 0, 0, 8, 8);
	const face = context.getImageData(0, 0, 8, 8);
	for (let i = 3; i < face.data.length; i += 4) face.data[i] = 255;
	context.putImageData(face, 0, 0);
	if (helm) context.drawImage(image, 40, 8, 8, 8, 0, 0, 8, 8);
	const url = canvas.toDataURL("image/png");
	heads.set(key, url);
	return url;
}
