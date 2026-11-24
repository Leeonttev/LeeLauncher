import { getVersion } from "@tauri-apps/api/app";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toAppError, type AppError } from "@/lib/errors";

export const appInfo = {
	version: (): Promise<string> => getVersion(),
};

export const appWindow = {
	show: (): Promise<void> => getCurrentWindow().show(),
	minimize: (): Promise<void> => getCurrentWindow().minimize(),
	toggleMaximize: (): Promise<void> => getCurrentWindow().toggleMaximize(),
	close: (): Promise<void> => getCurrentWindow().close(),
	isMaximized: (): Promise<boolean> => getCurrentWindow().isMaximized(),
	onResized: (handler: () => void): Promise<() => void> => getCurrentWindow().onResized(handler),
};

/** HTML5-перетаскивание в WebView на Windows перехватывает Tauri. */
export type FileDragEvent =
	| { type: "enter"; paths: string[] }
	| { type: "over" }
	| { type: "drop"; paths: string[] }
	| { type: "leave" };

export const fileDrop = {
	listen: (handler: (event: FileDragEvent) => void): Promise<() => void> =>
		getCurrentWebview().onDragDropEvent((event) => {
			const payload = event.payload;
			if (payload.type === "enter" || payload.type === "drop")
				handler({ type: payload.type, paths: payload.paths });
			else handler({ type: payload.type });
		}),
};

export const external = {
	open: (url: string): Promise<void> => openUrl(url),
};

/** Отклоняется всегда с `AppError`. */
async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
	try {
		return await invoke<T>(name, args);
	} catch (err) {
		throw toAppError(err);
	}
}

function subscribe<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
	return listen<T>(event, (e) => handler(e.payload));
}

/* Аккаунты */

export type AccountKind = "offline" | "microsoft";

export type Account = {
	/** Внутренний id записи (uuid v4), не uuid игрока. */
	id: string;
	kind: AccountKind;
	username: string;
	/** uuid игрока с дефисами. */
	uuid: string;
	/** ISO 8601. */
	addedAt: string;
	/** Только microsoft: обновить токен не удалось, нужен повторный вход. */
	needsRelogin: boolean;
};

export type AccountsState = {
	/** В порядке добавления. */
	accounts: Account[];
	activeId: string | null;
};

export const LOGIN_STEPS = ["waiting_browser", "xbox", "minecraft", "profile"] as const;
export type LoginStep = (typeof LOGIN_STEPS)[number];

export const accountsApi = {
	list: (): Promise<AccountsState> => command("accounts_list"),
	/** Новый аккаунт становится активным. */
	addOffline: (username: string): Promise<Account> => command("account_add_offline", { username }),
	/** Отмена — ошибка `cancelled`. */
	loginMicrosoft: (lang: "ru" | "en"): Promise<Account> => command("account_login_microsoft", { lang }),
	/** Закрывает окно входа; идущий `loginMicrosoft` завершится с `cancelled`. */
	cancelMicrosoftLogin: (): Promise<void> => command("account_login_microsoft_cancel"),
	setActive: (id: string): Promise<AccountsState> => command("account_set_active", { id }),
	/** Если удалён активный — активным становится первый оставшийся или `null`. */
	remove: (id: string): Promise<AccountsState> => command("account_remove", { id }),
	onChanged: (handler: (state: AccountsState) => void): Promise<() => void> =>
		subscribe("accounts-changed", handler),
	onLoginProgress: (handler: (step: LoginStep) => void): Promise<() => void> =>
		subscribe<{ step: LoginStep }>("account-login-progress", (payload) => handler(payload.step)),
};

/* Сборки, установка, Java, настройки, запуск */

export const LOADER_KINDS = ["vanilla", "fabric", "forge", "neoforge", "quilt"] as const;
export type LoaderKind = (typeof LOADER_KINDS)[number];
export type VersionKind = "release" | "snapshot" | "old_beta" | "old_alpha" | "other";

export type GameVersion = {
	id: string;
	kind: VersionKind;
	/** ISO 8601. */
	releaseTime: string;
	/** Клиент этой версии уже скачан в общую папку. */
	downloaded: boolean;
};

export type GameVersions = {
	latest: { release: string; snapshot: string };
	/** Новые первыми. */
	versions: GameVersion[];
	/** Сети нет, показан старый кэш (список может быть неполным). */
	offline: boolean;
};

export type Listed<T> = {
	items: T[];
	/** Сети нет, показан старый кэш. */
	offline: boolean;
};

export type LoaderVersion = {
	id: string;
	/** У Forge в метаданных все `false`. */
	stable: boolean;
	recommended: boolean;
};

/** Порядок важен: ядро считает обложку по умолчанию по индексу. */
export const INSTANCE_COVERS = [
	"meadow",
	"forest",
	"peaks",
	"ocean",
	"desert",
	"sakura",
	"village",
	"castle",
	"caves",
	"volcano",
	"cosmos",
] as const;
export type InstanceCover = (typeof INSTANCE_COVERS)[number];

export type Memory = { minMb: number; maxMb: number };
export type Resolution = { width: number; height: number };
export type GameWindow = Resolution | "fullscreen";
export type InstallState = "not_installed" | "installing" | "installed" | "failed";

export type Instance = {
	/** 12 символов a–z0–9, имя папки сборки. */
	id: string;
	name: string;
	gameVersion: string;
	/** `version: null` — рекомендуемая; после установки ядро запишет выбранную. */
	loader: { kind: LoaderKind; version: string | null };
	/** Обложка; ядро всегда отдаёт её заполненной (у старых сборок — по id). */
	cover: InstanceCover;
	/** Группа (один уровень), 1–32 символа; `null` — без группы. */
	group: string | null;
	/** ISO 8601. */
	createdAt: string;
	lastPlayedAt: string | null;
	/** Секунды. */
	playtime: number;
	/** `null` — из настроек лаунчера. */
	memory: Memory | null;
	/** `null` — авто. */
	javaPath: string | null;
	jvmArgs: string | null;
	/** `null` — размер окна по умолчанию. */
	resolution: GameWindow | null;
	installState: InstallState;
	installed?: { versionId: string; javaMajor: number };
	/** `null` — обычная сборка, не из модпака. */
	modpack: ModpackLink | null;
	/** Пока всегда `null`. */
	importedFrom: { launcher: string; path: string } | null;
	/** Версия игры с уязвимостью шрифтов (minecraft2shell). Старое ядро поле не присылает. */
	fontVulnerable?: boolean;
};

