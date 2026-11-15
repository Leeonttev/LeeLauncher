// Векторные данные для установщика: иконки lucide и знак LeeLauncher → installer/src/art_data.rs.
// Запуск: node scripts/build-installer-art.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const ICONS = [
	"check",
	"x",
	"circle-check",
	"triangle-exclamation-point",
	"octagon-x",
	"loader-circle",
	"download",
	"shield-check",
	"play",
	"external-link",
	"chevron-down",
	"info",
	"circle-exclamation-point",
	"folder-open",
	"monitor",
	"hard-drive",
	"globe",
	"app-window",
	"rocket",
	"wifi-off",
];

const num = (v) => Number(v ?? 0);

function rectPath({ x, y, width, height, rx, ry }) {
	x = num(x);
	y = num(y);
	const w = num(width);
	const h = num(height);
	const r = Math.min(num(rx ?? ry), w / 2, h / 2);
	if (!r) return `M${x} ${y}H${x + w}V${y + h}H${x}Z`;
	return [
		`M${x + r} ${y}`,
		`H${x + w - r}`,
		`A${r} ${r} 0 0 1 ${x + w} ${y + r}`,
		`V${y + h - r}`,
		`A${r} ${r} 0 0 1 ${x + w - r} ${y + h}`,
		`H${x + r}`,
		`A${r} ${r} 0 0 1 ${x} ${y + h - r}`,
		`V${y + r}`,
		`A${r} ${r} 0 0 1 ${x + r} ${y}`,
		"Z",
	].join("");
}

function element([tag, a]) {
	switch (tag) {
		case "path":
			return a.d;
		case "circle": {
			const [cx, cy, r] = [num(a.cx), num(a.cy), num(a.r)];
			return `M${cx - r} ${cy}A${r} ${r} 0 1 0 ${cx + r} ${cy}A${r} ${r} 0 1 0 ${cx - r} ${cy}Z`;
		}
		case "line":
			return `M${a.x1} ${a.y1}L${a.x2} ${a.y2}`;
		case "rect":
			return rectPath(a);
		case "polyline":
		case "polygon": {
			const pts = a.points
				.trim()
				.split(/[\s,]+/)
				.map(Number);
			let d = `M${pts[0]} ${pts[1]}`;
			for (let i = 2; i < pts.length; i += 2) d += `L${pts[i]} ${pts[i + 1]}`;
			return tag === "polygon" ? d + "Z" : d;
		}
		default:
			throw new Error(`unknown element ${tag}`);
	}
}

async function iconNode(name) {
	const url = pathToFileURL(`${root}node_modules/lucide-react/dist/esm/icons/${name}.mjs`);
	const mod = await import(url.href);
	return mod.__iconData.node.map(element);
}

const brand = readFileSync(`${root}src/components/brand/identity-data.ts`, "utf8");
const data = Function(
	brand
		.replace(/export const /g, "const ")
		.replace(/ as const/g, "")
		.concat("\nreturn { SPARK, LOCKUP, ICON_TILE, ICON_MARK };"),
)();

const str = (s) => JSON.stringify(s);
const float = (n) => (Number.isInteger(n) ? `${n}.0` : `${n}`);
const list = (arr) => `&[${arr.map(str).join(", ")}]`;
let out = "// Создан scripts/build-installer-art.mjs, руками не править.\n\n";
out += "pub const ICONS: &[(&str, &[&str])] = &[\n";
for (const name of ICONS) out += `\t(${str(name)}, ${list(await iconNode(name))}),\n`;
out += "];\n\n";
out += `pub const SPARK_SIZE: (f32, f32) = (${float(data.SPARK.width)}, ${float(data.SPARK.height)});\n`;
out += `pub const SPARK: &[&str] = ${list(data.SPARK.paths)};\n`;
out += `pub const LOCKUP_SIZE: (f32, f32) = (${float(data.LOCKUP.width)}, ${float(data.LOCKUP.height)});\n`;
out += `pub const LOCKUP_SPARK: &[&str] = ${list(data.LOCKUP.spark)};\n`;
out += `pub const LOCKUP_WORD: &[&str] = ${list(data.LOCKUP.word)};\n`;
out += `pub const ICON_TILE: &str = ${str(data.ICON_TILE)};\n`;
out += `pub const ICON_MARK_LARGE: f32 = ${float(data.ICON_MARK.large)};\n`;
writeFileSync(`${root}installer/src/art_data.rs`, out);
console.log("installer/src/art_data.rs");
