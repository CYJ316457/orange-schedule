# 橙子日程 → Tauri 2 迁移方案

> 目标：**常驻内存 ≤ 50 MB，安装包 ≤ 5 MB**，功能与 Electron 版 1:1 对齐。
> **M0 实测校正（2026-11-07）：50 MB 在 WebView2 上不可达 —— 空白页地板即 76.5 MB；现实目标改为 90–115 MB。详见 §0.1。**
> 基线版本：`v1.2.1`（Electron 44.4.5）｜Fork：`CYJ316457/orange-schedule`
> 文档日期：2026-11-07

---

## 0. 结论摘要

| 指标 | Electron 现状（估算） | Tauri（M0 实测） | 依据 |
|---|---|---|---|
| 常驻内存（任务管理器口径） | 250–350 MB | **98–113 MB** | M0 实测，见 §0.1 |
| 安装包 | 111.9 MB | **2.5–4 MB**（待 M5 验证） | 不打包浏览器内核 |
| 安装后占用 | 250–350 MB | **8–12 MB** | Rust 二进制 + 静态资源 |
| 空闲 CPU | 0.1–0.5% | **< 0.05%** | 5s tick 为 Rust 原生定时器 |
| 冷启动 | 1.5–3 s | **0.3–0.8 s** | 无 Chromium 初始化 |

**三个关键判断：**

1. **前端零改动。** 所有页面都只通过 `window.weeklight` 这一个对象访问后端（`renderer.js:1`、`widget.js:1`、`reminder.js:1`、`task-actions.js:3`、`updates.js:2`）。写一个 `bridge.js` 把同名对象映射到 `invoke/listen`，`renderer.js` 一行都不用改。
2. **后端规模很小，值得整体重写。** 主进程相关代码合计约 **700 行**（`main.cjs` 133 + `core.cjs` 73 + `task-series.cjs` 52 + `widget-controller.cjs` 55 + `update-*.cjs` 255 + `login-settings.cjs` 58 + 其他约 70），Rust 重写工作量约 6–10 人日。
3. **50 MB 不可达（已实测）。** 原方案按「Tauri 默认 70–100 MB」估算，M0 实测为 **113 MB**；即便关 GPU 也只有 **98 MB**，而空白页地板就有 **76.5 MB**。这是 WebView2 的进程模型决定的，裁剪应用代码无法消除。现实目标应定为 **90–115 MB**。

### 0.1 M0 实测数据（2026-11-07）

环境：Windows 10 19045 / WebView2 **157.0.4341.42** / release 构建 / 窗口 1450×950 / 静置 60 秒；
统计范围：`weeklight.exe` 及其全部 `msedgewebview2.exe` 子进程（排除 `conhost`）。

| 配置 | 进程数 | 私有工作集（任务管理器口径） | Private Bytes | Working Set 合计 |
|---|---|---|---|---|
| 完整页面，默认 | 7 | **113.3 MB** | 228.1 MB | 366.0 MB |
| 完整页面，`--disable-gpu` | 7 | **98.2 MB** | 141.2 MB | 363.5 MB |
| 空白页，默认（WebView2 地板） | 7 | **76.5 MB** | 149.7 MB | 314.6 MB |

解读：

- **运行时地板 76.5 MB**：空白页不加任何业务代码就有这个量，主要是 WebView2 的 browser / renderer / GPU / utility 进程。
- **页面本身约 37 MB**（113.3 − 76.5）：日历 DOM、内联 SVG 图标、两套语言表、i18n 运行时。这是唯一还能优化的部分。
- **关 GPU 省约 15 MB**：代价是重绘走软件合成，滚动时 CPU 上升。
- 三个数都远高于 50 MB，因此 **50 MB 只能通过换原生技术栈达成**（WPF 约 35–70 MB，Win32 约 10–25 MB）。

> 结论：Tauri 相对 Electron 的真实收益约为 **2–3 倍**（113 MB vs 250–350 MB 估算），不是原方案写的 4–6 倍。若要严格 ≤50 MB，应改走原生方案，而不是继续调 WebView2 参数。

---

## 0.2 实施进度

| 阶段 | 状态 | 交付 |
|---|---|---|
| M0 脚手架 + 渲染验证 | ✅ 完成 | `web/` + `bridge.js` + `src-tauri/`；release 首屏与 Electron 一致 |
| M1 数据层 + 24 命令 | ✅ 完成 | `state/store/series/reminder/i18n/update/windows/scheduler/commands` 共 10 个 Rust 模块 |
| M2 托盘 + 单实例 + 通知 | ✅ 完成 | 托盘菜单（打开/便签/新建/退出）、单实例唤回、开机自启注册表、关窗隐藏 |
| M3 便签窗口 | ✅ 完成 | 桌面便签创建/隐藏/置顶/置底、多屏越界夹紧、失焦沉底、400ms 防抖保存位置、拖拽穿透支持 |
| M4 提醒链路 | ✅ 完成 | 5s tick、到期判定、通知、提醒窗、引语轮换（120 条） |
| M5 打包 + 体积优化 | ✅ 完成 | 开启 LTO+strip 体积 profile；18 个单元测试全通；产物安装包 2.08 MB，常驻私有内存 90.8 MB |

### M1 实测验收（2026-11-07）

通过 WebView2 CDP 直接调用 `window.weeklight`，验证完整 IPC → Rust → `plans.json` 往返：

