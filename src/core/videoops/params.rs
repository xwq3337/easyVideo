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

/// 一次任务的全部上下文。
///
/// 五个能力从收单个 `src: &MediaInfo` 改成收它，是**合并**逼出来的：合并的 `args`
/// 要拿全部输入去拼 concat 列表、`validate` 要逐段比编码，单个 `src` 装不下。
/// 单输入的操作只是把 `src` 换成 `ctx.src()`，行为一字不改。
pub struct JobContext<'a> {
    pub inputs: &'a [PathBuf],
    /// 与 `inputs` 一一对应。合并的兼容性校验要逐段看。
    pub infos: &'a [MediaInfo],
    pub output: &'a Path,
    /// 本次任务要写的临时文件（合并的 concat 列表）。由 `run_job` 负责清理。
    pub scratch: Option<&'a Path>,
}

impl JobContext<'_> {
    /// 首段的媒体信息。单输入的操作看的就是它；多输入的操作（合并）也按首段
    /// 定名字与容器。
    pub fn src(&self) -> &MediaInfo {
        &self.infos[0]
    }
}

/// 一个视频操作要跑的东西。
///
/// 用**一个枚举 + match**，不引入 `Op` trait：只有几个实现，trait 只会多一层间接，
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

    /// 产物的扩展名。**不是常量** —— 转码要按目标格式走，裁剪的快速档要沿用源容器，
    /// 合并沿用首段容器。
    ///
    /// **收 `&MediaInfo` 而不是 `&JobContext`**，尽管别的能力都收了 ctx：产物名要
    /// **先于** ctx 算出来（ctx 里那个 `output` 字段就是它），而产物名又要靠 extension
    /// —— 收 ctx 就绕成一个圈了。它只看首段，收单段的信息就够。
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
    /// **要收 `ctx`**：裁剪的快速档得用源容器决定 `-f`（copy 到别的容器会直接失败），
    /// 而合并要把**全部**输入写进 concat 列表、还要临时文件的路径。压缩与转码用不上
    /// 这些，但签名统一 —— 否则调用方得先判断是哪个操作才知道传什么。
    pub fn args(&self, ctx: &JobContext) -> Vec<String> {
        match self {
            JobParams::Compress(p) => compress_args(&ctx.inputs[0], ctx.output, p),
            JobParams::Transcode(p) => transcode_args(&ctx.inputs[0], ctx.output, p),
            JobParams::Trim(p) => trim_args(&ctx.inputs[0], ctx.output, p, ctx.src()),
        }
    }

    /// 操作自己的语义校验，由 `add_job` 在校验链的最后统一调用。
    ///
    /// **第五个能力**，不是可选的装饰：裁剪的"起始超出时长""结束早于起始"必须在这里
    /// 被挡住，否则会排进队列、跑出一个 0 字节产物再报错 —— 用户已经等了半天。
    /// 压缩与转码目前没有额外校验，返回 `Ok(())`。
    pub fn validate(&self, ctx: &JobContext) -> anyhow::Result<()> {
        match self {
            JobParams::Compress(_) => Ok(()),
            // 目标格式是整组打包的，不存在"参数矛盾"这回事
            JobParams::Transcode(_) => Ok(()),
            // 时间范围必须在这里挡住：晚一步就会排进队列、跑出一个空产物再报错
            JobParams::Trim(p) => validate_trim(p, ctx.src().duration_secs),
        }
    }

    /// 进度条的分母。
    ///
    /// **合并是各段之和** —— concat 产物的时长就是它们相加。用首段的时长会让
    /// 进度条在第一段结束时就走到 100%，后面几段看起来像卡住了。
    /// （那个分支在下一轮加合并时补上；这里用穷举而不是 `_`，
    /// 好让新增变体在编译期就提醒。）
    pub fn duration_secs(&self, ctx: &JobContext) -> Option<f64> {
        match self {
            JobParams::Compress(_) | JobParams::Transcode(_) | JobParams::Trim(_) => {
                ctx.src().duration_secs
            }
        }
    }
}
