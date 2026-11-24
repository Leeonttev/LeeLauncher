// latest.json для автообновления:
// node scripts/make-latest-json.mjs <версия> <notes.md> <выход> <ключи> <пакет> <адрес> [<ключи> <пакет> <адрес> …]
// <ключи> — платформы через запятую (`windows-x86_64-nsis,windows-x86_64`), одна запись на все.
// Подпись берётся из `<пакет>.sig` (её делает `tauri signer sign --app-version` или `tauri build`).
import { readFileSync, statSync, writeFileSync } from "node:fs";

const [version, notesFile, out, ...rest] = process.argv.slice(2);
if (!version || !notesFile || !out || rest.length === 0 || rest.length % 3 !== 0) {
	console.error(
		"node scripts/make-latest-json.mjs <версия> <notes.md> <выход> <ключи> <пакет> <адрес> [...]",
	);
	process.exit(1);
}

const platforms = {};
for (let i = 0; i < rest.length; i += 3) {
	const [keys, file, url] = rest.slice(i, i + 3);
	const entry = {
		url,
		signature: readFileSync(`${file}.sig`, "utf8").trim(),
		size: statSync(file).size,
	};
	for (const key of keys.split(",").filter(Boolean)) {
		if (platforms[key]) {
			console.error(`Платформа ${key} указана дважды`);
			process.exit(1);
		}
		platforms[key] = entry;
	}
}

const manifest = {
	version,
	notes: readFileSync(notesFile, "utf8").trim(),
	pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
	// Updater сначала ищет запись с типом пакета (`-nsis`, `-app`), потом общую.
	platforms,
};
writeFileSync(out, JSON.stringify(manifest, null, "\t") + "\n");
console.log(out);