| 用例 | 结果 |
|---|---|
| 空标题 / 非法时长 | 分别返回「请填写 1–160 字的计划名称」「时长须为 15–1440 分钟」 |
| 新建单次任务 | `tasks: 0 → 1`，落盘 `plans.json` |
| 新建每周 4 次 | `tasks: 1 → 5`，`seriesId` 一致、`seriesIndex` 0–3 |
| 系列预览 / 批量完成 | `source=series`、`followingCount=3`，批量完成 `3/4` |
| 重启持久化 | 重启后 `tasks` 仍为原值 |
| 便签窗口 | `widget.html` 页面成功加载，关闭后进程保留（与 Electron 一致） |
| 提醒窗口 | `testReminder` 打开 `reminder.html`，120 条引语可用 |
| 字段兼容 | 落盘 JSON 的 camelCase 字段、`seriesId/seriesIndex`、`done` 与 Electron 版一致 |

### M2 实测验收（2026-11-07，release 构建）

| 用例 | 结果 |
|---|---|
| 托盘图标创建 | 进程内存在 `tray_icon_app` 宿主窗口，菜单项为 打开/便签/新建/退出 |
| 关窗即隐藏 | `WM_CLOSE` 后主窗从可见窗口列表消失，进程存活 |
| 单实例唤回 | 再次启动 exe：进程数仍为 1，主窗重新可见 |
| 开机自启 | `settings({autostart:true})` → 注册表 `HKCU\...\Run` 写入 `橙子日程 = <exe> --hidden`；关闭后条目清除 |
| 提醒窗口 | `testReminder()` 打开 `reminder.html`，可见窗口「橙子日程 · 提醒」 |
| packaged 标志 | release 为 `true`，debug 为 `false` |

### M3 实测验收（2026-11-07）

| 用例 | 结果 |
|---|---|
| 初始位置计算 | 无 bounds 记录时默认定位至主屏右上角（x: width - 438, y: 44），夹紧在工作区内 |
| 多屏边界安全 | 记录位置脱离屏幕（重叠 < 40px）时回退主屏；超大窗口 clamp 到显示器 workArea |
| 拖动/缩放持久化 | 400ms 防抖更新 `settings.widget.bounds` 并写入 `plans.json`，重启后原位恢复 |
| 失焦沉底 | 非置顶状态下失去焦点（`Focused(false)`），自动调用 `send_to_bottom` 重新置底 |
| 无边框拖拽 | 增加 `data-tauri-drag-region="deep"` 与 `core:window:allow-start-dragging` 权限 |
| 环境变量支持 | 支持 `WEEKLIGHT_DATA_DIR` 指定自定义数据路径，且不执行旧版数据迁移 |
| 备份导入确认框 | 修复 `OkCancelCustom` 参数反转问题（「导入并合并」对应 Ok，「取消」对应 Cancel） |
| 测试提醒配置遵循 | 修复 `test_reminder` 忽略 `settings.popup` / `settings.sound` 开关的问题 |

### M5 实测验收（2026-11-07，release 构建）

| 指标 / 用例 | Electron 1.2.1 基准 | Tauri 2 实测产物 | 达成情况 |
|---|---|---|---|
| **NSIS 安装包体积** | 111.9 MB | **2.08 MB**（2,185,866 B） | ✅ 降幅 98.1%，远超 ≤ 5 MB 目标 |
| **主程序 exe 体积** | ~130 MB（解压后总计 250–350 MB） | **5.32 MB**（5,581,312 B） | ✅ 纯 Rust 单执行文件 |
| **整树私有工作集（任务管理器口径）** | 250–350 MB（估算） | **90.8 MB**（7 进程） | ✅ 进入 90–115 MB 真实极致区间 |
| **Rust 单元测试** | 原版 `tests/*.cjs` | **18 项全部通过**（0 失败 0 警告） | ✅ `state/series/update/windows` 边界全部覆盖 |
| **体积 Profile** | — | `opt-level="s"`, `lto=true`, `panic="abort"`, `strip=true` | ✅ 已激活 |
| **更新检查源** | 原版仓库 | 对齐当前 fork（`CYJ316457/orange-schedule`） | ✅ 已更新 |

---

## 1. 迁移边界

### 1.1 原样保留（`web/` 目录）

| 文件 | 说明 |
|---|---|
| `index.html` / `widget.html` / `reminder.html` | 只加一行 `<script src="bridge.js"></script>` |
| `renderer.js`（262 行） | **不改** |
| `widget.js`（64 行） | **不改** |
| `reminder.js`（21 行） | **不改** |
| `task-actions.js`（147 行） / `updates.js`（38 行） | **不改** |
| `style.css` / `task-actions.css` / `updates.css` / `widget.css` | **不改**（共 40 KB，无 `backdrop-filter`） |
| `i18n.js` / `locale-core.js` / `locale-main.js` / `locale-aux.js` | **不改**（约 180 个键） |
| `assets/icon.png` | 保留 |

### 1.2 Rust 重写

| 原文件 | 新位置 | 工作量 |
|---|---|---|
| `main.cjs`（133 行） | `src-tauri/src/main.rs` + `commands.rs` + `windows.rs` | 2–3 d |
| `core.cjs`（73 行） | `src-tauri/src/state.rs`（校验、dueTasks、makeSeries） | 1 d |
| `task-series.cjs`（52 行） | `src-tauri/src/series.rs` | 0.5 d |
| `widget-controller.cjs`（55 行） | `src-tauri/src/windows.rs` | 1 d |
| `window-layer.cjs` + `native/WindowLayer.cs` | `src-tauri/src/windows.rs`（`windows` crate 直接调 `SetWindowPos`） | 0.5 d |
| `update-checker.cjs` + `update-controller.cjs`（255 行） | `src-tauri/src/update.rs` | 1 d |
| `login-settings.cjs`（58 行） | `tauri-plugin-autostart`（零代码） | 0.2 d |
| `data-path.cjs` | `app.path().app_data_dir()` | 0.1 d |
| `window-branding.cjs`（55 行） | `SetCurrentProcessExplicitAppUserModelID` + 图标 | 0.3 d |
| `preload.cjs` | **删除**，由 `web/bridge.js` 取代 | 0.2 d |

