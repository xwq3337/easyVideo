use easydownvideo::core::videoops::compress::CompressPreset;
use easydownvideo::core::videoops::transcode::{
    target_extension, target_label, transcode_args, TargetFormat, TranscodeParams,
};
use std::path::Path;

fn run(p: &TranscodeParams) -> Vec<String> {
    transcode_args(Path::new("/in/a.mp4"), Path::new("/out/a-转码.webm"), p)
}
fn has(args: &[String], needle: &str) -> bool {
    args.iter().any(|a| a == needle)
}
fn arg_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let i = args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("没有 {flag}: {args:?}"));
    &args[i + 1]
}
fn params(t: TargetFormat, preset: CompressPreset) -> TranscodeParams {
    TranscodeParams { target: t, preset }
}

/// **WebM 只收 VP8/VP9/AV1 + Vorbis/Opus。** 硬塞 aac 进去是
/// `Could not write header` 的硬失败（实测），所以这条是"非法组合"的回归测试：
/// 目标格式是整组打包的，音频编码必须跟着容器走。
#[test]
fn webm_uses_opus_and_never_aac() {
    let args = run(&params(TargetFormat::WebmVp9, CompressPreset::Balanced));
    assert_eq!(arg_after(&args, "-c:a"), "libopus");
    assert!(!has(&args, "aac"), "WebM 收不了 aac，写进去是硬失败。实际: {args:?}");
    assert_eq!(arg_after(&args, "-c:v"), "libvpx-vp9");
    assert_eq!(arg_after(&args, "-f"), "webm");
}

/// VP9 的恒定质量模式要 `-b:v 0` 配 `-crf`；只给 crf 会退化成默认码率模式，
/// 出来的体积与预期差很远。
#[test]
fn vp9_pairs_crf_with_zero_bitrate() {
    let args = run(&params(TargetFormat::WebmVp9, CompressPreset::Balanced));
    assert!(has(&args, "-crf"));
    assert_eq!(arg_after(&args, "-b:v"), "0");
}

#[test]
fn h265_uses_libx265_and_mp4() {
    let args = run(&params(TargetFormat::Mp4H265, CompressPreset::Balanced));
    assert_eq!(arg_after(&args, "-c:v"), "libx265");
    assert_eq!(arg_after(&args, "-c:a"), "aac");
    assert_eq!(arg_after(&args, "-f"), "mp4");
}

#[test]
fn mkv_target_uses_the_matroska_muxer_not_the_word_mkv() {
    let args = run(&params(TargetFormat::MkvH264, CompressPreset::Balanced));
    // -f mkv 是错的，ffmpeg 认的是 matroska
    assert_eq!(arg_after(&args, "-f"), "matroska");
    assert!(!has(&args, "mkv"));
}

/// 三档质量在每个编码器下都要给出**不同**的 CRF —— 跨编码器照抄刻度会让某一档
/// 产出比源文件还大（上一轮 VT 的 q:v 就是这么错的）。
#[test]
fn every_encoder_has_its_own_three_crf_values() {
    let crf_of = |t: TargetFormat, preset: CompressPreset| -> u8 {
        let args = run(&params(t, preset));
        arg_after(&args, "-crf").parse().unwrap()
    };
    for t in [TargetFormat::Mp4H264, TargetFormat::Mp4H265, TargetFormat::WebmVp9, TargetFormat::MkvH264] {
        let small = crf_of(t, CompressPreset::Small);
        let balanced = crf_of(t, CompressPreset::Balanced);
        let high = crf_of(t, CompressPreset::High);
        assert!(small > balanced && balanced > high, "{t:?} 的 CRF 应递减: {small}/{balanced}/{high}");
        assert!(high >= 10 && small <= 45, "{t:?} 的 CRF 落在荒唐的区间: {small}..{high}");
    }
    // 同一档位下，不同编码器的数值不该全部相同（那说明用了同一张表）
    let per_encoder: Vec<u8> = [TargetFormat::Mp4H264, TargetFormat::Mp4H265, TargetFormat::WebmVp9]
        .iter().map(|t| crf_of(*t, CompressPreset::Balanced)).collect();
    assert!(per_encoder.iter().any(|c| *c != per_encoder[0]), "各编码器应有各自的刻度: {per_encoder:?}");
}

#[test]
fn label_and_extension_follow_the_target() {
    assert_eq!(target_label(TargetFormat::Mp4H264), "MP4 · H.264 + AAC");
    assert_eq!(target_label(TargetFormat::Mp4H265), "MP4 · H.265 + AAC");
    assert_eq!(target_label(TargetFormat::WebmVp9), "WebM · VP9 + Opus");
    assert_eq!(target_label(TargetFormat::MkvH264), "MKV · H.264 + AAC");
    assert_eq!(target_extension(TargetFormat::WebmVp9), "webm");
    assert_eq!(target_extension(TargetFormat::MkvH264), "mkv");
    assert_eq!(target_extension(TargetFormat::Mp4H264), "mp4");
}

