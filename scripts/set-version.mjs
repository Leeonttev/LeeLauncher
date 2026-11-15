// `node scripts/set-version.mjs 0.2.0`. Источник версии — src-tauri/Cargo.toml; package.json и Cargo.lock подтягиваются следом.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
	console.error("Укажите версию вида 1.2.3");
	process.exit(1);
}

const root = fileURLToPath(new URL("..", import.meta.url));
const edit = (file, pattern) => {
	const path = root + file;
	const text = readFileSync(path, "utf8");
	if (!pattern.test(text)) {
		console.error(`${file}: версия не найдена`);
		process.exit(1);
	}
	writeFileSync(path, text.replace(pattern, `$1${version}"`));
};

edit("src-tauri/Cargo.toml", /(\[package\]\r?\nname = "leelauncher"\r?\nversion = ")[^"]+"/);
edit("src-tauri/Cargo.lock", /(\[\[package\]\]\r?\nname = "leelauncher"\r?\nversion = ")[^"]+"/);
edit("package.json", /("version": ")[^"]+"/);
console.log(`Версия ${version}. Дальше: проверки, затем тег v${version}`);