/** Все поля необязательны; `null` сбрасывает `memory`, `javaPath`, `jvmArgs`, `resolution`. */
export type InstancePatch = {
	name?: string;
	/** `null` — обложка по id. */
	cover?: InstanceCover | null;
	/** `null` — убрать из группы. Ядро обрезает пробелы и берёт написание существующей группы. */
	group?: string | null;
	gameVersion?: string;
	loader?: { kind: LoaderKind; version: string | null };
	memory?: Memory | null;
	javaPath?: string | null;
	jvmArgs?: string | null;
	resolution?: GameWindow | null;
};

export type InstallMode = "install" | "reinstall" | "repair";
export type InstallStage =
	| "queued"
	| "metadata"
	| "java"
	| "downloading"
	| "processors"
	| "finishing"
	/** Модпак: его моды и файлы индекса. */
	| "modpack_files"
	/** Модпак: распаковка `overrides`. */
	| "overrides"
	/** Обновление модпака с копией: копирование сборки. */
	| "copying";

export type InstallProgress = {
	instanceId: string;
	stage: InstallStage;
	percent: number;
	filesDone: number;
	filesTotal: number;
	bytesDone: number;
	bytesTotal: number;
};

export type InstallFinished = {
	instanceId: string;
	result: "installed" | "failed" | "cancelled";
	/** Только при `failed`. */
	error?: AppError;
};

export type JavaInstall = {
	/** Полный путь к javaw.exe. */
	path: string;
	version: string;
	major: number;
	arch: "x64" | "x86" | "arm64" | "arm32";
	vendor?: string;
	/** Скачана лаунчером. */
	managed: boolean;
	/** Только у скачанных лаунчером и в ответе `java_list`: новая версия от Mojang. */
	updateAvailable?: string;
};

export type JavaInstallProgress = { major: number; percent: number; bytesDone: number; bytesTotal: number };

export type DownloadSource = "auto" | "official" | "mirror";

export type Settings = {
	version: number;
	/** `null` — автоматически: `{ minMb: 512, maxMb: SystemMemory.recommendedMaxMb }`. */
	defaultMemory: Memory | null;
	/** `"17"` → путь к javaw.exe; нет записи — авто. */
	javaPaths: Record<string, string>;
	downloadSource: DownloadSource;
	/** Статус «играет» в Discord. */
	discordPresence: boolean;
	/** `false` — вместо названия сборки в статусе «Minecraft». */
	discordShowInstance: boolean;
	/** Защита от Java4Shell для Java 17+. */
	java4shellMitigation: boolean;
	/** Перед запуском уязвимой версии снимать автопринятие серверных ресурспаков. */
	protectServerPacks?: boolean;
};

export type SettingsPatch = {
	defaultMemory?: Memory | null;
	/** `null` убирает запись. */
	javaPaths?: Record<string, string | null>;
	downloadSource?: DownloadSource;
	discordPresence?: boolean;
	discordShowInstance?: boolean;
	java4shellMitigation?: boolean;
	protectServerPacks?: boolean;
};

export type SystemMemory = {
	/** 0 — не удалось определить. */
	totalMb: number;
	recommendedMaxMb: number;
};

export type ProcessState = "starting" | "running" | "exited";

/** Аккаунт, с которого идёт игра: снимок на момент запуска, удаление или смена активного его не меняют. */
export type GameAccountInfo = {
	/** `Account.id`. */
	id: string;
	uuid: string;
	username: string;
	kind: AccountKind;
};

export type InstanceProcessEvent = {
	instanceId: string;
	state: ProcessState;
	/** `running`. */
	pid?: number;
	/** Нет — аккаунт ещё выбирается (первое `starting`) или ядро старое. */
	account?: GameAccountInfo;
	/** `exited`: код выхода (нет — процесс не запускался). */
	exitCode?: number;
	/** `exited`: код не 0 и игру не останавливали из лаунчера. */
	crashed?: boolean;
	/** `exited`: остановлена командой `instance_kill`. */
	killed?: boolean;
	/** `exited`: сколько шла игра. */
	durationSecs?: number;
	/** `exited` без процесса: запуск не удался (та же ошибка, что вернула `instance_launch`). */
	error?: AppError;
};

export type RunningGame = {
	instanceId: string;
	state: "starting" | "running";
	pid?: number;
	/** Нет — аккаунт ещё выбирается. */
	account?: GameAccountInfo;
	/** ISO 8601. */
	startedAt: string;
	/** Запущена прошлым экземпляром лаунчера: лога нет. */
	recovered?: true;
};

export type LogLine = {
	/** Местное время «ЧЧ:ММ:СС». */
	time?: string;
	level?: "info" | "warn" | "error" | "debug";
	thread?: string;
	/** У записи с ошибкой — несколько строк (сообщение + стек). */
	text: string;
	stream: "stdout" | "stderr";
};

export const versionsApi = {
	minecraft: (kinds?: VersionKind[], refresh?: boolean): Promise<GameVersions> =>
		command("minecraft_versions", { kinds, refresh }),
	/** Версии игры, для которых есть загрузчик (для `vanilla` — все). */
	loaderGameVersions: (loader: LoaderKind): Promise<Listed<string>> =>
		command("loader_game_versions", { loader }),
	/** Новые первыми; пусто — не поддерживается (для `vanilla` всегда пусто). */
	loaderVersions: (loader: LoaderKind, gameVersion: string): Promise<Listed<LoaderVersion>> =>
		command("loader_versions", { loader, gameVersion }),
};