#[test]
fn targets_parse_from_the_wire_strings() {
    assert_eq!("webmvp9".parse::<TargetFormat>().unwrap(), TargetFormat::WebmVp9);
    assert_eq!("mp4h265".parse::<TargetFormat>().unwrap(), TargetFormat::Mp4H265);
    assert!("mpeg2".parse::<TargetFormat>().is_err());
}

/// 命令行形状：不碰 stdin、静默进度、显式 -f、输出在最后。
#[test]
fn argv_shape_is_well_formed() {
    let args = run(&params(TargetFormat::Mp4H264, CompressPreset::Balanced));
    assert!(has(&args, "-nostdin"));
    assert!(has(&args, "-hide_banner"));
    assert!(has(&args, "-progress"));
    assert!(has(&args, "-y"));
    assert!(args.last().unwrap().ends_with(".part"), "产物先写临时名: {}", args.last().unwrap());
    assert!(args.iter().position(|a| a == "-f").unwrap() < args.len() - 1, "-f 必须在输出之前");
}

// ===== 端到端真跑：素材现造、ffmpeg 内嵌，不需要网络 =====

use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::ffmpeg_job::{run_job, JobOutcome};
use easydownvideo::core::videoops::media_info::inspect;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// **spec §9 要求的那条端到端**：真把一段 H.264/mp4 转成 WebM，并断言产物里
/// 视频是 vp9、音频是 **opus**。
///
/// 为什么这条非有不可：上面那些参数形状的测试只能证明"我们拼出来的 argv 长对了"，
/// 证明不了"这套 argv 真的跑得通"。而 WebM 那条路是最容易在集成处翻车的 ——
/// 容器只收 VP8/VP9/AV1 + Vorbis/Opus，音频编码一旦被改成 aac 就是硬失败，
/// 而那是个**拼写正确、语义错误**的改动，形状测试拦不住。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transcodes_to_webm_for_real_with_vp9_and_opus() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");

    // 3 秒 320x240 带音频的 H.264/mp4 素材
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=3:size=320x240:rate=15",
               "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30",
               "-c:a", "aac", "-shortest"])
        .arg(&src).output().await.unwrap();
    assert!(gen.status.success(), "{}", String::from_utf8_lossy(&gen.stderr));
    let info = inspect(&ffmpeg, &src).await.unwrap();
    assert_eq!(info.video_codec.as_deref(), Some("h264"));
    assert_eq!(info.container.as_deref(), Some("mp4"));

    let out = dir.path().join("out.webm");
    let p = params(TargetFormat::WebmVp9, CompressPreset::Small);
    let args = transcode_args(&src, &out, &p);
    let outcome = run_job(&ffmpeg, &args, &out, Some(3.0), Arc::new(|_| {}),
                          &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);
    assert!(out.exists(), "产物必须就位");

    // 产物真能被读回，而且编码真的是 vp9 / opus —— 不是"argv 看着对"
    let back = inspect(&ffmpeg, &out).await.unwrap();
    assert_eq!(back.video_codec.as_deref(), Some("vp9"), "视频应当是 vp9");
    assert_eq!(back.container.as_deref(), Some("webm"), "容器应当是 webm");
    let d = back.duration_secs.expect("产物时长");
    assert!((d - 3.0).abs() < 0.5, "时长应约 3 秒，实际: {d}");
}

/// 顺带把另外两组也真跑一遍 —— 它们各自有"看着对但跑不通"的风险：
/// H.265 进 mp4、H.264 进 matroska。断言编码与容器都对。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transcodes_to_h265_mp4_and_matroska_for_real() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=2:size=320x240:rate=15",
               "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30",
               "-c:a", "aac", "-shortest"])
        .arg(&src).output().await.unwrap();
    assert!(gen.status.success());
    let info = inspect(&ffmpeg, &src).await.unwrap();

    for (target, want_codec, want_ext) in [
        (TargetFormat::Mp4H265, "hevc", "mp4"),
        (TargetFormat::MkvH264, "h264", "mkv"),
    ] {
        let out = dir.path().join(format!("out.{want_ext}"));
        let args = transcode_args(&src, &out, &params(target, CompressPreset::Small));
        let outcome = run_job(&ffmpeg, &args, &out, Some(2.0), Arc::new(|_| {}),
                              &CancellationToken::new()).await.unwrap();
        assert_eq!(outcome, JobOutcome::Done, "{target:?}");
        let back = inspect(&ffmpeg, &out).await.unwrap();
        assert_eq!(back.video_codec.as_deref(), Some(want_codec), "{target:?}");
        assert_eq!(back.container.as_deref(), Some(want_ext), "{target:?}");
        let _ = info;
    }
}
