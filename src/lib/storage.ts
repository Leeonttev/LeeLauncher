const PREFIX = "leelauncher:";
const RENAMED_PREFIX = "salauncher:";
const RESTORED_KEY = `${PREFIX}storage-restored`;
const RENAMED_MARKER = `${RENAMED_PREFIX}storage-moved`;

/** Ключи от прежнего названия saLauncher. Обычно их уже перенёс `public/theme-init.js` (логика та же — менять вместе). */
function restoreKeys() {
	try {
		if (localStorage.getItem(RESTORED_KEY) !== null) return;
		// Сначала собираем имена: запись по ходу обхода сдвигает номера ключей.
		const keys = Array.from({ length: localStorage.length }, (_, i) => localStorage.key(i));
		const renamed = localStorage.getItem(RENAMED_MARKER) !== null;
		for (const key of keys) {
			if (key === null) continue;
			if (key.startsWith(RENAMED_PREFIX)) {
				if (key === RENAMED_MARKER) continue;
				const value = localStorage.getItem(key);
				if (value !== null) localStorage.setItem(PREFIX + key.slice(RENAMED_PREFIX.length), value);
			} else if (
				renamed &&
				key.startsWith(PREFIX) &&
				localStorage.getItem(RENAMED_PREFIX + key.slice(PREFIX.length)) === null
			) {
				localStorage.removeItem(key);
			}
		}
		localStorage.setItem(RESTORED_KEY, "1");
	} catch {
		// Отметка не поставлена — попробуем при следующем запуске.
	}
}

restoreKeys();

export function readStorage(key: string): string | null {
	try {
		return localStorage.getItem(PREFIX + key);
	} catch {
		return null;
	}
}

/** Хранилище недоступно — `false`: настройка проживёт до перезапуска. */
export function writeStorage(key: string, value: string | null): boolean {
	try {
		if (value === null) localStorage.removeItem(PREFIX + key);
		else localStorage.setItem(PREFIX + key, value);
		return true;
	} catch {
		return false;
	}
}
