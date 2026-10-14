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
        }
    }
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
