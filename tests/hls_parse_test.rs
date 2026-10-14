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
