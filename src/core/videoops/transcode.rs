use std::path::Path;
use std::str::FromStr;

use crate::core::videoops::compress::CompressPreset;
use crate::core::videoops::params::part_path_for;

/// 目标格式是**整组打包**的：容器 + 视频编码 + 音频编码三样绑死。
///
/// 不做成三根自由的轴 —— 那等于允许用户拼出跑不通的组合（webm + aac 是
/// `Could not write header` 的硬失败，实测），而且要到任务失败才告诉他。
/// 这与 `-crf`/`-q:v` 是同一类错误：给出互相矛盾的旋钮。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetFormat {
    Mp4H264,
    Mp4H265,
    WebmVp9,
    MkvH264,
}

impl FromStr for TargetFormat {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mp4h264" => Ok(Self::Mp4H264),
            "mp4h265" => Ok(Self::Mp4H265),
            "webmvp9" => Ok(Self::WebmVp9),
            "mkvh264" => Ok(Self::MkvH264),
            other => Err(anyhow::anyhow!("未知的目标格式: {other}")),
        }
    }
}

pub fn target_label(t: TargetFormat) -> &'static str {
    match t {
        TargetFormat::Mp4H264 => "MP4 · H.264 + AAC",
        TargetFormat::Mp4H265 => "MP4 · H.265 + AAC",
        TargetFormat::WebmVp9 => "WebM · VP9 + Opus",
        TargetFormat::MkvH264 => "MKV · H.264 + AAC",
    }
}

/// 产物的扩展名。**与源无关** —— 目标格式自己决定容器。
pub fn target_extension(t: TargetFormat) -> &'static str {
    match t {
        TargetFormat::Mp4H264 | TargetFormat::Mp4H265 => "mp4",
        TargetFormat::WebmVp9 => "webm",
        TargetFormat::MkvH264 => "mkv",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscodeParams {
    pub target: TargetFormat,
    pub preset: CompressPreset,
}

/// 每个编码器一套 CRF。
///
/// **不能跨编码器照抄刻度**：x265 与 VP9 的 CRF 语义与 x264 不同。
///
/// ## 校准依据
///
/// 素材：10 秒 1280x720@30 的 `testsrc2` 加 440Hz 正弦（`testsrc2` 细节多，
/// 纯色块会让 CRF 完全失真）。判据两条：
///
/// 1. 同档位的产物体积落在 x264 同档产物的 **0.3–1.5 倍**之间；
/// 2. **任意一档都不得大于源文件** —— 后者才是真正会伤到用户的那条：
///    他要的是转码，拿到一个更大的文件就是白干（上一轮 VT 的 q:v 就是这么错的）。
///
/// 实测（源 5530343 字节）：
///
/// | 档位 | x264 | 判据区间 | x265 | 比值 | VP9 | 比值 |
/// |---|---|---|---|---|---|---|
/// | 小体积 | 1661808 (28) | 498542–2492712 | 1872397 (30) | 1.13 | 2332032 (40) | 1.40 |
/// | 均衡 | 2796040 (24) | 838812–4194060 | 2390745 (28) | 0.85 | 3569059 (34) | 1.28 |
/// | 高画质 | 4276543 (20) | 1282963–6414815 | 3871623 (24) | 0.91 | 4453485 (30) | 1.04 |
///
/// **九个数全部落在判据内、且全部小于源文件，所以这一版没有调整。**
/// x265 的体积普遍略小于同档 x264 —— 那是它的编码效率优势，不是刻度标错：
/// 同等感知质量下更省空间正是换 H.265 的理由。
fn crf_for(t: TargetFormat, preset: CompressPreset) -> u8 {
    let (small, balanced, high) = match t {
        // x264：沿用压缩页那套已校准过的数值
        TargetFormat::Mp4H264 | TargetFormat::MkvH264 => (28, 24, 20),
        // x265：CRF 刻度比 x264 宽，同数值下画质更高 —— 起点值，见 Task 3 Step 5 校准
        TargetFormat::Mp4H265 => (30, 28, 24),
        // VP9：刻度又不一样，且 libvpx 的 -crf 只在 -b:v 0 下才表示恒定质量
        TargetFormat::WebmVp9 => (40, 34, 30),
    };
    match preset {
        CompressPreset::Small => small,
        CompressPreset::Balanced => balanced,
        CompressPreset::High => high,
    }
}

fn audio_kbps(preset: CompressPreset) -> u32 {
    match preset {
        CompressPreset::Small => 96,
        CompressPreset::Balanced => 128,
        CompressPreset::High => 192,
    }
}

pub fn transcode_args(input: &Path, output: &Path, p: &TranscodeParams) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-y".into(),
        "-i".into(),
        input.to_string_lossy().into_owned(),
    ];

    // 三样**绑死**：容器 + 视频编码 + 音频编码。分开当旋钮使的理由见 TargetFormat 的说明。
    // 注意 mkv 的 muxer 名是 `matroska` 而不是 `mkv`。
    let (vcodec, acodec, muxer) = match p.target {
        TargetFormat::Mp4H264 => ("libx264", "aac", "mp4"),
        TargetFormat::Mp4H265 => ("libx265", "aac", "mp4"),
        TargetFormat::MkvH264 => ("libx264", "aac", "matroska"),
        TargetFormat::WebmVp9 => ("libvpx-vp9", "libopus", "webm"),
    };

    a.push("-c:v".into());
    a.push(vcodec.into());
    a.push("-crf".into());
    a.push(crf_for(p.target, p.preset).to_string());
    if p.target == TargetFormat::WebmVp9 {
        // VP9 的恒定质量模式：crf 必须配 -b:v 0，否则退回默认码率模式，
        // 出来的体积与"档位"完全对不上。
        a.push("-b:v".into());
        a.push("0".into());
    }
    a.push("-c:a".into());
    a.push(acodec.into());
    a.push("-b:a".into());
    a.push(format!("{}k", audio_kbps(p.preset)));

    // **容器必须显式指定**：产物以 .part 收尾，扩展名不参与推断（上一轮踩过）。
    a.push("-f".into());
    a.push(muxer.into());
    a.push(part_path_for(output).to_string_lossy().into_owned());
    a
}
