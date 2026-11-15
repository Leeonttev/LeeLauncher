import { spawn } from "node:child_process";
import {
	copyFileSync,
	existsSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	rmSync,
	unlinkSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SMALL = readFileSync(join(ROOT, "assets/logo/leelauncher-favicon.svg"), "utf8");
const LARGE = readFileSync(join(ROOT, "assets/logo/leelauncher-icon.svg"), "utf8");
const ICONS = join(ROOT, "src-tauri/icons");
const CHROME = process.env.CHROME ?? "C:/Program Files/Google/Chrome/Application/chrome.exe";

const SIZES = [16, 24, 32, 48, 64, 128, 256, 512, 1024];
const ICO_SIZES = [16, 24, 32, 48, 64, 256];
const ICNS_TYPES = [
	["icp4", 16],
	["icp5", 32],
	["ic11", 32],
	["ic12", 64],
	["ic07", 128],
	["ic13", 256],
	["ic08", 256],
	["ic14", 512],
	["ic09", 512],
	["ic10", 1024],
];
const COPIES = { "32x32.png": 32, "128x128.png": 128, "128x128@2x.png": 256, "icon.png": 512 };
const TILES = {
	StoreLogo: 50,
	...Object.fromEntries([30, 44, 71, 89, 107, 142, 150, 284, 310].map((s) => [`Square${s}x${s}Logo`, s])),
};

const svgFor = (size) => (size <= 48 ? SMALL : LARGE);

async function rasterize(sizes) {
	if (!existsSync(CHROME)) throw new Error(`Chrome не найден: ${CHROME} (задайте переменную CHROME)`);
	const port = 9300 + Math.floor(Math.random() * 400);
	const profile = mkdtempSync(join(tmpdir(), "leelauncher-icons-"));
	const chrome = spawn(
		CHROME,
		[
			"--headless=new",
			"--disable-gpu",
			"--hide-scrollbars",
			`--remote-debugging-port=${port}`,
			`--user-data-dir=${profile}`,
			"about:blank",
		],
		{ stdio: "ignore" },
	);
	const exited = new Promise((r) => chrome.once("exit", r));
	let ws;
	try {
		let target;
		for (let i = 0; i < 40 && !target; i++) {
			try {
				target = await (
					await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })
				).json();
			} catch {
				await new Promise((r) => setTimeout(r, 250));
			}
		}
		if (!target) throw new Error("Chrome не запустился");
		ws = new WebSocket(target.webSocketDebuggerUrl);
		await new Promise((r) => (ws.onopen = r));
		let seq = 0;
		const pending = new Map();
		ws.onmessage = (e) => {
			const m = JSON.parse(e.data);
			if (m.id && pending.has(m.id)) {
				pending.get(m.id)(m.result ?? { error: m.error });
				pending.delete(m.id);
			}
		};
		const send = (method, params = {}) =>
			new Promise((r) => {
				const id = ++seq;
				pending.set(id, r);
				ws.send(JSON.stringify({ id, method, params }));
			});
		await send("Page.enable");
		await send("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } });
		const out = new Map();
		for (const size of sizes) {
			await send("Emulation.setDeviceMetricsOverride", {
				width: Math.max(size, 64),
				height: Math.max(size, 64),
				deviceScaleFactor: 1,
				mobile: false,
			});
			const html = `<!doctype html><style>html,body{margin:0;background:transparent}img{display:block;width:${size}px;height:${size}px}</style><img src="data:image/svg+xml;base64,${Buffer.from(svgFor(size)).toString("base64")}">`;
			await send("Page.navigate", {
				url: "data:text/html;base64," + Buffer.from(html).toString("base64"),
			});
			await new Promise((r) => setTimeout(r, 300));
			const shot = await send("Page.captureScreenshot", {
				format: "png",
				clip: { x: 0, y: 0, width: size, height: size, scale: 1 },
			});
			out.set(size, Buffer.from(shot.data, "base64"));
		}
		return out;
	} finally {
		ws?.close();
		chrome.kill();
		await exited;
		try {
			rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
		} catch {
			// временный профиль удалит система
		}
	}
}

function ico(entries) {
	const head = Buffer.alloc(6 + 16 * entries.length);
	head.writeUInt16LE(0, 0);
	head.writeUInt16LE(1, 2);
	head.writeUInt16LE(entries.length, 4);
	let offset = head.length;
	entries.forEach(({ size, buf }, i) => {
		const o = 6 + i * 16;
		head.writeUInt8(size >= 256 ? 0 : size, o);
		head.writeUInt8(size >= 256 ? 0 : size, o + 1);
		head.writeUInt16LE(1, o + 4);
		head.writeUInt16LE(32, o + 6);
		head.writeUInt32LE(buf.length, o + 8);
		head.writeUInt32LE(offset, o + 12);
		offset += buf.length;
	});
	return Buffer.concat([head, ...entries.map((e) => e.buf)]);
}

// Свой писатель ICNS: каждый размер — готовый PNG, macOS ничего не пересчитывает.
function icns(png) {
	const chunks = ICNS_TYPES.map(([type, size]) => {
		const head = Buffer.alloc(8);
		head.write(type, 0, "ascii");
		head.writeUInt32BE(8 + png.get(size).length, 4);
		return Buffer.concat([head, png.get(size)]);
	});
	const body = Buffer.concat(chunks);
	const head = Buffer.alloc(8);
	head.write("icns", 0, "ascii");
	head.writeUInt32BE(8 + body.length, 4);
	return Buffer.concat([head, body]);
}

const png = await rasterize([...new Set([...SIZES, ...Object.values(TILES)])]);
const tmp = mkdtempSync(join(tmpdir(), "leelauncher-icons-out-"));
for (const [name, size] of Object.entries(COPIES)) writeFileSync(join(tmp, name), png.get(size));
for (const [name, size] of Object.entries(TILES)) writeFileSync(join(tmp, `${name}.png`), png.get(size));
writeFileSync(join(tmp, "icon.ico"), ico(ICO_SIZES.map((size) => ({ size, buf: png.get(size) }))));
writeFileSync(join(tmp, "icon.icns"), icns(png));
for (const f of readdirSync(ICONS)) unlinkSync(join(ICONS, f));
for (const f of readdirSync(tmp)) copyFileSync(join(tmp, f), join(ICONS, f));
rmSync(tmp, { recursive: true, force: true });
writeFileSync(join(ROOT, "public/favicon.svg"), SMALL);
console.log("icons updated:", ICONS);
