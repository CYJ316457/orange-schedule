// M0：只负责把主窗口跑起来，验证 WebView2 下的渲染一致性。
// M1 起在这里注册插件、命令、托盘与调度器（见 docs/tauri-migration.md §6.1）。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running 橙子日程");
}
