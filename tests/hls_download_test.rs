use easydownvideo::core::events::ProgressHandle;
use easydownvideo::core::hls::parse::{KeyRef, MediaInfo, SegmentRef};
use easydownvideo::core::hls::{download_media, plan_segment_indices};
use tokio_util::sync::CancellationToken;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn unhex(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    bytes
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decrypts_with_derived_iv_across_an_init_offset() {
    // 这是对「数组下标 vs 媒体序号」这条不变式的端到端验证，而不只是测那个纯函数。
    // 设计要点：MediaInfo 里第一个元素是 init 段，所以媒体分片的**数组下标**是 1、2，
    // 而**媒体序号**是 0、1。若实现用 idx 推 IV，第一个分片就会拿 IV1 去解，
    // PKCS7 校验必然失败（或即便碰巧通过，明文断言也会挂）→ 测试失败。
    //
    // 向量用 openssl 生成（明文 32 字节 "0123456789abcdef0123456789abcdef"，key/IV 如下）：
    //   IV0 = 0x00..00（media_sequence 0 + 媒体序号 0）
    //   IV1 = 0x00..01（media_sequence 0 + 媒体序号 1）
    //   openssl enc -aes-128-cbc -K 00112233445566778899aabbccddeeff -iv <IV> -in pt32.bin | xxd -p
    const CT_IV0: &str = "aebdd39b144089d5e31cacd35b7a113aad3edcb953ecb7bc0147f441cc48eff7\
                          71ac64c87a490d1f4f09cac5d740f31b";
    const CT_IV1: &str = "430df7fdf3d0ec5e7f44bf99464a00f098ff2795c5f955d82f453683c8dbf1c6\
                          2024e95979a1b81afe3cac283088663a";
    const KEY_HEX: &str = "00112233445566778899aabbccddeeff";
    const PLAINTEXT: &[u8] = b"0123456789abcdef0123456789abcdef";

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/key.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(unhex(KEY_HEX)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/init.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 16]))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/e0.m4s"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(unhex(CT_IV0)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/e1.m4s"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(unhex(CT_IV1)))
        .mount(&server)
        .await;

    // 注意 iv: None —— 走"按媒体序号推导 IV"这条路径，这正是要验证的分支
    let key = KeyRef {
        uri: format!("{}/key.bin", server.uri()),
        iv: None,
    };
    let media = MediaInfo {
        segments: vec![
            SegmentRef {
                uri: format!("{}/init.mp4", server.uri()),
                key: None,
                is_init: true,
            },
            SegmentRef {
                uri: format!("{}/e0.m4s", server.uri()),
                key: Some(key.clone()),
                is_init: false,
            },
            SegmentRef {
                uri: format!("{}/e1.m4s", server.uri()),
                key: Some(key),
                is_init: false,
            },
        ],
        media_sequence: 0,
        is_fmp4: true,
    };

    let dir = tempfile::tempdir().unwrap();
    let plan = download_media(
        &reqwest::Client::new(),
        &media,
        dir.path(),
        4,
        &ProgressHandle::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(plan.segment_files.len(), 3, "init + 两个媒体分片");
    // init 段按 idx 命名（`seg-00000.{ext}`），不是 `init.*`：断言它必须是第一个。
    assert_eq!(
        plan.segment_files[0],
        dir.path().join("seg-00000.m4s"),
        "init 段必须排在最前（数组下标 0）"
    );
    for i in [1usize, 2usize] {
        assert_eq!(
            std::fs::read(&plan.segment_files[i]).unwrap(),
            PLAINTEXT,
            "分片 {i} 解密结果不对 —— 检查 IV 是否用了媒体序号而非数组下标"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn truncated_leftover_segment_is_not_treated_as_complete() {
    // 只判 exists() 的续传会把上次中断留下的半截分片当完成，ffmpeg 之后产出损坏文件而不报错。
    // 完成标志必须是"写临时名后原子改名"的结果。
    let server = MockServer::start().await;
    let s0 = vec![7u8; 100];
    Mock::given(method("GET"))
        .and(path("/seg0.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s0.clone()))
        .mount(&server)
        .await;

    let media = media_no_crypto(&[format!("{}/seg0.ts", server.uri())]);
    let dir = tempfile::tempdir().unwrap();
    // 预置上次中断留下的半截**临时文件**（新协议下 create 写的是 `.part`，被杀即留下它）。
    // 注意：不能把它写成最终名 `seg-00000.ts`——按"改名成功才算完成"的语义，最终名存在就
    // 等价于完整，40 字节的最终名文件会被直接信任；且 `resumes_existing_segments` 证明
    // 续传判定不发请求、无从得知期望大小。被截断的产物只可能是 `.part`。
    std::fs::write(dir.path().join("seg-00000.ts.part"), vec![7u8; 40]).unwrap();

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
    assert_eq!(
        std::fs::read(&plan.segment_files[0]).unwrap(),
        s0,
        "被截断的遗留分片必须重新下载，不能当成已完成"
    );
}

#[test]
fn escape_concat_path_escapes_single_quotes() {
    use easydownvideo::core::hls::escape_concat_path;
    // 实测：不转义时 ffmpeg 把 `file '/tmp/My's Videos/seg.ts'` 解析成 `/tmp/Mys` 并报
    // "Impossible to open"。用户把下载目录设在含撇号的路径下（如 ~/My's Videos）很常见，
    // 不转义等于**每一次 HLS 下载都在合并这一步失败**。
    assert_eq!(escape_concat_path("/tmp/a.ts"), "/tmp/a.ts", "无特殊字符时原样返回");
    assert_eq!(
        escape_concat_path("/tmp/My's Videos/a.ts"),
        "/tmp/My'\\''s Videos/a.ts",
        "路径里的单引号必须转义成 '\\''"
    );
    // 多个撇号都要转义
    assert_eq!(escape_concat_path("it's a 'test'.ts"), "it'\\''s a '\\''test'\\''.ts");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concat_list_uses_escaped_paths() {
    // 端到端确认生成的 concat.txt 真的用了转义（纯函数测试保护不了"有没有调用它"）
    let server = MockServer::start().await;
    let s0 = vec![7u8; 100];
    Mock::given(method("GET"))
        .and(path("/seg0.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s0.clone()))
        .mount(&server)
        .await;

    let media = media_no_crypto(&[format!("{}/seg0.ts", server.uri())]);
    // 任务目录名固定为 task-<id>，但父目录来自 download_dir —— 这里用一个带撇号的临时目录
    let parent = tempfile::tempdir().unwrap();
    let weird = parent.path().join("My's Videos");
    std::fs::create_dir_all(&weird).unwrap();
    let task_dir = weird.join("task-1");

    let plan = download_media(
        &reqwest::Client::new(),
        &media,
        &task_dir,
        1,
        &ProgressHandle::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();

    let list = std::fs::read_to_string(&plan.list_file).unwrap();
    assert!(
        list.contains("'\\''"),
        "concat 列表里的单引号必须已转义，实际内容: {list:?}"
    );
    assert!(
        !list.contains("My's"),
        "未转义的 `My's` 形态不应出现（那说明转义没生效）: {list:?}"
    );
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
