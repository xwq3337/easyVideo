//! 浏览器扩展带过来的请求上下文：请求头与建议文件名。
//!
//! **这里是唯一一道闸。** 外部输入（本地 HTTP 接口的 JSON）会直接进 reqwest 请求
//! 和 yt-dlp 命令行，所以不能原样透传：
//!
//! - 请求头里 `Host` / `Content-Length` / `Accept-Encoding` 之类由客户端自己算，
//!   转发过去轻则请求无效，重则让响应体被压缩而我们不解压 —— 落盘的就是乱码。
//! - 文件名会进 `dir.join(filename)`，必须挡掉路径分隔符与 Windows 非法字符。

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

/// 名字 + 值的请求头列表。保持顺序，便于 `--add-headers` 原样转发给 yt-dlp。
pub type Headers = Vec<(String, String)>;

/// 单次请求最多接受多少条头 / 多少字节。正常浏览器的请求远低于这些上限，
/// 设上限只是不给本地接口一个"塞爆内存"的入口。
const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// 由客户端自己决定、不能从浏览器请求里照抄的头。
const BLOCKED: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "te",
    "trailer",
    "upgrade",
    "range",
    // 浏览器捕获时会带上条件请求头。重放它们，服务器可能回 304 Not Modified
    // （没有响应体），而下载路径会把它当正常响应读 —— 静默产出 0 字节文件。
    "if-none-match",
    "if-modified-since",
    "if-match",
    "if-unmodified-since",
    "if-range",
    // 我们没开 reqwest 的 gzip/brotli 解压；转发这个头会让服务端压缩响应，
    // 而我们拿到的是压缩字节，直接写进视频文件。
    "accept-encoding",
    "content-encoding",
];

/// 过滤并规范化外部传来的请求头。
///
/// 非法名字/值（reqwest 的 `HeaderName` / `HeaderValue` 会校验）直接丢弃；
/// 同名头保留最后一条，避免 `Cookie` 被重复发送。
pub fn sanitize_headers(input: &[(String, String)]) -> Headers {
    let mut out: Headers = Vec::new();
    let mut total = 0usize;
    for (name, value) in input {
        if out.len() >= MAX_HEADERS {
            break;
        }
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name.is_empty() || value.is_empty() || value.contains(['\r', '\n']) {
            continue;
        }
        if BLOCKED.contains(&name.as_str()) {
            continue;
        }
        if HeaderName::from_bytes(name.as_bytes()).is_err() || HeaderValue::from_str(value).is_err()
        {
            continue;
        }
        let cost = name.len() + value.len();
        if total + cost > MAX_HEADER_BYTES {
            break;
        }
        out.retain(|(n, _)| n != &name);
        out.push((name, value.to_string()));
        total += cost;
    }
    out
}

/// 把 `extra` 合并进 `base`，同名的以 `extra` 为准（按大小写不敏感匹配）。
///
/// 用途：任务先带着浏览器扩展捕获的页面头（可能有登录 Cookie），选中 yt-dlp 格式后
/// 再叠加该格式自己的 `http_headers`（页面 Referer、站点要求的 UA）。
/// **同名时必须是格式头获胜**：CDN 校验的是 yt-dlp 从播放页里解析出的那个 Referer，
/// 扩展捕获的可能是上一跳页面；而格式头里没有的 Cookie 则必须原样保留。
pub fn merge_headers(base: &[(String, String)], extra: &[(String, String)]) -> Headers {
    let mut out = base.to_vec();
    for (name, value) in extra {
        out.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
        out.push((name.clone(), value.clone()));
    }
    out
}

/// 去掉会跨站泄露的凭据头（`Cookie` / `Authorization`）。
///
/// 浏览器按 cookie 自己的 Domain 决定发给谁。任务 URL 从网页换成 CDN 直链后，
/// 页面 cookie 绝不该跟着去另一个域 —— 交给域作用域 Jar 决定才安全。
pub fn strip_credentials(headers: &[(String, String)]) -> Headers {
    headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case("cookie") && !name.eq_ignore_ascii_case("authorization")
        })
        .cloned()
        .collect()
}

