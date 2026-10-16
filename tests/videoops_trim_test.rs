use easydownvideo::core::videoops::media_info::MediaInfo;
use easydownvideo::core::videoops::trim::{
    parse_time, trim_args, trim_extension, trim_muxer, validate_trim, TrimMode, TrimParams,
};
use std::path::Path;

fn src(container: Option<&str>, dur: Option<f64>) -> MediaInfo {
    MediaInfo {
        duration_secs: dur,
        has_video: true,
        video_codec: Some("h264".into()),
        audio_codec: Some("aac".into()),
        container: container.map(|c| c.to_string()),
        width: Some(640),
        height: Some(360),
    }
}
fn p(start: f64, end: Option<f64>, mode: TrimMode) -> TrimParams {
    TrimParams { start, end, mode }
}
fn args_of(t: &TrimParams, s: &MediaInfo) -> Vec<String> {
    trim_args(Path::new("/in/a.mkv"), Path::new("/out/a-裁剪.mkv"), t, s)
}
fn has(args: &[String], n: &str) -> bool {
    args.iter().any(|a| a == n)
}
fn arg_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let i = args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("没有 {flag}: {args:?}"));
    &args[i + 1]
}

/// 人有三种写法习惯，只认一种会被骂。**纯数字一律按秒**（90 是 90 秒）。
#[test]
fn parses_the_three_habits() {
    assert_eq!(parse_time("12"), Some(12.0));
    assert_eq!(parse_time("1:30"), Some(90.0));
    assert_eq!(parse_time("01:30"), Some(90.0));
    assert_eq!(parse_time("0:00:12"), Some(12.0));
    assert_eq!(parse_time("1:02:03"), Some(3723.0));
    assert_eq!(parse_time("12.5"), Some(12.5));
    // 前后空白要容忍（用户从别处复制粘贴）
    assert_eq!(parse_time("  1:30  "), Some(90.0));
}

#[test]
fn rejects_garbage_times() {
    for bad in ["", "  ", "abc", "1:2:3:4", "-5", "1:", ":30", "1:xx", "9:99"] {
        assert_eq!(parse_time(bad), None, "应该拒绝: {bad:?}");
    }
}

/// 快速档用 copy，**容器必须沿用源** —— copy 到别的容器常常直接失败。
#[test]
fn fast_mode_keeps_the_source_container() {
    let t = p(12.0, Some(20.0), TrimMode::Fast);
    let s = src(Some("mkv"), Some(30.0));
    let args = args_of(&t, &s);
    assert_eq!(arg_after(&args, "-c"), "copy");
    assert_eq!(arg_after(&args, "-f"), "matroska"); // 不是 "mkv"
    assert!(!has(&args, "-crf"), "快速档不该出现重编码参数");
    assert_eq!(trim_extension(&t, &s), "mkv");
    assert_eq!(trim_muxer(&t, &s), "matroska");
}

/// **源容器认不出来时退到 matroska**（它什么都装得下，copy 一定成功），
/// 而这个退化会体现在产物名里（外框的预览会显示 .mkv）—— 不是悄悄改扩展名。
#[test]
fn fast_mode_falls_back_to_matroska_when_the_container_is_unknown() {
    let t = p(1.0, Some(2.0), TrimMode::Fast);
    let s = src(None, Some(30.0));
    assert_eq!(trim_muxer(&t, &s), "matroska");
    assert_eq!(trim_extension(&t, &s), "mkv");
}

/// 精确档重编码成 h264+aac，mp4 一定装得下 —— 与压缩页同一句话。
#[test]
fn accurate_mode_encodes_to_mp4() {
    let t = p(12.0, Some(20.0), TrimMode::Accurate);
    let s = src(Some("mkv"), Some(30.0));
    let args = args_of(&t, &s);
    assert_eq!(arg_after(&args, "-c:v"), "libx264");
    assert_eq!(arg_after(&args, "-c:a"), "aac");
    assert_eq!(arg_after(&args, "-f"), "mp4");
    assert_eq!(trim_extension(&t, &s), "mp4");
    assert_eq!(trim_muxer(&t, &s), "mp4");
}

/// `-ss`/`-to` 都放在 `-i` **之前**（输入选项）—— mp4 的索引能给出精确寻址，
/// spec §2 的实测就是按这个写法做的。
#[test]
fn seek_options_come_before_the_input() {
    let args = args_of(&p(12.0, Some(20.0), TrimMode::Fast), &src(Some("mp4"), Some(30.0)));
    let i = args.iter().position(|a| a == "-i").unwrap();
    assert!(args.iter().position(|a| a == "-ss").unwrap() < i);
    assert!(args.iter().position(|a| a == "-to").unwrap() < i);
    assert_eq!(arg_after(&args, "-ss"), "12.000000");
    assert_eq!(arg_after(&args, "-to"), "20.000000");
}

/// `end` 留空 = 到结尾（"从 1:30 到结束"是常见需求），此时不该出现 -to。
#[test]
fn no_end_means_to_the_end() {
    let args = args_of(&p(90.0, None, TrimMode::Fast), &src(Some("mp4"), Some(300.0)));
    assert!(!has(&args, "-to"), "没给结束时间就不该有 -to: {args:?}");
    assert_eq!(arg_after(&args, "-ss"), "90.000000");
}

