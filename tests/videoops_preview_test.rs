use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::preview::{base64_encode, frame_data_url, PREVIEW_MAX_WIDTH};
use std::path::Path;

/// 自己写 base64 是为了**不新增依赖** —— 十几行，配一条与已知编码对照的测试。
#[test]
fn base64_matches_known_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    // 非 ASCII 字节也要对（JPEG 里全是）
    assert_eq!(base64_encode(&[0xff, 0xd8, 0xff]), "/9j/");
}

/// 真抽一帧：产物必须是能认得出来的 JPEG，而且是 data URL。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extracts_a_jpeg_data_url() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=3:size=1280x720:rate=30",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "28"])
        .arg(&src).output().await.unwrap();
    assert!(gen.status.success());

    let url = frame_data_url(&ffmpeg, &src, 1.0, PREVIEW_MAX_WIDTH).await.unwrap();
    assert!(url.starts_with("data:image/jpeg;base64,"), "要能直接塞进 img.src");
    // 解码回来的头两个字节必须是 JPEG 的 SOI（FF D8）
    let b64 = url.strip_prefix("data:image/jpeg;base64,").unwrap();
    assert!(b64.starts_with("/9j/"), "JPEG 的 base64 一定以 /9j/ 开头，实际: {}", &b64[..8.min(b64.len())]);
    // 体积要小到能走 IPC（实测 640 宽约 13.7KB，base64 后 18KB；留足余量）
    assert!(b64.len() < 200_000, "预览图不该这么大: {} 字节", b64.len());
}

/// 时间超出时长时不 panic，而是给一个错误让界面显示一行"取预览失败"。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beyond_the_end_still_returns_something_or_an_error_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=1:size=320x240:rate=15",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"])
        .arg(&src).output().await.unwrap();
    assert!(gen.status.success());
    // 只能保证"不 panic"：ffmpeg 对超出时长 + -ss 的行为是给最后一帧或报错，
    // 两种都能接受，非法的输入不能把进程带走
    let _ = frame_data_url(&ffmpeg, &src, 9999.0, PREVIEW_MAX_WIDTH).await;
}

/// 文件不存在时给 Err，不 panic。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let r = frame_data_url(&ffmpeg, Path::new("/nope/none.mp4"), 0.0, PREVIEW_MAX_WIDTH).await;
    assert!(r.is_err());
}
