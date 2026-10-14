use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantChoice {
    pub uri: String,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRef {
    pub uri: String,
    pub iv: Option<[u8; 16]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentRef {
    pub uri: String,
    pub key: Option<KeyRef>,
    pub is_init: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaInfo {
    pub segments: Vec<SegmentRef>,
    pub media_sequence: u64,
    pub is_fmp4: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedPlaylist {
    Master { variants: Vec<VariantChoice> },
    Media(MediaInfo),
}

#[derive(Error, Debug)]
pub enum HlsParseError {
    #[error("unsupported key method: {0}")]
    UnsupportedKeyMethod(String),
    #[error("parse error: {0}")]
    Parse(String),
}

pub fn resolve_url(base: &str, relative: &str) -> Option<String> {
    let base_url = url::Url::parse(base).ok()?;
    base_url.join(relative).ok().map(|u| u.to_string())
}

pub fn parse_playlist(base_url: &str, content: &[u8]) -> Result<ParsedPlaylist, HlsParseError> {
    let normalized = patch_iv_less_method_none(content);
    let playlist = m3u8_rs::parse_playlist_res(&normalized)
        .map_err(|e| HlsParseError::Parse(format!("{e:?}")))?;
    match playlist {
        m3u8_rs::Playlist::MasterPlaylist(mp) => {
            let variants = mp
                .variants
                .into_iter()
                .map(|v| VariantChoice {
                    uri: resolve_url(base_url, &v.uri).unwrap_or(v.uri),
                    bandwidth: v.bandwidth,
                    resolution: v.resolution.map(|r| (r.width, r.height)),
                })
                .collect();
            Ok(ParsedPlaylist::Master { variants })
        }
        m3u8_rs::Playlist::MediaPlaylist(mp) => {
            let media_sequence = mp.media_sequence;
            let is_fmp4 = mp.segments.iter().any(|s| s.map.is_some());

            // 按 RFC 8216 §4.3.2.4 实现 EXT-X-KEY 的作用域：一个 KEY 标签对其后**所有**
            // 分片生效，直到出现下一个 KEY 标签。
            //
            // 这件事必须自己做，不能直接用 m3u8-rs 给出的 `segment.key`。**实测确认**：
            // m3u8-rs 6.0.1 只把 key 挂在紧随其后的那一个分片上（它的 parser 在每个 media
            // URI 之后会把 encryption_key 重置为 None）：
            //     一个 KEY + 3 个分片 → 只有 seg[0] 带 key，seg[1]/seg[2] 都是 None
            // 若不在这里补齐，真实播放列表（一个 KEY + 几百个分片）里除第一个分片外全部
            // 不会被解密——下载"成功"，产出的 mp4 却从约第 10 秒起全是乱码，且不报任何错误。
            let mut effective: Vec<Option<KeyRef>> = Vec::with_capacity(mp.segments.len());
            let mut current: Option<KeyRef> = None;
            for seg in mp.segments.iter() {
                if let Some(k) = seg.key.clone() {
                    // 显式设置：METHOD=NONE 经 parse_key 得到 None，表示"自此不再加密"
                    current = parse_key(base_url, k)?;
                }
                // seg.key 为 None 表示沿用上一个 KEY（而非"无加密"）
                effective.push(current.clone());
            }

            let mut segments: Vec<SegmentRef> = Vec::new();

            // init 段（EXT-X-MAP）：AES-128 + fMP4 下初始化段本身也被加密，必须挂上生效的
            // key，否则下载端跳过解密、静默产出损坏文件。RFC 8216 §4.3.2.5 规定加密的初始
            // 化段要求 EXT-X-KEY 带 IV——缺 IV 明确报错，因为按媒体序号推导 IV 对 init 段是错的。
            if let Some(map) = mp.segments.iter().find_map(|s| s.map.clone()) {
                let inherited = effective.iter().find_map(|k| k.clone());
                if inherited.as_ref().is_some_and(|kr| kr.iv.is_none()) {
                    return Err(HlsParseError::Parse(
                        "AES-128 加密的 fMP4 初始化段缺少 IV 属性（RFC 8216 §4.3.2.5）".into(),
                    ));
                }
                segments.push(SegmentRef {
                    uri: resolve_url(base_url, &map.uri).unwrap_or(map.uri),
                    key: inherited,
                    is_init: true,
                });
            }

            for (seg, key) in mp.segments.into_iter().zip(effective) {
                segments.push(SegmentRef {
                    uri: resolve_url(base_url, &seg.uri).unwrap_or(seg.uri),
                    key,
                    is_init: false,
                });
            }
            Ok(ParsedPlaylist::Media(MediaInfo {
                segments,
                media_sequence,
                is_fmp4,
            }))
        }
    }
}

/// 把 m3u8-rs 的 `Key` 转成本项目的 `KeyRef`。
/// 不支持的方法（SAMPLE-AES、未知方法）一律返回错误——绝不静默降级为"无加密"。
fn parse_key(base_url: &str, k: m3u8_rs::Key) -> Result<Option<KeyRef>, HlsParseError> {
    match k.method {
        m3u8_rs::KeyMethod::None => Ok(None),
        m3u8_rs::KeyMethod::AES128 => {
            let uri = k
                .uri
                .ok_or_else(|| HlsParseError::Parse("AES-128 key missing URI".into()))?;
            let iv = match &k.iv {
                Some(hexstr) => Some(parse_iv(hexstr)?),
                None => None,
            };
            Ok(Some(KeyRef {
                uri: resolve_url(base_url, &uri).unwrap_or(uri),
                iv,
            }))
        }
        m3u8_rs::KeyMethod::SampleAES => {
            Err(HlsParseError::UnsupportedKeyMethod("SAMPLE-AES".into()))
        }
        m3u8_rs::KeyMethod::Other(s) => Err(HlsParseError::UnsupportedKeyMethod(s)),
    }
}

fn parse_iv(hexstr: &str) -> Result<[u8; 16], HlsParseError> {
    let s = hexstr.strip_prefix("0x").unwrap_or(hexstr);
    // 先按字节校验长度与字符集，再切分。
    // 注意不能用 `s.len()` 判断"32 个十六进制字符"：`len()` 是字节数，若串里混入多字节
    // UTF-8（畸形 playlist 可以做到），`&s[2*i..2*i+2]` 会落在字符边界之外直接 panic。
    // 按字节遍历 chunks_exact 则完全不涉及 str 边界。
    let bytes = s.as_bytes();
    if bytes.len() != 32 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err(HlsParseError::Parse(format!("bad IV: {hexstr}")));
    }
    let mut out = [0u8; 16];
    for (i, pair) in bytes.chunks_exact(2).enumerate() {
        let hex = std::str::from_utf8(pair)
            .map_err(|_| HlsParseError::Parse(format!("bad IV hex: {hexstr}")))?;
        out[i] = u8::from_str_radix(hex, 16)
            .map_err(|_| HlsParseError::Parse(format!("bad IV hex: {hexstr}")))?;
    }
    Ok(out)
}

/// 给不带 IV 的 `#EXT-X-KEY:METHOD=NONE` 补一个占位 IV，仅用于越过 m3u8-rs 的一处缺陷。
///
/// **实测确认（m3u8-rs 6.0.1）**：`Key::from_hashmap`（playlist.rs:979）的守卫写反了——
/// 它在 `method == None && iv.is_none()` 时报错，而错误信息自述的语义是
/// "IV is required unless METHOD is NONE"。结果是不带 IV 的 `METHOD=NONE` 整行被丢弃：
/// ```text
/// #EXT-X-KEY:METHOD=NONE            ->  seg.key = None, 且标签落入 seg.unknown_tags
/// #EXT-X-KEY:METHOD=NONE,IV=0x00..  ->  seg.key = Some(Key{method: None})  ✓
/// ```
/// 被丢弃后我们既不知道"这里曾经有过一个 NONE"、也不知道它出现在哪个分片之前，于是
/// `METHOD=NONE` 之后的分片会被错误地继续解密——又是一次静默产出乱码。
///
/// 之所以敢直接改文本：占位 IV 对我们是**可证无用**的——`parse_key` 对
/// `KeyMethod::None` 立刻返回 `None`，从不读 iv；URI 等其余属性也不受影响。
/// 且该改写只命中"含 METHOD=NONE 且不含 IV=" 的 `#EXT-X-KEY:` 行：真实加密播放列表的
/// KEY 行是 `METHOD=AES-128,...,IV=0x..`，一次都不会被碰到（空转）。
///
/// 设为 `pub` 仅为让集成测试能直接验证"改写范围足够窄"这一性质。
pub fn patch_iv_less_method_none(content: &[u8]) -> Vec<u8> {
    const PLACEHOLDER_IV: &[u8] = b",IV=0x00000000000000000000000000000000";
    let mut out: Vec<u8> = Vec::with_capacity(content.len());
    for (i, raw_line) in content.split(|&b| b == b'\n').enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        // 单独摘出结尾的 CR，保证占位 IV 插在 CR 之前（否则 IV 会被 CR 截成新的一行）。
        let (line, had_cr) = match raw_line.strip_suffix(b"\r") {
            Some(l) => (l, true),
            None => (raw_line, false),
        };
        out.extend_from_slice(line);
        let is_iv_less_none_key = line.starts_with(b"#EXT-X-KEY:")
            && contains_subslice(line, b"METHOD=NONE")
            && !contains_subslice(line, b"IV=");
        if is_iv_less_none_key {
            out.extend_from_slice(PLACEHOLDER_IV);
        }
        if had_cr {
            out.push(b'\r');
        }
    }
    out
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}
