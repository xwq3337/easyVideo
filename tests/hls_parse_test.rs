use easydownvideo::core::hls::parse::{parse_playlist, resolve_url, ParsedPlaylist};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)).unwrap()
}

#[test]
fn parses_master_variants() {
    let p = parse_playlist("https://cdn.example.com/live/master.m3u8", fixture("master.m3u8").as_bytes()).unwrap();
    match p {
        ParsedPlaylist::Master { variants } => {
            assert_eq!(variants.len(), 2);
            assert_eq!(variants[0].resolution, Some((640, 360)));
            assert_eq!(variants[1].bandwidth, 2800000);
            assert_eq!(variants[1].uri, "https://cdn.example.com/live/high/index.m3u8");
        }
        _ => panic!("expected Master"),
    }
}

#[test]
fn parses_media_with_aes_key() {
    let p = parse_playlist("https://cdn.example.com/a/media.m3u8", fixture("media_aes.m3u8").as_bytes()).unwrap();
    match p {
        ParsedPlaylist::Media(m) => {
            assert_eq!(m.media_sequence, 7);
            assert!(!m.is_fmp4);
            assert_eq!(m.segments.len(), 2);
            let key = m.segments[0].key.as_ref().unwrap();
            assert_eq!(key.uri, "https://keys.example.com/k.bin");
            let mut expected_iv = [0u8; 16];
            expected_iv[15] = 5; // IV=0x...0005
            assert_eq!(key.iv, Some(expected_iv));
            assert!(!m.segments[0].is_init);
            // 关键：同一个 KEY 必须对**后面所有**分片继续生效。
            // m3u8-rs 只把 key 挂在紧随其后的那一个分片上，这里断了就等于后面全不解密。
            assert!(
                m.segments[1].key.is_some(),
                "第二个分片必须继承同一个 KEY（RFC 8216 §4.3.2.4）"
            );
            assert_eq!(m.segments[1].key.as_ref().unwrap().uri, "https://keys.example.com/k.bin");
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn parses_fmp4_with_init_segment() {
    let p = parse_playlist("https://cdn.example.com/f/", fixture("media_fmp4.m3u8").as_bytes()).unwrap();
    match p {
        ParsedPlaylist::Media(m) => {
            assert!(m.is_fmp4);
            assert_eq!(m.segments.len(), 3); // init + 2 fragments
            assert!(m.segments[0].is_init);
            assert_eq!(m.segments[0].uri, "https://cdn.example.com/f/init.mp4");
            assert_eq!(m.segments[2].uri, "https://cdn.example.com/f/s2.m4s");
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn resolve_url_handles_relative_and_protocol_relative() {
    let base = "https://cdn.example.com/a/b/index.m3u8";
    assert_eq!(resolve_url(base, "seg.ts").as_deref(), Some("https://cdn.example.com/a/b/seg.ts"));
    assert_eq!(resolve_url(base, "../seg.ts").as_deref(), Some("https://cdn.example.com/a/seg.ts"));
    assert_eq!(resolve_url(base, "//other.com/seg.ts").as_deref(), Some("https://other.com/seg.ts"));
    assert_eq!(resolve_url(base, "https://x.com/y.ts").as_deref(), Some("https://x.com/y.ts"));
}

#[test]
fn rejects_sample_aes_explicitly() {
    let content = b"#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://k/x\"\n#EXTINF:1,\na.ts\n";
    let err = parse_playlist("https://e.com/m.m3u8", content).unwrap_err();
    assert!(err.to_string().contains("SAMPLE-AES"));
}

#[test]
fn malformed_iv_errors_instead_of_panicking() {
    // 32 字节但含多字节 UTF-8：按字节切分不会 panic，必须返回 Err
    let mut content = String::from("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=");
    content.push_str(&"€".repeat(10)); // 3 字节 × 10 = 30 字节，再加 2 字节
    content.push_str("xy"); // 共 32 字节
    content.push_str("\n#EXTINF:1,\na.ts\n");
    let err = parse_playlist("https://e.com/m.m3u8", content.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("bad IV"), "实际错误: {err}");

    // 长度不对
    let short = b"#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=0x1234\n#EXTINF:1,\na.ts\n";
    assert!(parse_playlist("https://e.com/m.m3u8", short).is_err());
}

#[test]
fn encrypted_fmp4_init_segment_inherits_key() {
    // fMP4 + AES-128：EXT-X-MAP 的初始化段本身也被加密，必须挂上 key 且要求有 IV。
    // KEY 写在 MAP 之前（规范里最常见的顺序）。
    let with_iv = b"#EXTM3U\n#EXT-X-VERSION:7\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=0x00000000000000000000000000000001\n\
#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.0,\ns0.m4s\n#EXT-X-ENDLIST\n";
    match parse_playlist("https://c/f/i.m3u8", with_iv).unwrap() {
        ParsedPlaylist::Media(m) => {
            assert!(m.is_fmp4);
            let init = &m.segments[0];
            assert!(init.is_init);
            let k = init.key.as_ref().expect("init 段必须继承 key");
            assert_eq!(k.uri, "https://k/x");
            let mut expect_iv = [0u8; 16];
            expect_iv[15] = 1;
            assert_eq!(k.iv, Some(expect_iv));
        }
        _ => panic!("expected Media"),
    }

    // 缺 IV：按 RFC 8216 §4.3.2.5 必须明确报错，绝不能按媒体序号推导 IV 解出错的内容
    let no_iv = b"#EXTM3U\n#EXT-X-VERSION:7\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\"\n\
#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.0,\ns0.m4s\n#EXT-X-ENDLIST\n";
    let err = parse_playlist("https://c/f/i.m3u8", no_iv).unwrap_err();
    assert!(err.to_string().contains("IV"), "实际错误: {err}");
}

#[test]
fn rejects_unknown_key_method() {
    let content = b"#EXTM3U\n#EXT-X-KEY:METHOD=AES-256,URI=\"https://k/x\"\n#EXTINF:1,\na.ts\n";
    let err = parse_playlist("https://e.com/m.m3u8", content).unwrap_err();
    assert!(err.to_string().contains("AES-256"), "实际错误: {err}");
}

#[test]
fn key_applies_to_every_following_segment() {
    // 实测 m3u8-rs 6.0.1 只给第一个分片带 key：一个 KEY + 3 个分片时只有 seg[0] 有 key。
    // 我们要按 RFC 补齐：KEY 对其后所有分片生效，直到下一个 KEY 标签。
    let content = b"#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=0x00000000000000000000000000000009\n\
#EXTINF:9.9,\na.ts\n#EXTINF:9.9,\nb.ts\n#EXTINF:9.9,\nc.ts\n#EXT-X-ENDLIST\n";
    match parse_playlist("https://c/m.m3u8", content).unwrap() {
        ParsedPlaylist::Media(m) => {
            assert_eq!(m.segments.len(), 3);
            for (i, seg) in m.segments.iter().enumerate() {
                let k = seg.key.as_ref().unwrap_or_else(|| {
                    panic!("分片 {i} 丢了 key —— 真实播放列表里这些分片将不被解密")
                });
                assert_eq!(k.uri, "https://k/x", "分片 {i} 的 key 应沿用同一个");
            }
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn new_key_replaces_previous_for_later_segments() {
    let content = b"#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/1.bin\"\n#EXTINF:9.9,\na.ts\n#EXTINF:9.9,\nb.ts\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/2.bin\"\n#EXTINF:9.9,\nc.ts\n#EXT-X-ENDLIST\n";
    match parse_playlist("https://c/m.m3u8", content).unwrap() {
        ParsedPlaylist::Media(m) => {
            let uri = |i: usize| m.segments[i].key.as_ref().map(|k| k.uri.clone());
            assert_eq!(uri(0).as_deref(), Some("https://k/1.bin"));
            assert_eq!(uri(1).as_deref(), Some("https://k/1.bin"), "b 应沿用第一个 key");
            assert_eq!(uri(2).as_deref(), Some("https://k/2.bin"), "新 KEY 应替换旧的");
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn method_none_clears_the_key_for_later_segments() {
    // METHOD=NONE 表示"自此不再加密"，不能被当成"沿用上一个 key"
    let content = b"#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\"\n#EXTINF:9.9,\na.ts\n\
#EXT-X-KEY:METHOD=NONE\n#EXTINF:9.9,\nb.ts\n#EXT-X-ENDLIST\n";
    match parse_playlist("https://c/m.m3u8", content).unwrap() {
        ParsedPlaylist::Media(m) => {
            assert!(m.segments[0].key.is_some(), "第一个分片应加密");
            assert!(m.segments[1].key.is_none(), "METHOD=NONE 之后不应再解密");
        }
        _ => panic!("expected Media"),
    }
}
