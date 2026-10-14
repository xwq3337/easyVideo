use easydownvideo::settings::Settings;

mod app; // 只有 app 属于 binary；core / settings 一律走 lib crate

fn main() -> eframe::Result<()> {
    let _settings = Settings::load();
    eframe::run_native(
        "EasyDownVideo",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(app::App::default()))),
    )
}
