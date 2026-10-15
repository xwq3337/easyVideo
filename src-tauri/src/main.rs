// `get_webview_window` 来自这个 trait,而唯一的用处被 `#[cfg(not(macos))]` 圈住了;
// 不在 import 上也加同样的 cfg,否则 macOS 构建会报 unused_imports。
#[cfg(not(target_os = "macos"))]
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // **macOS 上必须保留 decorations。** Tauri 自己的配置文档
            // (`tauri-utils` 的 config.rs，`traffic_light_position` 那一项) 写明
            // `titleBarStyle: Overlay` "Requires ... decorations: true" —— 红绿灯是那圈
            // 边框的一部分，`decorations: false` 会把它一并拿走。而界面层的 `WindowControls`
            // 在 macOS 上按设计不渲染（它指望系统红绿灯），两者叠加就是一个**没有任何
            // 关闭/最小化按钮**的窗口。
            //
            // 所以：macOS 保留 decorations（Overlay 让标题栏那一条看不见、红绿灯浮在内容上），
            // Windows/Linux 才真正去掉边框，交给自绘按钮。
            #[cfg(not(target_os = "macos"))]
            if let Some(w) = app.get_webview_window("main") {
                w.set_decorations(false).ok();
            }
            // macOS 上这个钩子没有别的事要做(decorations 由 tauri.conf.json 保留),
            // 于是 `app` 没被用到。显式吃掉它,免得 macOS 构建多一条 unused_variables。
            #[cfg(target_os = "macos")]
            let _ = app;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
