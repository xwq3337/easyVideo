use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::events::TaskId;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskManifest {
    pub id: TaskId,
    pub url: String,
    pub filename: String,
    pub download_dir: PathBuf,
    pub kind: ManifestKind,
    /// 浏览器扩展捕获到的请求头。**必须带 `#[serde(default)]`**：旧版本的
    /// manifest.json 里没有这个键，缺了它整份清单反序列化失败，重启后这些任务
    /// 就再也续不上了（`scan_resume` 会静默跳过它们）。
    #[serde(default)]
    pub headers: crate::core::request::Headers,
    /// 选中 yt-dlp 纯视频格式时配套的音频轨。恢复任务后仍要把音频下完并合并，
    /// 否则用户会得到一个无声的视频。旧 manifest 没有这个键 —— `default` 必须保留。
    #[serde(default)]
    pub audio: Option<crate::core::ytdlp::YtdlpAudio>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ManifestKind {
    Direct { size: Option<u64> },
    Hls { playlist_url: String },
}

pub fn task_dir(download_root: &Path, id: TaskId) -> PathBuf {
    download_root.join(format!("task-{id}"))
}

pub fn write_manifest(dir: &Path, m: &TaskManifest) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(m)?;
    std::fs::write(dir.join("manifest.json"), text)?;
    Ok(())
}

pub fn scan_resume(download_root: &Path) -> Vec<TaskManifest> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(download_root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // **必须过滤 `task-<id>` 前缀**。下载目录是**用户可配置**的（设置里可以改成 ~/Documents 之类），
        // 而 `manifest.json` 是个极常见的文件名 —— Web App Manifest、Chrome 扩展、VS Code 都在用。
        // 不按前缀过滤的话，用户目录下任何一个含 manifest.json 的子目录都会被当成"可续传任务"，
        // 于是在启动时凭空冒出一堆假任务，甚至去"续传"一个根本不是下载的目录。
        // 仅靠"JSON 能解析成 TaskManifest"挡不住：外部 manifest 可能恰好字段同名而解析通过。
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("task-") {
            continue;
        }
        let manifest_path = path.join("manifest.json");
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        if let Ok(m) = serde_json::from_str::<TaskManifest>(&text) {
            out.push(m);
        }
    }
    out.sort_by_key(|m| m.id);
    out
}