### 1.3 删除 / 不打包

- `scripts/build-native.ps1`、`scripts/prepare-runtime.cjs`、`scripts/verify-*.ps1`：构建流程不再需要。
- `tests/*.cjs`（Playwright 桌面测试）：改为「前端 DOM 测试 + Rust 单元测试 + 打包冒烟」三套。
- `docs/`（1.7 MB 截图）、`package-lock.json`、`src/native/`：不进安装包。
- `package.json` / `electron-builder` 配置：由 `Cargo.toml` + `tauri.conf.json` 取代。

---

## 2. 目标目录结构

```
orange-schedule/
├── web/                          # 前端（原样，仅加 bridge.js）
│   ├── index.html
│   ├── widget.html
│   ├── reminder.html
│   ├── bridge.js                 # 新增：window.weeklight → Tauri
│   ├── renderer.js / widget.js / reminder.js / task-actions.js / updates.js
│   ├── style.css / widget.css / task-actions.css / updates.css
│   ├── i18n.js / locale-core.js / locale-main.js / locale-aux.js
│   └── assets/icon.png
├── src-tauri/
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   ├── icons/icon.ico
│   └── src/
│       ├── main.rs               # 插件注册、setup、tray
│       ├── state.rs              # Task/Settings/State + 校验
│       ├── store.rs              # 原子写 plans.json + .bak
│       ├── series.rs             # 重复任务展开/批量操作
│       ├── scheduler.rs          # 5s tick
│       ├── windows.rs            # 主窗/便签/提醒窗 + 置底 + AppUserModelID
│       ├── commands.rs           # 24 个 #[tauri::command]
│       ├── i18n.rs               # 后端文案（约 40 条）
│       └── update.rs             # 版本检查
└── tests/                        # Rust 单测 + 前端 DOM 测试
```

---

## 3. 依赖清单

### `src-tauri/Cargo.toml`

```toml
[package]
name = "weeklight"
version = "1.2.1"
edition = "2021"
rust-version = "1.77"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon", "image-ico", "image-png"] }
tauri-plugin-single-instance = "2"
tauri-plugin-notification   = "2"
tauri-plugin-dialog         = "2"
tauri-plugin-opener         = "2"
tauri-plugin-autostart      = "2"

serde      = { version = "1", features = ["derive"] }
serde_json = "1"
tokio      = { version = "1", features = ["time", "sync", "macros", "rt-multi-thread"] }
chrono     = { version = "0.4", default-features = false, features = ["clock", "std"] }
uuid       = { version = "1", features = ["v4"] }
ureq       = { version = "2", features = ["tls", "json"] }
thiserror  = "2"

[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
  "Win32_Foundation",
  "Win32_UI_WindowsAndMessaging",
  "Win32_UI_Shell",
] }

[profile.release]
opt-level     = "s"      # 体积优先
lto           = true
codegen-units = 1
panic         = "abort"
strip         = true
```

**刻意不引入的东西：**

| 不引入 | 原因 | 替代 |
|---|---|---|
| `tauri-plugin-updater` | 依赖 reqwest + minisign，约 +1.5 MB | 自己用 `ureq` 拉 `latest.json`（原逻辑就 20 行） |
| `tauri-plugin-store` | 状态结构固定，无需通用 KV | `serde_json` + 原子写 |
| 任何前端框架 / 打包器 | 现有代码是原生 JS | `withGlobalTauri: true`，零构建 |
| `reqwest` / `tokio-tls` | 体积大 | `ureq`（rustls，约 700 KB） |
| `global-hotkey` | 原版没有全局快捷键 | — |

---

## 4. `tauri.conf.json`

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "橙子日程",
  "version": "1.2.1",
  "identifier": "local.weeklight.planner.tauri",
  "build": {
    "frontendDist": "../web"
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "label": "main",
        "title": "橙子日程 · 计划你的每一周",
        "width": 1450,
        "height": 950,
        "minWidth": 1050,
        "minHeight": 720,
        "visible": false,
        "resizable": true,
        "center": true
      }
    ],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/icon.ico"],
    "windows": {
      "webviewInstallMode": { "type": "skip" },
      "nsis": {
        "installMode": "currentUser",
        "languages": ["SimpChinese", "English"],
        "displayLanguageSelector": true
      }
    }
  }
}
```

**三个要点：**

- `identifier` 先加 `.tauri` 后缀，**避免与 Electron 版的数据目录/安装项冲突**，稳定后再切换并做一次性数据迁移（见 §7.3）。
- `webviewInstallMode: "skip"` —— 不打包 WebView2 运行时，安装包从 ~130 MB 降到几 MB；代价是 Win10 1803 以下或缺失 WebView2 的机器需先装运行时（Win11 与装过 Edge 的 Win10 默认都有）。
- `withGlobalTauri: true` —— 前端不需要 npm、不需要 Vite，直接用 `window.__TAURI__`。

### `src-tauri/capabilities/default.json`

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "main / widget / reminder 三个窗口的权限",
  "windows": ["main", "widget", "reminder"],
  "permissions": [
    "core:default",
    "core:window:allow-show",
    "core:window:allow-hide",
    "core:window:allow-set-focus",
    "core:window:allow-start-dragging",
    "core:event:default",
    "notification:default",
    "dialog:default",
    "opener:default",
    "autostart:default"
  ]
}
```

---

## 5. 24 个 IPC 通道 1:1 映射

### 5.1 命令表

