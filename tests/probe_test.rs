use easydownvideo::core::probe::{probe, ProbeOutcome};
use wiremock::{matchers::{method, path}, Mock, MockServer, ResponseTemplate};

fn client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detects_direct_video_with_range() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/a/b/video.mp4"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/1048576")
                .set_body_bytes(vec![0u8; 64]),
        )
        .mount(&server)
        .await;
    let url = format!("{}/a/b/video.mp4", server.uri());
    match probe(&client(), &url).await.unwrap() {
        ProbeOutcome::Direct { size, accepts_ranges, filename, .. } => {
            assert_eq!(size, Some(1048576));
            assert!(accepts_ranges);
            assert_eq!(filename, "video.mp4");
        }
        other => panic!("expected Direct, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detects_hls_by_body_prefix() {
    let server = MockServer::start().await;
    let body = b"#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:4.0,\nseg0.ts\n";
    Mock::given(method("GET")).and(path("/index.m3u8"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/octet-stream")
                .set_body_bytes(body.to_vec()),
        )
        .mount(&server)
        .await;
    let url = format!("{}/index.m3u8", server.uri());
    match probe(&client(), &url).await.unwrap() {
        ProbeOutcome::Hls { .. } => {}
        other => panic!("expected Hls, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detects_webpage_by_content_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/page"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string("<html></html>"),
        )
        .mount(&server)
        .await;
    let url = format!("{}/page", server.uri());
    match probe(&client(), &url).await.unwrap() {
        ProbeOutcome::WebPage => {}
        other => panic!("expected WebPage, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ignores_ignored_range_and_falls_back() {
    // 服务器忽略 Range 返回 200 全量:仍应判为 Direct,不报错
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/file.mkv"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "video/x-matroska")
                .insert_header("content-length", "2048")
                .set_body_bytes(vec![1u8; 2048]),
        )
        .mount(&server)
        .await;
    let url = format!("{}/file.mkv", server.uri());
    match probe(&client(), &url).await.unwrap() {
        ProbeOutcome::Direct { size, accepts_ranges, .. } => {
            assert_eq!(size, Some(2048));
            assert!(!accepts_ranges);
        }
        other => panic!("expected Direct, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn url_without_filename_uses_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/999")
                .set_body_bytes(vec![0u8; 64]),
        )
        .mount(&server)
        .await;
    let outcome = probe(&client(), &server.uri()).await.unwrap();
    match outcome {
        ProbeOutcome::Direct { filename, .. } => assert_eq!(filename, "video.mp4"),
        other => panic!("expected Direct, got {:?}", other),
    }
}

#[test]
fn filename_from_url_decodes_and_falls_back() {
    use easydownvideo::core::probe::filename_from_url;
    // 百分号解码:URL 里带空格的文件名很常见
    assert_eq!(filename_from_url("https://host/a/My%20Video.mp4"), "My Video.mp4");
    // 查询串不参与文件名
    assert_eq!(filename_from_url("https://host/a/video.mp4?token=abc"), "video.mp4");
    // 无可用文件名 → 回退
    assert_eq!(filename_from_url("https://host/download?id=123"), "video.mp4");
    assert_eq!(filename_from_url("https://host/video"), "video.mp4");
    assert_eq!(filename_from_url("https://host/"), "video.mp4");
    assert_eq!(filename_from_url("not a url"), "video.mp4");
    // 非法转义原样保留,不 panic
    assert_eq!(filename_from_url("https://h/a%ZZb.mp4"), "a%ZZb.mp4");
    // 大写扩展名保留原样
    assert_eq!(filename_from_url("https://h/VIDEO.MP4"), "VIDEO.MP4");
    // 末尾截断的转义符也不 panic
    assert_eq!(filename_from_url("https://h/x%"), "video.mp4");
    assert_eq!(filename_from_url("https://h/x%2"), "video.mp4");
}

#[test]
fn filename_from_url_never_escapes_the_download_dir() {
    use easydownvideo::core::probe::filename_from_url;
    // 百分号解码会把 %2F 变成 '/'、%5C 变成 '\'。返回值绝不允许含路径分隔符,
    // 否则下游 dir.join(filename) 会让写入逃出下载目录。
    for url in [
        "https://h/a%2Fb.mp4",
        "https://h/a%5Cb.mp4",
        "https://h/%2e%2e%2fsecret.mp4",
        "https://h/..%2Fsecret.mp4",
        "https://h/%2e%2e",
        "https://h/..",
        "https://h/.",
    ] {
        let name = filename_from_url(url);
        assert!(
            !name.contains('/') && !name.contains('\\'),
            "{url} 产出含分隔符的文件名: {name:?}"
        );
        assert_ne!(name, "..", "{url} 产出父目录引用");
        assert_ne!(name, ".", "{url} 产出当前目录引用");
        assert!(!name.is_empty(), "{url} 产出空文件名");
    }
    // 具体期望:取最后一个路径分量,保留可用后缀
    assert_eq!(filename_from_url("https://h/a%2Fb.mp4"), "b.mp4");
    assert_eq!(filename_from_url("https://h/%2e%2e%2fsecret.mp4"), "secret.mp4");
    // 纯 `..` 无扩展名,回退默认名
    assert_eq!(filename_from_url("https://h/%2e%2e"), "video.mp4");
}
