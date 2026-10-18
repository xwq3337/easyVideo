// release 下不要附带控制台窗口（Windows 控制台子系统）；debug 保留它方便看日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 组装层:把 core 引擎、自建 tokio runtime、事件桥和命令层接成一个能跑的应用。
//!
//! **这里是另一个 crate root。** 所有模块都通过 `easydownvideo_tauri::...` 访问,
//! 绝不在这里重复 `mod dto;` / `mod bridge;` —— 两个 root 各声明一次会得到两套
//! 互不相干的类型,然后是极难定位的 `expected 'dto::X', found 'dto::X'`。

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use easydownvideo::settings::Settings;
use easydownvideo_tauri::{bridge, commands, state::AppState};

// `get_webview_window` 是 `Manager` 的方法,而唯一用到它的那段被 `#[cfg(not(macos))]`
// 圈掉了;不在 import 上加同样的 cfg,macOS 构建会多一条 unused_imports。
// (`app.handle()` 是 `App` 的**固有**方法,不经过 `Manager`,所以 macOS 上 `Manager`
//  确实一处都用不到。)
#[cfg(not(target_os = "macos"))]
use tauri::Manager;

/// core 需要一个 tokio runtime handle。**我们自己建这个 runtime**,不依赖 Tauri 内部用的是什么。
/// 它必须活得和进程一样久,所以放进 static。
static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn init_tracing(data_dir: &std::path::Path) {
    std::fs::create_dir_all(data_dir.join("logs")).ok();
    let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(
        data_dir.join("logs"),
        "easydownvideo.log",
    ));
    // guard 必须活到进程结束,泄漏它是有意的。
    std::mem::forget(guard);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(writer)
        .init();
}

fn main() {
    let data_dir: PathBuf = Settings::data_dir();
    init_tracing(&data_dir);

    let rt = RT.get_or_init(|| tokio::runtime::Runtime::new().expect("create tokio runtime"));
    let settings = Settings::load();

    let engine = easydownvideo_tauri::state::build_engine(
        rt.handle().clone(),
        settings.clone(),
        data_dir.clone(),
    );

    let videoops = easydownvideo_tauri::state::build_videoops_with_settings(
        rt.handle().clone(),
        data_dir.clone(),
        &settings,
    );

    // 断点恢复:扫描上次未完成的任务并重新入队(与 egui 版的 main 一致)。
    for m in easydownvideo::core::task::scan_resume(&settings.download_dir) {
        tracing::info!("resuming task {} ({})", m.id, m.filename);
        engine.resume(m);
    }

    let app_state = AppState {
        engine: engine.clone(),
        videoops: videoops.clone(),
        settings: std::sync::Mutex::new(settings),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::get_tasks,
            commands::add_url,
            commands::pause,
            commands::unpause,
            commands::retry,
            commands::cancel,
            commands::remove_task,
            commands::set_filename,
            commands::pending_choice,
            commands::set_choice,
            commands::get_settings,
            commands::update_settings,
            commands::pick_download_dir,
            commands::get_video_jobs,
            commands::add_video_job,
            commands::plan_output,
            commands::cancel_video_job,
            commands::remove_video_job,
            commands::pick_video_file,
            commands::pick_video_files,
            commands::probe_video,
            commands::check_merge,
            commands::preview_frame,
        ])
        .setup(move |app| {
            // **macOS 上必须保留 decorations。** Tauri 自己的配置文档
            // (`tauri-utils` 的 config.rs,`traffic_light_position` 那一项) 写明
            // `titleBarStyle: Overlay` "Requires ... decorations: true" —— 红绿灯是那圈
            // 边框的一部分,`decorations: false` 会把它一并拿走。而界面层的 `WindowControls`
            // 在 macOS 上按设计不渲染(它指望系统红绿灯),两者叠加就是一个**没有任何
            // 关闭/最小化按钮**的窗口。
            //
            // 所以:macOS 保留 decorations(Overlay 让标题栏那一条看不见、红绿灯浮在内容上),
            // Windows/Linux 才真正去掉边框,交给自绘按钮。
            #[cfg(not(target_os = "macos"))]
            if let Some(w) = app.get_webview_window("main") {
                w.set_decorations(false).ok();
            }

            // 事件桥:用同一个 engine 再订阅一路接收器。
            let sink: Arc<tauri::AppHandle> = Arc::new(app.handle().clone());
            tauri::async_runtime::spawn(bridge::run(
                sink.clone(),
                engine.events(),
                easydownvideo_tauri::dto::download_event_json,
                bridge::EVENT,
            ));
            // 第二条通道：视频操作。**共用同一个 RESYNC 事件名** —— 任一条落后都整体重取，
            // 宁可多取一次，也不要漏（漏一个 Done 就是那一行永久停在"运行中"）。
            tauri::async_runtime::spawn(bridge::run(
                sink,
                videoops.events(),
                easydownvideo_tauri::dto::video_event_dto,
                bridge::VIDEO_EVENT,
            ));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
