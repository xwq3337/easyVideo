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
    // 直链的 downloaded 本来就是字节数，所以字节计数必须与它同步 —— 界面算速度统一走字节，
    // 两者一旦分叉，直链任务的速度就会凭空变成 0 或翻倍。
    assert_eq!(progress.bytes(), 1024);
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
    let progress = ProgressHandle::new();
    download_direct(
        &reqwest::Client::new(),
        &format!("{}/f.bin", server.uri()),
        &final_path,
        dir.path(),
        Some(1024),
        true,
        2,
        512,
        &progress,
        CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(std::fs::read(&final_path).unwrap(), data);
    // 预置的那一块这一轮没走网络，字节数只该有真正下载的那 512。记成 1024 的话，
    // 用户一"继续下载"就会看到一个凭空冒出来的速度尖峰。
    assert_eq!(progress.bytes(), 512, "续传跳过的块不计入字节数");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rerun_does_not_accumulate_byte_progress() {
    // 与 HLS 同源：ProgressHandle 跨单次运行存活（retry/unpause/resume 会再次调用
    // download_direct）。第二次运行时块已在盘上/已合并，若不把本轮计数归零，就会把上一次
    // 的运行结果再加一遍——Direct 路径上就是 `1.4 GB / 2.0 GB` 那种超过总数的读数。
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
    let url = format!("{}/f.bin", server.uri());

    download_direct(
        &reqwest::Client::new(), &url, &final_path, dir.path(),
        Some(1024), true, 2, 512, &progress, CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(progress.snapshot(), (1024, 1024), "第一次运行结束后 downloaded 必须等于 total");
    assert_eq!(progress.bytes(), 1024, "首次下载：字节数与 downloaded 一致");
    assert_eq!(std::fs::read(&final_path).unwrap(), data);

    // 第二次运行：同一任务目录、同一 handle
    download_direct(
        &reqwest::Client::new(), &url, &final_path, dir.path(),
        Some(1024), true, 2, 512, &progress, CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(
        progress.snapshot(),
        (1024, 1024),
        "重跑后 downloaded 不能累加超过 total（这正是 1.4/2.0 GB 的成因）"
    );
    // 字节计数同理：这一轮的分块是**重新从网络下的**（合并时 .part 已被删掉），
    // 所以字节数应等于本轮真实流量 1024，而不是两轮叠加的 2048。
    // reset() 漏掉 bytes 的话这里正好会得到 2048。
    assert_eq!(progress.bytes(), 1024, "重跑后字节数不能累加到 2048");
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
