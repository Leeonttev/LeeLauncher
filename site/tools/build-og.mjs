// Картинка для соцсетей 1200×630: node site/tools/build-og.mjs (нужен Chrome; путь — переменная CHROME).
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const chrome = process.env.CHROME ?? "C:/Program Files/Google/Chrome/Application/chrome.exe";
const out = path.join(here, "..", "public", "assets", "og.png");
const profile = mkdtempSync(path.join(tmpdir(), "og-"));

try {
	execFileSync(chrome, [
		"--headless=new",
		"--hide-scrollbars",
		"--force-device-scale-factor=1",
		"--allow-file-access-from-files",
		`--user-data-dir=${profile}`,
		"--window-size=1200,630",
		"--virtual-time-budget=3000",
		`--screenshot=${out}`,
		pathToFileURL(path.join(here, "og.html")).href,
	]);
} finally {
	rmSync(profile, { recursive: true, force: true });
}
console.log(out);
