use std::path::Path;

use anyhow::Context;

/// 从 `ffmpeg -i <file>` 的 stderr 里抠出来的媒体信息。
///
/// **不是 `Copy`**：`video_codec` / `container` 是 `String`。调用方一律传引用。
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    /// 拿不到时为 `None`，调用方据此把进度退化成不确定态。
    pub duration_secs: Option<f64>,
    /// 有没有视频流。
    ///
    /// 纯音频输入时是 `false`，而 ffmpeg **不会**因此报错 —— 它会老老实实编出一个
    /// 只有音轨的 mp4 并退出 0。用户点的是"压缩视频"，拿到一个没有画面的文件是那种
    /// "看起来成功了"的坏结果，所以必须在**入队前**挡掉（见 `add_job`）。
    pub has_video: bool,
    /// 视频编码，形如 "h264" / "hevc" / "vp9"。取不到时 None。
    pub video_codec: Option<String>,
    /// 音频编码，形如 "aac" / "opus"。**没有音轨时是 None**（合并的兼容性检查
    /// 要区分"编码不同"与"压根没有音轨"——后者只是警告，不是拦阻）。
    pub audio_codec: Option<String>,
    /// 容器，形如 "mp4" / "mkv" / "webm" / "mov"。取不到时 None。
    pub container: Option<String>,
    /// 画面尺寸。裁剪框的坐标系就是它，取不到时裁剪不可用。
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// 从 `Stream #0:0…: Video: h264 (High) …` 里取编码名。
pub fn parse_video_codec(stderr: &str) -> Option<String> {
    let line = stderr
        .lines()
        .find(|l| l.trim_start().starts_with("Stream #") && l.contains("Video:"))?;
    codec_name(line.split("Video:").nth(1)?)
}

/// 编码名那一段的第一个**词**。
///
/// 空白与逗号都算分隔符：实测 `Audio: opus, 48000 Hz, mono, fltp` —— 没有档位括号的
/// 编码后面**直接跟逗号**，只按空白切会拿到 `"opus,"`。带括号的那些
/// （`aac (LC)`、`h264 (High)`）恰好侥幸对，所以这个坑只在部分源上露头。
fn codec_name(rest: &str) -> Option<String> {
    let codec = rest
        .trim()
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()?;
    (!codec.is_empty()).then(|| codec.to_string())
}

/// ffmpeg 打印的 demuxer 列表 → 容器名。
///
/// **mkv 与 webm 在 ffmpeg 眼里是同一个 demuxer**（`matroska,webm`），
/// 光靠 stderr 分不开 —— 所以要看输入文件的扩展名来定夺。
pub fn parse_container(stderr: &str, path: &Path) -> Option<String> {
    let line = stderr.lines().find(|l| l.trim_start().starts_with("Input #"))?;
    // `Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':`
    let list: Vec<&str> = line
        .split("Input #")
        .nth(1)?
        .split(',')
        .skip(1)
        .take_while(|s| !s.contains("from"))
        .map(str::trim)
        .collect();
    if list.iter().any(|s| matches!(*s, "mp4" | "mov" | "m4a" | "3gp")) {
        // mov 与 mp4 共用 demuxer，用扩展名把 .mov 区分出来
        return Some(if ext_is(path, "mov") { "mov" } else { "mp4" }.to_string());
    }
    if list.iter().any(|s| matches!(*s, "matroska" | "webm")) {
        return Some(if ext_is(path, "webm") { "webm" } else { "mkv" }.to_string());
    }
    None
}

fn ext_is(path: &Path, want: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(want))
        .unwrap_or(false)
}

/// 容器名 → `(ffmpeg 的 -f 名, 产物扩展名)`。
///
/// **muxer 名与扩展名不是同一个词**：mkv 的 muxer 叫 `matroska`，写成 `-f mkv`
/// 会直接失败。这张表是唯一的转换点。
pub fn container_muxer(c: &str) -> Option<(&'static str, &'static str)> {
    match c {
        "mp4" => Some(("mp4", "mp4")),
        "mov" => Some(("mov", "mov")),
        "mkv" => Some(("matroska", "mkv")),
        "webm" => Some(("webm", "webm")),
        _ => None,
    }
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

fn video_stream_line(stderr: &str) -> Option<&str> {
    stderr
        .lines()
        .find(|l| l.trim_start().starts_with("Stream #") && l.contains("Video:"))
}

/// 从 `Stream #0:1…: Audio: aac (LC) …` 里取音频编码名。没有音轨时 None。
pub fn parse_audio_codec(stderr: &str) -> Option<String> {
    let line = stderr
        .lines()
        .find(|l| l.trim_start().starts_with("Stream #") && l.contains("Audio:"))?;
    codec_name(line.split("Audio:").nth(1)?)
}

/// 从视频流那一行取 `宽x高`。
///
/// **两个坑**：`(avc1 / 0x31637661)` 里的十六进制 tag 也含 `x`，靠"两个数都 ≥ 16"
/// 排除；`SAR 1:1 DAR 16:9` 用的是冒号，天然不命中。
pub fn parse_dimensions(stderr: &str) -> Option<(u32, u32)> {
    let line = video_stream_line(stderr)?;
    for token in line.split(|c: char| c == ',' || c.is_whitespace()) {
        if let Some((w, h)) = token.split_once('x') {
            if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                if w >= 16 && h >= 16 {
                    return Some((w, h));
                }
            }
        }
    }
    None
}

pub fn parse_media_info(stderr: &str, path: &Path) -> MediaInfo {
    let (width, height) = match parse_dimensions(stderr) {
        Some((w, h)) => (Some(w), Some(h)),
        None => (None, None),
    };
    MediaInfo {
        duration_secs: parse_duration(stderr),
        has_video: parse_has_video(stderr),
        video_codec: parse_video_codec(stderr),
        audio_codec: parse_audio_codec(stderr),
        container: parse_container(stderr, path),
        width,
        height,
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
    Ok(parse_media_info(&String::from_utf8_lossy(&out.stderr), input))
}
