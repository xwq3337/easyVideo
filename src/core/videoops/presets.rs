use std::path::{Path, PathBuf};
use std::str::FromStr;

/// 临时产物名。**唯一的产出方式** —— `compress_args` 把它交给 ffmpeg，
/// `run_job` 用它做失败清理与成功改名。三处各写一遍 `format!` 迟早会分叉，
/// 而分叉的后果是清理清不掉、改名改了个不存在的文件。
pub fn part_path_for(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", output.to_string_lossy()))
}

/// 三档质量预设。数值取自 spec 第 6.1 节。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressPreset {
    Small,
    Balanced,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// **落到 `Software`。** 见 `resolved_encoder` 的说明 ——
    /// 默认走硬件等于默认降质。
    Auto,
    Hardware,
    Software,
}

impl FromStr for CompressPreset {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "small" => Ok(Self::Small),
            "balanced" => Ok(Self::Balanced),
            "high" => Ok(Self::High),
            other => Err(anyhow::anyhow!("未知的压缩档位: {other}")),
        }
    }
}

impl FromStr for Encoder {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "hardware" => Ok(Self::Hardware),
            "software" => Ok(Self::Software),
            other => Err(anyhow::anyhow!("未知的编码器: {other}")),
        }
    }
}

/// `(crf, qv, audio_kbps)`。
///
/// **两套质量刻度方向相反**：CRF 越小越清晰，`-q:v` 越大越清晰。写反了不会报错，
/// 只会让"小体积"档产出比"高画质"还大的文件。
///
/// ## `qv` 三档的校准依据
///
/// 素材：10 秒 1280x720@30 的 `testsrc2` 加 440Hz 正弦（`testsrc2` 细节多，
/// 纯色块会让 CRF 完全失真）。判据：**同档 VT 产物与 x264 产物的体积比落在 0.8–1.5**，
/// 且**任意一档都不得大于源文件** —— 后者是真正会伤到用户的那条：他要的是"压缩"，
/// 拿到一个更大的文件就是白干。
///
/// 实测体积（字节）：
///
/// | 档位 | x264 | VT | 比值 |
/// |---|---|---|---|
/// | 小体积 | 1855796 (crf 28) | 1956483 (q:v 38) | 1.05 |
/// | 均衡 | 3130828 (crf 24) | 3244367 (q:v 52) | 1.04 |
/// | 高画质 | 4606214 (crf 20) | 4560283 (q:v 60) | 0.99 |
///
/// 源文件是 6104040 字节，所以三档的 VT 产物都确实变小了。
///
/// **早先内插出来的 50/65/80 是错的**：那三个值给出 1.56/1.70/1.96 倍，
/// 其中高画质一档产出 9015621 字节 —— **比源文件还大 48%**。硬件编码器在同等
/// 感知质量下本就比 x264 占地方，所以同一档的 q:v 必须比"按刻度均匀内插"更低，
/// 不能想当然。
pub fn preset_defaults(preset: CompressPreset) -> (u8, u8, u32) {
    match preset {
        CompressPreset::Small => (28, 38, 96),
        CompressPreset::Balanced => (24, 52, 128),
        CompressPreset::High => (20, 60, 192),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressParams {
    pub preset: CompressPreset,
    pub encoder: Encoder,
    /// `None` = 保持原样。给了值表示"最长边不超过它"。
    pub max_height: Option<u32>,
    /// `None` = 用预设自带的值。
    pub crf: Option<u8>,
    pub qv: Option<u8>,
    pub audio_kbps: Option<u32>,
}

/// 「自动」落到软件 x264：体积画质比最好且可预测。
/// 默认走硬件等于默认降质 —— 硬件是给"要快"的人显式开的选项。
fn resolved_encoder(e: Encoder) -> Encoder {
    match e {
        Encoder::Auto => Encoder::Software,
        other => other,
    }
}

/// 拼出完整的 ffmpeg 参数（不含可执行文件本身）。
///
/// **产物一律先写 `<output>.part`。** 直接写目标名的话，中途被杀会留下一个半截文件，
/// 而"文件存在"在很多地方就等于"已完成"。成功后由调用方 rename —— 同一文件系统内
/// 原子，所以"改过名"等价于"完整"。
pub fn compress_args(input: &Path, output: &Path, p: &CompressParams) -> Vec<String> {
    let (crf_default, qv_default, kbps_default) = preset_defaults(p.preset);
    let encoder = resolved_encoder(p.encoder);

    let mut a: Vec<String> = vec![
        "-nostdin".into(), // 不阻断的话 ffmpeg 会去读 stdin，可能一直等在那里
        "-hide_banner".into(),
        "-nostats".into(), // 正常的进度噪声不要混进 stderr，那里只留错误
        "-progress".into(),
        "pipe:1".into(),
        "-y".into(),
        "-i".into(),
        input.to_string_lossy().into_owned(),
    ];

    // 质量旋钮必须**分成两条路径**，不能"换个编码器、质量参数照旧"：
    // VideoToolbox 对 -crf 是静默忽略的（实测 crf 18 与 crf 40 逐字节相同）。
    match encoder {
        Encoder::Hardware => {
            a.push("-c:v".into());
            a.push("h264_videotoolbox".into());
            a.push("-q:v".into());
            a.push(p.qv.unwrap_or(qv_default).to_string());
        }
        _ => {
            a.push("-c:v".into());
            a.push("libx264".into());
            a.push("-preset".into());
            a.push("veryfast".into());
            a.push("-crf".into());
            a.push(p.crf.unwrap_or(crf_default).to_string());
        }
    }

    // -2 让宽度按比例算出并取偶数。写成固定宽度会拉伸画面；
    // 写成奇数宽度则某些编码器直接拒绝。
    if let Some(h) = p.max_height {
        a.push("-vf".into());
        a.push(format!("scale=-2:{h}"));
    }

    a.push("-c:a".into());
    a.push("aac".into());
    a.push("-b:a".into());
    a.push(format!("{}k", p.audio_kbps.unwrap_or(kbps_default)));

    // **容器必须显式指定。** ffmpeg 默认按扩展名猜容器，而我们的产物以 `.part`
    // 收尾（先写临时名，成功后再 rename），`.part` 不是任何已知格式 ——
    // 实测直接报 "Error opening output file ...out.mp4.part. Invalid argument"，
    // 三个端到端测试全红。写成 `-f mp4` 之后扩展名就不参与推断了。
    // 容器固定 mp4 本来就是 spec 第 6.3 节的决定，所以这不算新增耦合。
    a.push("-f".into());
    a.push("mp4".into());

    // 最后一个是输出路径（先写临时名）
    a.push(part_path_for(output).to_string_lossy().into_owned());
    a
}
