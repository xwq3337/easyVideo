use easydownvideo::core::direct::{download_direct, plan_blocks, unique_path, MIN_BLOCK_SIZE};
use easydownvideo::core::events::ProgressHandle;
use tokio_util::sync::CancellationToken;
use wiremock::{matchers::{header, method, path}, Mock, MockServer, ResponseTemplate};

#[test]
fn plan_blocks_respects_min_block_and_concurrency() {
    // 100 / 4，min=1 → 每块 25，正好 4 块
    assert_eq!(plan_blocks(100, 4, 1), vec![(0, 25), (25, 50), (50, 75), (75, 100)]);
    // 小文件：min_block 主导，块数 < concurrency
    assert_eq!(plan_blocks(10, 4, 4), vec![(0, 4), (4, 8), (8, 10)]);
    // 恰好整除
    assert_eq!(plan_blocks(8, 4, 1), vec![(0, 2), (2, 4), (4, 6), (6, 8)]);
    assert!(MIN_BLOCK_SIZE >= 4 * 1024 * 1024);
}

#[test]
fn unique_path_increments_on_conflict() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(unique_path(dir.path(), "a.mp4"), dir.path().join("a.mp4"));
    std::fs::write(dir.path().join("a.mp4"), b"x").unwrap();
    assert_eq!(unique_path(dir.path(), "a.mp4"), dir.path().join("a (1).mp4"));
    std::fs::write(dir.path().join("a (1).mp4"), b"x").unwrap();
    assert_eq!(unique_path(dir.path(), "a.mp4"), dir.path().join("a (2).mp4"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_in_blocks_and_merges() {
    let data: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/f.bin")).and(header("range", "bytes=0-511"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-range", "bytes 0-511/1024")
            .set_body_bytes(data[0..512].to_vec()))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/f.bin")).and(header("range", "bytes=512-1023"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-range", "bytes 512-1023/1024")
            .set_body_bytes(data[512..].to_vec()))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("f.bin");
    let progress = ProgressHandle::new();
    download_direct(
        &reqwest::Client::new(),
        &format!("{}/f.bin", server.uri()),
        &final_path,
        dir.path(),
        Some(1024),
        true,
        2,
        512, // 测试用小 min_block
        &progress,
        CancellationToken::new(),
    ).await.unwrap();

    assert_eq!(std::fs::read(&final_path).unwrap(), data);
    assert_eq!(progress.snapshot(), (1024, 1024));
    assert!(std::fs::read_dir(dir.path()).unwrap().count() == 1); // .part 已清理
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumes_completed_parts() {
    let data: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
    let server = MockServer::start().await;
    // 只有第二个 range 有 mock：第一个 part 若被重新请求则 404 → 测试证明跳过了它
    Mock::given(method("GET")).and(path("/f.bin")).and(header("range", "bytes=512-1023"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-range", "bytes 512-1023/1024")
            .set_body_bytes(data[512..].to_vec()))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    // 预置已完成的第一块
    std::fs::write(dir.path().join("f.part-0"), &data[0..512]).unwrap();

    let final_path = dir.path().join("f.bin");
    download_direct(
        &reqwest::Client::new(),
        &format!("{}/f.bin", server.uri()),
        &final_path,
        dir.path(),
        Some(1024),
        true,
        2,
        512,
        &ProgressHandle::new(),
        CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(std::fs::read(&final_path).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_stops_download() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/slow.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 4096]))
        .mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let r = download_direct(
        &reqwest::Client::new(),
        &format!("{}/slow.bin", server.uri()),
        &dir.path().join("slow.bin"),
        dir.path(),
        Some(4096),
        false,
        1,
        MIN_BLOCK_SIZE,
        &ProgressHandle::new(),
        token,
    ).await;
    assert!(r.is_err());
}
