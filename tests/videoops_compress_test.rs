use easydownvideo::core::videoops::compress::{
    compress_args, preset_defaults, CompressParams, CompressPreset, Encoder,
};
use std::path::Path;

fn p(preset: CompressPreset, encoder: Encoder) -> CompressParams {
    CompressParams { preset, encoder, max_height: None, crf: None, qv: None, audio_kbps: None }
}

fn run(params: &CompressParams) -> Vec<String> {
    compress_args(Path::new("/in/a.mp4"), Path::new("/out/a-压缩.mp4"), params)
}

fn has(args: &[String], needle: &str) -> bool {
    args.iter().any(|a| a == needle)
}

/// 硬件编码器**静默忽略** `-crf`（实测：crf 18 与 crf 40 逐字节相同）。
/// 所以这不是"风格偏好"，而是正确性：选硬件时必须出现 `-q:v` 且绝不能出现 `-crf`。
#[test]
fn hardware_uses_qv_and_never_crf() {
    let args = run(&p(CompressPreset::Balanced, Encoder::Hardware));
    assert!(has(&args, "-q:v"), "硬件编码必须用 -q:v，实际: {args:?}");
    assert!(
        !has(&args, "-crf"),
        "硬件编码绝不能带 -crf —— VideoToolbox 会静默忽略它，\
         用户会拿到比源文件还大的产物且全程不报错。实际: {args:?}"
    );
    assert!(has(&args, "h264_videotoolbox"));
}

#[test]
fn software_uses_crf_and_never_qv() {
    let args = run(&p(CompressPreset::Balanced, Encoder::Software));
    assert!(has(&args, "-crf"));
    assert!(!has(&args, "-q:v"), "软件编码不该出现 -q:v。实际: {args:?}");
    assert!(has(&args, "libx264"));
}

/// 三档的数值必须真的不同，否则档位是个摆设。
#[test]
fn presets_differ_in_both_scales() {
    let small = preset_defaults(CompressPreset::Small);
    let balanced = preset_defaults(CompressPreset::Balanced);
    let high = preset_defaults(CompressPreset::High);
    // CRF 越小越清晰
    assert!(small.0 > balanced.0 && balanced.0 > high.0, "CRF 应递减: {small:?} {balanced:?} {high:?}");
    // -q:v 越大越清晰 —— 方向与 CRF 相反，这是最容易写反的一处
    assert!(small.1 < balanced.1 && balanced.1 < high.1, "q:v 应递增: {small:?} {balanced:?} {high:?}");
    assert!(small.2 < balanced.2 && balanced.2 < high.2, "音频码率应递增");
}

/// 高级项留空时一律取预设自带的值。
#[test]
fn advanced_fields_fall_back_to_the_preset() {
    let (crf, _qv, kbps) = preset_defaults(CompressPreset::High);
    let args = run(&p(CompressPreset::High, Encoder::Software));
    let crf_at = args.iter().position(|a| a == "-crf").expect("有 -crf");
    assert_eq!(args[crf_at + 1], crf.to_string());
    let a_at = args.iter().position(|a| a == "-b:a").expect("有 -b:a");
    assert_eq!(args[a_at + 1], format!("{kbps}k"));
    // 显式给了值就以给的为准
    let mut adv = p(CompressPreset::High, Encoder::Software);
    adv.crf = Some(31);
    let args = run(&adv);
    let crf_at = args.iter().position(|a| a == "-crf").unwrap();
    assert_eq!(args[crf_at + 1], "31");
}

#[test]
fn max_height_caps_but_never_upscales() {
    let mut params = p(CompressPreset::Balanced, Encoder::Software);
    params.max_height = Some(720);
    let args = run(&params);
    let vf = args.iter().position(|a| a == "-vf").expect("限制分辨率时要加 -vf");
    // -2 让宽度按比例自动算并取偶数：写成固定宽度会拉伸画面，写成奇数某些编码器会直接失败。
    //
    // **`min()` 是必须的。** 裸的 `scale=-2:720` 会把 426x240 的源**放大**到
    // 1280x720 —— 实测产物体积是源文件的 4.2 倍，而用户点的是"压缩"。
    // 字段的语义是"最长边不超过它"（见 CompressParams 的文档），那就必须真的封顶。
    assert_eq!(args[vf + 1], "scale=-2:'min(720,ih)'");
    assert!(
        !args.iter().any(|a| a == "scale=-2:720"),
        "裸的 scale=-2:h 会把小源放大，产物比源文件还大。实际: {args:?}"
    );
    // 不限制时不该出现 -vf（多余的滤镜会多一次无谓的拷贝）
    assert!(!has(&run(&p(CompressPreset::Balanced, Encoder::Software)), "-vf"));
}

/// 「自动」= 软件 x264：体积画质比最好且可预测。默认走硬件等于默认降质。
#[test]
fn auto_means_software_x264() {
    assert_eq!(run(&p(CompressPreset::Balanced, Encoder::Auto)), run(&p(CompressPreset::Balanced, Encoder::Software)));
}