/// 两个 URL 是否同主机（按大小写不敏感比较；任一解析失败都视为不同）。
///
/// 只比较主机名而不比较注册域：没有 Public Suffix List 就无法可靠判断
/// `a.example.com` 与 `b.example.com` 是否同站，宁可保守地剥掉凭据，
/// 由 `reqwest` 的 Jar 按 cookie 自己的 Domain 重新决定该带什么。
pub fn same_host(left: &str, right: &str) -> bool {
    let host = |raw: &str| {
        url::Url::parse(raw)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
    };
    matches!((host(left), host(right)), (Some(a), Some(b)) if a == b)
}

/// 转成 reqwest 的 `HeaderMap`。所有头都已在 [`sanitize_headers`] 里校验过，
/// 这里再校验一次只是为了让转换不可能 panic。
pub fn header_map(headers: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        map.insert(name, value);
    }
    map
}

/// Windows 文件名里不能出现的字符。`/` 与 `\` 同时是路径分隔符 —— 不挡掉的话
/// `../../x.mp4` 会直接写出下载目录。
const ILLEGAL: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Windows 保留设备名（不区分大小写；带扩展名时同样保留，例如 `CON.txt`）。
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

const MAX_FILENAME_CHARS: usize = 120;

/// 把外部建议的文件名变成一个可以安全 `dir.join(...)` 的名字。
///
/// 只取最后一段、替换非法字符、去掉首尾的点和空白、避开保留设备名，并按字符数
/// 截断（保留扩展名）。不合法时返回 `None`，调用方回退到 URL 推导出来的名字。
pub fn sanitize_filename(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if last.is_empty() {
        return None;
    }
    let replaced: String = last
        .chars()
        .map(|c| {
            if ILLEGAL.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    // 连续空白压成一个空格；中文标题里的普通空格保留。
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }

    let stem_upper = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    let name = if RESERVED.contains(&stem_upper.as_str()) {
        format!("_{trimmed}")
    } else {
        trimmed.to_string()
    };
    Some(truncate_preserving_extension(&name, MAX_FILENAME_CHARS))
}

fn truncate_preserving_extension(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let path = std::path::Path::new(name);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().count() <= 10)
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let keep = max_chars.saturating_sub(ext.chars().count());
    let stem: String = stem.chars().take(keep).collect();
    format!("{stem}{ext}")
}

