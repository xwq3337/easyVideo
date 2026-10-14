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