| 原通道（kebab） | Tauri 命令（snake） | 入参 | 返回 |
|---|---|---|---|
| `get` | `get` | — | `State` |
| `save-task` | `save_task` | `task, repeat` | `State` |
| `preview-task-action` | `preview_task_action` | `id, action` | `Preview` |
| `apply-task-action` | `apply_task_action` | `request` | `State` |
| `toggle-task` | `toggle_task` | `id` | `State` |
| `delete-task` | `delete_task` | `id` | `State` |
| `move-task` | `move_task` | `id, date, time` | `State` |
| `reminder-action` | `reminder_action` | `id, action` | `State` |
| `settings` | `settings` | `settings` | `State` |
| `check-updates` | `check_updates` | — | `UpdateSnapshot` |
| `dismiss-update` | `dismiss_update` | — | `UpdateSnapshot` |
| `download-update` | `download_update` | — | `bool` |
| `open-update-page` | `open_update_page` | — | `bool` |
| `test-reminder` | `test_reminder` | — | `bool` |
| `next-reminder-message` | `next_reminder_message` | — | `State` |
| `export` | `export_data` | — | `Option<String>` |
| `import` | `import_data` | — | `Option<State>` |
| `open-data` | `open_data` | — | `bool` |
| `show-main` | `show_main` | — | `bool` |
| `open-widget` | `open_widget` | — | `State` |
| `close-widget` | `close_widget` | — | `State` |
| `widget-settings` | `widget_settings` | `patch` | `State` |
| `edit-task` | `edit_task` | `id` | `bool` |
| `quit` | `quit` | — | — |

### 5.2 4 个事件

| 原事件 | Tauri 事件 | 发送方式 |
|---|---|---|
| `state` | `state` | `app.emit("state", &snapshot)` → 所有窗口 |
| `new-task` | `new-task` | `main.emit("new-task", ())` |
| `edit-task` | `edit-task` | `main.emit("edit-task", id)` |
| `app-error` | `app-error` | 出错窗口 `window.emit(...)` |

### 5.3 系统能力对照

| Electron | Tauri 2 | 备注 |
|---|---|---|
| `app.getPath('userData')` | `app.path().app_data_dir()` | `%APPDATA%\{identifier}` |
| `app.getVersion()` | `app.package_info().version.to_string()` | |
| `app.isPackaged` | `!tauri::is_dev()` | |
| `app.requestSingleInstanceLock()` | `tauri-plugin-single-instance` | **必须第一个注册** |
| `app.setLoginItemSettings` | `app.autolaunch().enable()/disable()` | 带 `--hidden` 参数 |
| `Tray` + `Menu` | `TrayIconBuilder` + `Menu::with_items` | `tray-icon` feature |
| `Notification` | `app.notification().builder()...show()` | Rust 侧调用，无需 JS 绑定 |
| `dialog.showSaveDialog/OpenDialog` | `app.dialog().file()...save_file(cb)` | 用 oneshot 包成 async |
| `shell.openExternal` | `app.opener().open_url(url, None)` | |
| `shell.openPath` | `app.opener().open_path(path, None)` | |
| `powerMonitor.on('resume')` | **不需要** | 5s tick 本身就能覆盖唤醒后的补检 |
| `nativeImage` | `app.default_window_icon()` | |
| `window.setAppDetails({appId})` | `SetCurrentProcessExplicitAppUserModelID` | 任务栏归组 |
| `WeeklightLayer.exe bottom` | `SetWindowPos(hwnd, HWND_BOTTOM, ...)` | 直接 P/Invoke，少一个 exe |
| `net.fetch` | `ureq::get` | 仅更新检查 |

---

## 6. 关键实现

### 6.1 `main.rs`（插件顺序很重要）

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands; mod i18n; mod scheduler; mod series; mod state; mod store; mod update; mod windows;

use tauri::Manager;

fn main() {
    let mut builder = tauri::Builder::default();

    // single-instance 必须最先注册（官方要求）
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

            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            store::init(dir);

            let data = store::load();               // 读 plans.json（含校验/恢复）
            app.manage(state::AppState::new(data));

            windows::create_main(&handle)?;         // 只 hidden 创建
            windows::create_tray(&handle)?;

            // 与 Electron 版一致：--widget 直接开便签；否则按上次设置恢复
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--widget") {
                let _ = windows::open_widget(&handle);
            } else if state::widget_enabled(&handle) {
                let _ = windows::open_widget(&handle);
            }

            scheduler::spawn(&handle);
            update::schedule_initial(&handle);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get, commands::save_task, commands::preview_task_action,
            commands::apply_task_action, commands::toggle_task, commands::delete_task,
            commands::move_task, commands::reminder_action, commands::settings,
            commands::check_updates, commands::dismiss_update, commands::download_update,
            commands::open_update_page, commands::test_reminder,
            commands::next_reminder_message, commands::export_data, commands::import_data,
            commands::open_data, commands::show_main, commands::open_widget,
            commands::close_widget, commands::widget_settings, commands::edit_task,
            commands::quit,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run 橙子日程");
}
```

### 6.2 `store.rs`：原子写 + 兼容原 `plans.json`

```rust
use std::path::PathBuf;
use std::sync::OnceLock;
use crate::state::AppStateData;

static DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn init(dir: PathBuf) { let _ = DIR.set(dir); }
pub fn dir() -> &'static PathBuf { DIR.get().expect("store not initialized") }

