use easydownvideo::core::videoops::media_info::MediaInfo;
use easydownvideo::core::videoops::merge::{
    check, concat_list, merge_args, merge_extension, merge_muxer,
};
use easydownvideo::core::videoops::params::{JobContext, JobParams};
use std::path::{Path, PathBuf};

fn m(vcodec: &str, acodec: Option<&str>, container: &str, w: u32, h: u32) -> MediaInfo {
    MediaInfo {
        duration_secs: Some(2.0),
        has_video: true,
        video_codec: Some(vcodec.into()),
        audio_codec: acodec.map(|s| s.to_string()),
        container: Some(container.into()),
        width: Some(w),
        height: Some(h),
    }
}

/// **列表的转义必须走 `escape_concat_path`。**
///
/// 这不是形式主义：上一轮 HLS 就是栽在同一个地方 —— 下载目录里带撇号
/// （`~/My's Videos`）时，ffmpeg 的 concat 解析器把 `'` 当成引号结束，
/// **每一次合并都会失败在最后一步**。所以这条是回归测试。
#[test]
fn concat_list_escapes_apostrophes_in_paths() {
    let inputs = vec![
        PathBuf::from("/vids/My's Videos/第一集.mp4"),
        PathBuf::from("/vids/普通 名字.mp4"),
    ];
    let list = concat_list(&inputs);
    assert_eq!(
        list,
        "file '/vids/My'\\''s Videos/第一集.mp4'\nfile '/vids/普通 名字.mp4'\n"
    );
    // 撇号被转义成 '\'' —— 一行里恰好出现一次 file
    assert_eq!(list.matches("file '").count(), 2);
}

/// 编码不一致是**拦阻**，不是警告 —— 实测：h264+mpeg4 拼出来的文件解码器报
/// 174 行错（而 ffmpeg 报 exit 0），h264+h265 报 3 行。
#[test]
fn mismatched_video_codec_is_a_blocker() {
    let c = check(&[m("h264", Some("aac"), "mp4", 640, 360),
                    m("hevc", Some("aac"), "mp4", 640, 360)]);
    assert_eq!(c.blockers.len(), 1);
    assert!(c.blockers[0].contains("hevc"), "要说清是哪一段的什么编码: {:?}", c.blockers[0]);
    assert!(c.blockers[0].contains("第 2 段"), "要说清是第几段: {:?}", c.blockers[0]);
}

#[test]
fn mismatched_audio_codec_is_a_blocker() {
    let c = check(&[m("h264", Some("aac"), "mp4", 640, 360),
                    m("h264", Some("opus"), "mp4", 640, 360)]);
    assert_eq!(c.blockers.len(), 1);
    assert!(c.blockers[0].contains("opus"));
}

/// **没有音轨不等于编码不一致。** 实测：一段没音轨 + 一段有，拼出来 0 解码错误，
/// 只是音轨是断的 —— 那该是警告而不是拦阻，否则用户会遇到"明明能拼却不让我拼"。
#[test]
fn missing_audio_is_a_warning_not_a_blocker() {
    let c = check(&[m("h264", None, "mp4", 640, 360),
                    m("h264", Some("aac"), "mp4", 640, 360)]);
    assert!(c.blockers.is_empty(), "不该拦: {:?}", c.blockers);
    assert_eq!(c.warnings.len(), 1);
    assert!(c.warnings[0].contains("音轨"));
}

/// 其余差异放行但警告 —— 分辨率不同实测 0 错误、帧率不同 2 处错误。
#[test]
fn other_differences_are_warnings() {
    let c = check(&[m("h264", Some("aac"), "mp4", 640, 360),
                    m("h264", Some("aac"), "mp4", 1280, 720)]);
    assert!(c.blockers.is_empty());
    assert_eq!(c.warnings.len(), 1);
    assert!(c.warnings[0].contains("尺寸"));
}

