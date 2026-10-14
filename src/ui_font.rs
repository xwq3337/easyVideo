//! 中文字体安装。详见计划中"中文字体"一节的取舍说明。

use eframe::egui;

/// 各平台候选路径（按优先级）。都是系统自带字体，取第一个能读到的。
/// 第二项是 `.ttc` 集合里的字体面序号（0 = 该集合的第一个面）。
#[cfg(target_os = "macos")]
const CANDIDATES: &[(&str, u32)] = &[
    ("/System/Library/Fonts/PingFang.ttc", 0), // 苹方 SC
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    ("/Library/Fonts/Arial Unicode.ttf", 0),
];

#[cfg(target_os = "windows")]
const CANDIDATES: &[(&str, u32)] = &[
    ("C:\\Windows\\Fonts\\msyh.ttc", 0),   // 微软雅黑
    ("C:\\Windows\\Fonts\\simhei.ttf", 0), // 黑体
    ("C:\\Windows\\Fonts\\simsun.ttc", 0), // 宋体
];

#[cfg(all(unix, not(target_os = "macos")))]
const CANDIDATES: &[(&str, u32)] = &[
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf", 0),
    ("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0),
];

/// 把系统中文字体装进 egui 字体表，作为**后备**字体（追加在默认字体之后）。
///
/// 返回是否装上了 —— `false` 意味着界面上所有中文都会显示成方框。
pub fn install_cjk_font(ctx: &egui::Context) -> bool {
    for (path, index) in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else { continue };
        if bytes.is_empty() {
            continue;
        }
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "cjk".to_owned(),
            egui::FontData {
                font: bytes.into(),
                index: *index,
                tweak: Default::default(),
            }
            .into(),
        );
        // **追加到末尾**而不是插到最前：拉丁字母和数字继续用 egui 默认字体渲染
        // （字形更协调），只有默认字体缺字的字符才回退到中文字体。
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        tracing::info!("中文字体已加载: {path} (index {index})");
        return true;
    }
    tracing::warn!(
        "未找到系统中文字体，界面中文将显示为方框（□）。候选路径: {:?}",
        CANDIDATES.iter().map(|(p, _)| *p).collect::<Vec<_>>()
    );
    false
}
