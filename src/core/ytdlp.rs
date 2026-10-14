use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YtdlpFormat {
    pub id: String,
    pub ext: String,
    pub note: String,
    pub filesize: Option<u64>,
    pub url: String,
}

pub fn find_ytdlp() -> Option<PathBuf> {
    find_ytdlp_in(std::env::var_os("PATH")?)
}

/// 纯函数：扫描给定的 PATH 字符串。拆出来是为了可测 —— 直接驱动 `find_ytdlp()` 就得改全局
/// 环境变量，那会与并行跑的其它测试互相干扰（env 是进程级的）。
pub fn find_ytdlp_in(path_var: impl AsRef<std::ffi::OsStr>) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["yt-dlp.exe", "youtube-dl.exe", "yt-dlp", "youtube-dl"]
    } else {
        &["yt-dlp", "youtube-dl"]
    };
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn parse_formats(json: &serde_json::Value) -> Vec<YtdlpFormat> {
    let mut out = Vec::new();
    let Some(formats) = json.get("formats").and_then(|f| f.as_array()) else {
        return out;
    };
    for f in formats {
        let vcodec = f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("");
        if vcodec == "none" {
            continue; // 纯音频
        }
        let Some(url) = f.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        let note = f
            .get("format_note")
            .and_then(|v| v.as_str())
            .or_else(|| f.get("resolution").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        out.push(YtdlpFormat {
            id: f.get("format_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            ext: f.get("ext").and_then(|v| v.as_str()).unwrap_or("mp4").to_string(),
            note,
            filesize: f.get("filesize").or_else(|| f.get("filesize_approx")).and_then(|v| v.as_u64()),
            url: url.to_string(),
        });
    }
    out
}

pub async fn fetch_formats(exe: &Path, page_url: &str) -> anyhow::Result<Vec<YtdlpFormat>> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new(exe)
            .arg("-J")
            .arg(page_url)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("yt-dlp 解析超时（120s）"))??;
    if !out.status.success() {
        return Err(anyhow::anyhow!(
            "yt-dlp 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    Ok(parse_formats(&json))
}
