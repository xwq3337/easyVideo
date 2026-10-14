use easydownvideo::core::ytdlp::{find_ytdlp_in, parse_formats};

#[test]
fn parse_formats_keeps_video_entries() {
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ytdlp_dump.json")).unwrap()
    ).unwrap();
    let formats = parse_formats(&json);
    // vcodec=none 的纯音频被剔除
    assert_eq!(formats.len(), 3);
    assert_eq!(formats[0].id, "18");
    assert_eq!(formats[0].note, "360p");
    assert_eq!(formats[0].filesize, Some(1_000_000));
    // 22 号格式是 `"filesize": null` + `"filesize_approx": 2000000` —— 真实 yt-dlp 就这么输出。
    // 必须回退到 approx；写成 `assert_eq!(filesize, None)` 等于把 bug 当成期望值钉死。
    assert_eq!(
        formats[1].filesize,
        Some(2_000_000),
        "filesize 为 null 时必须回退到 filesize_approx（or_else 不会被 Some(Null) 触发）"
    );
    // hls-1 没有 format_note，note 必须回退到 resolution
    assert_eq!(formats[2].note, "1920x1080", "format_note 缺失时应回退到 resolution");
    assert!(formats.iter().all(|f| f.url.starts_with("https://")));
}

#[test]
fn parse_formats_tolerates_missing_fields() {
    let json = serde_json::json!({ "formats": [ { "format_id": "x" } ] });
    assert!(parse_formats(&json).is_empty());
}

#[test]
fn find_ytdlp_scans_path_in_order() {
    // **不能**写成 `let _ = find_ytdlp();` —— 那是断言不了任何东西的空测试。
    // PATH 扫描必须测真行为：跳过没有该文件的目录、在后面的目录里找到、找不到返回 None。
    // 把 PATH 字符串作为参数传入纯函数，测试才能在不改全局环境（会与并行测试打架）的前提下驱动它。
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let target = b.path().join("bin");
    std::fs::create_dir_all(&target).unwrap();
    let exe = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
    std::fs::write(target.join(exe), b"stub").unwrap();

    // 前面的目录里没有该文件，必须继续往后找
    let path_var = std::env::join_paths([a.path(), &target]).unwrap();
    let expected = target.join(exe);
    assert_eq!(
        find_ytdlp_in(&path_var).as_deref(),
        Some(expected.as_path()),
        "应跳过不含该文件的目录，在后面的目录里找到"
    );

    // 完全没有该文件 -> None（而不是 panic）
    let none_var = std::env::join_paths([a.path()]).unwrap();
    assert_eq!(find_ytdlp_in(&none_var), None);
}
