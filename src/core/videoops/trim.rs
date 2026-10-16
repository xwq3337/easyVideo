use std::path::Path;
use std::str::FromStr;

use crate::core::videoops::media_info::{container_muxer, MediaInfo};
use crate::core::videoops::params::part_path_for;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimMode {
    /// `-c copy`：秒完、画质一点不丢。容器必须沿用源。
    Fast,
    /// 重新编码：慢、多一次质量损失，但换容器不受限制。
    Accurate,
}

impl FromStr for TrimMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fast" => Ok(Self::Fast),
            "accurate" => Ok(Self::Accurate),
            other => Err(anyhow::anyhow!("未知的裁剪档位: {other}")),
        }
    }
}

pub fn mode_label(m: TrimMode) -> &'static str {
    match m {
        TrimMode::Fast => "快速",
        TrimMode::Accurate => "精确",
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrimParams {
    pub start: f64,
    /// `None` = 到结尾（"从 1:30 到结束"是常见需求）。
    pub end: Option<f64>,
    pub mode: TrimMode,
}

/// 解析用户填的时间。接受 `mm:ss`、`hh:mm:ss`、纯数字三种写法。
///
/// **纯数字一律按秒解释**（`90` 是 90 秒，不是 90 分）。接受前后空白 ——
/// 用户常从别处复制粘贴。
///
/// 非法输入一律 `None`：调用方据此报错，而不是拿一个 0 去裁出个空文件。
///
/// 前端 `ui/src/lib/timecode.ts` 是同一套规则的镜像（前端要在输入时就把开始按钮
/// 禁掉，不能让用户提交一个明显越界的值再等 core 报错）。**改规则要两边一起改。**
pub fn parse_time(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        [secs] => {
            let v: f64 = secs.trim().parse().ok()?;
            (v >= 0.0).then_some(v)
        }
        [m, s] => {
            let m: u64 = m.trim().parse().ok()?;
            Some(m as f64 * 60.0 + parse_secs(s)?)
        }
        [h, m, s] => {
            let h: u64 = h.trim().parse().ok()?;
            let m: u64 = m.trim().parse().ok()?;
            Some(h as f64 * 3600.0 + m as f64 * 60.0 + parse_secs(s)?)
        }
        _ => None,
    }
}

/// 秒那一格：必须落在 0–60。`9:99` 要拒掉 —— 它不是"99 秒"，是打错了。
fn parse_secs(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    (0.0..60.0).contains(&v).then_some(v)
}

/// 入队前的校验，由 `JobParams::validate` 调用。
///
/// **时长读不出来时不拦 start** —— 读不出时长不等于这段区间不合理。
/// `end` 允许略超源时长：容器的 Duration 常常略短于实际可播长度，卡死在这上面
/// 会让用户裁不掉最后一点。但 `start >= end` 一律拒绝。
pub fn validate_trim(p: &TrimParams, duration_secs: Option<f64>) -> anyhow::Result<()> {
    if p.start < 0.0 {
        anyhow::bail!("起始时间不能是负数");
    }
    if let Some(end) = p.end {
        if end <= p.start {
            anyhow::bail!("结束时间必须晚于起始时间");
        }
    }
    if let Some(d) = duration_secs {
        if p.start >= d {
            anyhow::bail!("起始时间超出了视频长度（这个视频只有 {}）", fmt_human(d));
        }
    }
    Ok(())
}

fn fmt_human(s: f64) -> String {
    let total = s.max(0.0).round() as u64;
    let (h, m, sec) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// 产物的 muxer 名。**快速档必须沿用源容器**（copy 到别的容器常常直接失败），
/// 认不出来时退到 matroska —— 它什么都装得下、copy 一定成功；而这个退化会体现在
/// 产物名里（预览会显示 .mkv），不是悄悄改扩展名。
pub fn trim_muxer(p: &TrimParams, src: &MediaInfo) -> &'static str {
    match p.mode {
        TrimMode::Accurate => "mp4",
        TrimMode::Fast => src
            .container
            .as_deref()
            .and_then(container_muxer)
            .map(|(m, _)| m)
            .unwrap_or("matroska"),
    }
}

pub fn trim_extension(p: &TrimParams, src: &MediaInfo) -> String {
    match p.mode {
        TrimMode::Accurate => "mp4".to_string(),
        TrimMode::Fast => src
            .container
            .as_deref()
            .and_then(container_muxer)
            .map(|(_, e)| e.to_string())
            .unwrap_or_else(|| "mkv".to_string()),
    }
}

/// 精确档的质量。沿用压缩页「均衡」那一档（= `CompressPreset::Balanced` 的 CRF）。
const ACCURATE_CRF: u8 = 24;

pub fn trim_args(input: &Path, output: &Path, p: &TrimParams, src: &MediaInfo) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-y".into(),
    ];

    // **`-ss`/`-to` 放在 `-i` 之前**（输入选项）：mp4 的索引能给出精确寻址，
    // spec §2 那条实测就是按这个写法做的。
    a.push("-ss".into());
    a.push(fmt_secs(p.start));
    if let Some(end) = p.end {
        a.push("-to".into());
        a.push(fmt_secs(end));
    }
    a.push("-i".into());
    a.push(input.to_string_lossy().into_owned());

    match p.mode {
        TrimMode::Fast => {
            a.push("-c".into());
            a.push("copy".into());
        }
        TrimMode::Accurate => {
            // 逐个 push 而不是塞进一个数组：`ACCURATE_CRF.to_string()` 是临时值，
            // 放进数组会在借用检查上炸掉。
            a.push("-c:v".into());
            a.push("libx264".into());
            a.push("-preset".into());
            a.push("veryfast".into());
            a.push("-crf".into());
            a.push(ACCURATE_CRF.to_string());
            a.push("-c:a".into());
            a.push("aac".into());
        }
    }

    a.push("-f".into());
    a.push(trim_muxer(p, src).into());
    a.push(part_path_for(output).to_string_lossy().into_owned());
    a
}

/// 秒 → ffmpeg 能吃的字符串。**固定 6 位小数**：不用本地化格式，免得某些地区设置
/// 把小数点写成逗号（那会让 ffmpeg 直接报错）。
fn fmt_secs(s: f64) -> String {
    format!("{:.6}", s.max(0.0))
}
