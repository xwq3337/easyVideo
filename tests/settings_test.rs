use easydownvideo::settings::Settings;
use std::path::Path;

#[test]
fn defaults_are_spec_values() {
    let s = Settings::default();
    assert_eq!(s.segment_concurrency, 6);
    assert_eq!(s.max_active_tasks, 2);
    assert!(s.ytdlp_enabled);
    assert!(s.ffmpeg_path_override.is_none());
    assert!(s.download_dir.ends_with("easydownvideo"));
}

#[test]
fn save_load_roundtrip_via_tempdir() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("settings.json");
    let mut s = Settings::default();
    s.segment_concurrency = 9;
    s.ytdlp_enabled = false;
    s.save_to(&cfg).unwrap();
    let loaded = Settings::load_from(&cfg).unwrap();
    assert_eq!(loaded.segment_concurrency, 9);
    assert!(!loaded.ytdlp_enabled);
}

#[test]
fn load_from_missing_file_returns_none() {
    assert!(Settings::load_from(Path::new("/nonexistent/settings.json")).is_none());
}

/// 老版本的 settings.json 里没有这两个新键。反序列化必须**补默认值**，而不是整个失败。
///
/// 失败会很难查：`load_from` 的调用方（`load()`）用 `unwrap_or_default()` 兜底，于是
/// "解析失败"与"文件不存在"表现完全一样 —— 用户升级后所有设置静默回到默认值，
/// 保存目录、线程数全丢，而界面上没有任何提示。
#[test]
fn old_settings_json_without_new_keys_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("settings.json");
    std::fs::write(
        &cfg,
        r#"{
            "download_dir": "/tmp/dl",
            "segment_concurrency": 9,
            "max_active_tasks": 3,
            "ytdlp_enabled": false,
            "ffmpeg_path_override": null
        }"#,
    )
    .unwrap();

    let s = Settings::load_from(&cfg).expect("老配置必须能读进来，不能整份作废");
    assert_eq!(s.segment_concurrency, 9, "已有的键不能被丢掉");
    assert!(!s.ytdlp_enabled);
    assert_eq!(s.cookies_from_browser, None, "新键缺省 = 不读浏览器 cookie");
    assert!(!s.impersonate, "新键缺省 = 不伪装指纹");
}

#[test]
fn load_from_corrupt_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("settings.json");
    std::fs::write(&cfg, "{ not valid json").unwrap();
    assert!(Settings::load_from(&cfg).is_none());
}

#[test]
fn data_dir_is_app_scoped() {
    // 关键：数据目录必须带应用名，且**不等于**"取 config_dir 的 parent 再拼 data"那个共享位置。
    // macOS 上 data_dir 与 config_dir 恰好是同一个路径，两者都带应用名；Linux 上分别是
    // ~/.local/share/easydownvideo 与 ~/.config/easydownvideo。两种平台下本断言都成立。
    let d = Settings::data_dir();
    let c = Settings::config_dir();
    assert!(
        d.to_string_lossy().contains("easydownvideo"),
        "数据目录必须应用专属，实际: {}",
        d.display()
    );
    assert_ne!(
        d,
        c.parent().unwrap().join("data"),
        "不得退回到共享目录（config_dir 的父目录 + data）"
    );
}
