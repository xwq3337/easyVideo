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
async fn concurrent_tasks_with_the_same_derived_name_do_not_clobber() {
    // `unique_path` 是"先查后用"：两个任务在同一时刻都看到"最终文件不存在"，于是算出
    // **同一个**最终路径，各自下载到自己的任务目录后都往那一个路径拼接 —— 后完成的把
    // 先完成的成果覆盖掉。两个任务都报成功，用户以为自己下到了两个文件。
    //
    // 触发条件很普通：`filename_from_url` 对末段不含 `.` 的 URL 一律回落到 `video.mp4`，
    // 而默认 `max_active_tasks = 2`，连续贴两个这样的链接就会并发跑。
    //
    // 这里让两个任务都"先过 paths() 再写盘"：body 响应延迟 500ms，于是两者的 paths()
    // 都发生在任何一个最终文件落盘之前 —— 这不是碰运气，是构造出来的必然重叠。
    let a: Vec<u8> = (0..4096u32).map(|i| (i % 197) as u8).collect();
    let b: Vec<u8> = (0..4096u32).map(|i| (i % 181) as u8).collect();
    let server = MockServer::start().await;
    for (p, body) in [("/a/watch", a.clone()), ("/b/download", b.clone())] {
        Mock::given(method("GET"))
            .and(path(p))
            .and(header("range", "bytes=0-63"))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header("content-type", "video/mp4")
                    .insert_header("content-range", "bytes 0-63/4096")
                    .set_body_bytes(body[0..64].to_vec()),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header("content-type", "video/mp4")
                    .insert_header("content-range", "bytes 0-4095/4096")
                    .set_body_bytes(body.clone())
                    .set_delay(std::time::Duration::from_millis(500)),
            )
            .mount(&server)
            .await;
    }

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s = settings_with(dir.path());
    s.max_active_tasks = 2; // 默认值，这里写出来是为了让触发条件显式
    let engine = Engine::new(rt, s, dir.path().join("data"));

    engine.add_url(format!("{}/a/watch", server.uri()));
    engine.add_url(format!("{}/b/download", server.uri()));

    // 两个任务并发，**不能**按 id 分别 `wait_state`：broadcast 每个事件只投递一次，
    // 后一个 id 若先完成，它的事件会在等前一个 id 时被消费掉，之后就永远等不到了。
    // 这里轮询快照（项目里其他测试等清理也是这个写法），与事件到达顺序无关。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let snaps = engine.list_tasks();
        if let Some(bad) = snaps
            .iter()
            .find(|t| matches!(t.state, TaskState::Failed(_) | TaskState::Canceled))
        {
            panic!("任务没有成功：{:?}", bad.state);
        }
        if !snaps.is_empty() && snaps.iter().all(|t| t.state == TaskState::Done) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "两个同名任务没有在 60s 内完成"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let mut got: Vec<Vec<u8>> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| std::fs::read(e.path()).unwrap())
        .collect();
    got.sort();
    let mut want = vec![a.clone(), b.clone()];
    want.sort();
    assert_eq!(
        got.len(),
        2,
        "两个同名任务应当各落一个文件（video.mp4 与 video (1).mp4），实际只有 {} 个 —— 后完成的把先完成的覆盖了",
        got.len()
    );
    assert_eq!(got, want, "落盘的两个文件与两份响应体对不上");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_resumed_task_after_moving_the_dir_cleans_the_old_one() {
    // 上一条的另一个入口：`resume()` 的 eager 落盘用的是**那一刻**的设置，而清理原先按
    // 的是**取消那一刻**的设置。用户在两者之间改了保存位置，清理就会去删新目录里同号的
    // `task-<id>`，而真正写了 manifest 的旧目录原封不动 —— 下次启动 scan_resume 照旧复活。
    let root = tempfile::tempdir().unwrap();
    let old_dir = root.path().join("old");
    let new_dir = root.path().join("new");
    std::fs::create_dir_all(&old_dir).unwrap();
    std::fs::create_dir_all(&new_dir).unwrap();

    let rt = tokio::runtime::Handle::current();
    let mut s0 = settings_with(&old_dir);
    s0.max_active_tasks = 0; // 让恢复出来的任务一直排队，没有 worker 会替它收尾
    let engine = Engine::new(rt, s0, root.path().join("data"));

    let id = 5u64;
    engine.resume(TaskManifest {
        id,
        url: "https://example.com/a.mp4".into(),
        filename: "a.mp4".into(),
        download_dir: old_dir.clone(),
        kind: ManifestKind::Direct { size: Some(10) },
    });
    // **前提断言**：manifest 确实落在旧目录里，否则这个测试不成立
    assert_eq!(
        easydownvideo::core::task::scan_resume(&old_dir).len(),
        1,
        "resume 应当立刻把 manifest 写进当时的下载目录"
    );

    // 用户在任务还排队的时候改了保存位置
    // （上限保持 0：任务必须**一直排队**，否则 worker 会跑起来，清理就变成异步的了，
    //   这条测试关心的不是那个路径。）
    let mut s1 = settings_with(&new_dir);
    s1.max_active_tasks = 0;
    engine.update_settings(s1);
    let bystander = easydownvideo::core::task::task_dir(&new_dir, id);
    std::fs::create_dir_all(&bystander).unwrap();
    std::fs::write(bystander.join("keepme.txt"), b"OTHER TASK DATA").unwrap();

    engine.cancel(id);

    assert!(
        easydownvideo::core::task::scan_resume(&old_dir).is_empty(),
        "旧目录里仍残留可恢复的 manifest —— 下次启动会把它复活重下"
    );
    assert!(
        !easydownvideo::core::task::task_dir(&old_dir, id).exists(),
        "任务真正写入的目录没被清掉"
    );
    assert!(
        bystander.join("keepme.txt").exists(),
        "清理删到了新目录里同号的 task-{id} —— 那是另一个任务的数据"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_during_probe_ends_canceled_and_leaves_nothing() {
    // 用户在"探测中"点删除。`probe()` 是一个网络往返，任务在这段时间里是 `Probing`；
    // `cancel()` 已经把终态落成 `Canceled`。但 `run_task` 从 probe 返回后会**无条件**
    // `set_state(Downloading)`，把 `Canceled` 抹掉 —— 收尾分支随后看到的不是 Canceled，
    // 于是走了 `else`：标记 `Failed` **并且跳过清理**。任务目录与 manifest 留在盘上，
    // 下次启动 `scan_resume` 把它复活重下。实测复现见 progress.md 的 `cancel-during-probe`。
    //
    // 把这个窗口从毫秒拉长到可稳定命中：probe 的响应延迟 400ms。
    let server = MockServer::start().await;
    let data: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-63"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/4096")
                .set_body_bytes(data[0..64].to_vec())
                .set_delay(std::time::Duration::from_millis(400)),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-4095/4096")
                .set_body_bytes(data.clone()),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/v.mp4", server.uri()));

    wait_state(&mut rx, id, TaskState::Probing, 10).await;
    engine.cancel(id);

    // **取消之后不允许再有任何状态变化。** 这里要看的正是"取消之后再发生了什么"，
    // 所以不能等到下一个事件就断言（那会把 Downloading 当成正常流程）。用一个明确
    // 长于 probe 延迟的静默窗口把事件的"有/无"区分开 —— 这是下界等待，不是"睡一会儿
    // 希望它跑完"：probe 的延迟是 400ms，窗口是它的 4 倍。
    // （`cancel()` 自己发出的那个 `Canceled` 事件也会被这个循环收到，所以下面过滤掉它 ——
    //  这里要拦的是**取消之后又被改成别的状态**，不是取消本身。）
    let mut after_cancel: Vec<TaskState> = Vec::new();
    let quiet_until = std::time::Instant::now() + std::time::Duration::from_millis(1600);
    while std::time::Instant::now() < quiet_until {
        match tokio::time::timeout(
            quiet_until.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        )
        .await
        {
            Ok(Ok(EngineEvent::StateChanged { id: eid, state })) if eid == id => {
                if state != TaskState::Canceled {
                    after_cancel.push(state);
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let snap = engine
        .list_tasks()
        .into_iter()
        .find(|t| t.id == id)
        .expect("任务应仍在列表中");
    let tdir = easydownvideo::core::task::task_dir(dir.path(), id);

    assert!(
        after_cancel.is_empty(),
        "取消之后任务状态又被改了 {after_cancel:?} —— 用户的取消被覆盖了（终态 {:?}）",
        snap.state
    );
    assert_eq!(snap.state, TaskState::Canceled, "终态不是 Canceled");
    assert!(
        easydownvideo::core::task::scan_resume(dir.path()).is_empty(),
        "取消后仍残留可恢复的 manifest，重启会把已取消任务复活"
    );
    assert!(!tdir.exists(), "取消后任务目录仍在：{}", tdir.display());
    assert!(!dir.path().join("v.mp4").exists(), "被取消的任务产出了成品文件");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cleanup_uses_the_directory_the_task_actually_used() {
    // 用户可以在任务跑着的时候改保存位置（设置窗口两下就能改）。清理原先用的是
    // `current_settings().download_dir` —— 那是**新**目录，不是这个任务真正写入的那个：
    //   * 真正的 manifest 留在原地，被下次启动的 scan_resume 复活重新下载；
    //   * 新目录里同号的 `task-<id>`（别的任务的数据）反倒被整个删掉。
    let server = MockServer::start().await;
    let data: Vec<u8> = (0..4096u32).map(|i| (i % 211) as u8).collect();
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

    let root = tempfile::tempdir().unwrap();
    let old_dir = root.path().join("old");
    let new_dir = root.path().join("new");
    std::fs::create_dir_all(&old_dir).unwrap();
    std::fs::create_dir_all(&new_dir).unwrap();

    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(&old_dir), root.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/v.mp4", server.uri()));

    // 进入 Downloading 说明 manifest 已经落在**旧**目录
    wait_state(&mut rx, id, TaskState::Downloading, 10).await;

    // 用户在任务跑着的时候把保存位置改到新目录
    engine.update_settings(settings_with(&new_dir));

    // 新目录里恰好有一个同号的任务目录 —— 那是另一个任务的断点数据，绝不能碰
    let bystander = easydownvideo::core::task::task_dir(&new_dir, id);
    std::fs::create_dir_all(&bystander).unwrap();
    std::fs::write(bystander.join("keepme.txt"), b"OTHER TASK DATA").unwrap();

    engine.cancel(id);
    wait_state(&mut rx, id, TaskState::Canceled, 10).await;

    // 清理发生在 worker 退出路径，轮询等待（避免依赖固定 sleep 时长）
    let real = easydownvideo::core::task::task_dir(&old_dir, id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while real.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "任务**真正**写入的目录 {} 没被清掉 —— 它的 manifest 会被下次启动的 scan_resume 复活重下",
            real.display()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        easydownvideo::core::task::scan_resume(&old_dir).is_empty(),
        "旧目录里仍残留可恢复的 manifest"
    );
    assert!(
        bystander.join("keepme.txt").exists(),
        "清理删到了新目录里同号的 task-{id} —— 那是另一个任务的数据"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_then_immediate_resume_does_not_fail_the_task() {
    // 暂停后**立刻**继续。旧 worker 此时还挂在块请求上（响应延迟 1500ms）没有退出；
    // `unpause` 会换一个**新的** token 并重新排队，于是新 worker 起来下载同一个任务。
    // 旧 worker 拿到 Err 后按"当前状态"分派 —— 它看到的是新 worker 设下的 Probing/
    // Downloading，既不是 Canceled 也不是 Paused，于是把任务标记成 `Failed`。
    // 用户双击"继续"，任务就失败了。
    let server = MockServer::start().await;
    let data: Vec<u8> = (0..4096u32).map(|i| (i % 241) as u8).collect();
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
    // 全量请求延迟返回：旧 worker 会在这上面挂住，从而与新 worker 重叠
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

    wait_state(&mut rx, id, TaskState::Downloading, 10).await;
    engine.pause(id);
    engine.unpause(id); // 立刻继续，不给旧 worker 退出的时间

    // wait_state 见到 Failed 会 panic，正是这里要暴露的失败
    wait_state(&mut rx, id, TaskState::Done, 60).await;
    assert_eq!(
        std::fs::read(dir.path().join("v.mp4")).unwrap(),
        data,
        "暂停→继续后成品被两个 worker 并发写坏了"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_task_never_runs_two_workers_at_once() {
    // 「暂停 → 立刻继续」时旧 worker 还挂在网络请求上没退出，而 `unpause` 已经把它重新
    // 排进队列。若此时就起新 worker，两者会并发写同一批 `f.part-N`（都是 `File::create`
    // 截断写），产出一个**静默损坏**的文件 —— Direct 下载没有校验和，没有任何东西会暴露它。
    //
    // 判据直接取"新 worker 有没有立刻起来"：新 worker 起来的第一步就是置 `Probing`，
    // 而旧 worker 要等 1500ms 的延迟响应回来才会退出。所以取消暂停后的 600ms 内
    // **不应该有任何状态事件**；出现 `Probing` 就说明两个 worker 重叠了。
    let server = MockServer::start().await;
    let data: Vec<u8> = (0..4096u32).map(|i| (i % 233) as u8).collect();
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

    wait_state(&mut rx, id, TaskState::Downloading, 10).await;
    engine.pause(id);
    engine.unpause(id);

    let mut early: Vec<TaskState> = Vec::new();
    let window = std::time::Instant::now() + std::time::Duration::from_millis(600);
    while std::time::Instant::now() < window {
        match tokio::time::timeout(
            window.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        )
        .await
        {
            // `pause()` 自己发出的那个 `Paused` 事件也会被这个窗口收到，滤掉它 ——
            // 这里要拦的是"新 worker 已经起来"，也就是 `Probing`。
            Ok(Ok(EngineEvent::StateChanged { id: eid, state })) if eid == id => {
                if state != TaskState::Paused {
                    early.push(state);
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(
        early.is_empty(),
        "旧 worker 还没退出，新 worker 就已经起来了（看到 {early:?}）—— 两个 worker 会并发写同一个 .part 文件"
    );

    // 旧 worker 退出后新 worker 接手，任务最终必须成功
    wait_state(&mut rx, id, TaskState::Done, 60).await;
    assert_eq!(std::fs::read(dir.path().join("v.mp4")).unwrap(), data);
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
async fn hls_output_does_not_overwrite_an_existing_file() {
    // HLS 的最终产物是 `.mp4`，但唯一性检查用的是**入口的扩展名**（`master.m3u8`）。
    // `master.m3u8` 当然不存在，于是 `unique_path` 放行，而产物 `master.mp4` 被交给
    // ffmpeg 的 `-y` 直接覆盖 —— 用户自己的同名文件**永久丢失**，全程没有任何提示。
    // 重复下载同一个 m3u8（很常见）、或下载目录里本来就有个同名 mp4，都会踩到。
    let ts = ts_fixture();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/master.m3u8"))
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
    // 用户自己的、与将要产出的成品同名的文件
    let existing = dir.path().join("master.mp4");
    let sentinel = b"USER-OWNED-FILE-DO-NOT-TOUCH".to_vec();
    std::fs::write(&existing, &sentinel).unwrap();

    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/master.m3u8", server.uri()));

    wait_state(&mut rx, id, TaskState::Done, 120).await;

    assert_eq!(
        std::fs::read(&existing).unwrap(),
        sentinel,
        "已存在的 master.mp4 被静默覆盖了 —— 用户的文件就这么丢了"
    );
    let renamed = dir.path().join("master (1).mp4");
    let meta = std::fs::metadata(&renamed)
        .unwrap_or_else(|e| panic!("新产物应当落到 {}：{e}", renamed.display()));
    assert!(meta.len() > 0, "改名后的产物不应为空");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn awaiting_choice_does_not_starve_the_queue() {
    // `AwaitingChoice` 没有 worker 在跑，却算作"活跃"、占着并发槽。默认 max_active_tasks = 2，
    // 于是连续两个 master playlist 任务都停在"等待选择"时，队列里其他任务**永远不会开始** ——
    // 用户再加一个普通直链也没用，界面上看就是"点了下载什么也没发生"。
    // 真实世界的 m3u8 大多是 master playlist，这条路很容易走到。
    let server = MockServer::start().await;
    let master = "#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=160x120\nmedia.m3u8\n";
    for p in ["/m1.m3u8", "/m2.m3u8"] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/vnd.apple.mpegurl")
                    .set_body_string(master),
            )
            .mount(&server)
            .await;
    }
    let data: Vec<u8> = (0..2048u32).map(|i| (i % 223) as u8).collect();
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
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
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
    let mut s = settings_with(dir.path());
    s.max_active_tasks = 2; // 默认值：两个"等待选择"就足以把队列占满
    let engine = Engine::new(rt, s, dir.path().join("data"));

    engine.add_url(format!("{}/m1.m3u8", server.uri()));
    engine.add_url(format!("{}/m2.m3u8", server.uri()));

    // 两个任务都要停到 AwaitingChoice。**不能**按 id 分别 wait_state：broadcast 每个事件只
    // 投递一次，先到的那个 id 的事件会在等另一个 id 时被消费掉。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let snaps = engine.list_tasks();
        if let Some(bad) = snaps
            .iter()
            .find(|t| matches!(t.state, TaskState::Failed(_) | TaskState::Canceled))
        {
            panic!("任务没有停在等待选择：{:?}", bad.state);
        }
        if snaps.iter().filter(|t| t.state == TaskState::AwaitingChoice).count() == 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "两个 master playlist 任务没有停到 AwaitingChoice"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // 两个任务都在等选择、**没有任何 worker 在跑**。现在加一个普通直链，它必须能开始。
    let mut rx = engine.events();
    let c = engine.add_url(format!("{}/v.mp4", server.uri()));
    wait_state(&mut rx, c, TaskState::Done, 30).await;
    assert_eq!(std::fs::read(dir.path().join("v.mp4")).unwrap(), data);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invalid_choice_does_not_strand_the_task() {
    // `set_choice` 先 `take()` 再校验：索引越界、或类型与候选不符时提前返回，而候选列表
    // **已经被取走**。任务从此永久卡在 `AwaitingChoice` —— 界面上「选择格式」还能点，
    // 点了弹出同样的对话框，再点仍然毫无反应，而这个任务再也下不完，只能删掉重加。
    // 引擎是公开 API，越界索引不是只有手滑才造得出来。
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

    wait_state(&mut rx, id, TaskState::AwaitingChoice, 30).await;

    // 两次无效调用：索引越界、以及类型与候选不符。之后那次**有效**的选择必须仍然管用。
    engine.set_choice(id, Choice::Variant(7));
    engine.set_choice(id, Choice::YtdlpFormat(0));
    engine.set_choice(id, Choice::Variant(0));

    wait_state(&mut rx, id, TaskState::Done, 120).await;
    assert!(
        dir.path().join("master.mp4").exists(),
        "无效选择之后，正确的选择应当照常完成下载"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_works_while_probing() {
    // 暂停原先只认 Queued|Downloading，而界面在 Probing 上同样给出暂停按钮
    // （src/app.rs:290-297）—— 点了毫无反应、无报错无反馈：用户以为暂停了，任务继续往下走。
    // `probe()` 是一个网络往返，几百毫秒到几秒，正好点在这上面很正常。
    let server = MockServer::start().await;
    let data: Vec<u8> = (0..2048u32).map(|i| (i % 199) as u8).collect();
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-63"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/2048")
                .set_body_bytes(data[0..64].to_vec())
                .set_delay(std::time::Duration::from_millis(400)),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
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
    let id = engine.add_url(format!("{}/v.mp4", server.uri()));

    wait_state(&mut rx, id, TaskState::Probing, 10).await;
    engine.pause(id);

    // 静默窗口长于 probe 的延迟：任务必须停在 Paused，而不是继续走到 Downloading/Done。
    let quiet_until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    let mut after: Vec<String> = Vec::new();
    while std::time::Instant::now() < quiet_until {
        match tokio::time::timeout(
            quiet_until.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        )
        .await
        {
            Ok(Ok(EngineEvent::StateChanged { id: eid, state })) if eid == id => {
                after.push(format!("{state:?}"));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let snap = engine
        .list_tasks()
        .into_iter()
        .find(|t| t.id == id)
        .expect("任务应仍在列表中");
    assert_eq!(
        snap.state,
        TaskState::Paused,
        "探测期间点暂停没有生效（观察到 {after:?}）"
    );

    // 继续之后必须能正常下完 —— 暂停不能把任务弄成再也起不来的状态
    engine.unpause(id);
    wait_state(&mut rx, id, TaskState::Done, 30).await;
    assert_eq!(std::fs::read(dir.path().join("v.mp4")).unwrap(), data);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_works_while_merging() {
    // 界面在 Merging 上同样给暂停按钮（src/app.rs:290-297），而 pause 原先无视它，
    // remux_to_mp4 也不看 token —— 合并会一路跑到结束。Merging 可能很久：`-c copy`
    // 要把整个文件搬一遍，长视频是几秒到几分钟，用户完全可能想停下来。
    //
    // 这里用**可控的假 ffmpeg**：真 ffmpeg 没法确定性地慢下来，而这一步要验的是引擎的
    // 取消语义（中断子进程、清掉它写出来的东西、保留分片目录供继续），不是 ffmpeg 本身。
    //
    // 脚本每次都一样：先写出产物，再睡 2 秒，最后成功退出。用"每次都成功"是为了让测试
    // 对**暂停落地的时刻**不敏感 —— 第一版按调用次数分支（第一次失败、第二次成功），结果
    // 暂停常常赶在脚本写计数文件之前就把它杀了，于是"继续"时它仍以为是第一次运行。
    let server = MockServer::start().await;
    let media = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:1.0,\nseg0.ts\n#EXT-X-ENDLIST\n";
    Mock::given(method("GET"))
        .and(path("/media.m3u8"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.apple.mpegurl")
                .set_body_string(media),
        )
        .mount(&server)
        .await;
    // expect(1)：分片只准被下载一次。继续之后只重跑合并，不该重下分片。
    Mock::given(method("GET"))
        .and(path("/seg0.ts"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "video/mp2t")
                .set_body_bytes(vec![7u8; 1024]),
        )
        .expect(1)
        .mount(&server)
        .await;

    let bindir = tempfile::tempdir().unwrap();
    let fake = bindir.path().join("fake-ffmpeg");
    std::fs::write(
        &fake,
        "#!/bin/sh\nout=\"\"\nfor a in \"$@\"; do out=\"$a\"; done\n\
printf 'MERGED-BY-FAKE-FFMPEG' > \"$out\"\nsleep 2\nexit 0\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&fake).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&fake, perm).unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s = settings_with(dir.path());
    s.ffmpeg_path_override = Some(fake.clone());
    let engine = Engine::new(rt, s, dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/media.m3u8", server.uri()));

    wait_state(&mut rx, id, TaskState::Merging, 60).await;
    engine.pause(id);

    // 静默窗口长于假 ffmpeg 的 2 秒：若暂停没生效，它会成功跑完，任务变成 Done。
    let quiet_until = std::time::Instant::now() + std::time::Duration::from_millis(3000);
    let mut after: Vec<String> = Vec::new();
    while std::time::Instant::now() < quiet_until {
        match tokio::time::timeout(
            quiet_until.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        )
        .await
        {
            Ok(Ok(EngineEvent::StateChanged { id: eid, state })) if eid == id => {
                after.push(format!("{state:?}"));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let snap = engine
        .list_tasks()
        .into_iter()
        .find(|t| t.id == id)
        .expect("任务应仍在列表中");
    assert_eq!(
        snap.state,
        TaskState::Paused,
        "合并期间点暂停没有生效（观察到 {after:?}）"
    );
    let tdir = easydownvideo::core::task::task_dir(dir.path(), id);
    assert!(
        tdir.exists(),
        "暂停应当保留断点目录 —— 分片还在，继续时只重跑合并这一步"
    );
    assert!(
        !dir.path().join("media.mp4").exists(),
        "被中断的合并写出来的东西必须清掉，否则下载目录里会留下一个用不了的 mp4"
    );

    // 继续：只重跑合并（/seg0.ts 的 expect(1) 会在 mock 析构时校验分片没有被重下）
    engine.unpause(id);
    wait_state(&mut rx, id, TaskState::Done, 60).await;
    assert_eq!(
        std::fs::read(dir.path().join("media.mp4")).unwrap(),
        b"MERGED-BY-FAKE-FFMPEG".to_vec()
    );
    assert!(!tdir.exists(), "完成后任务目录应当清掉");
}
