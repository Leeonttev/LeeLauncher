(function () {
	"use strict";

	var REPO = "leeonttev/LeeLauncher";
	var LANG_KEY = "leelauncher:site-lang";

	var TEXT = {
		ru: {
			pageTitle: "LeeLauncher — лаунчер Minecraft",
			description:
				"Лаунчер Minecraft со сборками, модами из Modrinth и CurseForge и скинами в 3D. Бесплатно, с открытым кодом, без рекламы.",
			skip: "К содержимому",
			nav: "Разделы",
			navFeatures: "Возможности",
			navGallery: "Скриншоты",
			navInstall: "Установка",
			langGroup: "Язык",
			github: "Исходный код на GitHub",
			eyebrow: "Лаунчер Minecraft",
			title: "Свои сборки.<br />В пару щелчков.",
			lead: "Игра, загрузчики, моды и Java ставятся сами. Бесплатно, с открытым кодом, без рекламы.",
			download: "Скачать для Windows",
			seeShots: "Смотреть скриншоты",
			soon: "macOS и Linux — скоро",
			windows: "Windows 10 и 11",
			version: "Версия {v}",
			altHome: "Главная LeeLauncher: последняя сборка, недавние и версии Minecraft",
			featuresTitle: "Всё для игры. Ничего лишнего.",
			f1: "Сборки на любой вкус",
			f1Text: "Vanilla, Fabric, Forge, NeoForge и Quilt. Выбрали версию — игру, загрузчик и Java лаунчер скачает сам.",
			f2: "Моды из Modrinth и CurseForge",
			f2Text: "Моды, шейдеры, ресурспаки и модпаки в одном каталоге. Зависимости ставятся вместе с модом.",
			f3: "Всё о сборке на одной странице",
			f3Text: "Моды, миры, скриншоты, лог игры и настройки.",
			f4: "Скины и плащи в 3D",
			f4Text: "Примеряйте скины и выбирайте плащ для аккаунта Microsoft.",
			f5: "Аккаунты",
			f5Text: "Вход через Microsoft или просто по нику, быстрое переключение.",
			f6: "Без рекламы и телеметрии",
			f6Text: "Лаунчер ничего не отправляет о вас и не показывает баннеров.",
			galleryTitle: "Как это выглядит",
			cap1: "Каталог Modrinth и CurseForge",
			cap2: "Страница сборки",
			cap3: "Скины и плащи в 3D",
			cap4: "Светлая тема",
			installTitle: "Установка",
			installLead: "Займёт минуту. Права администратора не нужны.",
			s1: "Скачайте установщик",
			s1Text: "Кнопка на этой странице берёт последнюю версию прямо из релизов на GitHub.",
			s2: "Запустите его",
			s2Text: "Лаунчер ставится только для вашего пользователя. WebView2 установщик добавит сам, если его нет.",
			s3: "Войдите и играйте",
			s3Text: "Через Microsoft или по нику. Игру и Java лаунчер скачает сам.",
			noteTitle: "Синее окно «Windows защитила ваш компьютер»",
			noteText:
				"Так Windows встречает программы без платной цифровой подписи. Нажмите <b>«Подробнее»</b>, затем <b>«Выполнить в любом случае»</b>. Код лаунчера открыт, установщик собирается на GitHub прямо из репозитория.",
			ctaTitle: "Соберите первую сборку сегодня.",
			about: "Лаунчер Minecraft с открытым кодом.",
			project: "Проект",
			source: "Исходный код на GitHub",
			releases: "Релизы",
			license: "Лицензия GPL-3.0",
			help: "Помощь",
			bug: "Сообщить об ошибке",
			trademark:
				"Minecraft — торговая марка Mojang Synergies AB. LeeLauncher не связан с Mojang и Microsoft и не одобрен ими.",
		},
		en: {
			pageTitle: "LeeLauncher — a Minecraft launcher",
			description:
				"A Minecraft launcher with instances, mods from Modrinth and CurseForge, and 3D skins. Free, open source, no ads.",
			skip: "Skip to content",
			nav: "Sections",
			navFeatures: "Features",
			navGallery: "Screenshots",
			navInstall: "Install",
			langGroup: "Language",
			github: "Source code on GitHub",
			eyebrow: "Minecraft launcher",
			title: "Your own instances.<br />In a couple of clicks.",
			lead: "The game, loaders, mods and Java install themselves. Free, open source, no ads.",
			download: "Download for Windows",
			seeShots: "See screenshots",
			soon: "macOS and Linux — coming soon",
			windows: "Windows 10 and 11",
			version: "Version {v}",
			altHome: "LeeLauncher home screen: last instance, recent ones and Minecraft versions",
			featuresTitle: "Everything to play. Nothing extra.",
			f1: "Instances for every taste",
			f1Text: "Vanilla, Fabric, Forge, NeoForge and Quilt. Pick a version — the launcher downloads the game, the loader and Java.",
			f2: "Mods from Modrinth and CurseForge",
			f2Text: "Mods, shaders, resource packs and modpacks in one catalog. Dependencies come with the mod.",
			f3: "Everything about an instance on one page",
			f3Text: "Mods, worlds, screenshots, the game log and settings.",
			f4: "Skins and capes in 3D",
			f4Text: "Try on skins and pick a cape for your Microsoft account.",
			f5: "Accounts",
			f5Text: "Sign in with Microsoft or just with a nickname, switch in a click.",
			f6: "No ads, no telemetry",
			f6Text: "The launcher sends nothing about you and shows no banners.",
			galleryTitle: "How it looks",
			cap1: "Modrinth and CurseForge catalog",
			cap2: "Instance page",
			cap3: "Skins and capes in 3D",
			cap4: "Light theme",
			installTitle: "Install",
			installLead: "Takes a minute. No admin rights needed.",
			s1: "Download the installer",
			s1Text: "The button on this page takes the latest version straight from the GitHub releases.",
			s2: "Run it",
			s2Text: "The launcher installs for your user only. The installer adds WebView2 if it’s missing.",
			s3: "Sign in and play",
			s3Text: "With Microsoft or a nickname. The launcher downloads the game and Java.",
			noteTitle: "A blue “Windows protected your PC” window",
			noteText:
				"That’s how Windows greets apps without a paid code signature. Click <b>“More info”</b>, then <b>“Run anyway”</b>. The launcher is open source, and the installer is built on GitHub straight from the repository.",
			ctaTitle: "Build your first instance today.",
			about: "An open source Minecraft launcher.",
			project: "Project",
			source: "Source code on GitHub",
			releases: "Releases",
			license: "GPL-3.0 license",
			help: "Help",
			bug: "Report a bug",
			trademark:
				"Minecraft is a trademark of Mojang Synergies AB. LeeLauncher is not affiliated with or endorsed by Mojang or Microsoft.",
		},
	};

	var root = document.documentElement;
	var lang = root.lang === "en" ? "en" : "ru";
	var release = null;

	function t(key) {
		return TEXT[lang][key];
	}

	function formatSize(bytes) {
		var mb = bytes / (1024 * 1024);
		var number = new Intl.NumberFormat(lang, { maximumFractionDigits: 1 }).format(mb);
		return number + (lang === "ru" ? " МБ" : " MB");
	}

	function renderRelease() {
		var parts = [];
		if (release && release.version) parts.push(t("version").replace("{v}", release.version));
		if (release && release.size) parts.push(formatSize(release.size));
		document.querySelectorAll("[data-release]").forEach(function (el) {
			if (el.getAttribute("data-release") === "full") {
				el.textContent = parts.concat(t("windows")).join(" · ");
			} else {
				el.textContent = parts.length ? parts.join(" · ") : t("windows");
			}
		});
	}

	function apply() {
		root.lang = lang;
		document.title = t("pageTitle");
		document.querySelector('meta[name="description"]').setAttribute("content", t("description"));
		document.querySelectorAll("[data-i18n]").forEach(function (el) {
			el.textContent = t(el.getAttribute("data-i18n"));
		});
		document.querySelectorAll("[data-i18n-html]").forEach(function (el) {
			el.innerHTML = t(el.getAttribute("data-i18n-html"));
		});
		document.querySelectorAll("[data-i18n-label]").forEach(function (el) {
			el.setAttribute("aria-label", t(el.getAttribute("data-i18n-label")));
		});
		document.querySelectorAll("[data-i18n-alt]").forEach(function (el) {
			el.setAttribute("alt", t(el.getAttribute("data-i18n-alt")));
		});
		document.querySelectorAll("[data-lang]").forEach(function (el) {
			el.setAttribute("aria-pressed", String(el.getAttribute("data-lang") === lang));
		});
		renderRelease();
		root.classList.remove("i18n-pending");
	}

	document.querySelectorAll("[data-lang]").forEach(function (button) {
		button.addEventListener("click", function () {
			lang = button.getAttribute("data-lang");
			try {
				localStorage.setItem(LANG_KEY, lang);
			} catch (e) {
				// Без хранилища выбор проживёт до перезагрузки.
			}
			apply();
		});
	});

	apply();
	root.setAttribute("data-ready", "");

	// Установщик назывался по-разному: LeeLauncher-Setup.exe и LeeLauncher_X.Y.Z_x64-setup.exe.
	function findInstaller(assets) {
		var exact = null;
		var fallback = null;
		for (var i = 0; i < assets.length; i++) {
			var name = String(assets[i].name || "").toLowerCase();
			if (name === "leelauncher-setup.exe" || /^leelauncher-setup-[\w.]+\.exe$/.test(name))
				exact = exact || assets[i];
			else if (/-setup\.exe$/.test(name)) fallback = fallback || assets[i];
		}
		return exact || fallback;
	}

	if (window.fetch) {
		fetch("https://api.github.com/repos/" + REPO + "/releases/latest", {
			headers: { Accept: "application/vnd.github+json" },
		})
			.then(function (response) {
				if (!response.ok) throw new Error("HTTP " + response.status);
				return response.json();
			})
			.then(function (data) {
				var asset = findInstaller(data.assets || []);
				if (!asset || !asset.browser_download_url) return;
				release = {
					version: String(data.tag_name || "").replace(/^v/i, ""),
					size: asset.size || null,
				};
				document.querySelectorAll("[data-download]").forEach(function (link) {
					link.setAttribute("href", asset.browser_download_url);
				});
				renderRelease();
			})
			.catch(function () {
				// Ссылка на страницу релиза остаётся, подпись — «Windows 10 и 11».
			});
	}

	var items = document.querySelectorAll(".rv");
	if ("IntersectionObserver" in window) {
		var observer = new IntersectionObserver(
			function (entries) {
				entries.forEach(function (entry) {
					if (!entry.isIntersecting) return;
					entry.target.classList.add("in");
					observer.unobserve(entry.target);
				});
			},
			{ rootMargin: "0px 0px -8% 0px" },
		);
		items.forEach(function (item) {
			observer.observe(item);
		});
	} else {
		items.forEach(function (item) {
			item.classList.add("in");
		});
	}
})();
