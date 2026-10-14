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

#[test]
fn load_from_corrupt_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("settings.json");
    std::fs::write(&cfg, "{ not valid json").unwrap();
    assert!(Settings::load_from(&cfg).is_none());
}
