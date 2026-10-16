use std::path::Path;

use anyhow::Context;

/// 预览图缩到多宽。640 宽的一张 JPEG 实测 13.7 KB（base64 后 18 KB）——
/// 走 IPC 毫无压力，所以**不必**去开 Tauri 的 assetProtocol。
pub const PREVIEW_MAX_WIDTH: u32 = 640;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64。自己写是因为全局约束说了不新增依赖 —— 十几行，标准向量测着。
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// 抽一帧，缩到 `max_width` 宽，返回可以直接塞进 `<img src>` 的 data URL。
///
/// `-ss` 放在 `-i` **之前**（输入选项）：对单帧来说那是"跳过去再解一帧"，
/// 比解到那一帧快得多（长视频上差一个数量级）。
pub async fn frame_data_url(
    ffmpeg: &Path,
    input: &Path,
    at_secs: f64,
    max_width: u32,
) -> anyhow::Result<String> {
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-nostdin", "-v", "error", "-y"])
        .arg("-ss").arg(format!("{:.3}", at_secs.max(0.0)))
        .arg("-i").arg(input)
        .args(["-frames:v", "1"])
        .args(["-vf", &format!("scale={max_width}:-2")])
        .args(["-c:v", "mjpeg", "-q:v", "5", "-f", "image2", "-"])
        .output()
        .await
        .context("run ffmpeg (frame)")?;
    if !out.status.success() {
        anyhow::bail!("抽帧失败: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    if out.stdout.is_empty() {
        anyhow::bail!("抽帧没有输出（可能是时间超出了视频长度）");
    }
    Ok(format!("data:image/jpeg;base64,{}", base64_encode(&out.stdout)))
}
