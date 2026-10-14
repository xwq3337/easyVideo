#[test]
fn cjk_font_loads_from_the_system() {
    // 界面全是中文。这个函数返回 false，用户看到的就是满屏方框（□）。
    // 用 Context::default() 即可 —— egui 允许在无窗口环境下建 Context。
    let ctx = eframe::egui::Context::default();
    assert!(
        easydownvideo::ui_font::install_cjk_font(&ctx),
        "本机没找到任何可用的系统中文字体 —— 界面中文会渲染成方框。\
         这说明候选路径在本平台不对（或字体文件读不出来），不是无关紧要的失败。"
    );
}