export const instancesApi = {
	/** По дате создания. */
	list: (): Promise<Instance[]> => command("instances_list"),
	/** Сборка создаётся неустановленной: установку запускает `install`. */
	create: (args: {
		name: string;
		gameVersion: string;
		loader: { kind: LoaderKind; version: string | null };
		cover?: InstanceCover;
		group?: string | null;
	}): Promise<Instance> => command("instance_create", args),
	update: (id: string, patch: InstancePatch): Promise<Instance> =>
		command("instance_update", { id, patch }),
	/** Папка сборки удаляется целиком, с мирами. */
	remove: (id: string): Promise<void> => command("instance_remove", { id }),
	duplicate: (id: string, name: string): Promise<Instance> => command("instance_duplicate", { id, name }),
	/** Совпала с другой группой — объединение. `storage` — часть сборок могла измениться: перечитайте список. */
	renameGroup: (from: string, to: string): Promise<Instance[]> =>
		command("instance_group_rename", { from, to }),
	removeGroup: (name: string): Promise<Instance[]> => command("instance_group_remove", { name }),
	/** Возвращается сразу, дальше — события установки. */
	install: (id: string, mode?: InstallMode): Promise<void> => command("instance_install", { id, mode }),
	cancelInstall: (id: string): Promise<void> => command("instance_install_cancel", { id }),
	/** `sub` — подпапка (создаётся), `select` — выделить файл. */
	openFolder: (id: string, options?: { sub?: InstanceSubfolder; select?: string }): Promise<void> =>
		command("instance_open_folder", { id, sub: options?.sub, select: options?.select }),
	/** Без `accountId` — активный аккаунт. `world` — сразу войти в мир (1.20+). */
	launch: (id: string, options?: { accountId?: string; world?: string }): Promise<void> =>
		command("instance_launch", { id, accountId: options?.accountId, world: options?.world }),
	kill: (id: string): Promise<void> => command("instance_kill", { id }),
	/** Буфер лога последнего запуска. */
	log: (id: string): Promise<LogLine[]> => command("instance_log", { id }),
	/** Игры, запущенные до перезагрузки окна. */
	running: (): Promise<RunningGame[]> => command("instances_running"),
	onChanged: (handler: (instances: Instance[]) => void): Promise<() => void> =>
		subscribe("instances-changed", handler),
	onInstallProgress: (handler: (progress: InstallProgress) => void): Promise<() => void> =>
		subscribe("instance-install-progress", handler),
	onInstallFinished: (handler: (finished: InstallFinished) => void): Promise<() => void> =>
		subscribe("instance-install-finished", handler),
	onProcess: (handler: (event: InstanceProcessEvent) => void): Promise<() => void> =>
		subscribe("instance-process", handler),
	onLog: (handler: (event: { instanceId: string; lines: LogLine[] }) => void): Promise<() => void> =>
		subscribe("instance-log", handler),
};

export const javaApi = {
	/** Найденные на компьютере и скачанные; новые версии первыми. */
	list: (): Promise<JavaInstall[]> => command("java_list"),
	/** `path` — javaw.exe, папка bin или папка Java. */
	check: (path: string): Promise<JavaInstall> => command("java_check", { path }),
	/** Системное окно выбора файла; выбранный файл сразу проверяется. `null` — окно закрыли без выбора. */
	pickFile: (lang?: "ru" | "en"): Promise<JavaInstall | null> => command("java_pick_file", { lang }),
	install: (major: number): Promise<JavaInstall> => command("java_install", { major }),
	/** Обновить скачанную лаунчером Java этой версии; `file_in_use` — ею пользуется игра. */
	update: (major: number): Promise<JavaInstall> => command("java_update", { major }),
	/** Отменяет идущий `install` этой версии (тот вернёт `cancelled`). */
	cancelInstall: (major: number): Promise<void> => command("java_install_cancel", { major }),
	onInstallProgress: (handler: (progress: JavaInstallProgress) => void): Promise<() => void> =>
		subscribe("java-install-progress", handler),
};

export type DownloadKind = "instance_install" | "java" | "content" | "modpack";
/** Отменённые задачи сразу убираются из списка. */
export type DownloadState = "queued" | "running" | "finished" | "failed";

export type Download = {
	id: string;
	kind: DownloadKind;
	/** `instance_install`, `content` и `modpack`. */
	instanceId?: string;
	/** У `modpack` — `install` или `update`. */
	mode?: InstallMode | "update";
	/** `modpack`: название модпака (имя сборки — по `instanceId`). */
	title?: string;
	/** `modpack`: иконка модпака (у файла — `null`). */
	iconUrl?: string | null;
	/** `failed` — часть файлов не скачалась (повтор докачает только их), `manual` — ждут ручной загрузки. */
	warnings?: { failed: { path: string; error: AppError }[]; manual?: ManualFile[] };
	content?: ContentDownload;
	javaMajor?: number;
	state: DownloadState;
	/** `instance_install` — этап; `java` и `content` — всегда `downloading`. */
	stage?: InstallStage;
	/** 0–100 за всю задачу; `null` — в очереди. */
	percent: number | null;
	/** Скачано из сети за всю задачу. */
	bytesDone: number;
	/** Сколько надо скачать, без уже скачанного; `null` — пока неизвестно. Может уменьшаться при проверке файлов. */
	bytesTotal: number | null;
	/** `content` и этап `modpack_files`/`copying`: файлы. */
	filesDone?: number;
	filesTotal?: number;
	/** Этап `processors`: шаги. */
	stepsDone?: number;
	stepsTotal?: number;
	/** Байт/с за последние 3 с; `null` — сейчас ничего не качается. */
	speed: number | null;
	etaSecs: number | null;
	/** Unix, мс. */
	queuedAt: number;
	startedAt?: number;
	finishedAt?: number;
	/** Только `failed`. */
	error?: AppError;
};

export const downloadsApi = {
	list: (): Promise<Download[]> => command("downloads_list"),
	/** Задача исчезает из списка, когда остановится. */
	cancel: (id: string): Promise<void> => command("download_cancel", { id }),
	/** Новая задача с теми же параметрами; старая строка убирается. */
	retry: (id: string): Promise<Download> => command("download_retry", { id }),
	dismiss: (id: string): Promise<void> => command("download_dismiss", { id }),
	clear: (): Promise<void> => command("downloads_clear"),
	onChanged: (handler: (downloads: Download[]) => void): Promise<() => void> =>
		subscribe("downloads-changed", handler),
	onProgress: (handler: (download: Download) => void): Promise<() => void> =>
		subscribe("download-progress", handler),
};

export type FolderKind = "data" | "logs" | "instances" | "java";

export type AppPaths = {
	data: string;
	logs: string;
	instances: string;
	java: string;
};

/** `conflicts` — есть и старая, и новая папка данных. */
export type MigrationStatus = {
	unfinished: boolean;
	conflicts: { old: string; new: string }[];
	dataDir: string;
	webviewDir: string;
};

