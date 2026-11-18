mod accounts;
mod app;
mod content;
mod download;
mod downloads;
mod error;
mod fs_util;
mod game;
mod gamedata;
mod http;
mod instances;
mod java;
mod lang;
mod launch;
mod launcher;
mod meta;
mod migrate;
mod modpacks;
mod paths;
mod presence;
mod secure;
mod settings;
mod skins;
mod sysmem;
mod trash;
mod tray;
mod updates;

use std::time::Duration;

use tauri::{Manager, WebviewWindowBuilder};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

pub(crate) const MAIN_WINDOW: &str = "main";

/// Окно создаётся скрытым, его показывает фронт; если он упал до рендера — показываем сами.
const SHOW_FALLBACK_DELAY: Duration = Duration::from_secs(3);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	let context = tauri::generate_context!();
	let args = launch::parse(std::env::args_os().map(|a| a.to_string_lossy().into_owned()));
	// Пробный запуск из установщика: ни данных, ни плагинов.
	if args.self_test {
		launch::self_test(context);
		return;
	}
	launch::set_lang(args.lang);
	// Первым делом: плагины ниже берут пути отсюда, а плагин логов сразу создаёт папку `logs`.
	let migration = paths::init();
	let mut builder = tauri::Builder::default();

	#[cfg(desktop)]
	{
		use tauri_plugin_window_state::StateFlags;

		// single-instance должен регистрироваться первым.
		builder = builder
			.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
				// Окно могло быть спрятано в трей во время игры.
				tray::show(app);
			}))
			.plugin(
				tauri_plugin_window_state::Builder::new()
					.with_filename(
						paths::data_dir()
							.join("window-state.json")
							.to_string_lossy(),
					)
					// Видимость не восстанавливаем, иначе плагин покажет окно до отрисовки интерфейса.
					.with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
					.build(),
			)
			// У окна нет прав `updater:*`, обновлением занимается только ядро.
			.plugin(tauri_plugin_updater::Builder::new().build());
	}

	builder
		.plugin(log_plugin())
		.plugin(tauri_plugin_opener::init())
		// Плагин нужен только ради Rust API: прав `dialog:*` у окна нет.
		.plugin(tauri_plugin_dialog::init())
		// Крестик, Alt+F4 и закрытие из панели задач: пока идёт игра, окно уходит в трей.
		.on_window_event(|window, event| {
			if let tauri::WindowEvent::CloseRequested { api, .. } = event
				&& tray::on_close_requested(window)
			{
				api.prevent_close();
			}
		})
		.invoke_handler(tauri::generate_handler![
			accounts::commands::accounts_list,
			accounts::commands::account_add_offline,
			accounts::commands::account_login_microsoft,
			accounts::commands::account_login_microsoft_cancel,
			accounts::commands::account_set_active,
			accounts::commands::account_remove,
			instances::commands::minecraft_versions,
			instances::commands::loader_game_versions,
			instances::commands::loader_versions,
			instances::commands::instances_list,
			instances::commands::instance_create,
			instances::commands::instance_update,
			instances::commands::instance_remove,
			instances::commands::instance_duplicate,
			instances::commands::instance_group_rename,
			instances::commands::instance_group_remove,
			instances::commands::instance_install,
			instances::commands::instance_install_cancel,
			instances::commands::instance_open_folder,
			instances::commands::java_list,
			instances::commands::java_check,
			instances::commands::java_pick_file,
			instances::commands::java_install,
			instances::commands::java_install_cancel,
			instances::commands::java_update,
			instances::commands::settings_get,
			instances::commands::settings_update,
			instances::commands::system_memory,
			instances::commands::downloads_list,
			instances::commands::download_cancel,
			instances::commands::download_retry,
			instances::commands::downloads_clear,
			instances::commands::download_dismiss,
			game::commands::instance_launch,
			game::commands::instance_kill,
			game::commands::instance_log,
			game::commands::instances_running,
			app::app_paths,
			app::app_open_folder,
			app::app_migration_status,
			content::commands::catalog_search,
			content::commands::catalog_categories,
			content::commands::catalog_project,
			content::commands::catalog_versions,
			content::commands::catalog_version,
			content::commands::catalog_resolve,
			content::commands::content_install,
			content::commands::catalog_sources,
			content::commands::catalog_all_info,
			content::manual::instance_manual,
			content::manual::instance_manual_begin,
			content::manual::instance_manual_end,
			content::manual::instance_manual_add,
			content::manual::instance_manual_pick,
			content::manual::instance_manual_end_all,
			content::commands::instance_content,
			content::commands::instance_content_updates,
			content::commands::instance_content_toggle,
			content::commands::instance_content_remove,
			content::commands::instance_content_update,
			content::commands::instance_content_add_files,
			gamedata::commands::instance_worlds,
			gamedata::commands::instance_world_remove,
			gamedata::commands::instance_screenshots,
			gamedata::commands::instance_screenshot_copy,
			gamedata::commands::instance_screenshot_remove,
			gamedata::commands::instance_log_file,
			gamedata::commands::instance_logs_list,
			gamedata::commands::instance_logs_read,
			gamedata::commands::instance_watch,
			modpacks::commands::modpack_inspect,
			modpacks::commands::modpack_inspect_cancel,
			modpacks::commands::modpack_install,
			modpacks::commands::modpack_check_update,
			modpacks::commands::modpack_update_preview,
			modpacks::commands::modpack_update,
			modpacks::commands::modpack_unlink,
			modpacks::commands::modpack_pick_file,
			modpacks::commands::import_scan,
			modpacks::commands::import_pick_folder,
			modpacks::commands::import_run,
			modpacks::commands::instance_export_preview,
			modpacks::commands::instance_export_pick_path,
			modpacks::commands::instance_export,
			skins::commands::skins_library,
			skins::commands::skins_worn,
			skins::commands::skin_add_files,
			skins::commands::skin_pick_files,
			skins::commands::skin_update,
			skins::commands::skin_remove,
			skins::commands::skin_reveal,
			skins::commands::skin_apply,
			skins::commands::skin_profile,
			skins::commands::skin_save_current,
			skins::commands::skin_reset,
			updates::app_update_check,
			updates::app_update_download,
			updates::app_update_install,
			updates::app_update_cancel,
			updates::app_last_update,
			updates::app_restart,
			launch::app_launch_lang,
			app::app_set_lang,
			presence::discord_status,
			tray::app_quit,
		])
		.setup(move |app| {
			let data_dir = paths::data_dir();
			std::fs::create_dir_all(&data_dir)?;
			log::info!("LeeLauncher {} запущен", env!("CARGO_PKG_VERSION"));
			migration.write_log();
			app.manage(app::MigrationStatus::new(&migration));

			let http = http::build_client()?;
			app.manage(accounts::Accounts::load(&data_dir, http));
			let layout = paths::Layout::new(data_dir.clone());
			updates::manage(app, &layout);
			app.manage(skins::Skins::load(layout.skins_dir())?);
			let launcher = launcher::Launcher::new(layout)?;
			let handle = app.handle().clone();
			tauri::async_runtime::spawn(launcher.downloads.clone().run_emitter(move |item| {
				use tauri::Emitter;
				let (event, sent) = match item {
					downloads::Emit::List(list) => (
						downloads::EVENT_CHANGED,
						handle.emit(downloads::EVENT_CHANGED, list),
					),
					downloads::Emit::Progress(task) => (
						downloads::EVENT_PROGRESS,
						handle.emit(downloads::EVENT_PROGRESS, task),
					),
				};
				if let Err(err) = sent {
					log::warn!("не удалось отправить {event}: {err}");
				}
			}));
			// До окна: интерфейс сразу видит игры, которые пережили прошлый запуск лаунчера.
			tauri::async_runtime::block_on(game::commands::recover(
				app.handle().clone(),
				launcher.clone(),
			));
			app.manage(presence::Presence::start(launcher.clone()));
			app.manage(tray::Tray::default());
			app.manage(launcher);

			let config = app
				.config()
				.app
				.windows
				.iter()
				.find(|w| w.label == MAIN_WINDOW)
				.expect("в tauri.conf.json нет окна main")
				.clone();
			let window = WebviewWindowBuilder::from_config(app.handle(), &config)?
				.data_directory(paths::webview_data_dir())
				.build()?;

			std::thread::spawn(move || {
				std::thread::sleep(SHOW_FALLBACK_DELAY);
				if !window.is_visible().unwrap_or(true) {
					let _ = window.show();
				}
			});
			Ok(())
		})
		.run(context)
		.expect("не удалось запустить LeeLauncher");
}

/// Логи: `<данные>/logs/leelauncher.log` (5 файлов по 5 МБ) и консоль.
fn log_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
	let level = if cfg!(debug_assertions) {
		log::LevelFilter::Debug
	} else {
		log::LevelFilter::Info
	};
	tauri_plugin_log::Builder::new()
		.clear_targets()
		.target(Target::new(TargetKind::Stdout))
		.target(Target::new(TargetKind::Folder {
			path: paths::data_dir().join("logs"),
			file_name: Some("leelauncher".into()),
		}))
		.level(log::LevelFilter::Warn)
		.level_for("leelauncher_lib", level)
		.max_file_size(5 * 1024 * 1024)
		.rotation_strategy(RotationStrategy::KeepSome(5))
		.timezone_strategy(TimezoneStrategy::UseLocal)
		.build()
}
