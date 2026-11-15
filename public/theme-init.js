(function () {
	var root = document.documentElement;
	var theme = "system";
	var reduce = null;
	var language = null;

	function media(query) {
		try {
			return window.matchMedia(query).matches;
		} catch (e) {
			return false;
		}
	}

	// Ключи от прежнего названия saLauncher переносим до чтения темы, иначе тема мигнёт. Логика как в src/lib/storage.ts.
	try {
		if (localStorage.getItem("leelauncher:storage-restored") === null) {
			// Сначала собираем имена: запись по ходу обхода сдвигает номера ключей.
			var keys = [];
			for (var i = 0; i < localStorage.length; i++) keys.push(localStorage.key(i));
			var renamed = localStorage.getItem("salauncher:storage-moved") !== null;
			for (var j = 0; j < keys.length; j++) {
				var key = keys[j];
				if (key === null) continue;
				if (key.indexOf("salauncher:") === 0) {
					if (key === "salauncher:storage-moved") continue;
					var value = localStorage.getItem(key);
					if (value !== null)
						localStorage.setItem("leelauncher:" + key.slice("salauncher:".length), value);
				} else if (
					renamed &&
					key.indexOf("leelauncher:") === 0 &&
					localStorage.getItem("salauncher:" + key.slice("leelauncher:".length)) === null
				) {
					localStorage.removeItem(key);
				}
			}
			localStorage.setItem("leelauncher:storage-restored", "1");
		}
	} catch (e) {}

	try {
		var storedTheme = localStorage.getItem("leelauncher:theme");
		if (storedTheme === "light" || storedTheme === "dark" || storedTheme === "system")
			theme = storedTheme;
		var storedReduce = localStorage.getItem("leelauncher:reduce-motion");
		if (storedReduce === "true") reduce = true;
		else if (storedReduce === "false") reduce = false;
		var storedLanguage = localStorage.getItem("leelauncher:language");
		if (storedLanguage === "ru" || storedLanguage === "en") language = storedLanguage;
	} catch (e) {}

	// Как в src/lib/i18n.ts: первый язык системы без региона, ru/be/kk/ky → ru.
	if (language === null) {
		try {
			var tag = (navigator.languages && navigator.languages[0]) || navigator.language || "";
			var base = tag.toLowerCase().split("-")[0];
			language = ["ru", "be", "kk", "ky"].indexOf(base) !== -1 ? "ru" : "en";
		} catch (e) {
			language = "en";
		}
	}

	var dark = theme === "dark" || (theme === "system" && media("(prefers-color-scheme: dark)"));
	if (reduce === null) reduce = media("(prefers-reduced-motion: reduce)");

	root.classList.remove("light", "dark");
	root.classList.add(dark ? "dark" : "light");
	root.style.colorScheme = dark ? "dark" : "light";
	root.lang = language;
	if (reduce) root.setAttribute("data-reduce-motion", "");
	else root.removeAttribute("data-reduce-motion");
})();
