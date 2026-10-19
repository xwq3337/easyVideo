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
        headers: Vec::new(),
        audio: None,
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

#[test]
fn scan_ignores_dirs_without_task_prefix() {
    // 下载目录可以由用户改成 ~/Documents 这类地方，而 `manifest.json` 是极常见的文件名
    // （Web App Manifest / Chrome 扩展 / VS Code）。没有前缀过滤，这些目录会被当成可续传任务。
    let root = tempfile::tempdir().unwrap();

    // 一个**合法但非任务**的目录：manifest.json 内容完全能解析成 TaskManifest
    let alien = root.path().join("some-web-app");
    std::fs::create_dir_all(&alien).unwrap();
    std::fs::write(
        alien.join("manifest.json"),
        r#"{"id":7,"url":"https://x/v.mp4","filename":"v.mp4","download_dir":"/tmp","kind":{"Direct":{"size":null}}}"#,
    )
    .unwrap();

    // 同时放一个真任务，确认过滤没有把真任务也滤掉
    let real = task_dir(root.path(), 3);
    std::fs::create_dir_all(&real).unwrap();
    write_manifest(
        &real,
        &TaskManifest {
            id: 3,
            url: "https://example.com/v.mp4".into(),
            filename: "v.mp4".into(),
            download_dir: root.path().join("done"),
            kind: ManifestKind::Direct { size: None },
            headers: Vec::new(),
            audio: None,
        },
    )
    .unwrap();

    let found = scan_resume(root.path());
    assert_eq!(
        found.len(),
        1,
        "只应认出 task-3；`some-web-app` 里的 manifest.json 再合法也不是任务。实际: {found:?}"
    );
    assert_eq!(found[0].id, 3);
}

#[test]
fn manifest_roundtrips_hls_kind() {
    // HLS 续传是核心功能（Task 8 就是为它写的），它的 kind 变体必须真的能往返，
    // 否则 serde 一旦标签写错，HLS 任务的断点恢复会静默失效 —— 而单测只测 Direct 是看不出来的。
    let root = tempfile::tempdir().unwrap();
    let dir = task_dir(root.path(), 9);
    std::fs::create_dir_all(&dir).unwrap();
    let m = TaskManifest {
        id: 9,
        url: "https://example.com/page".into(),
        filename: "video.mp4".into(),
        download_dir: root.path().join("done"),
        kind: ManifestKind::Hls { playlist_url: "https://cdn.example.com/master.m3u8".into() },
        headers: Vec::new(),
        audio: None,
    };
    write_manifest(&dir, &m).unwrap();

    let found = scan_resume(root.path());
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].kind,
        ManifestKind::Hls { playlist_url: "https://cdn.example.com/master.m3u8".into() },
        "Hls 变体必须原样往返"
    );
    assert_eq!(found[0].kind, m.kind, "整体必须相等");
}
