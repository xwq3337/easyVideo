use std::path::{Path, PathBuf};

use crate::core::hls::escape_concat_path;
use crate::core::videoops::media_info::{container_muxer, MediaInfo};
use crate::core::videoops::params::part_path_for;
use crate::core::videoops::JobId;

/// ffmpeg concat 解复用器的列表文件内容。
///
/// **转义必须走 `escape_concat_path`** —— 上一轮 HLS 就栽在同一个地方：
/// 路径里带撇号（`~/My's Videos`）时 ffmpeg 把 `'` 当成引号结束，
/// 每一次 HLS 下载都会在最后合并那一步失败。同一个坑不踩第二次。
pub fn concat_list(inputs: &[PathBuf]) -> String {
    let mut out = String::new();
    for p in inputs {
        out.push_str(&format!(
            "file '{}'\n",
            escape_concat_path(&p.to_string_lossy())
        ));
    }
    out
}

/// 合并的兼容性结论。`blockers` 非空时**不允许提交**。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergeCheck {
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
}

/// 逐段比编码。
///
/// 为什么必须**主动**比：实测编码不一致时 `-c copy` 拼接**报 exit 0**，
/// 而产物解码器报 3 行（h264+h265）到 **174 行**（h264+mpeg4）错 —— 基本打不开。
/// ffmpeg 不会告诉我们。
///
/// 判据的分界来自实测：编码不同 → 拦；分辨率/帧率/音频参数不同、有无音轨混合
/// → 警告（那些能解码，只是接缝处有瑕疵）。
pub fn check(infos: &[MediaInfo]) -> MergeCheck {
    let mut c = MergeCheck::default();
    let Some(first) = infos.first() else { return c };
    let name = |o: &Option<String>| o.clone().unwrap_or_else(|| "未知".to_string());

    // 音频编码的基准是**第一段有音轨的**，不是第 1 段。
    //
    // spec §5.2 的原话是"只比**有音轨的那些段之间**"。拿第 1 段当基准的话，
    // 第 1 段没音轨时（那本来就是合法的、只该出警告的情况）这条规则**整条失效**：
    // [无音轨, aac, mp3] 会一条都不拦，`-c copy` 拼出一个混合音轨的文件 ——
    // 而那正是这一节特意要拦住的"编码不一致"。
    let audio_ref = infos
        .iter()
        .enumerate()
        .find(|(_, m)| m.audio_codec.is_some())
        .map(|(i, m)| (i + 1, m.audio_codec.clone().unwrap_or_default()));

    for (i, m) in infos.iter().enumerate().skip(1) {
        let n = i + 1;
        if m.video_codec != first.video_codec {
            c.blockers.push(format!(
                "第 {n} 段的视频编码是 {}，与第 1 段（{}）不一致",
                name(&m.video_codec),
                name(&first.video_codec)
            ));
        }
        // **只比有音轨的那些段之间**：一段压根没有音轨不算"编码不一致"
        // （实测那种情况能拼，只是音轨是断的 → 那是警告）
        if let (Some(a), Some((r, r_codec))) = (&m.audio_codec, &audio_ref) {
            if a != r_codec {
                c.blockers.push(format!("第 {n} 段的音频编码是 {a}，与第 {r} 段（{r_codec}）不一致"));
            }
        }
        if m.audio_codec.is_some() != first.audio_codec.is_some() {
            c.warnings
                .push(format!("第 {n} 段与第 1 段的音轨有无不一致，拼出来的音轨是断的"));
        }
        if (m.width, m.height) != (first.width, first.height) {
            c.warnings.push(format!(
                "第 {n} 段的画面尺寸（{}x{}）与第 1 段（{}x{}）不同，接缝处画面会变",
                m.width.unwrap_or(0),
                m.height.unwrap_or(0),
                first.width.unwrap_or(0),
                first.height.unwrap_or(0)
            ));
        }
        // 帧率不同：实测 30 + 25 拼出来接缝处 2 行解码错误（画面一跳）。
        // 读不出来时不比 —— 不拿"未知"去跟一个已知值比。
        if let (Some(a), Some(b)) = (m.fps, first.fps) {
            if (a - b).abs() > 0.01 {
                c.warnings.push(format!("第 {n} 段的帧率（{a}）与第 1 段（{b}）不同，接缝处会一跳"));
            }
        }
        // 音频参数不同：实测拼出来 0 解码错误，但总时长偏 +0.36 秒。
        // 只在**两段都有音轨**时比 —— 缺音轨是上面那条警告的事，别重复说一遍。
        if let (Some(a), Some(b)) = (&m.audio_params, &first.audio_params) {
            if a != b {
                c.warnings.push(format!(
                    "第 {n} 段的音频参数（{a}，即 采样率:声道）与第 1 段（{b}）不同，总时长可能略有出入"
                ));
            }
        }
    }
    c
}

/// 容器沿用**首段**；认不出来退 matroska（什么都装得下）—— 与裁剪的快速档
/// 同一套规则、同一个 `container_muxer`。
pub fn merge_muxer(src: &MediaInfo) -> &'static str {
    src.container
        .as_deref()
        .and_then(container_muxer)
        .map(|(m, _)| m)
        .unwrap_or("matroska")
}

pub fn merge_extension(src: &MediaInfo) -> String {
    src.container
        .as_deref()
        .and_then(container_muxer)
        .map(|(_, e)| e.to_string())
        .unwrap_or_else(|| "mkv".to_string())
}

/// 合并的临时列表文件放在**应用数据目录**里 —— 我们自己的地盘，
/// 不往用户的输出目录里丢东西。
pub fn scratch_path(data_dir: &Path, id: JobId) -> PathBuf {
    data_dir.join(format!("merge-{id}.txt"))
}

pub fn merge_args(list: &Path, output: &Path, src: &MediaInfo) -> Vec<String> {
    vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-y".into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list.to_string_lossy().into_owned(),
        // 无损拼接：不重编码，所以"合并"是秒完的
        "-c".into(),
        "copy".into(),
        // 容器显式指定，别指望扩展名推断
        "-f".into(),
        merge_muxer(src).into(),
        part_path_for(output).to_string_lossy().into_owned(),
    ]
}
