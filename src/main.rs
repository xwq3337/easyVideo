mod app; // 只声明 app；core / settings 走 lib crate（不要写 `mod core;`）

use eframe::egui;
use easydownvideo::settings::Settings;

fn main() -> eframe::Result<()> {
    // 日志：应用数据目录 logs/easydownvideo.log（按天滚动）
    // 注意用 Settings::data_dir() 而不是 config_dir().parent().join("data") —— 后者会落到共享目录
    let data_dir = Settings::data_dir();
    std::fs::create_dir_all(data_dir.join("logs")).ok();
    let (file_writer, _guard) = tracing_appender::non_blocking(
        tracing_appender::rolling::daily(&data_dir.join("logs"), "easydownvideo.log"),
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(file_writer)
        .init();

    let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
    let _enter = rt.enter();

    let settings = Settings::load();
    let engine = app::build_engine(rt.handle().clone(), settings.clone(), data_dir);

    // 断点恢复：扫描上次未完成的任务并重新入队
    for m in easydownvideo::core::task::scan_resume(&settings.download_dir) {
        tracing::info!("resuming task {} ({})", m.id, m.filename);
        engine.resume(m);
    }

    eframe::run_native(
        "EasyDownVideo",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([860.0, 560.0])
                .with_min_inner_size([720.0, 480.0]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, engine, settings)))),
    )
}
