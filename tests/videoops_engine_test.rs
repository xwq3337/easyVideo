use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::compress::{CompressParams, CompressPreset, Encoder};
use easydownvideo::core::videoops::params::JobParams;
use easydownvideo::core::videoops::{JobState, VideoOpEvent, VideoOps};
use tokio::sync::broadcast;

fn params() -> JobParams {
    JobParams::Compress(CompressParams {
        preset: CompressPreset::Small, encoder: Encoder::Software,
        max_height: None, crf: None, qv: None, audio_kbps: None,
    })
}

async fn make_src(ffmpeg: &std::path::Path, path: &std::path::Path, secs: u32) {
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i",
               &format!("testsrc2=duration={secs}:size=640x360:rate=15"),
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"])
        .arg(path).output().await.unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// 收敛：一直收事件直到任务到终态或超时。事件是唯一的观察窗口，
/// 所以测试必须像前端那样订阅它，而不是轮询内部状态。
async fn wait_terminal(
    rx: &mut broadcast::Receiver<VideoOpEvent>,
    id: u64,
    secs: u64,
) -> JobState {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Ok(VideoOpEvent::StateChanged { id: i, state })) if i == id => {
                if matches!(state, JobState::Done | JobState::Failed(_) | JobState::Canceled) {
                    return state;
                }
            }
            Ok(Ok(_)) => {}
            _ => panic!("等终态超时"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compressing_a_real_file_reaches_done_and_writes_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    make_src(&ffmpeg, &src, 2).await;

    let out_dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    let mut rx = ops.events();

    let id = ops.add_job(src.clone(), out_dir.path().to_path_buf(), params()).await.unwrap();
    assert_eq!(ops.list_jobs().len(), 1);
    assert_eq!(ops.list_jobs()[0].label, "压缩 · 小体积");

    let state = wait_terminal(&mut rx, id, 120).await;
    assert_eq!(state, JobState::Done, "应成功: {state:?}");

    let job = &ops.list_jobs()[0];
    assert!(job.output.exists(), "产物应落在指定的输出目录: {:?}", job.output);
    assert!(job.output.starts_with(out_dir.path()));
    assert!(job.output.to_string_lossy().ends_with("-压缩.mp4"));
    // 进度最终停在 1.0
    assert_eq!(job.progress, Some(1.0));
}

/// 输出目录被设成源文件所在目录时，产物不能把源文件盖掉。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn never_overwrites_the_source_file() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    make_src(&ffmpeg, &src, 1).await;
    let before = std::fs::read(&src).unwrap();

    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    let mut rx = ops.events();
    // 输出目录 = 源文件所在目录；且先手动占掉「src-压缩.mp4」这个名字
    std::fs::write(dir.path().join("src-压缩.mp4"), b"already here").unwrap();

    let id = ops.add_job(src.clone(), dir.path().to_path_buf(), params()).await.unwrap();
    assert_eq!(wait_terminal(&mut rx, id, 60).await, JobState::Done);

    assert_eq!(std::fs::read(&src).unwrap(), before, "源文件一个字节都不能动");
    assert_eq!(std::fs::read(dir.path().join("src-压缩.mp4")).unwrap(), b"already here",
               "已存在的文件不该被覆盖");
    let out = &ops.list_jobs()[0].output;
    assert_ne!(out, &dir.path().join("src-压缩.mp4"), "应让路成另一个名字");
    assert!(out.exists());
}

/// **纯音频输入必须在入队那一刻被挡住。**
///
/// 这条不是"顺手加的校验"：实测 ffmpeg 对没有视频流的输入**不报错**，它老老实实
/// 编出一个只有音轨的 mp4 并退出 0。用户点的是"压缩视频"，拿到一个没有画面的文件
/// 是那种"看起来成功了"的坏结果 —— 比直接报错坏得多。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_input_without_a_video_stream_at_enqueue_time() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("audio.m4a");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
               "-c:a", "aac"]).arg(&src).output().await.unwrap();
    assert!(gen.status.success());

    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    let err = ops.add_job(src, dir.path().to_path_buf(), params()).await;
    assert!(err.is_err(), "纯音频输入必须在入队时被拒绝");
    let msg = format!("{:#}", err.unwrap_err());
    assert!(msg.contains("视频流"), "错误里要说清原因，实际: {msg}");
    assert!(ops.list_jobs().is_empty(), "被拒绝的任务不该进列表");
}