export const appApi = {
	paths: (): Promise<AppPaths> => command("app_paths"),
	migrationStatus: (): Promise<MigrationStatus> => command("app_migration_status"),
	/** Язык, выбранный в установщике (`--lang`); `null` — не передан. */
	launchLang: (): Promise<"ru" | "en" | null> => command("app_launch_lang"),
	/** Папка в проводнике; нет — создаётся. */
	openFolder: (kind: FolderKind): Promise<void> => command("app_open_folder", { kind }),
	/** Язык текстов ядра вне окна: статус Discord, меню трея. */
	setLang: (lang: "ru" | "en"): Promise<void> => command("app_set_lang", { lang }),
	/** `first` — окно спрятано в трей впервые за запуск. Окно в этот момент уже скрыто. */
	onHiddenToTray: (handler: (payload: { first: boolean }) => void): Promise<() => void> =>
		subscribe("app-hidden-to-tray", handler),
	/** Закрыть все игры и выйти. */
	quit: (): Promise<void> => command("app_quit"),
	/** «Выйти» в трее, пока идут игры; окно уже показано. */
	onQuitRequested: (handler: (payload: { games: number }) => void): Promise<() => void> =>
		subscribe("app-quit-requested", handler),
	/** Ядро сняло автопринятие ресурспаков на `servers` серверах сборки `id` перед её запуском. */
	onServerPacksGuarded: (
		handler: (payload: { id: string; servers: number }) => void,
	): Promise<() => void> => subscribe("server-packs-guarded", handler),
};

/* Discord */

export type DiscordStatus = {
	/** В этой сборке лаунчера статус вообще есть. */
	configured: boolean;
	/** Сейчас есть связь с Discord. */
	connected: boolean;
};

export const discordApi = {
	status: (): Promise<DiscordStatus> => command("discord_status"),
};

/* Обновление лаунчера */

export type UpdateInfo = {
	version: string;
	currentVersion: string;
	/** RFC 3339. */
	date: string | null;
	/** Markdown; пустой — `null`. */
	notes: string | null;
	size: number | null;
	/** Пакет этой версии уже скачан и проверен. */
	downloaded: boolean;
};

export type UpdateProgress = {
	/** Байты в файле вместе с докачанными раньше. */
	downloaded: number;
	total: number | null;
	/** За последние 3 с; `null` — байтов нет 5 с или замер ещё не набрался. */
	speedBps: number | null;
	etaSecs: number | null;
	slow: boolean;
	/** Идёт повтор после обрыва. */
	reconnect: { attempt: number; of: number } | null;
};

export type LastUpdate = {
	updatedFrom: string;
	version: string;
	notes: string | null;
};

export const updatesApi = {
	/** `null` — обновлений нет. Не удалось проверить — `update_unavailable`. */
	check: (): Promise<UpdateInfo | null> => command("app_update_check"),
	download: (): Promise<void> => command("app_update_download"),
	/** При успехе процесс завершается: новую версию запускает установщик. */
	install: (): Promise<void> => command("app_update_install"),
	cancel: (): Promise<void> => command("app_update_cancel"),
	/** Не `null` весь первый запуск после смены версии. */
	lastUpdate: (): Promise<LastUpdate | null> => command("app_last_update"),
	restart: (): Promise<void> => command("app_restart"),
	onProgress: (handler: (progress: UpdateProgress) => void): Promise<() => void> =>
		subscribe("app-update-progress", handler),
};

export const settingsApi = {
	get: (): Promise<Settings> => command("settings_get"),
	update: (patch: SettingsPatch): Promise<Settings> => command("settings_update", { patch }),
	systemMemory: (): Promise<SystemMemory> => command("system_memory"),
};

/* Каталог, контент сборок, миры, скриншоты, логи */

export type ContentSource = "modrinth" | "curseforge";
export const CONTENT_TYPES = ["mod", "resourcepack", "shader", "datapack"] as const;
/** `world` — мир из каталога (есть не у всех источников, см. `SourceInfo.types`). */
export type ContentType = (typeof CONTENT_TYPES)[number] | "world";
/** В ответах каталога бывает и `modpack`. */
export type CatalogProjectType = ContentType | "modpack";
export type SearchSort = "relevance" | "downloads" | "follows" | "newest" | "updated" | "name";

/** Экраны каталога строятся по возможностям источника, а не по его имени. */
export type SourceInfo = {
	id: ContentSource;
	/** `false` — нет ключа API в этой сборке лаунчера. */
	enabled: boolean;
	types: CatalogProjectType[];
	sorts: SearchSort[];
	/** Есть ли у шейдеров фильтр загрузчика. */
	shaderLoaders: boolean;
	/** Сколько категорий можно отметить сразу; `null` — без предела. */
	maxCategories: number | null;
	hasFollows: boolean;
};

export type ManualFileState = "waiting" | "added" | "wrong" | "failed";

/** Автор запретил скачивать файл из лаунчеров. */
export type ManualFile = {
	source: ContentSource;
	projectId: string;
	fileId: string;
	title: string;
	iconUrl: string | null;
	/** Точное имя и размер нужного файла. */
	file: string;
	size: number;
	type: ContentType;
	world?: string;
	/** Страница файла на сайте: скачивание начинается сразу. */
	pageUrl: string;
	/** Ключ старого файла (обновление): в корзину после добавления. */
	replaces?: string;
	/** Служебные: ядро получает их обратно в `instance_manual_begin { files }`. */
	sha1: string;
	versionNumber: string;
	author: string | null;
	dependency: boolean;
	state: ManualFileState;
	/** `wrong`: имя скачанного файла. */
	got?: string;
	/** `wrong` — `fingerprint_mismatch`, `failed` — ошибка записи. */
	error?: AppError;
};

export type InstallResult = {
	/** Задача загрузок; `null` — скачивать нечего (всё «только с сайта» или нечего обновлять). */
	download: Download | null;
	/** «Только с сайта». */
	manual: ManualFile[];
	/** Только у `instance_content_update`: каталоги не ответили, их обновления не установлены. */
	failedSources?: FailedSource[];
};

export type ManualSession = {
	/** Папка «Загрузки», за которой следит ядро; `null` — не нашли. */
	path: string | null;
	watching: boolean;
	files: ManualFile[];
};

export type ManualAdded = {
	/** Все файлы сеанса (или сборки). */
	files: ManualFile[];
	/** Пути, которые не подошли ни к одному ждущему файлу. */
	rejected: string[];
};
export type Side = "required" | "optional" | "unsupported" | "unknown";

