mod app;
mod core;
mod settings;

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "EasyDownVideo",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(app::App::default()))),
    )
}