#[test]
fn identical_inputs_have_no_blockers_and_no_warnings() {
    let c = check(&[m("h264", Some("aac"), "mp4", 640, 360),
                    m("h264", Some("aac"), "mp4", 640, 360)]);
    assert!(c.blockers.is_empty() && c.warnings.is_empty(), "{c:?}");
    // 单段也无话可说
    let c = check(&[m("h264", Some("aac"), "mp4", 640, 360)]);
    assert!(c.blockers.is_empty() && c.warnings.is_empty());
}

#[test]
fn argv_uses_the_concat_demuxer_and_copies() {
    let args = merge_args(Path::new("/tmp/list.txt"), Path::new("/out/all.mp4"),
                          &m("h264", Some("aac"), "mp4", 640, 360));
    let has = |n: &str| args.iter().any(|a| a == n);
    assert!(has("-f") && has("concat") && has("-safe") && has("0"));
    assert!(has("-i") && args.iter().any(|a| a == "/tmp/list.txt"));
    assert!(has("-c") && args.iter().any(|a| a == "copy"), "合并是无损拼接");
    assert!(has("-nostdin") && has("-progress") && has("-y"));
    assert_eq!(args[args.len() - 2], "mp4", "显式 -f mp4 在输出之前");
    assert!(args.last().unwrap().ends_with(".part"), "产物先写临时名");
    assert!(!args.iter().any(|a| a == "-crf"), "拼接不该重编码");
}

/// 容器沿用**首段**；认不出来退 matroska（什么都装得下）。退化的结果会显示在
/// 产物名预览里，不是悄悄发生的。
#[test]
fn output_container_follows_the_first_input() {
    let mk = m("h264", Some("aac"), "mkv", 640, 360);
    assert_eq!(merge_muxer(&mk), "matroska");
    assert_eq!(merge_extension(&mk), "mkv");
    let webm = m("vp9", Some("opus"), "webm", 640, 360);
    assert_eq!(merge_muxer(&webm), "webm");
    // 认不出来 → matroska
    let odd = MediaInfo { container: None, ..m("h264", Some("aac"), "mp4", 640, 360) };
    assert_eq!(merge_muxer(&odd), "matroska");
    assert_eq!(merge_extension(&odd), "mkv");
}

/// 进度条的分母对合并是**各段之和**。
///
/// 用首段的时长会让进度条在第一段结束时就走到 100%，后面几段看起来像卡住了 ——
/// 而那正是 spec §5.4 点名的坏结果。这条纯测试把分母钉死，不必去观测时序。
#[test]
fn merge_duration_is_the_sum_of_all_segments() {
    let inputs = vec![PathBuf::from("/in/a.mp4"), PathBuf::from("/in/b.mp4")];
    let infos = vec![
        MediaInfo { duration_secs: Some(2.0), ..m("h264", Some("aac"), "mp4", 640, 360) },
        MediaInfo { duration_secs: Some(3.5), ..m("h264", Some("aac"), "mp4", 640, 360) },
    ];
    let out = PathBuf::from("/out/all.mp4");
    let ctx = JobContext { inputs: &inputs, infos: &infos, output: &out, scratch: None };
    assert_eq!(JobParams::Merge.duration_secs(&ctx), Some(5.5), "各段之和");

    // 任一段读不出来 → 整体退化成"时长未知"，不编一个数出来
    let infos = vec![
        MediaInfo { duration_secs: Some(2.0), ..m("h264", Some("aac"), "mp4", 640, 360) },
        MediaInfo { duration_secs: None, ..m("h264", Some("aac"), "mp4", 640, 360) },
    ];
    let ctx = JobContext { inputs: &inputs, infos: &infos, output: &out, scratch: None };
    assert_eq!(JobParams::Merge.duration_secs(&ctx), None);
}

// ===== 端到端真跑（素材现造、ffmpeg 内嵌，不需要网络）=====

use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::ffmpeg_job::{run_job, JobOutcome};
use easydownvideo::core::videoops::media_info::inspect;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

