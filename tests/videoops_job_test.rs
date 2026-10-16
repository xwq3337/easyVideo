use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::ffmpeg_job::{parse_progress_fraction, run_job, JobOutcome};
use easydownvideo::core::videoops::media_info::inspect;
use easydownvideo::core::videoops::presets::{compress_args, CompressParams, CompressPreset, Encoder};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

fn params(preset: CompressPreset) -> CompressParams {
    CompressParams { preset, encoder: Encoder::Software, max_height: None, crf: None, qv: None, audio_kbps: None }
}

/// **`out_time_ms` 也是微秒。** 这是 ffmpeg 有名的错名：两个字段一个说 us 一个说 ms，
/// 实测 5 秒的片子两者都给出 4933333。照字面当毫秒用，进度会永远停在 0.5%，
/// 而且看起来像卡死而不像出错。
#[test]
fn out_time_ms_is_microseconds_despite_its_name() {
    // 真实输出（ffmpeg 9.0.2，5 秒素材）
    let real = "out_time_us=4933333\nout_time_ms=4933333\nout_time=00:00:04.933333\nprogress=end\n";
    let mut last = None;
    for line in real.lines() {
        if let Some(f) = parse_progress_fraction(line, 5.0) {
            last = Some(f);
        }
    }
    let f = last.expect("应能解析出进度");
    assert!((f - 0.9866).abs() < 0.01, "5 秒片子走到 4.93 秒应是 0.987，实际: {f}");
}

#[test]
fn unrelated_lines_and_garbage_yield_none() {
    assert_eq!(parse_progress_fraction("frame=123", 5.0), None);
    assert_eq!(parse_progress_fraction("progress=continue", 5.0), None);
    assert_eq!(parse_progress_fraction("out_time_us=abc", 5.0), None);
    // 时长未知时一律 None：没有分母，编一个分数出来就是骗人
    assert_eq!(parse_progress_fraction("out_time_us=1000000", 0.0), None);
}

#[test]
fn fraction_is_clamped_to_zero_one() {
    // 有的封装会报出比容器时长还长的 out_time（尾帧、时间戳回绕）
    assert_eq!(parse_progress_fraction("out_time_us=99999999", 5.0), Some(1.0));
    assert_eq!(parse_progress_fraction("out_time_us=0", 5.0), Some(0.0));
}

/// 端到端真跑：素材是现造的、ffmpeg 是内嵌的，**不需要网络**，可以进常规测试套件。
/// 这条是整份计划里最强的验证 —— 上面那些纯函数测试都拦不住"参数拼错了"。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compresses_a_real_file_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");

    // 造一段 3 秒、720p、带音频的素材
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=3:size=1280x720:rate=30",
               "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18",
               "-c:a", "aac", "-shortest"])
        .arg(&src)
        .output().await.unwrap();
    assert!(gen.status.success(), "造素材失败: {}", String::from_utf8_lossy(&gen.stderr));
    let src_size = std::fs::metadata(&src).unwrap().len();

    let out = dir.path().join("out.mp4");
    let part = dir.path().join("out.mp4.part");
    let info = inspect(&ffmpeg, &src).await.unwrap();
    assert!(info.has_video, "造出来的素材必须有视频流");
    let duration = info.duration_secs;
    assert!(duration.is_some(), "造出来的素材必须能读出时长");
    assert!((duration.unwrap() - 3.0).abs() < 0.3, "时长应约 3 秒，实际: {duration:?}");

    let args = compress_args(&src, &out, &params(CompressPreset::Small));
    let seen: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let outcome = run_job(
        &ffmpeg, &args, &out, duration,
        Arc::new(move |f| sink.lock().unwrap().push(f)),
        &CancellationToken::new(),
    ).await.unwrap();

    assert_eq!(outcome, JobOutcome::Done);
    assert!(out.exists(), "成功后产物必须就位");
    assert!(!part.exists(), "临时名不该还留着");
    let out_size = std::fs::metadata(&out).unwrap().len();
    assert!(out_size < src_size, "压缩后应更小: {out_size} vs {src_size}");
    assert!(out_size > 0, "产物不能是空文件");
    // 进度回调真的被调过，且不是只有一个 1.0
    let seen = seen.lock().unwrap();
    assert!(seen.len() >= 2, "进度回调应被多次触发，实际 {} 次: {seen:?}", seen.len());

    // 产物真的能读回来（对拍一遍时长）
    let back = inspect(&ffmpeg, &out).await.unwrap();
    assert!(back.has_video, "产物必须仍然有视频流");
    let back = back.duration_secs.expect("产物应能被 ffmpeg 读回");
    assert!((back - 3.0).abs() < 0.5, "产物的时长应仍约 3 秒，实际: {back}");
}