/** Режим «Все»: Modrinth и CurseForge одним списком. */
export type AllInfo = {
	/** `false` — включён только один каталог, «Все» не нужен. */
	enabled: boolean;
	sources: ContentSource[];
	types: CatalogProjectType[];
	sorts: SearchSort[];
	/** Категорий в «Все» нет. */
	categories: false;
};

export type PartialFailure = { source: ContentSource; error: AppError };

/** Каталог, который не ответил при проверке обновлений: его файлы в этот раз не проверены. */
export type FailedSource = {
	source: ContentSource;
	code: string;
	/** Столько его файлов нашлось и проверено (или поставлено) через другой каталог; `null` — ни одного. */
	coveredBy?: { source: ContentSource; files: number } | null;
};

export type ContentUpdates = { items: ContentItem[]; failedSources: FailedSource[] };

export type SearchParams = {
	/** По умолчанию `modrinth`; `all` — все каталоги одним списком. */
	source?: ContentSource | "all";
	/** Только `all`: курсор следующей порции из прошлого ответа (`offset` не читается). */
	cursor?: string;
	type: CatalogProjectType;
	query?: string;
	/** Любая из. */
	gameVersions?: string[];
	/** Любой из: fabric, forge, neoforge, quilt / iris, optifine, canvas. */
	loaders?: string[];
	/** Все сразу (slug). */
	categories?: string[];
	sort?: SearchSort;
	offset?: number;
	/** 1–100, по умолчанию 20. */
	limit?: number;
	/** Тогда у каждого результата есть `install`. */
	instanceId?: string;
};

export type ContentInstallState =
	| { kind: "none" }
	| { kind: "installed"; versionId: string; file: string; enabled: boolean }
	| { kind: "update"; versionId: string; latestVersionId: string; latestNumber: string }
	| { kind: "incompatible"; reason: "game_version" | "loader" | "needs_loader" };

export type ProjectHit = {
	source: ContentSource;
	id: string;
	slug: string;
	type: CatalogProjectType;
	title: string;
	/** Бывает пустым (каталог не дал). */
	author: string;
	description: string;
	iconUrl: string | null;
	downloads: number;
	/** `null` — источник не считает подписчиков. */
	follows: number | null;
	/** Автор разрешил скачивать только на сайте источника. */
	siteOnly: boolean;
	/** Страница проекта на сайте источника. */
	url: string;
	/** Без загрузчиков. */
	categories: string[];
	/** Unix мс, 0 — неизвестно. */
	updatedAt: number;
	gameVersions: string[];
	loaders: string[];
	clientSide: Side;
	serverSide: Side;
	/** Только при `instanceId` в запросе. */
	install?: ContentInstallState;
	/** Режим «Все»: тот же проект в других каталогах; иначе `[]`. */
	alsoOn: AlsoOn[];
};

export type AlsoOn = {
	source: ContentSource;
	id: string;
	slug: string;
	url: string;
	siteOnly: boolean;
	downloads: number;
	install?: ContentInstallState;
};

export type SearchResult = {
	hits: ProjectHit[];
	offset: number;
	limit: number;
	total: number;
	/** Сети нет, показан сохранённый ответ. */
	offline: boolean;
	/** Только `all`: следующая порция, `null` — дальше ничего. */
	cursor: string | null;
	/** Только `all`: каталоги, которые не ответили в этой порции. */
	partial: PartialFailure[];
};

export type VersionChannel = "release" | "beta" | "alpha";

export type ContentVersion = {
	id: string;
	projectId: string;
	name: string;
	number: string;
	channel: VersionChannel;
	gameVersions: string[];
	loaders: string[];
	/** Unix мс. */
	publishedAt: number;
	downloads: number;
	/** Только в `catalog_version`. */
	changelog?: string;
	files: { name: string; size: number; primary: boolean }[];
	dependencies: {
		projectId: string | null;
		versionId: string | null;
		kind: "required" | "optional" | "incompatible" | "embedded";
	}[];
	/** У файла нет адреса для лаунчера — только с сайта (`pageUrl`). */
	siteOnly: boolean;
	/** Страница файла на сайте (у Modrinth — `null`). */
	pageUrl: string | null;
	/** Где работает (если источник указал). */
	side: { client: Side; server: Side } | null;
	/** `catalog_versions` с `instanceId`: эта версия стоит в сборке. */
	installed?: boolean;
};

export type Project = ProjectHit & {
	/** Markdown как есть (бывает HTML) — санитизирует фронт. */
	body: string;
	gallery: {
		url: string;
		thumbUrl: string;
		title: string | null;
		description: string | null;
		featured: boolean;
	}[];
	links: {
		site: string;
		source?: string;
		wiki?: string;
		issues?: string;
		discord?: string;
		donate?: { platform: string; url: string }[];
	};
	license: { id: string; name: string; url: string | null } | null;
	versionsCount: number;
	team: { id: string; username: string; avatarUrl: string | null; role: string }[];
	/** Unix мс. */
	publishedAt: number;
	/** При `instanceId`: что поставится (без `changelog`). */
	/** `null` — не успели подобрать в срок. */
	bestVersion?: ContentVersion | null;
};

/** `icon` — SVG текстом (в интерфейсе не вставляем). */
export type CategoryTag = {
	slug: string;
	projectType: string;
	header: string;
	icon: string;
	iconUrl: string | null;
	/** Название от источника (у Modrinth = slug). */
	name: string;
	/** slug родителя (двухуровневые категории), иначе `null`. */
	parent: string | null;
};

/** Файл контента в командах: имя без `.disabled` + тип (+ мир для датапака в мире). */
export type FileRef = { file: string; type: ContentType; world?: string };

export type ContentItem = {
	/** Имя без `.disabled`. */
	file: string;
	type: ContentType;
	/** `false` — файл `<имя>.disabled`. */
	enabled: boolean;
	size: number;
	/** Датапак внутри `saves/<world>/datapacks`. */
	world?: string;
	project: {
		source: ContentSource;
		id: string;
		versionId: string;
		title: string;
		author: string;
		iconUrl: string | null;
		versionNumber: string;
		/** Страница на сайте (для «Открыть на сайте», когда источник выключен). */
		url: string | null;
	} | null;
	/**
	 * По последней проверке; `siteOnly` — новую версию скачать только на сайте;
	 * `via` — найдено в другом каталоге (свой не ответил) и ставится оттуда.
	 */
	update: { versionId: string; number: string; siteOnly: boolean; via?: ContentSource | null } | null;
	/** Файл из модпака сборки; `false` — добавил пользователь (у сборок без модпака всегда `false`). */
	fromModpack: boolean;
};