#[test]
fn validate_rejects_impossible_ranges() {
    let d = Some(30.0);
    assert!(validate_trim(&p(31.0, None, TrimMode::Fast), d).is_err(), "start 超出源时长");
    assert!(validate_trim(&p(20.0, Some(20.0), TrimMode::Fast), d).is_err(), "start == end");
    assert!(validate_trim(&p(20.0, Some(10.0), TrimMode::Fast), d).is_err(), "start > end");
    assert!(validate_trim(&p(-1.0, Some(10.0), TrimMode::Fast), d).is_err(), "负数");
    assert!(validate_trim(&p(0.0, Some(30.0), TrimMode::Fast), d).is_ok());
    assert!(validate_trim(&p(0.0, None, TrimMode::Fast), d).is_ok());
    // 时长未知时不拦 start（读不出时长不等于 clip 不合理）
    assert!(validate_trim(&p(9999.0, None, TrimMode::Fast), None).is_ok());
    // end 超过源时长允许（容器时长常常略短于实际可播长度）
    assert!(validate_trim(&p(28.0, Some(31.0), TrimMode::Fast), d).is_ok());
}

#[test]
fn modes_parse_from_the_wire_strings() {
    assert_eq!("fast".parse::<TrimMode>().unwrap(), TrimMode::Fast);
    assert_eq!("accurate".parse::<TrimMode>().unwrap(), TrimMode::Accurate);
    assert!("quick".parse::<TrimMode>().is_err());
}

// ===== 端到端真跑：素材现造、ffmpeg 内嵌，不需要网络 =====

use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::ffmpeg_job::{run_job, JobOutcome};
use easydownvideo::core::videoops::media_info::inspect;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 造一段 30 秒、**强制 GOP = 10 秒**的素材。
/// GOP 必须可控，否则切口精度那条测试测不出东西（关键帧密到每秒一个的话，
/// 吸附与不吸附看起来一样）。
async fn make_gop10(ffmpeg: &Path, path: &Path) {
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i",
               "testsrc2=duration=30:size=640x360:rate=30",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30",
               "-g", "300", "-keyint_min", "300", "-sc_threshold", "0"])
        .arg(path).output().await.unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// 取某一帧的 MD5。`at` 为 None 表示取第 0 帧（用于取产物首帧）。
/// testsrc2 每帧内容都不同，所以 MD5 相同 ⇔ 是同一帧。
async fn frame_md5(ffmpeg: &Path, path: &Path, at: Option<f64>) -> String {
    let mut cmd = tokio::process::Command::new(ffmpeg);
    cmd.args(["-v", "error"]);
    if let Some(t) = at {
        cmd.args(["-ss", &t.to_string()]);
    }
    cmd.arg("-i").arg(path).args(["-frames:v", "1", "-f", "md5", "-"]);
    let out = cmd.output().await.unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// **把 spec §2 的实测钉成回归测试。**
///
/// 流传的说法是 `-c copy` 会把切口吸附到关键帧（于是起点提前、产物更长）。
/// 我们在这台 ffmpeg 9.0.2 上复现不出来：10 秒 GOP 的素材、切 12.5→20，
/// 产物首帧与源在 12.5s 的帧**逐字节相同**、时长也精确 7.5 秒。
///
/// 这条测试守住该行为 —— 哪天它变了（换 ffmpeg 版本），界面文案（把偏移写成
/// **可能**而不是**会**）就该跟着改。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fast_trim_cut_is_frame_accurate_on_a_sparse_gop_source() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src_path = dir.path().join("gop10.mp4");
    make_gop10(&ffmpeg, &src_path).await;

    let info = inspect(&ffmpeg, &src_path).await.unwrap();
    assert_eq!(info.container.as_deref(), Some("mp4"));

    let out = dir.path().join("cut.mp4");
    let t = TrimParams { start: 12.5, end: Some(20.0), mode: TrimMode::Fast };
    let args = trim_args(&src_path, &out, &t, &info);
    let outcome = run_job(&ffmpeg, &args, &out, Some(30.0), Arc::new(|_| {}),
                          &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);

    let got = frame_md5(&ffmpeg, &out, None).await;
    let want = frame_md5(&ffmpeg, &src_path, Some(12.5)).await;
    assert_eq!(got, want, "快速切的首帧应与源在请求时刻的帧完全相同（吸附的话会差一整个 GOP）");

    // 时长也要对：吸附了会是 10 秒而不是 7.5 秒
    let cut = inspect(&ffmpeg, &out).await.unwrap();
    let d = cut.duration_secs.expect("产物时长");
    assert!((d - 7.5).abs() < 0.2, "产物应约 7.5 秒（吸附了会变成 10 秒），实际: {d}");
}

/// 精确档真跑一次，断言时长与编码都对。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accurate_trim_runs_and_has_the_right_duration() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src_path = dir.path().join("src.mp4");
    make_gop10(&ffmpeg, &src_path).await;
    let info = inspect(&ffmpeg, &src_path).await.unwrap();

    let out = dir.path().join("cut.mp4");
    let t = TrimParams { start: 5.0, end: Some(9.0), mode: TrimMode::Accurate };
    let args = trim_args(&src_path, &out, &t, &info);
    let outcome = run_job(&ffmpeg, &args, &out, Some(30.0), Arc::new(|_| {}),
                          &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);
    assert!(out.exists());

    let cut = inspect(&ffmpeg, &out).await.unwrap();
    let d = cut.duration_secs.expect("产物时长");
    assert!((d - 4.0).abs() < 0.3, "产物应约 4 秒，实际: {d}");
    assert_eq!(cut.video_codec.as_deref(), Some("h264"), "精确档重编码成 h264");
    assert_eq!(cut.container.as_deref(), Some("mp4"), "精确档输出 mp4");
}
