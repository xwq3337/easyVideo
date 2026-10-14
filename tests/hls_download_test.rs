use easydownvideo::core::events::ProgressHandle;
use easydownvideo::core::hls::parse::{MediaInfo, SegmentRef};
use easydownvideo::core::hls::{download_media, plan_segment_indices};
use tokio_util::sync::CancellationToken;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn media_no_crypto(seg_uris: &[String]) -> MediaInfo {
    MediaInfo {
        segments: seg_uris
            .iter()
            .map(|u| SegmentRef {
                uri: u.clone(),
                key: None,
                is_init: false,
            })
            .collect(),
        media_sequence: 0,
        is_fmp4: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_segments_and_builds_concat_plan() {
    let server = MockServer::start().await;
    let s0 = vec![7u8; 100];
    let s1 = vec![8u8; 200];
    Mock::given(method("GET"))
        .and(path("/seg0.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s0.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/seg1.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s1.clone()))
        .mount(&server)
        .await;

    let media = media_no_crypto(&[
        format!("{}/seg0.ts", server.uri()),
        format!("{}/seg1.ts", server.uri()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let progress = ProgressHandle::new();

    let plan = download_media(
        &reqwest::Client::new(),
        &media,
        dir.path(),
        4,
        &progress,
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(plan.segment_files.len(), 2);
    assert_eq!(std::fs::read(&plan.segment_files[0]).unwrap(), s0);
    assert_eq!(std::fs::read(&plan.segment_files[1]).unwrap(), s1);
    // HLS 进度以分片计数：2 个分片
    assert_eq!(progress.snapshot(), (2, 2));
    assert_eq!(progress.unit(), easydownvideo::core::events::ProgressUnit::Segments);
    // concat 列表已生成
    let list = std::fs::read_to_string(&plan.list_file).unwrap();
    assert!(list.contains("file '"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumes_existing_segments() {
    let server = MockServer::start().await;
    let s1 = vec![9u8; 50];
    // 只挂 seg1：seg0 若被重新请求会 404
    Mock::given(method("GET"))
        .and(path("/seg1.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s1.clone()))
        .mount(&server)
        .await;

    let media = media_no_crypto(&[
        format!("{}/seg0.ts", server.uri()),
        format!("{}/seg1.ts", server.uri()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("seg-00000.ts"), vec![7u8; 100]).unwrap();

    let plan = download_media(
        &reqwest::Client::new(),
        &media,
        dir.path(),
        2,
        &ProgressHandle::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&plan.segment_files[0]).unwrap(), vec![7u8; 100]);
    assert_eq!(std::fs::read(&plan.segment_files[1]).unwrap(), s1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_failed_segment() {
    let server = MockServer::start().await;
    let s0 = vec![7u8; 100];
    // 第一次请求 500，之后 200：验证重试逻辑
    Mock::given(method("GET"))
        .and(path("/flaky.ts"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/flaky.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s0.clone()))
        .mount(&server)
        .await;
    let media = media_no_crypto(&[format!("{}/flaky.ts", server.uri())]);
    let dir = tempfile::tempdir().unwrap();
    let plan = download_media(
        &reqwest::Client::new(),
        &media,
        dir.path(),
        1,
        &ProgressHandle::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&plan.segment_files[0]).unwrap(), s0);
}

#[test]
fn init_segment_does_not_shift_media_indices() {
    // IV 由 `media_sequence + 媒体序号` 推导。init 段（fMP4 的 EXT-X-MAP）不占媒体序号，
    // 若按 segments 数组下标推导 IV，init 之后的每个分片序号都会多 1，解出乱码。
    let segs = vec![
        SegmentRef { uri: "init.mp4".into(), key: None, is_init: true },
        SegmentRef { uri: "s0.m4s".into(), key: None, is_init: false },
        SegmentRef { uri: "s1.m4s".into(), key: None, is_init: false },
        SegmentRef { uri: "s2.m4s".into(), key: None, is_init: false },
    ];
    assert_eq!(
        plan_segment_indices(&segs),
        vec![(0, 0), (1, 0), (2, 1), (3, 2)],
        "第一个媒体分片的媒体序号必须是 0，而不是它在数组里的下标 1"
    );

    // 无 init 段时，下标即媒体序号
    let plain = vec![
        SegmentRef { uri: "a.ts".into(), key: None, is_init: false },
        SegmentRef { uri: "b.ts".into(), key: None, is_init: false },
    ];
    assert_eq!(plan_segment_indices(&plain), vec![(0, 0), (1, 1)]);
}
