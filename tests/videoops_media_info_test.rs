use easydownvideo::core::videoops::media_info::{parse_duration, parse_has_video, parse_media_info};

/// 真实输出（ffmpeg 9.0.2 实测）。
#[test]
fn parses_the_real_duration_line() {
    let stderr = "\
ffmpeg version 9.0.2 Copyright (c) 2000-2026 the FFmpeg developers
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'src.mp4':
  Metadata:
    major_brand     : isom
  Duration: 00:00:05.00, start: 0.000000, bitrate: 55 kb/s
  Stream #0:0[0x1](und): Video: h264 (High), yuv420p, 640x360, 50 kb/s, 30 fps
";
    assert_eq!(parse_duration(stderr), Some(5.0));
}

#[test]
fn handles_hours_and_fractional_seconds() {
    let long = "  Duration: 01:23:45.67, start: 0.000000, bitrate: 1200 kb/s";
    let got = parse_duration(long).expect("应能解析");
    assert!((got - (3600.0 + 23.0 * 60.0 + 45.67)).abs() < 0.01, "实际: {got}");
}

/// Duration 为 `N/A` 时是有的（流式输入、损坏文件）。
/// **不能返回 Some(0.0)** —— 那会让进度条永远停在 100% 或除零。
#[test]
fn unavailable_duration_is_none_not_zero() {
    assert_eq!(parse_duration("  Duration: N/A, start: 0.000000, bitrate: N/A"), None);
}

/// 没有 Duration 行（纯音频、损坏文件、ffmpeg 直接报错）时一律 None。
/// 调用方据此把进度退化为不确定态，任务本身照跑。
#[test]
fn missing_line_is_none() {
    assert_eq!(parse_duration(""), None);
    assert_eq!(parse_duration("a.mp4: Invalid data found when processing input"), None);
    assert_eq!(parse_duration("  Duration: 00:00:0"), None); // 截断的行
}

/// 认的是 ffmpeg 打印的流信息行，形如
/// `  Stream #0:0[0x1](und): Video: h264 (High), yuv420p, ...`
#[test]
fn detects_a_video_stream() {
    let real = "  Stream #0:0[0x1](und): Video: h264 (High) (avc1), yuv420p, 640x360, 30 fps";
    assert!(parse_has_video(real));

    // 只有音频 —— 这正是"压缩视频却拿到一个没有画面的文件"那个坑
    assert!(!parse_has_video("  Stream #0:0(und): Audio: aac (LC), 44100 Hz, stereo"));

    // 元数据里出现 Video 字样**不能**误判成有视频流
    assert!(!parse_has_video("    handler_name    : VideoHandler"));

    // 损坏文件：连 Stream 行都没有
    assert!(!parse_has_video("a.mp4: Invalid data found when processing input"));
}

/// 一次 `ffmpeg -i` 同时给出时长与有没有视频流 —— 只跑一遍，不是两遍。
#[test]
fn one_pass_yields_everything_we_need() {
    // 一次 `ffmpeg -i` 同时给出时长、有没有视频流、编码、容器 —— 不跑第二遍
    let stderr = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':\n\
  Duration: 00:00:05.00, start: 0.000000, bitrate: 55 kb/s\n\
  Stream #0:0[0x1](und): Video: h264 (High), yuv420p, 640x360";
    assert_eq!(
        parse_media_info(stderr, Path::new("/a/x.mp4")),
        easydownvideo::core::videoops::media_info::MediaInfo {
            duration_secs: Some(5.0),
            has_video: true,
            video_codec: Some("h264".to_string()),
            container: Some("mp4".to_string()),
        }
    );
}

// ===== 容器与视频编码：转码页要靠它们显示"源：H.264 · mp4" =====

use easydownvideo::core::videoops::media_info::{
    container_muxer, parse_container, parse_video_codec,
};
use std::path::Path;

/// 真实输出（ffmpeg 9.0.2 实测）。
#[test]
fn parses_video_codec_from_the_stream_line() {
    let stderr = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':
  Duration: 00:00:30.00, start: 0.000000, bitrate: 55 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1), yuv420p, 640x360, 30 fps
";
    assert_eq!(parse_video_codec(stderr).as_deref(), Some("h264"));
    let hevc = "  Stream #0:0(und): Video: hevc (Main) (hev1), yuv420p, 1920x1080";
    assert_eq!(parse_video_codec(hevc).as_deref(), Some("hevc"));
    // 纯音频：没有 Video: 行
    let audio = "  Stream #0:0(und): Audio: aac (LC), 44100 Hz, stereo";
    assert_eq!(parse_video_codec(audio), None);
}

/// **mkv 与 webm 在 ffmpeg 眼里是同一个 demuxer**（`matroska,webm`），
/// 光靠 stderr 分不开 —— 只能看扩展名定夺。
#[test]
fn parses_container_and_disambiguates_mkv_from_webm() {
    let mp4 = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':";
    assert_eq!(parse_container(mp4, Path::new("/a/x.mp4")).as_deref(), Some("mp4"));
    assert_eq!(parse_container(mp4, Path::new("/a/x.mov")).as_deref(), Some("mov"));

    let matroska = "Input #0, matroska,webm, from 'x.mkv':";
    assert_eq!(parse_container(matroska, Path::new("/a/x.mkv")).as_deref(), Some("mkv"));
    assert_eq!(parse_container(matroska, Path::new("/a/x.webm")).as_deref(), Some("webm"));

    // 认不出来时 None，不编一个
    let odd = "Input #0, mpegts, from 'x.ts':";
    assert_eq!(parse_container(odd, Path::new("/a/x.ts")), None);
    assert_eq!(parse_container("no input line at all", Path::new("/a/x.mp4")), None);
}

#[test]
fn maps_container_to_muxer_and_extension() {
    // **muxer 名与扩展名不是同一个词** —— mkv 的 muxer 叫 matroska。
    // 写成 -f mkv 会直接失败。
    assert_eq!(container_muxer("mkv"), Some(("matroska", "mkv")));
    assert_eq!(container_muxer("webm"), Some(("webm", "webm")));
    assert_eq!(container_muxer("mp4"), Some(("mp4", "mp4")));
    assert_eq!(container_muxer("mov"), Some(("mov", "mov")));
    assert_eq!(container_muxer("nope"), None);
}
