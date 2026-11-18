# Что нового

Раздел `## X.Y.Z` попадает в релиз и на экран обновления в лаунчере. Внутри раздела — блоки `<!-- ru -->` и `<!-- en -->`.

## 0.1.2

<!-- ru -->

- Защита от уязвимости Java4Shell (CVE-2026-114938): для Java 17 и новее лаунчер сам добавляет аргументы, которые отключают уязвимое место. Выключается в настройках
- Java, скачанная лаунчером, теперь обновляется до новой версии
- Новая Java на компьютере видна в списке сразу, без перезапуска лаунчера
- Выход из трея при запущенной игре спрашивает подтверждение и закрывает игру. Если лаунчер закрылся сам, после запуска он снова видит свою игру
- Если Modrinth или CurseForge не открывается, каталог показывает, что загрузилось, и честно пишет, какой сайт недоступен, вместо бесконечной загрузки
- Обновления модов проверяются по каждому сайту отдельно, а при недоступном сайте — через второй
- Библиотека, уже установленная с одного сайта, больше не предлагается к скачиванию с другого
- На пустой странице «Сборки» появилась кнопка «Импорт»

<!-- en -->

- Protection against the Java4Shell vulnerability (CVE-2026-114938): for Java 17 and newer the launcher adds arguments that disable the vulnerable code. Can be turned off in settings
- Java downloaded by the launcher is now updated to newer versions
- Newly installed Java shows up in the list right away, without restarting the launcher
- Quitting from the tray while a game is running asks for confirmation and closes the game. If the launcher closed on its own, it finds its running game again after restart
- If Modrinth or CurseForge can't be reached, the catalog shows what did load and says which site is unavailable instead of loading forever
- Mod updates are checked per site, and through the other site when one is unavailable
- A library already installed from one site is no longer offered again from the other
- The empty Instances page now has an Import button

## 0.1.1

<!-- ru -->

- Если запущено несколько игр, у каждой видно, с какого ника она идёт: на карточке, на Главной, на странице сборки, в логе и в уведомлениях
- В меню аккаунтов отмечено, какой аккаунт сейчас в игре
- «Играть за…» в меню сборки — запуск с другого аккаунта без переключения
- Предупреждение, если запускаете вторую игру с аккаунта, который уже играет (выключается в настройках)
- Кнопка «Сайт» в статусе Discord ведёт на новый сайт launcher.leeonttev.pro

<!-- en -->

- When several games are running, each one shows which account it's playing as: on the card, on Home, on the instance page, in the log and in notifications
- The account menu marks which accounts are in a game
- "Play as…" in the instance menu starts a game with another account without switching
- A warning when you start a second game with an account that's already playing (can be turned off in settings)
- The "Website" button in the Discord status now opens the new site launcher.leeonttev.pro

## 0.1.0

<!-- ru -->

Первая версия LeeLauncher.

- Вход по нику и через Microsoft, несколько аккаунтов и быстрое переключение
- Сборки на Vanilla, Fabric, Forge, NeoForge и Quilt: установка в один щелчок, Java скачивается сама
- Каталог модов, шейдеров, ресурспаков, датапаков и модпаков из Modrinth и CurseForge, зависимости ставятся вместе с модом
- Страница сборки: моды, миры, скриншоты, лог игры и настройки
- Скины в 3D и выбор плаща для аккаунта Microsoft
- Тёмная и светлая темы, русский и английский языки

<!-- en -->

The first version of LeeLauncher.

- Sign in with a nickname or a Microsoft account, keep several accounts and switch between them quickly
- Instances on Vanilla, Fabric, Forge, NeoForge and Quilt: one-click install, Java is downloaded for you
- Browse mods, shaders, resource packs, data packs and modpacks from Modrinth and CurseForge, dependencies install along with the mod
- Instance page: mods, worlds, screenshots, game log and settings
- 3D skins and cape selection for Microsoft accounts
- Dark and light themes, Russian and English
