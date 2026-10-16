use std::path::Path;

use anyhow::Context;

/// 从 `ffmpeg -i <file>` 的 stderr 里抠出来的媒体信息。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaInfo {
    /// 拿不到时为 `None`，调用方据此把进度退化成不确定态。
    pub duration_secs: Option<f64>,
    /// 有没有视频流。
    ///
    /// 纯音频输入时是 `false`，而 ffmpeg **不会**因此报错 —— 它会老老实实编出一个
    /// 只有音轨的 mp4 并退出 0。用户点的是"压缩视频"，拿到一个没有画面的文件是那种
    /// "看起来成功了"的坏结果，所以必须在**入队前**挡掉（见 `add_compress_job`）。
    pub has_video: bool,
}

/// 从 `ffmpeg -i <file>` 的 stderr 里抠出时长（秒）。
///
/// **为什么要写这个**：内嵌的只有 ffmpeg，没有 ffprobe（`vendor/ffmpeg/` 下就一个二进制）。
/// 不带输出文件时 ffmpeg 会以非零码退出，那是正常的 —— 我们要的就是它打在 stderr 上的信息。
///
/// 拿不到时返回 `None` 而不是 0：调用方据此把进度退化成不确定态。
/// 返回 0 会让进度条要么永远停在 0%、要么在除法里炸掉。
pub fn parse_duration(stderr: &str) -> Option<f64> {
    let line = stderr.lines().find(|l| l.contains("Duration:"))?.trim();
    let rest = line.split("Duration:").nth(1)?.trim();
    let stamp = rest.split(',').next()?.trim();
    // 格式固定是 HH:MM:SS.ss，三段冒号分隔。
    // `N/A` 会在这里被拆成 1 段而落空。
    let parts: Vec<&str> = stamp.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    // 秒那一段**必须带小数点**。ffmpeg 永远打两位小数，所以这是对已知格式的形状校验：
    // 少了它，"Duration: 00:00:0" 这种被截断的行会三段都 parse 成功、得到 Some(0.0) ——
    // 一个假时长比拿不到时长更坏，它会让进度条除出一个无意义的数。
    if !parts[2].contains('.') {
        return None;
    }
    let h: f64 = parts[0].trim().parse().ok()?;
    let m: f64 = parts[1].trim().parse().ok()?;
    let s: f64 = parts[2].trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}


/// 从同一份 stderr 里判断有没有视频流。
///
/// 认的是 ffmpeg 打印的流信息行，长这样：
/// `  Stream #0:0[0x1](und): Video: h264 (High), yuv420p, 640x360, ...`
/// 所以要求 `Stream #` 开头**且**含 `Video:` —— 只看 `Video:` 会误判
/// `Metadata: handler_name : VideoHandler` 这类行。
pub fn parse_has_video(stderr: &str) -> bool {
    stderr
        .lines()
        .any(|l| l.trim_start().starts_with("Stream #") && l.contains("Video:"))
}

pub fn parse_media_info(stderr: &str) -> MediaInfo {
    MediaInfo {
        duration_secs: parse_duration(stderr),
        has_video: parse_has_video(stderr),
    }
}

/// 真去跑一次 `ffmpeg -i`。
///
/// 只能返回 `Err` 表示"ffmpeg 这个进程没跑起来"（可执行文件不存在之类）；
/// **文件本身读不了不算错误** —— 那种情况下 `has_video` 为 false、`duration_secs`
/// 为 None，由调用方决定怎么处理（入队校验会据此拒绝它）。
pub async fn inspect(ffmpeg: &Path, input: &Path) -> anyhow::Result<MediaInfo> {
    // 不带输出文件：ffmpeg 打印完流信息就以非零码退出，这正是我们要的
    let out = tokio::process::Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .output()
        .await
        .context("run ffmpeg -i")?;
    Ok(parse_media_info(&String::from_utf8_lossy(&out.stderr)))
}
