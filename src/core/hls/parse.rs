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
    let playlist = m3u8_rs::parse_playlist_res(content)
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
            let mut segments: Vec<SegmentRef> = Vec::new();

            // init 段（EXT-X-MAP）：在 AES-128 + fMP4 下它同样是被加密的，必须挂上当前
            // 生效的 key，否则下载端会跳过解密、静默产出损坏文件而不报错。
            // RFC 8216 §4.3.2.5 规定：加密的初始化段要求 EXT-X-KEY 带 IV 属性——
            // 缺 IV 时我们明确报错，因为按媒体序号推导 IV 对 init 段是错的。
            if let Some(map) = mp.segments.iter().find_map(|s| s.map.clone()) {
                let inherited = match mp.segments.iter().find_map(|s| s.key.clone()) {
                    Some(k) => {
                        let parsed = parse_key(base_url, k)?;
                        if parsed.as_ref().is_some_and(|kr| kr.iv.is_none()) {
                            return Err(HlsParseError::Parse(
                                "AES-128 加密的 fMP4 初始化段缺少 IV 属性（RFC 8216 §4.3.2.5）"
                                    .into(),
                            ));
                        }
                        parsed
                    }
                    None => None,
                };
                segments.push(SegmentRef {
                    uri: resolve_url(base_url, &map.uri).unwrap_or(map.uri),
                    key: inherited,
                    is_init: true,
                });
            }

            for seg in mp.segments.into_iter() {
                let key = match seg.key {
                    Some(k) => parse_key(base_url, k)?,
                    None => None,
                };
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