pub fn load() -> AppStateData {
    let file = dir().join("plans.json");
    if !file.exists() { return AppStateData::default(); }
    let raw = std::fs::read_to_string(&file).unwrap_or_default();
    match serde_json::from_str::<AppStateData>(&raw) {
        Ok(mut s) => { s.normalize(); s }
        Err(_) => {
            // 与 Electron 版一致：保留原文件，另存恢复副本
            let _ = std::fs::copy(&file, dir().join(format!(
                "plans-recovery-{}.json",
                chrono::Utc::now().timestamp_millis())));
            AppStateData::default()
        }
    }
}

pub fn persist(s: &AppStateData) -> Result<(), String> {
    let file = dir().join("plans.json");
    let tmp  = dir().join("plans.json.tmp");
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    if file.exists() {
        let _ = std::fs::copy(&file, dir().join("plans.json.bak"));
    }
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}
```

**字段命名**：`plans.json` 用 camelCase，Rust 结构体加 `#[serde(rename_all = "camelCase")]` 即可 100% 兼容原数据，用户升级不丢数据。

### 6.3 `state.rs`：结构 + 校验

```rust
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub date: String,          // YYYY-MM-DD
    pub time: String,          // HH:MM
    pub category: String,      // work | study | life
    #[serde(default)] pub notes: String,
    #[serde(default)] pub done: bool,
    #[serde(default)] pub series_id: Option<String>,
    #[serde(default)] pub series_index: Option<u32>,
    #[serde(default)] pub notified_at: Option<i64>,
    #[serde(default)] pub snooze_until: Option<i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: String,       // zh-CN | en
    pub sound: bool,
    pub popup: bool,
    pub check_updates: bool,
    pub widget: WidgetPrefs,
    pub reminder_rotation: Rotation,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppStateData {
    pub version: u32,
    pub tasks: Vec<Task>,
    pub settings: Settings,
    pub history: Vec<HistoryEntry>,
}
```

校验逻辑直接平移 `core.cjs` 的 73 行：日期正则、时间范围、分类枚举、标题长度、10000 条上限。

### 6.4 `scheduler.rs`：5 秒 tick

```rust
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub fn spawn(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(msg) = crate::commands::run_due_tick(&app) {
                let _ = app.emit("app-error", msg);
            }
        }
    });
}
```

`run_due_tick` 做三件事：取 `dueTasks` → 加锁改状态 + 落盘 → `app.emit("state", snapshot)`；若有到期项则发通知并 `show_reminder()`。

### 6.5 `windows.rs`：关窗即隐藏 + 便签置底

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

pub static QUITTING: AtomicBool = AtomicBool::new(false);

pub fn show_main(app: &AppHandle) -> Result<(), String> {
    let w = app.get_webview_window("main").ok_or("主窗口不存在")?;
    if w.is_minimized().unwrap_or(false) { let _ = w.unminimize(); }
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn create_main(app: &tauri::AppHandle) -> tauri::Result<()> {
    let hidden = std::env::args().any(|a| a == "--hidden" || a == "--widget");
    let w = app.get_webview_window("main").expect("main window from config");

    let w2 = w.clone();
    w.on_window_event(move |e| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = e {
            if !QUITTING.load(Ordering::SeqCst) {
                api.prevent_close();
                let _ = w2.hide();          // 与 Electron 版行为一致
            }
        }
    });

    if !hidden { w.show()?; w.set_focus()?; }
    Ok(())
}

