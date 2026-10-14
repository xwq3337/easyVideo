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
            if let Some(map) = mp.segments.iter().find_map(|s| s.map.clone()) {
                segments.push(SegmentRef {
                    uri: resolve_url(base_url, &map.uri).unwrap_or(map.uri),
                    key: None,
                    is_init: true,
                });
            }
            for seg in mp.segments.into_iter() {
                let key = match seg.key {
                    Some(k) => match k.method {
                        m3u8_rs::KeyMethod::None => None,
                        m3u8_rs::KeyMethod::AES128 => {
                            let uri = k.uri.ok_or_else(|| {
                                HlsParseError::Parse("AES-128 key missing URI".into())
                            })?;
                            let iv = match &k.iv {
                                Some(hexstr) => Some(parse_iv(hexstr)?),
                                None => None,
                            };
                            Some(KeyRef {
                                uri: resolve_url(base_url, &uri).unwrap_or(uri),
                                iv,
                            })
                        }
                        m3u8_rs::KeyMethod::SampleAES => {
                            return Err(HlsParseError::UnsupportedKeyMethod("SAMPLE-AES".into()))
                        }
                        m3u8_rs::KeyMethod::Other(s) => {
                            return Err(HlsParseError::UnsupportedKeyMethod(s))
                        }
                    },
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

fn parse_iv(hexstr: &str) -> Result<[u8; 16], HlsParseError> {
    let s = hexstr.strip_prefix("0x").unwrap_or(hexstr);
    if s.len() != 32 {
        return Err(HlsParseError::Parse(format!("bad IV length: {hexstr}")));
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
            .map_err(|_| HlsParseError::Parse(format!("bad IV hex: {hexstr}")))?;
    }
    Ok(out)
}
