use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub segment_concurrency: usize,
    pub max_active_tasks: usize,
    pub ytdlp_enabled: bool,
    pub ffmpeg_path_override: Option<PathBuf>,
    /// 从哪个浏览器读 cookie（`chrome` / `firefox` / `safari` …）。`None` = 不读。
    ///
    /// **必须带 `#[serde(default)]`**：老配置里没有这个键，缺了它整份 settings.json
    /// 反序列化失败，而 `load()` 用 `unwrap_or_default()` 兜底 —— 用户升级后所有设置
    /// 会静默回到默认值（保存目录都丢），且界面上没有任何提示。
    #[serde(default)]
    pub cookies_from_browser: Option<String>,
    /// 让 yt-dlp 以浏览器指纹发请求（`--extractor-args generic:impersonate`），
    /// 并让本程序自己的下载客户端也用浏览器 UA。绕过 Cloudflare 的主要手段。
    /// 需要 yt-dlp 装了 `curl_cffi`，否则它会报错（见 `ytdlp_error_hint`）。
    #[serde(default)]
    pub impersonate: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let download_dir = directories::UserDirs::new()
            .map(|d| d.download_dir().map(|p| p.to_path_buf()))
            .flatten()
            .unwrap_or_else(|| std::env::temp_dir())
            .join("easydownvideo");
        Self {
            download_dir,
            segment_concurrency: 6,
            max_active_tasks: 2,
            ytdlp_enabled: true,
            ffmpeg_path_override: None,
            cookies_from_browser: None,
            impersonate: false,
        }
    }
}

/// 允许出现在 `--cookies-from-browser` 里的浏览器名。
///
/// 这是一份**白名单**，不是"看着像就放行"：这个值会被原样拼进 yt-dlp 的命令行，
/// 而设置是从界面来的。任意的字符串（比如以 `-` 开头的东西）会被 yt-dlp 当成新的
/// 参数解析 —— 那就是一条从设置页通往任意命令行的路。
pub const BROWSERS: &[&str] = &[
    "brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi", "whale",
];

/// 校验并规范化浏览器名。不认识的一律当成"不读 cookie"。
pub fn sanitize_browser(name: &str) -> Option<String> {
    let n = name.trim().to_ascii_lowercase();
    BROWSERS.contains(&n.as_str()).then_some(n)
}

impl Settings {
    pub fn config_dir() -> PathBuf {
        let dir = directories::ProjectDirs::from("dev", "easydownvideo", "easydownvideo")
            .expect("home directory must exist")
            .config_dir()
            .to_path_buf();
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    /// 应用数据目录（`ProjectDirs::data_dir()`）：释放出来的 ffmpeg 与日志放这里。
    ///
    /// **不要用 `config_dir().parent()` 去凑**：`config_dir()` 已经是应用专属的那一级，
    /// 取 parent 会退回到 `~/.config` / `~/Library/Application Support` 这类**多应用共享**目录，
    /// 于是 ffmpeg 和日志会落到一个不带应用名的公共位置。
    pub fn data_dir() -> PathBuf {
        let dir = directories::ProjectDirs::from("dev", "easydownvideo", "easydownvideo")
            .expect("home directory must exist")
            .data_dir()
            .to_path_buf();
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    pub fn load() -> Self {
        let path = Self::config_dir().join("settings.json");
        Self::load_from(&path).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_dir().join("settings.json");
        self.save_to(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create config dir")?;
        }
        let text = serde_json::to_string_pretty(self).context("serialize settings")?;
        std::fs::write(path, text).context("write settings")
    }
}
