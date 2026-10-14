use easydownvideo::core::task::{scan_resume, task_dir, write_manifest, ManifestKind, TaskManifest};

#[test]
fn manifest_roundtrips_and_scans() {
    let root = tempfile::tempdir().unwrap();
    let dir = task_dir(root.path(), 42);
    std::fs::create_dir_all(&dir).unwrap();
    let m = TaskManifest {
        id: 42,
        url: "https://example.com/v.mp4".into(),
        filename: "v.mp4".into(),
        download_dir: root.path().join("done"),
        kind: ManifestKind::Direct { size: Some(123) },
    };
    write_manifest(&dir, &m).unwrap();

    let found = scan_resume(root.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, 42);
    assert_eq!(found[0].filename, "v.mp4");
    match found[0].kind {
        ManifestKind::Direct { size } => assert_eq!(size, Some(123)),
        _ => panic!("expected Direct"),
    }
}

#[test]
fn scan_ignores_broken_and_done_dirs() {
    let root = tempfile::tempdir().unwrap();
    let d = task_dir(root.path(), 1);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("manifest.json"), "not json").unwrap();
    // 已完成任务目录里没有 manifest（引擎完成后删除），此处用空目录模拟
    std::fs::create_dir_all(task_dir(root.path(), 2)).unwrap();
    assert!(scan_resume(root.path()).is_empty());
}
