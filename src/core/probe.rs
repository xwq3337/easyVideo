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

fn filename_from_url(url: &str) -> String {
    let parsed = url::Url::parse(url).ok();
    let last = parsed
        .as_ref()
        .and_then(|u| u.path_segments())
        .and_then(|mut segs| segs.next_back())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.contains('.'))
        .unwrap_or_else(|| "video.mp4".to_string());
    if last.contains('.') {
        last
    } else {
        format!("{last}.mp4")
    }
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
