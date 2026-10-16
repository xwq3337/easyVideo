use easydownvideo::core::videoops::compress::CompressPreset;
use easydownvideo::core::videoops::crop::{crop_args, validate_crop, CropParams};
use easydownvideo::core::videoops::media_info::MediaInfo;
use std::path::Path;

fn src(w: Option<u32>, h: Option<u32>) -> MediaInfo {
    MediaInfo {
        duration_secs: Some(10.0),
        has_video: true,
        video_codec: Some("h264".into()),
        audio_codec: Some("aac".into()),
        container: Some("mp4".into()),
        width: w,
        height: h,
    }
}

fn p(w: u32, h: u32, x: u32, y: u32) -> CropParams {
    CropParams { width: w, height: h, x, y, preset: CompressPreset::Balanced }
}

#[test]
fn argv_uses_the_crop_filter_and_re_encodes() {
    let args = crop_args(Path::new("/in/a.mp4"), Path::new("/out/a-裁剪.mp4"), &p(640, 360, 320, 180));
    let has = |n: &str| args.iter().any(|a| a == n);
    assert!(has("-nostdin") && has("-hide_banner") && has("-progress") && has("-y"));
    let vf = args.iter().position(|a| a == "-vf").expect("要有 -vf");
    assert_eq!(args[vf + 1], "crop=640:360:320:180");
    // 裁剪必须重编码（没有 -c copy 这条路）
    assert_eq!(args[args.iter().position(|a| a == "-c:v").unwrap() + 1], "libx264");
    assert_eq!(args[args.iter().position(|a| a == "-f").unwrap() + 1], "mp4");
    assert!(args.last().unwrap().ends_with(".part"));
    // **绝对不能出现 -ss / -to** —— 画面裁剪不改时长，那正是它与「裁剪视频」的分界
    assert!(!has("-ss") && !has("-to"), "画面裁剪不许碰时间轴: {args:?}");
}

/// **越界必须拒绝，不能交给 ffmpeg。** 实测：源 1920 宽、请求 x=1200 再裁 1280
/// （1200+1280=2480 已超出），ffmpeg **把 x 静默钳到 640** 照常输出 —— 用户要的是
/// 画面右侧那一块，拿到的是完全另一个位置的一块，而 stderr 一条警告都没有。
#[test]
fn rejects_out_of_bounds() {
    let s = src(Some(1920), Some(1080));
    let e = validate_crop(&p(1280, 720, 1200, 800), &s).unwrap_err();
    assert!(format!("{e:#}").contains("1920"), "要说清源有多宽: {e:#}");
    assert!(validate_crop(&p(1920, 1080, 1, 0), &s).is_err(), "x+宽 超出也要拒");
    assert!(validate_crop(&p(1280, 1080, 640, 0), &s).is_ok(), "正好贴边可以");
}

/// **奇数宽高必须拒绝。** 实测：请求 crop=641:361 得到的是 **640:360** ——
/// ffmpeg 静默取偶，没有提示。
#[test]
fn rejects_odd_dimensions() {
    let s = src(Some(1920), Some(1080));
    for (w, h) in [(641, 360), (640, 361), (1, 100), (100, 1), (0, 100)] {
        assert!(validate_crop(&p(w, h, 0, 0), &s).is_err(), "{w}x{h} 应当被拒");
    }
    assert!(validate_crop(&p(640, 360, 0, 0), &s).is_ok());
    assert!(validate_crop(&p(2, 2, 0, 0), &s).is_ok(), "2x2 是技术下限，允许");
}

/// **源尺寸读不出来时拒绝**，并说清原因 —— 不能拿 0 去算裁剪框。
#[test]
fn rejects_when_the_source_dimensions_are_unknown() {
    let e = validate_crop(&p(640, 360, 0, 0), &src(None, None)).unwrap_err();
    assert!(format!("{e:#}").contains("画面尺寸"), "要说清读不出尺寸: {e:#}");
}

#[test]
fn quality_preset_shows_up_in_the_argv() {
    let mut params = p(640, 360, 0, 0);
    params.preset = CompressPreset::Small;
    let args = crop_args(Path::new("/in/a.mp4"), Path::new("/out/a.mp4"), &params);
    assert_eq!(args[args.iter().position(|a| a == "-crf").unwrap() + 1], "28");
}

// ===== 端到端真跑：尺寸对、**时长与帧数都不变**（本轮验收的硬指标）=====

use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::ffmpeg_job::{run_job, JobOutcome};
use easydownvideo::core::videoops::media_info::inspect;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 数帧数：`showinfo` 每个解码出来的帧打一行 `pts_time:`。
///
/// **必须用 `-v info`** —— `showinfo` 走的是 info 级，被 `-v error` 掐掉之后
/// 一行都不剩，于是"0 帧"看起来像个通过。（这个坑本轮实测踩过一次。）
/// `-fps_mode passthrough` 保证不因帧率归一化而增删帧。
async fn frame_count(ffmpeg: &Path, p: &Path) -> usize {
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-v", "info", "-i"])
        .arg(p)
        .args(["-vf", "showinfo", "-fps_mode", "passthrough", "-f", "null", "-"])
        .output()
        .await
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let n = String::from_utf8_lossy(&out.stderr).matches("pts_time:").count();
    assert!(n > 0, "一帧都没数到 —— 多半是把 -v 调成了 error（showinfo 走 info 级）");
    n
}

/// **裁剪的时长与帧数都必须与源完全相同。**
///
/// 这是本轮验收的硬指标（用户明确要求"生成的视频长度不能改变"）。
/// `-vf` 不碰时间戳，所以它天然成立 —— 但正因为"天然成立"，才必须钉住：
/// 将来有人往这段 argv 里加 `-ss`/`-to` 或换滤镜，这条会立刻红。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crop_preserves_duration_and_frame_count_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src_path = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=3:size=640x360:rate=30",
               "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "28",
               "-c:a", "aac", "-shortest"])
        .arg(&src_path).output().await.unwrap();
    assert!(gen.status.success(), "{}", String::from_utf8_lossy(&gen.stderr));

    let info = inspect(&ffmpeg, &src_path).await.unwrap();
    assert_eq!((info.width, info.height), (Some(640), Some(360)));
    let src_frames = frame_count(&ffmpeg, &src_path).await;
    assert!(src_frames > 80, "源应当有约 90 帧，实际 {src_frames}");

    let out = dir.path().join("cropped.mp4");
    let params = CropParams { width: 320, height: 180, x: 160, y: 90, preset: CompressPreset::Balanced };
    validate_crop(&params, &info).expect("这个框在 640x360 里是合法的");
    let args = crop_args(&src_path, &out, &params);
    let outcome = run_job(&ffmpeg, &args, &out, Some(3.0), None,
                          Arc::new(|_| {}), &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);

    let back = inspect(&ffmpeg, &out).await.unwrap();
    assert_eq!((back.width, back.height), (Some(320), Some(180)), "尺寸就是请求的那个");
    let d_in = info.duration_secs.unwrap();
    let d_out = back.duration_secs.unwrap();
    assert!((d_in - d_out).abs() < 0.05, "**时长必须一模一样**：源 {d_in} vs 产物 {d_out}");
    let out_frames = frame_count(&ffmpeg, &out).await;
    assert_eq!(out_frames, src_frames, "**帧数也必须一模一样**：源 {src_frames} vs 产物 {out_frames}");
}
