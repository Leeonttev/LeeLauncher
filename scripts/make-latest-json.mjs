// latest.json для автообновления: node scripts/make-latest-json.mjs <версия> <установщик> <notes.md> <адрес установщика> <выход>
// Подпись берётся из `<установщик>.sig` (её делает `tauri signer sign --app-version`).
import { readFileSync, statSync, writeFileSync } from "node:fs";

const [version, setup, notesFile, url, out] = process.argv.slice(2);
if (!version || !setup || !notesFile || !url || !out) {
	console.error("node scripts/make-latest-json.mjs <версия> <установщик> <notes.md> <адрес> <выход>");
	process.exit(1);
}

const entry = {
	url,
	signature: readFileSync(`${setup}.sig`, "utf8").trim(),
	size: statSync(setup).size,
};
const manifest = {
	version,
	notes: readFileSync(notesFile, "utf8").trim(),
	pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
	// Updater сначала ищет запись с типом пакета, потом общую.
	platforms: {
		"windows-x86_64-nsis": entry,
		"windows-x86_64": entry,
	},
};
writeFileSync(out, JSON.stringify(manifest, null, "\t") + "\n");
console.log(out);