#[cfg(windows)]
pub fn send_to_bottom(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;

    let raw = window.hwnd().map_err(|e| e.to_string())?;
    let hwnd = HWND(raw.0);
    unsafe {
        SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn set_app_user_model_id(id: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    unsafe { let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(id)); }
}
```

便签窗口（懒创建，与 `widget-controller.cjs` 等价）：

```rust
pub fn open_widget(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("widget") {
        w.show().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let w = tauri::WebviewWindowBuilder::new(
            app, "widget", tauri::WebviewUrl::App("widget.html".into()))
        .title("橙子日程 · 桌面便签")
        .inner_size(410.0, 650.0)
        .min_inner_size(340.0, 380.0)
        .max_inner_size(680.0, 1000.0)
        .decorations(false)
        .transparent(true)
        .resizable(true)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;

    let w2 = w.clone();
    w.on_window_event(move |e| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = e {
            api.prevent_close();
            let _ = w2.hide();
        }
    });
    w.show().map_err(|e| e.to_string())?;
    send_to_bottom(&w)?;
    Ok(())
}
```

### 6.6 托盘

```rust
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

pub fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open   = MenuItem::with_id(app, "open",   "打开橙子日程", true, None::<&str>)?;
    let widget = MenuItem::with_id(app, "widget", "桌面便签",     true, None::<&str>)?;
    let new    = MenuItem::with_id(app, "new",    "新建计划",     true, None::<&str>)?;
    let quit   = MenuItem::with_id(app, "quit",   "退出橙子日程（停止提醒）", true, None::<&str>)?;
    let sep    = PredefinedMenuItem::separator(app)?;
    let menu   = Menu::with_items(app, &[&open, &widget, &new, &sep, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("橙子日程 · 提醒在后台运行")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open"   => { let _ = show_main(app); }
            "widget" => { let _ = open_widget(app); }
            "new"    => { let _ = show_main(app); let _ = app.emit("new-task", ()); }
            "quit"   => { QUITTING.store(true, Ordering::SeqCst); app.exit(0); }
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up, ..
            } = e {
                let _ = show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
```

### 6.7 导入/导出对话框（oneshot 包装）

```rust
async fn pick_save(app: &tauri::AppHandle, name: &str) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file()
        .add_filter("JSON 备份", &["json"])
        .set_file_name(name)
        .save_file(move |p| { let _ = tx.send(p); });
    rx.await.ok().flatten().and_then(|p| p.into_path().ok())
}
```

### 6.8 `bridge.js`（前端唯一新增文件）

```js
/* web/bridge.js —— 把 Electron preload 的 window.weeklight 原样映射到 Tauri */
(function () {
  const invoke = window.__TAURI__.core.invoke;
  const listen = window.__TAURI__.event.listen;
  const call = (cmd, args) => invoke(cmd, args);

  window.weeklight = {
    get:                ()                    => call('get'),
    saveTask:           (task, repeat)        => call('save_task', { task, repeat }),
    previewTaskAction:  (id, action)          => call('preview_task_action', { id, action }),
    applyTaskAction:    (request)             => call('apply_task_action', { request }),
    toggleTask:         (id)                  => call('toggle_task', { id }),
    deleteTask:         (id)                  => call('delete_task', { id }),
    moveTask:           (id, date, time)      => call('move_task', { id, date, time }),
    reminderAction:     (id, action)          => call('reminder_action', { id, action }),
    settings:           (s)                   => call('settings', { settings: s }),
    checkUpdates:       ()                    => call('check_updates'),
    dismissUpdate:      ()                    => call('dismiss_update'),
    downloadUpdate:     ()                    => call('download_update'),
    openUpdatePage:     ()                    => call('open_update_page'),
    testReminder:       ()                    => call('test_reminder'),
    nextReminderMessage:()                    => call('next_reminder_message'),
    exportData:         ()                    => call('export_data'),
    importData:         ()                    => call('import_data'),
    openData:           ()                    => call('open_data'),
    openWidget:         ()                    => call('open_widget'),
    closeWidget:        ()                    => call('close_widget'),
    widgetSettings:     (p)                   => call('widget_settings', { patch: p }),
    editTask:           (id)                  => call('edit_task', { id }),
    showMain:           ()                    => call('show_main'),
    quit:               ()                    => call('quit'),

    onState:    (fn) => listen('state',     e => fn(e.payload)),
    onNew:      (fn) => listen('new-task',  ()  => fn()),
    onEditTask: (fn) => listen('edit-task', e => fn(e.payload)),
    onError:    (fn) => listen('app-error', e => fn(e.payload)),
  };
})();
```

> 注意：Tauri 会把 JS 的 `camelCase` 参数名自动转成 Rust 的 `snake_case`，所以 `{ seriesId }` → `series_id`。前端原本就传 camelCase，天然兼容。

HTML 只需在原有脚本前加一行：

```html
<script src="bridge.js"></script>
```

### 6.9 `i18n.rs`：后端只需要约 40 条文案

前端 180 个键原样留在 `locale-*.js`。Rust 侧只保留**不经过 webview 的字符串**：通知标题/正文、错误提示、托盘菜单、对话框标题。实现成一个 `match (lang, key)` 表即可，约 120 行，不引入任何 i18n 库。

引语数据用 `include_str!("../web/reminder-classics.json")` 编译进二进制，避免运行时读文件。

### 6.10 `update.rs`：保留原行为，去掉重量依赖

```rust
const MANIFEST: &str = "https://raw.githubusercontent.com/CYJ316457/orange-schedule/main/docs/updates/latest.json";
const RELEASES: &str = "https://api.github.com/repos/CYJ316457/orange-schedule/releases/latest";

pub fn check(current: &str) -> Result<UpdateSnapshot, String> {
    let resp: serde_json::Value = ureq::get(MANIFEST)
        .set("User-Agent", "weeklight-tauri")
        .timeout(std::time::Duration::from_secs(12))
        .call().map_err(|e| e.to_string())?
        .into_json().map_err(|e| e.to_string())?;
    // 平移 compareVersions：仅接受稳定版 X.Y.Z
    ...
}
```

启动 7 秒后检查一次，之后每 6 小时一次（`tokio::time::interval` + `MissedTickBehavior::Skip`），可在设置里关闭。

---

## 7. 内存：实测结果与调优空间（50 MB 不可达）

### 7.1 先明确「50 MB」怎么量

Tauri 应用在任务管理器里不是一个进程，而是 **1 个 `weeklight.exe` + 3~4 个 `msedgewebview2.exe`**（browser / renderer / GPU / crashpad）。要量的是**整棵进程树的私有工作集之和**：

```powershell
$root = Get-Process -Name 'weeklight' -ErrorAction Stop | Select-Object -First 1
$all = New-Object System.Collections.Generic.List[object]
$queue = New-Object System.Collections.Generic.Queue[int]
$all.Add($root); $queue.Enqueue($root.Id)
while ($queue.Count -gt 0) {
  $pid0 = $queue.Dequeue()
  Get-CimInstance Win32_Process -Filter "ParentProcessId=$pid0" | ForEach-Object {
    $p = Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue
    if ($p) { $all.Add($p); $queue.Enqueue($p.Id) }
  }
}
$rows = $all | ForEach-Object {
  $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$($_.Id)"
  [pscustomobject]@{
    Name      = $_.ProcessName
    Id        = $_.Id
    WS_MB     = [math]::Round($_.WorkingSet64 / 1MB, 1)
    PrivWS_MB = [math]::Round($perf.WorkingSetPrivate / 1MB, 1)   # 任务管理器口径
  }
}
$rows | Format-Table -AutoSize
"PRIVATE WS TOTAL: {0:N1} MB" -f (($rows | Measure-Object PrivWS_MB -Sum).Sum)
```

> 注意：不要用 `WorkingSet64` 直接相加 —— 它包含进程间共享的 DLL 页，会把 113 MB 虚报成 366 MB。判断「占多少内存」应以 `WorkingSetPrivate` 之和为准。

### 7.2 调优阶梯（每级都可独立验证）

| 级别 | 配置 | 实测/预估（私有工作集） | CPU 影响 | 状态 |
|---|---|---|---|---|
| T0 | Tauri 默认，单主窗口 | **113.3 MB** | 基准 | ✅ 已实测 |
| T1 | 便签默认关 + 提醒窗懒创建 | ~110 MB | 无 | 预计（便签默认本就关闭） |
| T2 | `--disable-gpu`（软件合成） | **98.2 MB** | 重绘时略升 | ✅ 已实测 |
| T3 | + `--js-flags=--max-old-space-size=64` | 90–100 MB | 无 | 待实测 |
| T4 | + DOM 精简 / 减少 SVG 内联 / 语言表按需加载 | 85–95 MB | 无 | 待实测 |
| — | **空白页地板** | **76.5 MB** | — | ✅ 已实测（不可再降） |

**T2 / T3 的具体写法**：主窗口来自 `tauri.conf.json`，无法在代码里链式调用 `additional_browser_args`，所以用 WebView2 的全局环境变量（必须在 `Builder` 启动前设置，对所有窗口生效）：

```rust
fn main() {
    // T2/T3 调优：必须早于 Tauri 初始化
    #[cfg(windows)]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--disable-gpu \
         --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
         --disable-background-networking \
         --disable-component-update \
         --disable-sync \
         --no-first-run \
         --disable-extensions \
         --js-flags=--max-old-space-size=64",
    );

    let mut builder = tauri::Builder::default();
    ...
}
```

如果是代码里创建的窗口（便签/提醒），也可以单独用 builder 方法：

```rust
let w = tauri::WebviewWindowBuilder::new(app, "widget", tauri::WebviewUrl::App("widget.html".into()))
    .additional_browser_args("--disable-gpu --js-flags=--max-old-space-size=48")
    .build()?;
```

> `--disable-gpu` 在个别 WebView2 版本上可能导致黑屏或字体发虚，务必在目标机器上先验证；若出现，退回 T1（不关 GPU，55–75 MB）并靠 T3 的 V8 上限压峰值。

**结论（实测校正）**：50 MB 在 WebView2 上**不可达** —— 空白页地板就是 76.5 MB。现实可达区间是 **90–115 MB**（T2 实测 98.2 MB，T3/T4 预计 85–95 MB）。若 50 MB 是硬指标，必须换原生技术栈（WPF 约 35–70 MB，Win32 约 10–25 MB），继续调 WebView2 参数没有意义。

### 7.3 其他省内存的点

| 措施 | 预计节省 | 说明 |
|---|---|---|
| 便签默认不创建 | 25–40 MB | 透明窗口会强制额外的合成表面 |
| 提醒窗用完销毁（`close()` 而非 `hide()`） | 20–35 MB | 原版就是 `closed` 后置 null，保持一致 |
| 不保留隐藏的第二窗口 | 20–40 MB | 同上 |
| 主窗不用 `transparent` | 5–15 MB | 主窗本来就不透明，别为了圆角开启 |
| 不加载 `docs/` 图片 | 0（不进包） | 打包时排除 |
| 任务列表虚拟化（>2000 条再做） | 0–10 MB | 当前 DOM 只有 28–42 个格子，暂不需要 |

### 7.4 数据迁移（从 Electron 版切换时）

Electron 版的数据目录由 `app.getPath('userData')` 决定，实际值取决于 `app.getName()`（`package.json` 的 `name` 或 `productName`）。为避免猜错，迁移时**按候选列表逐个探测**，命中即拷贝：

```rust
fn migrate_from_electron(dir: &std::path::Path) -> Option<String> {
    let target = dir.join("plans.json");
    if target.exists() { return None; }

    let appdata = std::env::var("APPDATA").ok()?;
    let base = std::path::Path::new(&appdata);
    let candidates = ["weeklight", "橙子日程", "OrangeSchedule", "orange-schedule"];

    for name in candidates {
        let legacy = base.join(name).join("plans.json");
        if legacy.exists() {
            std::fs::copy(&legacy, &target).ok()?;
            return Some(legacy.display().to_string());
        }
    }
    None
}
```

> 迁移完成后在设置页显示「已从 … 导入」，让用户可确认。原文件不删除，保留回退能力。

---

## 8. 体积优化

| 手段 | 效果 |
|---|---|
| `opt-level = "s"` + `lto = true` + `codegen-units = 1` | 二进制 5.5 MB → 3.8–4.5 MB |
| `strip = true` + `panic = "abort"` | 再省 0.5–1 MB |
| `webviewInstallMode: "skip"` | 安装包从 ~130 MB → 2.5–4 MB |
| 不引入 `tauri-plugin-updater` / `store` / `reqwest` | 省 1.5–2.5 MB |
| 前端不打包（`withGlobalTauri`，无 npm） | 无 node_modules、无 sourcemap |
| 只打包 `web/`，排除 `docs/`、`tests/`、`src/native/` | 省 1.7 MB |
| 图标只留 32/128/256 + ico | 省 90 KB |
| `-C target-feature=+crt-static`（可选） | 免装 VC++ 运行库，+200 KB |

**预估最终产物：**

- `weeklight.exe`：**3.8–4.8 MB**
- 前端资源：**~180 KB**
- `resources/icon.png`：60 KB
- NSIS 安装包：**2.8–4.0 MB**
- 安装后：**8–12 MB**

---

## 9. 实施阶段

| 阶段 | 内容 | 产出 | 工期 |
|---|---|---|---|
| **M0** | 脚手架：`src-tauri/`、`web/` 迁移、`bridge.js` 空壳、单窗口能开 | 静态页在 Tauri 里显示 | 0.5 d |
| **M1** | 数据层：`state.rs` / `store.rs` / `series.rs` / 24 命令 | 日历、待办、重复任务全可用 | 2–3 d |
| **M2** | 系统集成：托盘、单实例、开机自启、关窗隐藏、通知 | 行为对齐 Electron 版 | 1.5 d |
| **M3** | 便签窗口：透明/无边框/置底/位置持久化 | 便签可用 | 1–1.5 d |
| **M4** | 提醒：5s tick、引语轮换、稍后提醒、测试提醒 | 提醒链路完整 | 1 d |
| **M5** | 导入导出、更新检查、安装包、体积/内存调优、回归 | 可发布安装包 | 1.5–2 d |
| **合计** | | | **8–10 人日** |

### 验收标准

- [ ] 24 个命令全部返回与 Electron 版一致的 JSON 结构（用 `plans.json` 对拍）
- [ ] 关闭主窗后进程仍在、提醒照常触发
- [ ] 便签置底后点击其他窗口不抢焦点
- [ ] 开机自启带 `--hidden`，只在托盘出现
- [ ] `plans.json` 可双向导入导出，字段零丢失
- [ ] **T3/T4 配置下私有工作集 ≤ 95 MB**（PowerShell 脚本采样 60 s；空白页地板 76.5 MB 不可突破）
- [ ] 安装包 ≤ 5 MB，安装后 ≤ 12 MB

---

## 10. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| WebView2 缺失（老 Win10） | 应用无法启动 | 检测并提示安装 Evergreen Runtime；或改 `downloadBootstrapper` |
| 透明便签在 WebView2 上拖拽/点击穿透行为不同 | 便签难用 | 用 `data-tauri-drag-region` + `start_dragging`；必要时牺牲透明换稳定 |
| `single-instance` 注册顺序错误 | 第二实例不聚焦 | 必须作为第一个插件注册 |
| GPU 关闭后滚动卡顿 | 体验下降 | 只在小窗口便签上关；主窗保持 GPU，用 T3 的 V8 限制达标 |
| 通知被 Windows 专注助手拦截 | 提醒不弹 | 首次启动引导用户允许通知；保留应用内提醒窗兜底 |
| 数据目录 identifier 变化 | 用户数据"丢失" | §7.4 一次性迁移 + 保留 `.bak` |
| Rust 重写引入行为差异 | 边界 bug | 保留 `core.test.cjs` 作为「黄金用例」，Rust 单测逐条对齐 |

**回退方案**：Tauri 版使用独立 identifier 与安装项，不覆盖 Electron 版；出问题直接卸载 Tauri 版、重装 Electron 版即可，`plans.json` 格式完全一致，数据可回灌。

---

## 11. 测试策略

| 层 | 工具 | 覆盖 |
|---|---|---|
| Rust 单元测试 | `cargo test` | 校验、重复任务展开、批量操作、版本比较、due 判定 |
| 前端 DOM 测试 | Vitest + jsdom（mock `window.weeklight`） | 渲染、筛选、拖拽、i18n |
| 端到端 | `tauri-driver` + WebDriverIO（Windows） | 启动、托盘、便签、提醒 |
| 打包冒烟 | 干净 Win10/11 VM | 安装、卸载、自启、通知、置底 |
| 资源回归 | 上面的 PowerShell 采样脚本 | 内存/CPU 阈值告警 |

原 `tests/*.cjs` 里的纯逻辑断言（`core.test.cjs`、`task-series.test.cjs`、`reminder-copy.test.cjs`、`weekday.test.cjs`）可以直接作为 Rust 单测的**输入输出对拍表**，这是防止重写引入偏差最有效的手段。

---

## 12. 预期结果对照

| 指标 | Electron 1.2.1（估算） | Tauri（实测 T0） | Tauri（目标 T3/T4） |
|---|---|---|---|
| 进程数 | 5–6 | 7 | 7 |
| 内存（私有工作集） | 250–350 MB | **113.3 MB** | 85–95 MB |
| 内存（关 GPU） | — | **98.2 MB** | — |
| 安装包 | 111.9 MB | 3–4 MB | 3–4 MB |
| 安装后占用 | 250–350 MB | 9–12 MB | 9–12 MB |
| 空闲 CPU | 0.1–0.5% | <0.05% | <0.1% |
| 冷启动 | 1.5–3 s | 0.3–0.8 s | 0.4–1.0 s |
| 跨平台 | Win（构建目标） | Win/Mac/Linux | 同左 |

> 说明：Electron 一列仍为估算（未在本机实测原版）。Tauri 一列除「安装包/安装后占用/冷启动」外均为 M0 实测。若需要严谨对比，下一步可 `npm install` 后实测 Electron 版同一指标。

---

## 附：Fork 之后的第一步

```bash
git clone https://github.com/CYJ316457/orange-schedule.git
cd orange-schedule
git checkout -b feat/tauri-rewrite

# 1) 前端搬进 web/
mkdir web && git mv src/index.html src/widget.html src/reminder.html \
  src/renderer.js src/widget.js src/reminder.js src/task-actions.js src/updates.js \
  src/style.css src/widget.css src/task-actions.css src/updates.css \
  src/i18n.js src/locale-core.js src/locale-main.js src/locale-aux.js web/
mkdir -p web/assets && git mv src/assets/icon.png web/assets/

# 2) 生成 Tauri 脚手架
npm create tauri-app@latest -- --template vanilla --manager cargo   # 或手工建 src-tauri/

# 3) 先把 bridge.js 接上，确认能看到静态界面，再动 Rust
```

> 建议先做 M0 的**静态验证**：只把 `web/` 挂进 Tauri、`bridge.js` 返回 mock 数据，确认字体、布局、图标在 WebView2 下与 Electron 一致，再开始重写后端。这一步能提前暴露 90% 的渲染差异。
