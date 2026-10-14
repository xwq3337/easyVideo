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