export type InstallRequest = {
	source?: ContentSource;
	/** id или slug. */
	projectId: string;
	/** Нет — лучшая подходящая: первая стабильная, иначе бета, иначе любая. */
	versionId?: string;
	/** Под каким типом ставить (вкладка каталога); нет — по проекту. */
	type?: ContentType;
	/** Только датапак: в `saves/<world>/datapacks`; нет — общая папка `datapacks/`. */
	world?: string;
	/** Необязательные зависимости (id проектов), которые тоже поставить. */
	optional?: string[];
};

export type InstalledElsewhere = {
	project: ProjectHit;
	/** `null` — добавлен файлом, каталог неизвестен. */
	installedFrom: ContentSource | null;
	file: string;
	optional: boolean;
};

export type ResolveView = {
	/** Что поставится (без `changelog`). */
	version: ContentVersion;
	project: ProjectHit;
	/** Поставятся вместе: обязательные зависимости, загрузчик шейдеров. */
	required: ProjectHit[];
	/** Можно выбрать (`optional` в `content_install`). */
	optional: ProjectHit[];
	/** Обязательные без подходящей версии. */
	missing: { projectId: string; title: string | null }[];
	/** Установленное несовместимое — при установке выключится. */
	conflicts: ContentItem[];
	/** Эта версия уже стоит. */
	alreadyInstalled: boolean;
	/** Зависимости, которые уже стоят из другого каталога или файлом: не ставятся. */
	installedElsewhere: InstalledElsewhere[];
	/** «Только с сайта»: обязательные, выбранные необязательные и сам проект, если он такой. */
	manual: ManualFile[];
};

export type AddFilesResult = {
	added: ContentItem[];
	/** `modpack` — `.mrpack`/zip Modrinth, `curseforge` — zip CurseForge: ставятся через `modpackApi.inspect`. */
	unsupported: { path: string; reason: "type" | "modpack" | "curseforge" }[];
	/** Тип не понятен — спросить и вызвать снова с `projectType`. */
	needsType: string[];
};

export type GameMode = "survival" | "creative" | "adventure" | "spectator";

export type World = {
	/** Папка в `saves/` — id мира. */
	folder: string;
	/** `LevelName`, иначе имя папки. */
	name: string;
	mode: GameMode;
	hardcore: boolean;
	cheats: boolean;
	/** Unix мс. */
	lastPlayedAt: number | null;
	/** Версия игры, в которой мир открывали. */
	version: string | null;
	/** `data:image/png;base64,…`. */
	iconUrl: string | null;
	/** Можно `instancesApi.launch(id, { world })` (версия сборки 1.20+). */
	quickPlay: boolean;
	/** `level.dat` не прочитался. */
	damaged: boolean;
	/** Мир поставлен из каталога. */
	source: { source: ContentSource; projectId: string } | null;
};

export type Screenshot = {
	file: string;
	/** Unix мс. */
	takenAt: number;
	width: number;
	height: number;
	size: number;
	/** Полный путь — для `<img>` через `assetUrl`. */
	path: string;
	/** Миниатюра 480px (JPEG); `null` — не удалось. */
	thumbPath: string | null;
};

export type LogFileKind = "latest" | "debug" | "launcher" | "archive" | "crash" | "other";
export type LogFile = { name: string; kind: LogFileKind; size: number; modified: number };
export type LogContent = { lines: LogLine[]; truncated: boolean };

export type InstanceSubfolder =
	| "mods"
	| "resourcepacks"
	| "shaderpacks"
	| "datapacks"
	| "saves"
	| `saves/${string}`
	| "screenshots"
	| "logs"
	| "crash-reports"
	| "config";

export type ContentDownload = {
	action: "install" | "update";
	source: ContentSource;
	/** У обновления нескольких файлов — `null`. */
	projectId: string | null;
	projectTitle: string | null;
	iconUrl: string | null;
	/** Сколько файлов кроме главного («Iris Shaders + 2»). */
	extraCount: number;
};

/** Asset protocol; папку разрешает ядро в `instance_screenshots`. */
export function assetUrl(path: string): string {
	return convertFileSrc(path);
}

export const catalogApi = {
	sources: (): Promise<SourceInfo[]> => command("catalog_sources"),
	allInfo: (): Promise<AllInfo> => command("catalog_all_info"),
	search: (params: SearchParams): Promise<SearchResult> => command("catalog_search", { params }),
	/** Категории этого типа (у датапаков — категории модов). */
	categories: (projectType: CatalogProjectType, source?: ContentSource): Promise<CategoryTag[]> =>
		command("catalog_categories", { source, projectType }),
	/** `instanceId` — тогда `install` и `bestVersion`; `projectType` — показать как тип вкладки. */
	project: (args: {
		id: string;
		source?: ContentSource;
		instanceId?: string;
		projectType?: CatalogProjectType;
	}): Promise<Project> => command("catalog_project", args),
	/** Новые первыми, без `changelog`; при `instanceId` — `installed`. */
	versions: (args: {
		projectId: string;
		source?: ContentSource;
		gameVersion?: string;
		loader?: string;
		channel?: VersionChannel;
		instanceId?: string;
	}): Promise<ContentVersion[]> => command("catalog_versions", args),
	/** С `changelog`. */
	version: (versionId: string, source?: ContentSource): Promise<ContentVersion> =>
		command("catalog_version", { source, versionId }),
	resolve: (instanceId: string, request: InstallRequest): Promise<ResolveView> =>
		command("catalog_resolve", { instanceId, request }),
	/** Скачивать нечего — `download: null`; файлы «только с сайта» открывать через `manualApi.begin`. */
	install: (instanceId: string, request: InstallRequest): Promise<InstallResult> =>
		command("content_install", { instanceId, request }),
};

