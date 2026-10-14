use easydownvideo::core::ffmpeg::{build_remux, ensure_ffmpeg};
use std::path::PathBuf;

#[test]
fn ensure_extracts_and_reuses() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = ensure_ffmpeg(dir.path()).unwrap();
    assert!(p1.exists());
    let m1 = std::fs::metadata(&p1).unwrap().modified().unwrap();
    let p2 = ensure_ffmpeg(dir.path()).unwrap();
    assert_eq!(p1, p2);
    // 二次调用不重写（修改时间不变即复用）
    assert_eq!(m1, std::fs::metadata(&p2).unwrap().modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p1).unwrap().permissions().mode() & 0o111, 0o111);
    }
}

#[test]
fn build_remux_args_are_container_agnostic() {
    let ffmpeg = PathBuf::from("/usr/bin/ffmpeg");
    let list = PathBuf::from("/tmp/concat.txt");
    let out = PathBuf::from("/tmp/o.mp4");
    let cmd = build_remux(&ffmpeg, &list, &out);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let joined = args.join(" ");
    assert!(joined.contains("-f concat"));
    assert!(joined.contains("-safe 0"));
    assert!(joined.contains("-c copy"), "必须是无损转封装");
    assert!(args.iter().any(|a| a == "/tmp/o.mp4"), "输出路径要作为最后一个参数");
    assert!(args.iter().any(|a| a == "-y"), "需覆盖已存在的输出文件");
    // 显式 aac_adtstoasc 会让非 AAC 音频（AC-3 等）的 TS 合并直接失败；
    // 交给 ffmpeg 按需自动插入。见 build_remux 中的实测说明。
    assert!(
        !joined.contains("aac_adtstoasc"),
        "不应硬编码 aac_adtstoasc，否则 AC-3 音轨的 TS 会合并失败"
    );
}