/// 路径里带撇号、空格、中文 —— 这些曾经让 HLS 的 concat 列表解析失败过。
/// 走 argv 传参就不会有这个问题（不经过任何 shell）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handles_paths_with_quotes_spaces_and_cjk() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("My's Videos 第一集");
    std::fs::create_dir_all(&sub).unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = sub.join("源 文件.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=2:size=320x240:rate=15",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"]).arg(&src).output().await.unwrap();
    assert!(gen.status.success());

    let out = sub.join("源 文件-压缩.mp4");
    let part = sub.join("源 文件-压缩.mp4.part");
    let args = compress_args(&src, &out, &params(CompressPreset::Balanced));
    let outcome = run_job(&ffmpeg, &args, &out, Some(2.0), Arc::new(|_| {}), &CancellationToken::new())
        .await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);
    assert!(out.exists());
}

/// 取消的收尾顺序：先 kill、再等它真的退出、最后才删产物。
/// 顺序反了的话进程还有一次写在做，删完它会把半截文件重新写出来。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_kills_the_process_and_removes_the_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    // 20 秒 1080p60，保证压缩过程足够长，来得及取消。
    // 生成用 ultrafast 纯粹是为了测试跑得快（质量参数没变，不影响任何断言）。
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=20:size=1920x1080:rate=60",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18"]).arg(&src).output().await.unwrap();
    assert!(gen.status.success(), "{}", String::from_utf8_lossy(&gen.stderr));

    let out = dir.path().join("out.mp4");
    let part = dir.path().join("out.mp4.part");
    let args = compress_args(&src, &out, &params(CompressPreset::High));

    let token = CancellationToken::new();
    let t = token.clone();
    // 200ms 后取消：此时 ffmpeg 一定已经起来了，但绝不可能已经跑完
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        t.cancel();
    });

    let outcome = run_job(&ffmpeg, &args, &out, Some(20.0), Arc::new(|_| {}), &token).await.unwrap();
    assert_eq!(outcome, JobOutcome::Interrupted);
    assert!(!part.exists(), "取消后必须删掉半截产物 —— 否则用户会看到一个打不开的 mp4");
    assert!(!out.exists());
}

/// 改名是**唯一**一个曾经会留下半截产物的失败路径。
///
/// spec §7 写死了"失败或取消绝不留下半截文件"：产物留在盘上，用户会在目录里看到
/// 一个打不开的 `<名字>-压缩.mp4.part`，而列表里那一行写着"失败"。其余路径
/// （ffmpeg 非零退出、取消、spawn 失败）都会清理，只有这里的 `?` 直接抛了出去。
///
/// 触发方式：目标位置上放一个**非空目录**，rename 必然失败。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_rename_does_not_leave_the_part_file_behind() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=2:size=320x240:rate=15",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"]).arg(&src).output().await.unwrap();
    assert!(gen.status.success());

    let out = dir.path().join("out.mp4");
    std::fs::create_dir(&out).unwrap();
    std::fs::write(out.join("occupied"), b"x").unwrap(); // 非空，rename 必失败

    let args = compress_args(&src, &out, &params(CompressPreset::Balanced));
    let err = run_job(&ffmpeg, &args, &out, Some(2.0), Arc::new(|_| {}), &CancellationToken::new())
        .await;
    assert!(err.is_err(), "改名到非空目录上必须失败");

    let part = easydownvideo::core::videoops::presets::part_path_for(&out);
    assert!(
        !part.exists(),
        "改名失败时也必须删掉 .part —— 否则用户会在目录里看到一个打不开的半截文件，实际: {part:?}"
    );
}