export const contentApi = {
	/** Сразу, без сети; неопознанное ядро опознаёт в фоне и шлёт `instance-content-changed`. */
	list: (id: string): Promise<{ items: ContentItem[]; offline: boolean; failedSources: FailedSource[] }> =>
		command("instance_content", { id }),
	/** Проверка обновлений (без `force` — не чаще раза в 10 минут, иначе кэш). */
	updates: (id: string, force?: boolean): Promise<ContentUpdates> =>
		command("instance_content_updates", { id, force }),
	toggle: (id: string, item: FileRef, enabled: boolean): Promise<ContentItem> =>
		command("instance_content_toggle", { id, item, enabled }),
	/** В корзину; пропавшие пропускаются. */
	remove: (id: string, files: FileRef[]): Promise<void> =>
		command("instance_content_remove", { id, files }),
	/** Без `files` — всё с обновлением. Обновления «только с сайта» ядро сразу кладёт в ждущие сборки. */
	update: (id: string, files?: FileRef[]): Promise<InstallResult> =>
		command("instance_content_update", { id, files }),
	addFiles: (
		id: string,
		paths: string[],
		options?: { projectType?: ContentType; world?: string },
	): Promise<AddFilesResult> =>
		command("instance_content_add_files", {
			id,
			paths,
			projectType: options?.projectType,
			world: options?.world,
		}),
	/** Последние сыгранные первыми. */
	worlds: (id: string): Promise<World[]> => command("instance_worlds", { id }),
	removeWorld: (id: string, folder: string): Promise<void> =>
		command("instance_world_remove", { id, folder }),
	/** Новые первыми; `limit` по умолчанию 60 (до 200). Разрешает папку скриншотов для `assetUrl`. */
	screenshots: (
		id: string,
		offset?: number,
		limit?: number,
	): Promise<{ items: Screenshot[]; total: number }> =>
		command("instance_screenshots", { id, offset, limit }),
	/** Картинкой в буфер обмена. */
	copyScreenshot: (id: string, file: string): Promise<void> =>
		command("instance_screenshot_copy", { id, file }),
	removeScreenshot: (id: string, file: string): Promise<void> =>
		command("instance_screenshot_remove", { id, file }),
	/** Открыть `logs/latest.log` программой по умолчанию. */
	openLogFile: (id: string): Promise<void> => command("instance_log_file", { id }),
	logsList: (id: string): Promise<LogFile[]> => command("instance_logs_list", { id }),
	logsRead: (id: string, name: string): Promise<LogContent> => command("instance_logs_read", { id, name }),
	/** Следить за папкой игры сборки (одна за раз); `null` — перестать. */
	watch: (id: string | null): Promise<void> => command("instance_watch", { id }),
	onContentChanged: (handler: (id: string) => void): Promise<() => void> =>
		subscribe<{ id: string }>("instance-content-changed", (payload) => handler(payload.id)),
	onWorldsChanged: (handler: (id: string) => void): Promise<() => void> =>
		subscribe<{ id: string }>("instance-worlds-changed", (payload) => handler(payload.id)),
	onScreenshotsChanged: (handler: (id: string) => void): Promise<() => void> =>
		subscribe<{ id: string }>("instance-screenshots-changed", (payload) => handler(payload.id)),
};

/** Файлы «только с сайта» */
export const manualApi = {
	/** Ждущие файлы сборки (во время сеанса — файлы сеанса с состояниями). */
	list: (id: string): Promise<ManualFile[]> => command("instance_manual", { id }),
	/** Ядро следит за папкой «Загрузки» и сразу проверяет уже лежащие там файлы. */
	begin: (id: string, files?: ManualFile[]): Promise<ManualSession> =>
		command("instance_manual_begin", { id, files }),
	end: (id: string): Promise<void> => command("instance_manual_end", { id }),
	/** Прежние сеансы после перезагрузки окна некому закрыть. */
	endAll: (): Promise<void> => command("instance_manual_end_all"),
	/** Скачанные файлы (перетаскивание): сверка и добавление. */
	add: (id: string, paths: string[]): Promise<ManualAdded> => command("instance_manual_add", { id, paths }),
	/** «Указать файлы…»: системное окно выбора; `null` — окно закрыли. */
	pick: (id: string, lang: "ru" | "en"): Promise<ManualAdded | null> =>
		command("instance_manual_pick", { id, lang }),
	onChanged: (handler: (event: { id: string; files: ManualFile[] }) => void): Promise<() => void> =>
		subscribe("instance-manual-changed", handler),
};

/* Модпаки */

/** Откуда модпак сборки. */
export type ModpackOrigin = "modrinth" | "curseforge" | "file";

export type ModpackLink = {
	source: ModpackOrigin;
	/** `null` у `file`. */
	projectId: string | null;
	versionId: string | null;
	title: string;
	versionNumber: string;
	iconUrl: string | null;
	/** Unix мс: когда поставлена или обновлена эта версия. */
	installedAt: number;
};

export type ModpackOptionalFile = {
	/** Путь от папки игры («mods/xyz.jar») — его и отдавать в `optional`. */
	path: string;
	size: number;
	/** Имя проекта в каталоге (без сети — `null`). */
	title: string | null;
	iconUrl: string | null;
	/** `null` — не мод/ресурспак/шейдер/датапак (например, `config/…`). */
	type: ContentType | null;
};

export type ModpackPreview = {
	token: string;
	name: string;
	versionNumber: string;
	summary: string | null;
	gameVersion: string;
	loader: { kind: LoaderKind; version: string | null };
	/** Опознан в каталоге — будут обновления. */
	project: {
		source: ContentSource;
		projectId: string;
		versionId: string;
		title: string;
		iconUrl: string | null;
	} | null;
	/** Файлов в `mods/`, если отмечены все необязательные. */
	mods: number;
	/** Сумма размеров файлов модпака (все необязательные отмечены). */
	downloadBytes: number;
	javaMajor: number | null;
	optional: ModpackOptionalFile[];
	/** CurseForge: файлов «только с сайта» (после установки — ждущие сборки). */
	manualCount: number;
};

export type ModpackUpdateCheck = {
	/** Лучшая версия проекта (без `changelog`). */
	latest: ContentVersion | null;
	/** `latest` новее установленной. */
	available: boolean;
	currentVersionId: string | null;
	/** Unix мс. */
	checkedAt: number;
};

export type ModpackUpdatePreview = {
	added: string[];
	updated: string[];
	removed: string[];
	/** Контент не из модпака — останется. */
	userFiles: number;
	/** Файлы `overrides`, изменённые пользователем, — заменятся. */
	modifiedOverrides: string[];
	/** Размер папки сборки (для «Сначала сделать копию»). */
	backupBytes: number;
};

export type ModpackInspectProgress = { requestId: string; bytesDone: number; bytesTotal: number };