/// 允许下载的 URL：只接受 http/https。
///
/// `file://` 会让本地接口变成一个"读任意文件"的入口；`data:` / `blob:` 也没有可
/// 下载的实体。扩展本来就只发 http(s)，这里再挡一次。
pub fn is_downloadable_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")) && lower.len() > 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_hop_by_hop_and_encoding_headers() {
        let headers = sanitize_headers(&[
            ("Host".into(), "evil.example".into()),
            ("Content-Length".into(), "10".into()),
            ("Accept-Encoding".into(), "gzip".into()),
            ("Range".into(), "bytes=0-1".into()),
            ("If-None-Match".into(), "\"abc\"".into()),
            (
                "If-Modified-Since".into(),
                "Wed, 21 Oct 2015 07:28:00 GMT".into(),
            ),
            ("Referer".into(), "https://example.com/page".into()),
            ("Cookie".into(), "sid=abc".into()),
        ]);
        assert_eq!(
            headers,
            vec![
                ("referer".into(), "https://example.com/page".into()),
                ("cookie".into(), "sid=abc".into()),
            ]
        );
    }

    /// 选中 yt-dlp 格式时的头合并：格式的 Referer 必须盖掉扩展捕获的那个，
    /// 而扩展那边独有的 Cookie 不能被丢。
    #[test]
    fn merge_prefers_format_headers_but_keeps_captured_cookies() {
        let captured = vec![
            ("referer".into(), "https://www.bilibili.com/".into()),
            ("cookie".into(), "SESSDATA=abc".into()),
            ("user-agent".into(), "captured-ua".into()),
        ];
        let format = vec![
            (
                "Referer".into(),
                "https://www.bilibili.com/video/BV1xx".into(),
            ),
            ("User-Agent".into(), "format-ua".into()),
            ("Origin".into(), "https://www.bilibili.com".into()),
        ];
        let merged = merge_headers(&captured, &format);
        assert_eq!(
            merged,
            vec![
                ("cookie".into(), "SESSDATA=abc".into()),
                (
                    "Referer".into(),
                    "https://www.bilibili.com/video/BV1xx".into()
                ),
                ("User-Agent".into(), "format-ua".into()),
                ("Origin".into(), "https://www.bilibili.com".into()),
            ]
        );
    }

    /// 换到另一个主机下载时，页面凭据必须剥掉；Referer/UA 这类不敏感的上下文保留。
    #[test]
    fn cross_host_switch_strips_captured_credentials() {
        let captured = vec![
            ("cookie".into(), "SESSDATA=secret".into()),
            ("Authorization".into(), "Bearer t".into()),
            ("Referer".into(), "https://www.bilibili.com/video/BV1xx".into()),
            ("user-agent".into(), "ua".into()),
        ];
        assert_eq!(
            strip_credentials(&captured),
            vec![
                (
                    "Referer".into(),
                    "https://www.bilibili.com/video/BV1xx".into()
                ),
                ("user-agent".into(), "ua".into()),
            ]
        );
    }

    #[test]
    fn same_host_compares_case_insensitively_and_rejects_unparsable() {
        assert!(same_host(
            "https://WWW.Bilibili.com/video/BV1xx",
            "https://www.bilibili.com/watch"
        ));
        assert!(!same_host(
            "https://www.bilibili.com/video/BV1xx",
            "https://upos-sz.bilivideo.com/v/1.m4s"
        ));
        assert!(!same_host("not a url", "https://example.com/"));
        assert!(!same_host("https://example.com/", "not a url"));
    }

    #[test]
    fn keeps_custom_auth_headers_and_dedupes_last_value() {
        let headers = sanitize_headers(&[
            ("X-Auth-Token".into(), "one".into()),
            ("x-auth-token".into(), "two".into()),
            ("Authorization".into(), "Bearer t".into()),
        ]);
        assert_eq!(
            headers,
            vec![
                ("x-auth-token".into(), "two".into()),
                ("authorization".into(), "Bearer t".into()),
            ]
        );
    }

    #[test]
    fn rejects_crlf_and_invalid_header_names() {
        let headers = sanitize_headers(&[
            ("X-Bad".into(), "a\r\nInjected: 1".into()),
            ("bad header".into(), "x".into()),
            ("X-Good".into(), "ok".into()),
        ]);
        assert_eq!(headers, vec![("x-good".into(), "ok".into())]);
    }

    #[test]
    fn filename_strips_paths_and_illegal_chars() {
        assert_eq!(
            sanitize_filename("../../etc/passwd").as_deref(),
            Some("passwd")
        );
        assert_eq!(
            sanitize_filename(r#"我的视频:第一集?.mp4"#).as_deref(),
            Some("我的视频_第一集_.mp4")
        );
        assert_eq!(sanitize_filename("  ..  "), None);
        assert_eq!(sanitize_filename(""), None);
    }

    #[test]
    fn filename_avoids_windows_reserved_names() {
        assert_eq!(sanitize_filename("CON").as_deref(), Some("_CON"));
        assert_eq!(sanitize_filename("con.txt").as_deref(), Some("_con.txt"));
        assert_eq!(
            sanitize_filename("console.mp4").as_deref(),
            Some("console.mp4")
        );
    }

    #[test]
    fn filename_truncation_keeps_extension() {
        let long = format!("{}.mp4", "长".repeat(300));
        let out = sanitize_filename(&long).unwrap();
        assert!(out.ends_with(".mp4"));
        assert_eq!(out.chars().count(), MAX_FILENAME_CHARS);
    }

    #[test]
    fn only_http_urls_are_downloadable() {
        assert!(is_downloadable_url("https://example.com/a.mp4"));
        assert!(is_downloadable_url("http://127.0.0.1:1/a"));
        assert!(!is_downloadable_url("file:///etc/passwd"));
        assert!(!is_downloadable_url("javascript:alert(1)"));
    }
}
