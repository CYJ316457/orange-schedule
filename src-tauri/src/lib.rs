mod commands;
mod i18n;
mod i18n_map;
mod reminder;
mod scheduler;
mod series;
mod state;
mod store;
mod update;
mod windows;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    // single-instance 必须最先注册：第二实例直接聚焦主窗口（或打开便签）
    builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        if argv.iter().any(|a| a == "--widget") {
            let _ = windows::open_widget(app);
        } else {
            let _ = windows::show_main(app);
        }
    }));

    builder
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .setup(|app| {
            let handle = app.handle().clone();

            #[cfg(windows)]
            windows::set_app_user_model_id("local.weeklight.planner.tauri");

            store::init(&handle).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let (data, startup_error) = store::load();
            let widget_enabled = data.settings.widget.enabled;
            app.manage(state::AppState::new(data, startup_error.unwrap_or_default()));

            windows::create_main(&handle).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--widget") || widget_enabled {
                let _ = windows::open_widget(&handle);
            }

            scheduler::spawn(&handle);
            update::schedule(&handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get,
            commands::save_task,
            commands::preview_task_action,
            commands::apply_task_action,
            commands::toggle_task,
            commands::delete_task,
            commands::move_task,
            commands::reminder_action,
            commands::settings,
            commands::check_updates,
            commands::dismiss_update,
            commands::download_update,
            commands::open_update_page,
            commands::test_reminder,
            commands::next_reminder_message,
            commands::export_data,
            commands::import_data,
            commands::open_data,
            commands::show_main,
            commands::open_widget,
            commands::close_widget,
            commands::widget_settings,
            commands::edit_task,
            commands::quit,
        ])
        .run(tauri::generate_context!())
        .expect("error while running 橙子日程");
}