export const modpackApi = {
	/** Из каталога (`projectId`, без `versionId` — лучшая версия) или с диска (`path`). */
	inspect: (args: {
		requestId?: string;
		source?: ContentSource;
		projectId?: string;
		versionId?: string;
		path?: string;
	}): Promise<ModpackPreview> => command("modpack_inspect", args),
	/** Прервать `modpack_inspect` с этим `requestId`: его ответ — ошибка `cancelled` (не показывать). */
	inspectCancel: (requestId: string): Promise<void> => command("modpack_inspect_cancel", { requestId }),
	/** Сборка создаётся сразу (видна как `installing`). `optional` — отмеченные необязательные (нет — все). */
	install: (args: {
		token: string;
		name?: string;
		group?: string | null;
		cover?: InstanceCover;
		optional?: string[];
	}): Promise<{ instance: Instance; download: Download }> => command("modpack_install", args),
	/** Кэш 10 минут на сборку (`force` — мимо кэша); у сборки из файла — `modpack_not_linked`. */
	checkUpdate: (id: string, force?: boolean): Promise<ModpackUpdateCheck> =>
		command("modpack_check_update", { id, force }),
	updatePreview: (id: string, token: string, optional?: string[]): Promise<ModpackUpdatePreview> =>
		command("modpack_update_preview", { id, token, optional }),
	/** Задача `modpack` с `mode: "update"`. `lang` — название копии. */
	update: (args: {
		id: string;
		token: string;
		optional?: string[];
		backup?: boolean;
		lang?: "ru" | "en";
	}): Promise<Download> => command("modpack_update", args),
	unlink: (id: string): Promise<Instance> => command("modpack_unlink", { id }),
	/** Системное окно выбора `.mrpack`/`.zip`; `null` — закрыли. */
	pickFile: (lang?: "ru" | "en"): Promise<string | null> => command("modpack_pick_file", { lang }),
	onInspectProgress: (handler: (progress: ModpackInspectProgress) => void): Promise<() => void> =>
		subscribe("modpack-inspect-progress", handler),
};

/* Скины. Текстуры приходят как `data:`, скачивает их только ядро. */

export type SkinModel = "classic" | "slim";

export type SkinEntry = {
	id: string;
	name: string;
	/** Последний выбор рук для этого скина; меняет только `skin_update`. */
	model: SkinModel;
	/** Файл 64×32. */
	legacy: boolean;
	/** ISO 8601. */
	addedAt: string;
	/** `data:image/png;base64,…` */
	texture: string;
	/** Нет поля — `upload`. `account` — лаунчер сам сохранил скин, который видел на аккаунте. */
	origin?: SkinOrigin;
	/** На каких аккаунтах скин был. */
	seen?: SkinSeen[];
};

export type SkinOrigin = "upload" | "account";

export type SkinSeen = {
	accountId: string;
	/** ISO 8601. */
	firstSeenAt: string;
	/** ISO 8601. */
	lastSeenAt: string;
};

export type WornSkin = {
	key: string;
	/** Скин библиотеки; `null` — его там нет. */
	skinId: string | null;
	model: SkinModel;
	/** Обычный скин Minecraft: не сохраняется и не снимается. */
	isDefault: boolean;
	/** `data:…`; `null` — текстура не получена. */
	texture: string | null;
	/** ISO 8601. */
	fetchedAt: string;
};

export type Cape = {
	id: string;
	name: string;
	/** `data:…`; `null` — не получена. */
	texture: string | null;
};

export type SkinProfile = {
	accountId: string;
	skin: WornSkin | null;
	capes: Cape[];
	activeCapeId: string | null;
	/** Показан сохранённый: сервер не ответил или ответ не разобрался. */
	stale: boolean;
	/** Почему `stale`; `null`, если ответ на изменение не разобрался. */
	error: AppError | null;
};

export type SkinApplied = {
	profile: SkinProfile;
	/** Скин надет, плащ нет. */
	capeError: AppError | null;
	/** Страховка: прежний несохранённый скин положен в библиотеку. */
	savedPrevious: SkinEntry | null;
};

/** Что изменить. Поля нет — не меняется; `capeId: null` — скрыть плащ. Передавать только изменившееся. */
export type SkinChanges = {
	skinId?: string;
	model?: SkinModel;
	capeId?: string | null;
};

export type SkinRejected = {
	path: string;
	reason: "not_png" | "bad_size" | "too_large" | "unreadable";
	/** Только `bad_size`. */
	width?: number;
	height?: number;
};

export type SkinsAdded = {
	/** В порядке файлов; последний — сверху библиотеки. */
	added: SkinEntry[];
	/** Такие уже были, под прежним именем. */
	duplicates: SkinEntry[];
	rejected: SkinRejected[];
};

export type SkinPatch = { name?: string; model?: SkinModel };

export const skinsApi = {
	/** Новые сверху. */
	library: (): Promise<SkinEntry[]> => command("skins_library"),
	/** Сохранённые текущие скины, только с текстурой, без сети. */
	worn: (): Promise<Record<string, WornSkin>> => command("skins_worn"),
	profile: (accountId: string, refresh = false): Promise<SkinProfile> =>
		command("skin_profile", { accountId, refresh }),
	addFiles: (paths: string[]): Promise<SkinsAdded> => command("skin_add_files", { paths }),
	/** Системное окно выбора PNG; `null` — закрыли. */
	pickFiles: (lang: "ru" | "en"): Promise<SkinsAdded | null> => command("skin_pick_files", { lang }),
	update: (id: string, patch: SkinPatch): Promise<SkinEntry> => command("skin_update", { id, patch }),
	/** Файл — в корзину, стираются запись и ключи этого скина. На профили не влияет: надетый виден из кэша. */
	remove: (id: string): Promise<void> => command("skin_remove", { id }),
	reveal: (id: string): Promise<void> => command("skin_reveal", { id }),
	/** Ядро различает отсутствие `capeId` и `null` по телу запроса: `changes` — только изменившееся. */
	apply: (accountId: string, changes: SkinChanges, allowLosePrevious = false): Promise<SkinApplied> =>
		command("skin_apply", { accountId, ...changes, ...(allowLosePrevious && { allowLosePrevious }) }),
	/** Снять скин с аккаунта. Страховки нет: несохранённый скин пропадёт. */
	reset: (accountId: string): Promise<SkinProfile> => command("skin_reset", { accountId }),
	onChanged: (handler: () => void): Promise<() => void> =>
		subscribe<unknown>("skins-changed", () => handler()),
};