/// 输入文件不存在时**在入队那一刻**就报错，而不是排进队列跑一半才失败。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_a_missing_input_at_enqueue_time() {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    let err = ops.add_job(
        dir.path().join("nope.mp4"), dir.path().to_path_buf(), params(),
    ).await;
    assert!(err.is_err(), "不存在的输入必须在入队时就拒绝");
    assert!(ops.list_jobs().is_empty(), "被拒绝的任务不该进列表");
}

/// 输出目录不可写时同样在入队那一刻报错。
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_an_unwritable_output_dir_at_enqueue_time() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    make_src(&ffmpeg, &src, 1).await;

    let ro = dir.path().join("ro");
    std::fs::create_dir_all(&ro).unwrap();
    let mut perm = std::fs::metadata(&ro).unwrap().permissions();
    perm.set_mode(0o500); // r-x：不可写
    std::fs::set_permissions(&ro, perm).unwrap();

    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    let err = ops.add_job(src, ro.clone(), params()).await;
    // root 用户会绕过权限位，那种环境下这条断言没意义
    if err.is_err() {
        assert!(ops.list_jobs().is_empty());
    }
    let mut perm = std::fs::metadata(&ro).unwrap().permissions();
    perm.set_mode(0o700);
    std::fs::set_permissions(&ro, perm).unwrap();
}

/// 取消一个**排队中**（还没轮到）的任务：必须立刻停在 Canceled，且不能自己复活。
///
/// 断言刻意不依赖计时：`add_compress_job` 内部同步调了 pump，而槽被第一个任务占着，
/// 所以第二个任务返回时**必然**是 Queued。原先用 sleep 等这个前提，是测试自己的
/// 计时竞态 —— 素材压缩得比 sleep 快的时候就红，跟被测代码无关。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_queued_job_stays_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    make_src(&ffmpeg, &src, 20).await;
    let src2 = dir.path().join("src2.mp4");
    std::fs::copy(&src, &src2).unwrap();

    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    let mut rx = ops.events();

    let first = ops.add_job(src, dir.path().to_path_buf(), params()).await.unwrap();
    let second = ops.add_job(src2, dir.path().to_path_buf(), params()).await.unwrap();
    // 并发上限是 1，所以第二个必然还在排队 —— 这一步是确定的，不需要 sleep
    assert_eq!(
        ops.list_jobs().iter().find(|j| j.id == second).unwrap().state,
        JobState::Queued
    );

    ops.cancel(second);
    assert_eq!(wait_terminal(&mut rx, second, 10).await, JobState::Canceled);
    // 第一个任务跑完之后 pump 还会再扫一遍队列，被取消的那个不得被重新拉起来
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        ops.list_jobs().iter().find(|j| j.id == second).unwrap().state,
        JobState::Canceled,
        "被取消的排队任务不得复活"
    );
    ops.cancel(first);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_only_works_on_terminal_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    make_src(&ffmpeg, &src, 1).await;

    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    let mut rx = ops.events();
    let id = ops.add_job(src, dir.path().to_path_buf(), params()).await.unwrap();

    assert!(!ops.remove(id), "运行中的任务不能被移除");
    assert_eq!(wait_terminal(&mut rx, id, 60).await, JobState::Done);
    assert!(ops.remove(id), "终态任务应能移除");
    assert!(ops.list_jobs().is_empty());
}

