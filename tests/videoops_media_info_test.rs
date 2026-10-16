use easydownvideo::core::videoops::media_info::parse_duration;

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
