use std::path::Path;

use crate::core::videoops::compress::{preset_defaults, CompressPreset};
use crate::core::videoops::media_info::MediaInfo;
use crate::core::videoops::params::part_path_for;

/// 画面裁剪：改画面尺寸、**不动时间轴**。
///
/// 这与「裁剪视频」（截一段时间）是两件事 —— 那边的产物时长会变，这边必须一模一样。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropParams {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    pub preset: CompressPreset,
}

/// 合法性**必须我们自己判**，不能交给 ffmpeg：
///
/// - 越界：实测源 1920 宽、请求 x=1200 再裁 1280（2480 > 1920）时，ffmpeg
///   把 x **静默钳到 640** 照常输出，stderr 一条警告都没有 —— 用户要的是画面
///   右侧那一块，拿到的是完全另一个位置的一块。
/// - 奇数：请求 641 得到 640，同样没有任何提示（h264/yuv420p 要求偶数）。
pub fn validate_crop(p: &CropParams, src: &MediaInfo) -> anyhow::Result<()> {
    if p.width < 2 || p.height < 2 {
        anyhow::bail!("宽高至少要 2 像素（当前 {}x{}）", p.width, p.height);
    }
    if p.width % 2 != 0 || p.height % 2 != 0 {
        anyhow::bail!(
            "宽高必须是偶数（当前 {}x{}）—— 奇数会被编码器静默改成相邻的偶数",
            p.width,
            p.height
        );
    }
    let (Some(sw), Some(sh)) = (src.width, src.height) else {
        // 读不出尺寸就没法判越界，而那正是最容易出错的地方 —— 宁可拒绝
        anyhow::bail!("读不出这个视频的画面尺寸，无法裁剪");
    };
    if p.x + p.width > sw {
        anyhow::bail!("左 {} + 宽 {} 超出了画面的宽度 {}", p.x, p.width, sw);
    }
    if p.y + p.height > sh {
        anyhow::bail!("上 {} + 高 {} 超出了画面的高度 {}", p.y, p.height, sh);
    }
    Ok(())
}

pub fn crop_args(input: &Path, output: &Path, p: &CropParams) -> Vec<String> {
    let (crf, _qv, kbps) = preset_defaults(p.preset);
    vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-y".into(),
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-vf".into(),
        format!("crop={}:{}:{}:{}", p.width, p.height, p.x, p.y),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        crf.to_string(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        format!("{kbps}k"),
        // 容器显式指定；重编码的操作统一出 mp4（与压缩页、裁剪的精确档一致）
        "-f".into(),
        "mp4".into(),
        part_path_for(output).to_string_lossy().into_owned(),
    ]
}
