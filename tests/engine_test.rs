use easydownvideo::core::engine::Engine;
use easydownvideo::core::events::{Choice, EngineEvent, TaskState};
use easydownvideo::core::task::{ManifestKind, TaskManifest};
use easydownvideo::settings::Settings;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn settings_with(dir: &std::path::Path) -> Settings {
    Settings {
        download_dir: dir.to_path_buf(),
        segment_concurrency: 2,
        max_active_tasks: 1,
        ytdlp_enabled: false,
        ffmpeg_path_override: None,
    }
}

async fn wait_failed(
    rx: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    id: u64,
    secs: u64,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let ev = tokio::time::timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        )
        .await
        .expect("timeout waiting for failure");
        if let Ok(EngineEvent::StateChanged {
            id: eid,
            state: TaskState::Failed(_),
        }) = ev
        {
            if eid == id {
                return;
            }
        }
    }
}

async fn wait_state(
    rx: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    id: u64,
    want: TaskState,
    secs: u64,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let ev = tokio::time::timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        )
        .await
        .expect("timeout waiting for state");
        match ev {
            Ok(EngineEvent::StateChanged { id: eid, state }) if eid == id => {
                if state == want {
                    return;
                }
                if let TaskState::Failed(_) = state {
                    panic!("task failed while waiting for {want:?}: {state:?}");
                }
            }
            _ => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn end_to_end_direct_download() {
    let data: Vec<u8> = (0..2048u32).map(|i| (i % 253) as u8).collect();
    let server = MockServer::start().await;
    // probe 请求
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-63"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/2048")
                .set_body_bytes(data[0..64].to_vec()),
        )
        .mount(&server)
        .await;
    // 引擎用 MIN_BLOCK_SIZE(4MB)，2048 字节的小文件只会切出 1 个块 → 单次全量 range 请求
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-2047"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-2047/2048")
                .set_body_bytes(data.clone()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let url = format!("{}/v.mp4", server.uri());
    let id = engine.add_url(url);

    wait_state(&mut rx, id, TaskState::Done, 30).await;
    let saved = dir.path().join("v.mp4");
    assert_eq!(std::fs::read(&saved).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failure_does_not_block_queue() {
    let server = MockServer::start().await;
    // 坏 URL 直接 404
    Mock::given(method("GET"))
        .and(path("/missing.mp4"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    // 好 URL 可完整下载（512 字节 → 1 个块 → 单次全量 range）
    let data: Vec<u8> = (0..512u32).map(|i| (i % 251) as u8).collect();
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-63"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/512")
                .set_body_bytes(data[0..64].to_vec()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-511"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-511/512")
                .set_body_bytes(data.clone()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let bad = engine.add_url(format!("{}/missing.mp4", server.uri()));
    let good = engine.add_url(format!("{}/v.mp4", server.uri()));

    // 坏任务先失败（max_active=1，它占着槽位）
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let ev = tokio::time::timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        )
        .await
        .expect("timeout");
        if let Ok(EngineEvent::StateChanged {
            id,
            state: TaskState::Failed(_),
        }) = ev
        {
            if id == bad {
                break;
            }
        }
    }
    // 好任务随后照常完成，证明队列未被失败阻塞
    wait_state(&mut rx, good, TaskState::Done, 30).await;
    assert!(dir.path().join("v.mp4").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn existing_file_gets_renamed() {
    let data: Vec<u8> = (0..128u32).map(|i| i as u8).collect();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-63"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/128")
                .set_body_bytes(data[0..64].to_vec()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "video/mp4")
                .set_body_bytes(data.clone()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("v.mp4"), b"existing").unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/v.mp4", server.uri()));
    wait_state(&mut rx, id, TaskState::Done, 30).await;
    assert_eq!(std::fs::read(dir.path().join("v.mp4")).unwrap(), b"existing");
    assert!(dir.path().join("v (1).mp4").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_settings_changes_concurrency() {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut s = settings_with(dir.path());
    s.segment_concurrency = 12;
    s.ytdlp_enabled = false;
    engine.update_settings(s);
    assert_eq!(engine.current_settings().segment_concurrency, 12);
    assert!(!engine.current_settings().ytdlp_enabled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_cleans_resume_artifacts() {
    // 取消是"不要了"：断点目录必须清掉，否则下次启动 scan_resume 会把它复活重下
    let server = MockServer::start().await;
    let data: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-63"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/4096")
                .set_body_bytes(data[0..64].to_vec()),
        )
        .mount(&server)
        .await;
    // 全量请求延迟返回，留出取消窗口
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-4095/4096")
                .set_body_bytes(data.clone())
                .set_delay(std::time::Duration::from_millis(1500)),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/v.mp4", server.uri()));

    // 进入 Downloading 说明 manifest 已落盘
    wait_state(&mut rx, id, TaskState::Downloading, 10).await;
    engine.cancel(id);
    wait_state(&mut rx, id, TaskState::Canceled, 10).await;

    // 清理发生在 worker 退出路径，轮询等待（避免依赖固定 sleep 时长）
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if easydownvideo::core::task::scan_resume(dir.path()).is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "取消后仍残留可恢复的 manifest，重启会把已取消任务复活"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!easydownvideo::core::task::task_dir(dir.path(), id).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumed_ids_do_not_collide_with_new_tasks() {
    // 恢复的任务带着它原来的 id，而 add_url 从 1 开始分配。不抬高 next_id，
    // 新增任务会拿到已被占用的 id，insert_entry 直接把它覆盖 —— 恢复的任务无声消失。
    //
    // max_active_tasks = 0 让任何 worker 都不启动：这个测试只关心 id 分配，
    // 不该联网，也不该受下载时序影响。
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s = settings_with(dir.path());
    s.max_active_tasks = 0;
    let engine = Engine::new(rt, s, dir.path().join("data"));

    engine.resume(TaskManifest {
        id: 7,
        url: "https://example.com/a.mp4".into(),
        filename: "a.mp4".into(),
        download_dir: dir.path().to_path_buf(),
        kind: ManifestKind::Direct { size: Some(10) },
    });

    let fresh = engine.add_url("https://example.com/b.mp4".into());
    assert_ne!(fresh, 7, "add_url 分配到了已被恢复任务占用的 id");
    assert_eq!(
        engine.task_info(7).map(|(_, f)| f).as_deref(),
        Some("a.mp4"),
        "恢复出来的任务被新增任务覆盖了"
    );
    assert_eq!(engine.list_tasks().len(), 2, "两个任务都应存在");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_while_queued_is_not_resurrected() {
    // 取消发生在"排队期间"时，id 仍留在 VecDeque 里。pump 启动任务前必须检查状态，
    // 否则用户点了取消，任务随后自己复活并开始下载。
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s0 = settings_with(dir.path());
    s0.max_active_tasks = 0; // 先让任务停在队列里
    let engine = Engine::new(rt, s0, dir.path().join("data"));

    let id = engine.add_url("https://example.com/x.mp4".into());
    engine.cancel(id);

    // 提高并发上限触发 pump —— 这是会把它复活的那条路径
    let mut s1 = settings_with(dir.path());
    s1.max_active_tasks = 1;
    engine.update_settings(s1);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let snap = engine
        .list_tasks()
        .into_iter()
        .find(|t| t.id == id)
        .expect("任务应仍在列表中");
    assert_eq!(
        snap.state,
        TaskState::Canceled,
        "被取消的排队任务被 pump 复活了（状态变成 {:?}）",
        snap.state
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retry_requeues_a_failed_task() {
    // 失败的"重试"必须真的重新跑一遍，而不是只把状态字段改一改、或者干脆没反应。
    // 用一个恒定 404 的 URL：重试成功的话会**再次**失败，于是能等到第二个 Failed 事件。
    // （GUI 的"重试"按钮曾经调的是 unpause，而 unpause 只认 Paused —— 死按钮。）
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing.mp4"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/missing.mp4", server.uri()));

    wait_failed(&mut rx, id, 30).await; // 第一次失败
    engine.retry(id);
    wait_failed(&mut rx, id, 30).await; // 重试后第二次失败 —— 证明它真的重跑了
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_queued_resumed_task_removes_its_manifest() {
    // `resume()` 会**立刻**把 manifest 写盘（insert_entry 的 eager write）。若这个任务因为并发
    // 槽位已满一直排队（max_active=0），就没有任何 worker 会跑收尾清理 —— 清理原先只写在
    // worker 的错误路径里。cancel 自己不删目录的话，manifest 留在盘上，下次启动就被复活。
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s = settings_with(dir.path());
    s.max_active_tasks = 0;
    let engine = Engine::new(rt, s, dir.path().join("data"));

    let id = 5u64;
    engine.resume(TaskManifest {
        id,
        url: "https://example.com/a.mp4".into(),
        filename: "a.mp4".into(),
        download_dir: dir.path().to_path_buf(),
        kind: ManifestKind::Direct { size: Some(10) },
    });
    // **前提断言**：manifest 确实已经落盘。没有这一条，下面的断言可能因为"压根没写过"而假通过。
    assert_eq!(
        easydownvideo::core::task::scan_resume(dir.path()).len(),
        1,
        "resume 应当立刻写盘；否则这个测试不成立"
    );

    engine.cancel(id);

    assert!(
        easydownvideo::core::task::scan_resume(dir.path()).is_empty(),
        "取消排队中的恢复任务后 manifest 仍在盘上，下次启动会被 scan_resume 复活"
    );
    assert!(!easydownvideo::core::task::task_dir(dir.path(), id).exists());
}

/// 引擎的 HLS 接线（playlist → 分片 → ffmpeg 合并 → Done → 清目录）此前**完全没有测试**，
/// 只做到"能编译"。假 `.ts` 字节不行：ffmpeg 的 concat 解复用器会拒绝非法 TS，合并直接失败。
/// 所以用 `tests/fixtures/seg.ts` —— 一个真实的小分片（160x120 h264，1 秒，约 10KB）。
fn ts_fixture() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seg.ts"))
        .expect("缺少 tests/fixtures/seg.ts")
}

const MEDIA_PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:1.0,\nseg0.ts\n#EXTINF:1.0,\nseg1.ts\n#EXT-X-ENDLIST\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_media_playlist_end_to_end() {
    let ts = ts_fixture();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media.m3u8"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.apple.mpegurl")
                .set_body_string(MEDIA_PLAYLIST),
        )
        .mount(&server)
        .await;
    for seg in ["/seg0.ts", "/seg1.ts"] {
        Mock::given(method("GET"))
            .and(path(seg))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp2t")
                    .set_body_bytes(ts.clone()),
            )
            .mount(&server)
            .await;
    }

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/media.m3u8", server.uri()));

    // 合并要跑真实 ffmpeg（首次会把内嵌二进制释放到 data_dir），给足时间
    wait_state(&mut rx, id, TaskState::Done, 120).await;

    let out = dir.path().join("media.mp4");
    let meta = std::fs::metadata(&out)
        .unwrap_or_else(|e| panic!("合并产物 {} 应当存在: {e}", out.display()));
    assert!(meta.len() > 0, "合并产物不应为空");
    assert!(
        !easydownvideo::core::task::task_dir(dir.path(), id).exists(),
        "完成后任务目录（manifest/分片/concat 列表）必须清掉，否则会被 scan_resume 复活"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_master_playlist_offers_choice_then_completes() {
    // 真实世界的 m3u8 大多是 master playlist：先停在 AwaitingChoice 等用户选码率，
    // 再经 set_choice 换 URL 重新入队。这条接线（含 set_choice）同样没有测试。
    let ts = ts_fixture();
    let server = MockServer::start().await;
    let master = "#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=160x120\nmedia.m3u8\n";
    let media = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:1.0,\nseg0.ts\n#EXT-X-ENDLIST\n";
    Mock::given(method("GET"))
        .and(path("/master.m3u8"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.apple.mpegurl")
                .set_body_string(master),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/media.m3u8"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.apple.mpegurl")
                .set_body_string(media),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/seg0.ts"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "video/mp2t")
                .set_body_bytes(ts.clone()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/master.m3u8", server.uri()));

    // 先停在等待选择
    wait_state(&mut rx, id, TaskState::AwaitingChoice, 30).await;
    // 选 0 号（唯一的）码率 —— 这一步走 set_choice 的重新入队
    engine.set_choice(id, Choice::Variant(0));
    wait_state(&mut rx, id, TaskState::Done, 120).await;

    assert!(
        dir.path().join("master.mp4").exists(),
        "选择码率后应当合并出 master.mp4"
    );
}