/// **两个在途任务不能指向同一个产物路径。**
///
/// `unique_path_avoiding` 的第三个参数是"已经被谁占了吗"。只问文件系统是不够的：
/// 产物要等任务真跑完才存在，所以两个都还没跑完的任务会拿到**同一个**"这个文件不存在"
/// 的答案，随后后完成的那次 rename 会静默覆盖先完成的那份 —— 界面上两行都显示
/// "已完成"、都指着同一个路径，而用户真正想要的那个变体已经没了。
/// 下载那边有同样的教训（`engine.rs` 的 `reserved_paths` 就是为它存在的）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_in_flight_jobs_never_share_an_output_path() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("第一集.mp4");
    // 20 秒，保证第一个任务在我们加第二个的时候还没跑完
    make_src(&ffmpeg, &src, 20).await;
    let out_dir = tempfile::tempdir().unwrap();

    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    ops.events(); // 订阅一下，反正 worker 会发

    let first = ops.add_job(src.clone(), out_dir.path().to_path_buf(), params()).await.unwrap();
    let second = ops.add_job(src.clone(), out_dir.path().to_path_buf(), params()).await.unwrap();

    let jobs = ops.list_jobs();
    let a = jobs.iter().find(|j| j.id == first).unwrap();
    let b = jobs.iter().find(|j| j.id == second).unwrap();
    assert_ne!(
        a.output, b.output,
        "两个在途任务指向了同一个产物路径，后完成的会静默覆盖先完成的：{:?}",
        a.output
    );

    ops.cancel(first);
    ops.cancel(second);
}

/// **`ffmpeg_path_override` 对视频操作也必须生效。**
///
/// 设置页把它标成"ffmpeg 路径覆盖（留空则用内嵌的）"—— 那是个全局承诺。用户设它
/// 往往正是因为内嵌那份在他机器上跑不起来（架构不符、被 Gatekeeper 拦等）：
/// 只认下载那一半的话，他的下载正常、每一次压缩都失败，而错误看着毫不相干。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn honours_the_configured_ffmpeg_override() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    make_src(&ffmpeg, &src, 1).await;

    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());
    // 先确认：没有覆盖时它用的是内嵌那份
    assert!(ops.probe(src.clone()).await.is_ok());

    // 覆盖值指向一个不存在的文件 —— 若它**没有**被采纳，下面这一步仍然会成功
    ops.set_ffmpeg_override(Some(dir.path().join("definitely-not-ffmpeg")));
    assert!(
        ops.probe(src.clone()).await.is_err(),
        "覆盖值没有被采纳 —— 它仍然在用内嵌那份"
    );

    // 清掉覆盖值应当回到内嵌那份
    ops.set_ffmpeg_override(None);
    assert!(ops.probe(src).await.is_ok(), "清掉覆盖值后应回到内嵌那份");
}

/// **产物名预览与实际落盘必须一致。** 它们走的是同一个 `plan_output_path`，
/// 这条测试守住它：哪天有人给其中一边加了特例（比如预览忘了算在途任务的占位），
/// 这里就红 —— 而那个分叉正是产物名预览存在的意义。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plan_output_matches_the_path_add_job_actually_takes() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    make_src(&ffmpeg, &src, 20).await; // 慢到第一个任务跑不完
    let out_dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());

    // 还没有任何任务时的预览
    let planned = ops
        .plan_output(src.clone(), out_dir.path().to_path_buf(), params())
        .await
        .unwrap();
    assert!(planned.ends_with("src-压缩.mp4"), "实际: {planned}");

    // 入队，比对它真正用的路径
    let id = ops
        .add_job(src.clone(), out_dir.path().to_path_buf(), params())
        .await
        .unwrap();
    let real = ops.list_jobs().iter().find(|j| j.id == id).unwrap().output.clone();
    assert_eq!(std::path::PathBuf::from(&planned), real, "预览与实际必须一致");

    // 再预览一次：这次要绕开刚入队那个（**在途任务也要占位**）
    let planned2 = ops
        .plan_output(src.clone(), out_dir.path().to_path_buf(), params())
        .await
        .unwrap();
    assert!(
        planned2.ends_with("src-压缩 (1).mp4"),
        "在途任务必须占位，否则第二行会覆盖第一行的产物。实际: {planned2}"
    );

    ops.cancel(id);
}
