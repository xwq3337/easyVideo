use anyhow::Context;
use futures_util::StreamExt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    Direct {
        url: String,
        size: Option<u64>,
        accepts_ranges: bool,
        filename: String,
    },
    Hls {
        url: String,
    },
    WebPage,
}

/// 从 URL 提取下载文件名。这是本项目内该逻辑的唯一实现:
/// engine(Task 13 的 `add_url`)与 GUI(Task 14 的 `add_task`)都调用本函数,
/// 不要各自复制一份。
///
/// 保证:返回值绝不含路径分隔符(`/` 或 `\`),绝不为空,也绝不为 `.` 或 `..`,
/// 因此可安全用于 `dir.join(filename)`。
pub fn filename_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .as_ref()
        .and_then(|u| u.path_segments())
        .and_then(|mut segs| segs.next_back())
        .and_then(sanitize_segment)
        .filter(|s| s.contains('.'))
        .unwrap_or_else(|| "video.mp4".to_string())
}

/// 把 URL 的一个路径段变成安全的文件名。返回 `None` 表示该段不可用,调用方回退默认名。
///
/// **安全要点**:百分号解码会把 `%2F` 变成 `/`、`%5C` 变成 `\`。若原样返回,
/// `https://host/%2e%2e%2fsecret.mp4` 就会得到 `../secret.mp4`,而下游用
/// `dir.join(filename)` 落盘——写入会逃出下载目录。所以解码后只取最后一个路径分量,
/// 并显式拒绝 `.` 与 `..`。
fn sanitize_segment(seg: &str) -> Option<String> {
    let decoded = percent_decode(seg);
    let last = decoded.rsplit(['/', '\\']).next().unwrap_or("");
    let trimmed = last.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }
    Some(trimmed.to_string())
}

/// 将 `%XX` 十六进制转义还原为字节,其余字符原样通过。
/// 非法转义(如 `%ZZ`、结尾被截断的 `%A`)原样保留,不报错也不 panic。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub async fn probe(client: &reqwest::Client, url: &str) -> anyhow::Result<ProbeOutcome> {
    let resp = client
        .get(url)
        .header("Range", "bytes=0-63")
        .send()
        .await
        .context("probe request failed")?;

    let status = resp.status();
    // 先克隆 headers:bytes_stream() 会拿走 resp 的 ownership,之后再读 headers 会 move 报错
    let headers = resp.headers().clone();
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    // 读取首帧(≤64 字节)用于 #EXTM3U 嗅探,随后立即丢弃响应体与连接;
    // 绝不读取整个 body —— 内存占用不得随文件大小增长。
    let mut first_frame = Vec::new();
    let mut stream = resp.bytes_stream();
    if let Some(chunk) = stream.next().await {
        let chunk = chunk.context("probe body read failed")?;
        first_frame = chunk.iter().take(64).copied().collect();
    }
    drop(stream);

    if first_frame.starts_with(b"#EXTM3U")
        || content_type.contains("mpegurl")
        || url
            .split('?')
            .next()
            .unwrap_or("")
            .to_lowercase()
            .ends_with(".m3u8")
    {
        return Ok(ProbeOutcome::Hls {
            url: url.to_string(),
        });
    }

    let is_video = content_type.starts_with("video/")
        || content_type.contains("octet-stream")
        || ["mp4", "webm", "mkv", "mov", "ts"].iter().any(|ext| {
            url.split('?')
                .next()
                .unwrap_or("")
                .to_lowercase()
                .ends_with(&format!(".{ext}"))
        });
    if !is_video || content_type.contains("text/html") {
        return Ok(ProbeOutcome::WebPage);
    }

    let accepts_ranges = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let size = if accepts_ranges {
        parse_total_from_content_range(&headers)
    } else {
        headers
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    };

    Ok(ProbeOutcome::Direct {
        url: url.to_string(),
        size,
        accepts_ranges,
        filename: filename_from_url(url),
    })
}

fn parse_total_from_content_range(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let val = headers.get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    // "bytes 0-63/1048576" 或 "bytes 0-63/*"
    val.split('/').next_back()?.parse::<u64>().ok()
}