/// **源文件绝不能被覆盖。** 输出目录被设成源文件所在目录时，用户点两下就撞上了。
#[test]
fn refuses_to_overwrite_the_source_file() {
    let args = compress_args(
        Path::new("/in/a.mp4"),
        Path::new("/in/a.mp4"),
        &p(CompressPreset::Balanced, Encoder::Software),
    );
    // 输入与输出同名时，输出必须以 .part 临时名收尾（由调用方在成功后改名），
    // 且**不得**在输入文件身上动刀。这条靠"输出不是输入"来保证。
    let last = args.last().unwrap();
    assert_ne!(last, "/in/a.mp4");
    assert!(last.ends_with(".part"), "产物应先写临时名，实际: {last}");
}

/// **产物名以 `.part` 收尾会让 ffmpeg 的格式推断失败。**
/// 实测：`ffmpeg ... out.mp4.part` 直接报 "Error opening output file ... Invalid argument"，
/// 因为它按扩展名猜容器，而 `.part` 不是任何已知格式。所以容器必须显式指定。
/// （`.part` 在 HLS 那边没问题，是因为那里的分片文件由我们自己的代码写，不经 ffmpeg。）
#[test]
fn names_the_container_explicitly_so_the_part_suffix_cannot_break_it() {
    let args = run(&p(CompressPreset::Balanced, Encoder::Software));
    let f = args.iter().position(|a| a == "-f").expect("必须显式指定容器格式");
    assert_eq!(args[f + 1], "mp4");
    // `-f` 是**输出**选项，必须排在输出文件之前，否则会作用到输入上
    assert!(f < args.len() - 1, "-f 必须在输出文件之前，实际: {args:?}");
    assert!(args.last().unwrap().ends_with(".part"));
}

#[test]
fn argv_shape_is_well_formed() {
    let args = run(&p(CompressPreset::Balanced, Encoder::Software));
    assert!(has(&args, "-nostdin"), "不阻断 stdin 的话 ffmpeg 会去读它并可能挂住");
    assert!(has(&args, "-hide_banner"));
    assert!(has(&args, "-y"));
    assert!(has(&args, "-progress"), "-progress 是进度的唯一来源");
    // 最后一个参数必须是输出路径
    assert!(args.last().unwrap().ends_with(".mp4.part"));
}

#[test]
fn preset_and_encoder_parse_from_the_wire_strings() {
    assert_eq!("small".parse::<CompressPreset>().unwrap(), CompressPreset::Small);
    assert_eq!("high".parse::<CompressPreset>().unwrap(), CompressPreset::High);
    assert!("huge".parse::<CompressPreset>().is_err());
    assert_eq!("hardware".parse::<Encoder>().unwrap(), Encoder::Hardware);
    assert!("cuda".parse::<Encoder>().is_err());
}

// ===== JobParams：多操作共用的分派点 =====

use easydownvideo::core::videoops::media_info::MediaInfo;
use easydownvideo::core::videoops::params::JobParams;

fn src_mp4() -> MediaInfo {
    // 文件大小**不在** MediaInfo 里（它在命令层，是纯文件系统的事）
    MediaInfo {
        duration_secs: Some(5.0),
        has_video: true,
        video_codec: Some("h264".to_string()),
        audio_codec: Some("aac".to_string()),
        container: Some("mp4".to_string()),
        width: Some(640),
        height: Some(360),
    }
}

/// 同一组参数的一个副本。故意重复构造而不是共享一个可变值 ——
/// 测试之间共享可变状态是难查的 flake 来源。
fn p_match() -> CompressParams {
    CompressParams { preset: CompressPreset::Balanced, encoder: Encoder::Software,
                     max_height: None, crf: None, qv: None, audio_kbps: None }
}

/// `JobParams` 是三个操作共用的分派点，**五个能力缺一不可** ——
/// 少一个，后续的转码/裁剪就得在调用方加 special case。
///
/// `validate` 尤其重要：它一度被我漏掉（定义了却没接线），于是测试全绿而
/// "起始时间超出视频长度"根本挡不住 —— 那是最坏的一种"完成"。
#[test]
fn job_params_exposes_the_five_capabilities_for_compress() {
    let jp = JobParams::Compress(p(CompressPreset::Balanced, Encoder::Software));
    assert_eq!(jp.label(), "压缩 · 均衡");
    assert_eq!(jp.suffix(), "-压缩");
    assert_eq!(jp.extension(&src_mp4()), "mp4");
    // 压缩目前没有额外的语义校验
    assert!(jp.validate(&src_mp4()).is_ok());
    // args 与直接调 compress_args 必须一致 —— 分派层不能顺手改行为
    let want = compress_args(Path::new("/in/a.mp4"), Path::new("/out/a-压缩.mp4.part"), &p_match());
    assert_eq!(
        jp.args(Path::new("/in/a.mp4"), Path::new("/out/a-压缩.mp4.part"), &src_mp4()),
        want
    );
}
