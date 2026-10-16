use std::path::{Path, PathBuf};

use crate::core::videoops::compress::{compress_args, preset_label, CompressParams};
use crate::core::videoops::media_info::MediaInfo;
use crate::core::videoops::transcode::{
    target_extension, target_label, transcode_args, TranscodeParams,
};
use crate::core::videoops::trim::{mode_label, trim_args, trim_extension, validate_trim, TrimParams};

/// 临时产物名。**唯一的产出方式** —— 每个操作的 `args` 把它交给 ffmpeg，
/// `ffmpeg_job::run_job` 用它做失败清理与成功改名。三处各写一遍 `format!`
/// 迟早会分叉，而分叉的后果是清理清不掉、改名改了个不存在的文件。
pub fn part_path_for(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", output.to_string_lossy()))
}

/// 一个视频操作要跑的东西。
///
/// 用**一个枚举 + match**，不引入 `Op` trait：只有三个实现，trait 只会多一层间接，
/// 而分派集中在一处更好读。五个能力一个不少。
#[derive(Clone, Debug, PartialEq)]
pub enum JobParams {
    Compress(CompressParams),
    Transcode(TranscodeParams),
    Trim(TrimParams),
}

impl JobParams {
    /// 队列那一行显示的标签，形如 "压缩 · 均衡"。
    pub fn label(&self) -> String {
        match self {
            JobParams::Compress(p) => format!("压缩 · {}", preset_label(p.preset)),
            // 转码的标签是目标格式（用户选的就是它），不带档位 —— 带上了会太长
            JobParams::Transcode(p) => format!("转码 · {}", target_label(p.target)),
            JobParams::Trim(p) => format!("裁剪 · {}", mode_label(p.mode)),
        }
    }

    /// 产物名的后缀，形如 "-压缩"。
    pub fn suffix(&self) -> &'static str {
        match self {
            JobParams::Compress(_) => "-压缩",
            JobParams::Transcode(_) => "-转码",
            JobParams::Trim(_) => "-裁剪",
        }
    }

    /// 产物的扩展名。**不是常量** —— 转码要按目标格式走，裁剪的快速档要沿用源容器。
    pub fn extension(&self, src: &MediaInfo) -> String {
        match self {
            JobParams::Compress(_) => "mp4".to_string(),
            // 转码的扩展名由目标格式决定，与源无关
            JobParams::Transcode(p) => target_extension(p.target).to_string(),
            JobParams::Trim(p) => trim_extension(p, src),
        }
    }

    /// 拼出完整的 ffmpeg 参数。
    ///
    /// **要收 `src`**：裁剪的快速档得用源容器决定 `-f`（copy 到别的容器会直接失败）。
    /// 压缩与转码用不上它，但签名统一 —— 否则调用方得先判断是哪个操作才知道传什么。
    pub fn args(&self, input: &Path, output: &Path, src: &MediaInfo) -> Vec<String> {
        match self {
            JobParams::Compress(p) => compress_args(input, output, p),
            JobParams::Transcode(p) => transcode_args(input, output, p),
            JobParams::Trim(p) => trim_args(input, output, p, src),
        }
    }

    /// 操作自己的语义校验，由 `add_job` 在校验链的最后统一调用。
    ///
    /// **第五个能力**，不是可选的装饰：裁剪的"起始超出时长""结束早于起始"必须在这里
    /// 被挡住，否则会排进队列、跑出一个 0 字节产物再报错 —— 用户已经等了半天。
    /// 压缩与转码目前没有额外校验，返回 `Ok(())`。
    pub fn validate(&self, src: &MediaInfo) -> anyhow::Result<()> {
        match self {
            JobParams::Compress(_) => Ok(()),
            // 目标格式是整组打包的，不存在"参数矛盾"这回事
            JobParams::Transcode(_) => Ok(()),
            // 时间范围必须在这里挡住：晚一步就会排进队列、跑出一个空产物再报错
            JobParams::Trim(p) => validate_trim(p, src.duration_secs),
        }
    }
}