async fn make(ffmpeg: &Path, path: &Path, secs: u32, vcodec: &str) {
    let mut cmd = tokio::process::Command::new(ffmpeg);
    cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i",
              &format!("testsrc2=duration={secs}:size=320x240:rate=15"),
              "-f", "lavfi", "-i", &format!("sine=frequency=440:duration={secs}")]);
    match vcodec {
        "h265" => { cmd.args(["-c:v", "libx265", "-crf", "30", "-tag:v", "hvc1"]); }
        _ => { cmd.args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"]); }
    }
    let out = cmd.args(["-c:a", "aac", "-shortest"]).arg(path).output().await.unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// 三段同规格 → 拼成一个，**时长≈三段之和**，能解码。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn merges_three_identical_clips_for_real() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let parts: Vec<PathBuf> =
        ["a", "b", "c"].iter().map(|n| dir.path().join(format!("{n}.mp4"))).collect();
    for p in &parts { make(&ffmpeg, p, 2, "h264").await; }
    let infos: Vec<_> = { let mut v = Vec::new(); for p in &parts { v.push(inspect(&ffmpeg, p).await.unwrap()); } v };
    assert!(check(&infos).blockers.is_empty(), "同规格不该有拦阻");

    let list = dir.path().join("merge-list.txt");
    std::fs::write(&list, concat_list(&parts)).unwrap();
    let out = dir.path().join("all.mp4");
    let args = merge_args(&list, &out, &infos[0]);
    let outcome = run_job(&ffmpeg, &args, &out, Some(6.0), Some(&list),
                          Arc::new(|_| {}), &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);
    assert!(!list.exists(), "列表文件必须被清掉");

    let back = inspect(&ffmpeg, &out).await.unwrap();
    let d = back.duration_secs.expect("产物时长");
    assert!((d - 6.0).abs() < 0.4, "时长应≈6 秒（三段各 2 秒），实际: {d}");
    assert_eq!(back.video_codec.as_deref(), Some("h264"));
    assert_eq!(back.has_video, true);
}

/// **h264 + h265 必须被拦下。** 这条守的是 spec §2 那 174 行（h264+mpeg4）/
/// 3 行（h264+h265）解码错误 —— ffmpeg 会报 exit 0，所以拦的只能是我们的 check。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_to_merge_mismatched_codecs() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let a = dir.path().join("a.mp4");
    let b = dir.path().join("b.mp4");
    make(&ffmpeg, &a, 2, "h264").await;
    make(&ffmpeg, &b, 2, "h265").await;
    let infos = vec![inspect(&ffmpeg, &a).await.unwrap(), inspect(&ffmpeg, &b).await.unwrap()];
    assert_eq!(infos[0].video_codec.as_deref(), Some("h264"));
    assert_eq!(infos[1].video_codec.as_deref(), Some("hevc"));
    let c = check(&infos);
    assert_eq!(c.blockers.len(), 1, "必须拦下: {c:?}");
}

/// 路径里带撇号也拼得成 —— 转义的端到端版本。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn merges_from_a_directory_with_an_apostrophe() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let sub = dir.path().join("My's Videos");
    std::fs::create_dir_all(&sub).unwrap();
    let a = sub.join("第一集.mp4");
    let b = sub.join("第二集.mp4");
    make(&ffmpeg, &a, 1, "h264").await;
    make(&ffmpeg, &b, 1, "h264").await;
    let infos = vec![inspect(&ffmpeg, &a).await.unwrap(), inspect(&ffmpeg, &b).await.unwrap()];

    let list = dir.path().join("list.txt");
    std::fs::write(&list, concat_list(&[a.clone(), b.clone()])).unwrap();
    let out = sub.join("合起来.mp4");
    let args = merge_args(&list, &out, &infos[0]);
    let outcome = run_job(&ffmpeg, &args, &out, Some(2.0), None,
                          Arc::new(|_| {}), &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);
    let d = inspect(&ffmpeg, &out).await.unwrap().duration_secs.unwrap();
    assert!((d - 2.0).abs() < 0.3, "两段各 1 秒应得 2 秒，实际: {d}");
}
