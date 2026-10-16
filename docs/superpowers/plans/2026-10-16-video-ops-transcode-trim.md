# 视频操作（其二）· 转码与裁剪 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `add_compress_job` 泛化成按操作分派的 `add_job`，并在此之上实现「转码」（四种目标格式）与「裁剪视频」（快速 / 精确两档）。

**Architecture:** core 侧新增 `params.rs`（`JobParams` 枚举 + 四个能力）与 `transcode.rs`、`trim.rs`；`presets.rs` 改名 `compress.rs`。命令面收成一条 `add_video_job`（带 tag 的 `JobParamsDto`）外加一条 `plan_output`（产物名预览，与入队共用同一个取名函数）。前端把 `VideoOpPage` 拆成外框 + 三个表单。

**Tech Stack:** Rust 2021 / tokio / 内嵌 ffmpeg 9.0.2 / Tauri 2.12.4 / React 19 + TypeScript / vitest

**Spec:** `docs/superpowers/specs/2026-10-16-video-ops-transcode-trim-design.md`

## Global Constraints

- 语言：界面文案、注释、commit message 全部用中文（仓库既有约定）。
- 界面**不做假按钮**：本轮只把「转码」「裁剪视频」变成 ready，合并与画面裁剪继续灰显 + 悬停「还没做」+ 点了不导航。
- `core` 是唯一真相源：产物名由 core 算，前端不自己拼。
- **不新增第三方依赖。**
- 产物**绝不覆盖源文件**；失败或取消**绝不留下半截文件**。
- 产物的 `-f`（muxer）**必须显式指定**，别指望扩展名推断 —— 上一轮已经踩过（`.part` 后缀让推断失败）。
- 容器与编码器**不是自由组合**：界面给的是整组打包的目标格式，不是三根自由轴。
- 提交结尾带 `Co-Authored-By: Claude Code <noreply@anthropic.com>`。

## Review Focus

以下是 spec 默认满足、但没有单个任务的测试能覆盖到的输入与失败模式。每条都在指名的任务里配了测试。

1. **源容器认不出来**（非常见的封装，或 `Input #0` 那行缺失）→ 用户期望：转码照常（目标格式自己决定容器）；快速裁剪不能瞎猜容器，要退到一个一定装得下的容器并**在产物名预览里显示出来**，而不是失败或悄悄改扩展名。→ Task 4
2. **裁剪的 `start` 落在源时长之外**（用户看错了时间轴）→ 用户期望：入队时就报错并说清源有多长，而不是排进队列跑出一个空产物。→ Task 4
3. **目标格式与源格式相同**（H.264/mp4 → MP4·H.264）→ 用户期望：不阻拦，但能从"源：H.264 · mp4"那行看出来这是在白重编一遍。→ Task 2（解析）+ Task 7（显示）
4. **转码到 WebM 时音频编码被写成 aac** → 用户期望：不出现这种情况。这是硬失败（`Could not write header`），必须在参数构造层就杜绝。→ Task 3
5. **裁剪的 `start` 与 `end` 之间不足一帧**（用户填了 12 → 12.01）→ 用户期望：要么报错要么给出一帧，不能产出 0 字节文件。→ Task 4

---

### Task 1: 引擎泛化 —— `JobParams` 分派 + `presets.rs` 改名

本轮所有后续任务的地基。**行为不变**：改完之后现有测试必须全绿（含上一轮的 engine / job / media_info 测试）。

**Files:**
- Rename: `src/core/videoops/presets.rs` → `src/core/videoops/compress.rs`
- Rename: `tests/videoops_presets_test.rs` → `tests/videoops_compress_test.rs`
- Create: `src/core/videoops/params.rs`
- Modify: `src/core/videoops/mod.rs`（`pub mod` 列表、`add_compress_job` → `add_job`、取名抽成共享 helper）
- Modify: `src/core/videoops/ffmpeg_job.rs`（`part_path_for` 的 import 路径）
- Modify: `tests/videoops_engine_test.rs`（`add_compress_job` 调用点 → `add_job`）

**Interfaces:**
- Consumes: `compress::{compress_args, preset_label, CompressParams}`、`media_info::MediaInfo`
- Produces:
  - `pub enum JobParams { Compress(CompressParams) }`
  - `JobParams::{label, args, suffix, extension, validate}`（签名见下）
  - `VideoOps::add_job(input: PathBuf, output_dir: PathBuf, params: JobParams) -> anyhow::Result<JobId>`
  - `VideoOps::plan_output_path(jobs, output_dir, stem, suffix, extension) -> PathBuf`（私有，`add_job` 与后续的 `plan_output` 共用）

- [ ] **Step 1: 先做纯改名，确认全量测试仍然绿**

```bash
git mv src/core/videoops/presets.rs src/core/videoops/compress.rs
git mv tests/videoops_presets_test.rs tests/videoops_compress_test.rs
python3 - <<'PY'
import pathlib, re
for f in ["src/core/videoops/mod.rs", "src/core/videoops/compress.rs",
          "src/core/videoops/ffmpeg_job.rs", "tests/videoops_compress_test.rs",
          "tests/videoops_job_test.rs"]:
    p = pathlib.Path(f)
    s = p.read_text()
    s = s.replace("videoops::presets", "videoops::compress")
    s = s.replace("pub mod presets;", "pub mod compress;")
    p.write_text(s)
PY
rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"passed,",f,"failed"}'
```

Expected: `root: 126 passed, 0 failed`（与改名前同一个数字；改名不该动任何行为）

- [ ] **Step 2: 写失败的测试**

在 `tests/videoops_compress_test.rs` 末尾追加：

```rust
use easydownvideo::core::videoops::media_info::MediaInfo;
use easydownvideo::core::videoops::params::JobParams;

fn src_mp4() -> MediaInfo {
    MediaInfo { duration_secs: Some(5.0), has_video: true, size_bytes: 1000 }
}

/// `JobParams` 是三个操作共用的分派点，四个能力缺一不可 —— 少一个，
/// 后续的转码/裁剪就得在调用方加 special case。
#[test]
fn job_params_exposes_the_four_capabilities_for_compress() {
    let p = JobParams::Compress(p(CompressPreset::Balanced, Encoder::Software));
    assert_eq!(p.label(), "压缩 · 均衡");
    assert_eq!(p.suffix(), "-压缩");
    assert_eq!(p.extension(&src_mp4()), "mp4");
    // args 与直接调 compress_args 必须一致 —— 分派层不能顺手改行为
    let want = compress_args(Path::new("/in/a.mp4"), Path::new("/out/a-压缩.mp4.part"),
                             &p_match());
    assert_eq!(p.args(Path::new("/in/a.mp4"), Path::new("/out/a-压缩.mp4.part"), &src_mp4()), want);
}
```

其中 `p_match()` 是同一组参数的一个副本（写测试时直接重复构造一次 `CompressParams`，
不要引入共享的可变状态）：

```rust
fn p_match() -> CompressParams {
    CompressParams { preset: CompressPreset::Balanced, encoder: Encoder::Software,
                     max_height: None, crf: None, qv: None, audio_kbps: None }
}
```

- [ ] **Step 3: 跑测试看它失败**

Run: `rtk proxy cargo test --test videoops_compress_test 2>&1 | grep -E "^error" | head -3`
Expected: `unresolved import ... videoops::params`

- [ ] **Step 4: 写最小实现**

`src/core/videoops/params.rs`：

```rust
use std::path::{Path, PathBuf};

use crate::core::videoops::compress::{compress_args, preset_label, CompressParams};
use crate::core::videoops::media_info::MediaInfo;

/// 临时产物名。**唯一的产出方式** —— 每个操作的 `args` 把它交给 ffmpeg，
/// `ffmpeg_job::run_job` 用它做失败清理与成功改名。三处各写一遍 `format!`
/// 迟早会分叉，而分叉的后果是清理清不掉、改名改了个不存在的文件。
pub fn part_path_for(output: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", output.to_string_lossy()))
}

/// 一个视频操作要跑的东西。
///
/// 用**一个枚举 + match**，不引入 `Op` trait：只有三个实现，trait 只会多一层间接，
/// 而分派集中在一处更好读。四个能力一个不少。
#[derive(Clone, Debug, PartialEq)]
pub enum JobParams {
    Compress(CompressParams),
}

impl JobParams {
    /// 队列那一行显示的标签，形如 "压缩 · 均衡"。
    pub fn label(&self) -> String {
        match self {
            JobParams::Compress(p) => format!("压缩 · {}", preset_label(p.preset)),
        }
    }

    /// 产物名的后缀，形如 "-压缩"。
    pub fn suffix(&self) -> &'static str {
        match self {
            JobParams::Compress(_) => "-压缩",
        }
    }

    /// 产物的扩展名。**不是常量**——转码要按目标格式走，裁剪的快速档要沿用源容器。
    pub fn extension(&self, _src: &MediaInfo) -> String {
        match self {
            JobParams::Compress(_) => "mp4".to_string(),
        }
    }

    /// 拼出完整的 ffmpeg 参数。
    ///
    /// **要收 `src`**：裁剪的快速档得用源容器决定 `-f`（copy 到别的容器会直接失败）。
    /// 压缩与转码用不上它，但签名统一 —— 否则调用方得先判断是哪个操作才知道传什么。
    pub fn args(&self, input: &Path, output: &Path, _src: &MediaInfo) -> Vec<String> {
        match self {
            JobParams::Compress(p) => compress_args(input, output, p),
        }
    }

    /// 操作自己的语义校验，由 `add_job` 在校验链的最后统一调用。
    ///
    /// **第五个能力**，不是可选的装饰：裁剪的"起始超出时长""结束早于起始"必须在这里
    /// 被挡住，否则会排进队列、跑出一个 0 字节产物再报错 —— 用户已经等了半天。
    /// 压缩与转码目前没有额外校验，返回 `Ok(())`。
    pub fn validate(&self, _src: &MediaInfo) -> anyhow::Result<()> {
        match self {
            JobParams::Compress(_) => Ok(()),
        }
    }
}
```

`src/core/videoops/compress.rs` 里把 `preset_label` 从私有改成公开（`fn preset_label` → `pub fn preset_label`），
并**删掉 `part_path_for`**（它搬去了 `params.rs`）。

`src/core/videoops/mod.rs` 的模块列表加 `pub mod params;`；`ffmpeg_job.rs` 里
`crate::core::videoops::presets::part_path_for` 改成 `crate::core::videoops::params::part_path_for`。

`mod.rs` 的 `add_compress_job` 改成 `add_job`，并把取名抽成一个共享的私有方法：

```rust
    /// 算产物的完整路径。**`plan_output` 与 `add_job` 都走这里** —— 两处各写一遍的话，
    /// 预览与实际落盘会在某个边角上分叉，而那正是产物名预览存在的意义。
    ///
    /// `taken` 问的是"有没有别的**在途任务**已经占了它"。只问文件系统是不够的：
    /// 产物要等任务真跑完才存在，于是两个都还没跑完的任务会拿到同一个"这个文件不存在"
    /// 的答案，随后后完成的那次 rename 会静默覆盖先完成的那份。
    fn plan_output_path(
        jobs: &HashMap<JobId, JobEntry>,
        output_dir: &Path,
        stem: &str,
        suffix: &str,
        extension: &str,
    ) -> PathBuf {
        unique_path_avoiding(output_dir, &format!("{stem}{suffix}.{extension}"), |c| {
            jobs.values().any(|j| j.output == c)
        })
    }
```

`add_job` 的主体（校验与登记照旧，只有这几处变）：

```rust
    pub async fn add_job(
        &self,
        input: PathBuf,
        output_dir: PathBuf,
        params: JobParams,
    ) -> anyhow::Result<JobId> {
        if !input.is_file() {
            anyhow::bail!("找不到输入文件：{}", input.display());
        }
        if !output_dir.is_dir() {
            anyhow::bail!("输出目录不存在：{}", output_dir.display());
        }
        let ffmpeg = self.ffmpeg()?;
        // 现在要**整份** MediaInfo 而不只是 has_video：产物扩展名要看源容器
        let src = media_info::inspect(&ffmpeg, &input).await?;
        if !src.has_video {
            anyhow::bail!(
                "这个文件里读不到视频流（可能是纯音频，或者文件已损坏）：{}",
                input.display()
            );
        }
        // **操作自己的语义校验**（裁剪的时间范围就靠这一步挡住）。
        // 放在这里而不是命令层：入队校验全在 core，命令层只做薄封装，这条规矩别破。
        params.validate(&src)?;
        // 真写一个探针文件，而不是查权限位 —— 权限位在 ACL、只读卷、网络盘上都不作数
        let probe = output_dir.join(".easydownvideo-write-probe");
        std::fs::write(&probe, b"")
            .map_err(|e| anyhow::anyhow!("输出目录不可写（{}）：{e}", output_dir.display()))?;
        std::fs::remove_file(&probe).ok();

        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video")
            .to_string();

        let id = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let output = Self::plan_output_path(
                &jobs,
                &output_dir,
                &stem,
                params.suffix(),
                &params.extension(&src),
            );
            let args = params.args(&input, &output, &src);
            let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
            jobs.insert(
                id,
                JobEntry {
                    label: params.label(),
                    input,
                    output,
                    args,
                    state: JobState::Queued,
                    progress: None,
                    cancel: CancellationToken::new(),
                },
            );
            id
        };
        self.inner.queue.lock().unwrap().push_back(id);
        let _ = self.inner.tx.send(VideoOpEvent::Added { id });
        self.pump();
        Ok(id)
    }
```

同时把 `mod.rs` 里那个已经不需要的 `preset_label` 自由函数删掉（它搬进了 `compress.rs`）。
`mod.rs` 顶部按需补 `use crate::core::videoops::params::JobParams;` 与 `use std::path::Path;`。

- [ ] **Step 5: 改测试调用点并跑全量**

`tests/videoops_engine_test.rs` 里所有 `ops.add_compress_job(` 改成 `ops.add_job(`，
参数里的 `params()`（`CompressParams`）用 `JobParams::Compress(params())` 包一层
（在文件顶部把 `params()` 的返回类型改成 `JobParams` 更省事：`fn params() -> JobParams { JobParams::Compress(CompressParams { … }) }`）。

Run:

```bash
rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"passed,",f,"failed"}'
```

Expected: `root: 127 passed, 0 failed`（126 + 新增的那条 `job_params_exposes_the_four_capabilities_for_compress`）

- [ ] **Step 6: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
refactor(videoops): 压缩参数分派成 JobParams,为多操作打开接口

行为零变化(presets.rs 只是改名 compress.rs、add_compress_job 只是改名 add_job),
改的是接口的形状:后续的转码与裁剪各是一个变体、一套 args,而不是往一个
写死"压缩"的函数里加分支。

四个能力(label / args / suffix / extension)是枚举上的方法而不是一个 Op trait:
只有三个实现,trait 只会多一层间接。extension 收 &MediaInfo 是因为它不再是常量
—— 转码按目标格式走、裁剪的快速档要沿用源容器;args 也收它,因为快速档得用
源容器决定 -f(copy 到别的容器会直接失败)。

取名抽成 plan_output_path,由 add_job 与后续的 plan_output 共用 —— 两处各写
一遍的话,产物名预览与实际落盘会在某个边角上分叉,而那正是预览存在的意义。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 2: `MediaInfo` 扩容器与编码

**Files:**
- Modify: `src/core/videoops/media_info.rs`
- Test: `tests/videoops_media_info_test.rs`

**Interfaces:**
- Consumes: 无
- Produces:
  - `MediaInfo { duration_secs, has_video, video_codec: Option<String>, container: Option<String>, size_bytes }`
  - `pub fn parse_video_codec(stderr: &str) -> Option<String>`
  - `pub fn parse_container(stderr: &str, path: &Path) -> Option<String>`
  - `pub fn container_muxer(c: &str) -> Option<(&'static str, &'static str)>` → `(ffmpeg 的 -f 名, 扩展名)`

- [ ] **Step 1: 写失败的测试**

在 `tests/videoops_media_info_test.rs` 末尾追加：

```rust
use easydownvideo::core::videoops::media_info::{
    container_muxer, parse_container, parse_video_codec,
};
use std::path::Path;

/// 真实输出（ffmpeg 9.0.2 实测）。
#[test]
fn parses_video_codec_from_the_stream_line() {
    let stderr = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':
  Duration: 00:00:30.00, start: 0.000000, bitrate: 55 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1), yuv420p, 640x360, 30 fps
";
    assert_eq!(parse_video_codec(stderr).as_deref(), Some("h264"));
    // hevc 的真实写法
    let hevc = "  Stream #0:0(und): Video: hevc (Main) (hev1), yuv420p, 1920x1080";
    assert_eq!(parse_video_codec(hevc).as_deref(), Some("hevc"));
    // 纯音频：没有 Video: 行
    let audio = "  Stream #0:0(und): Audio: aac (LC), 44100 Hz, stereo";
    assert_eq!(parse_video_codec(audio), None);
}

#[test]
fn parses_container_and_disambiguates_mkv_from_webm() {
    // mp4 家族：mov,mp4,m4a,3gp,3g2,mj2 —— 取 mp4
    let mp4 = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':";
    assert_eq!(parse_container(mp4, Path::new("/a/x.mp4")).as_deref(), Some("mp4"));
    // mkv 与 webm 在 ffmpeg 眼里都是 matroska,webm —— **只能靠扩展名区分**
    let mk = "Input #0, matroska,webm, from 'x.mkv':";
    assert_eq!(parse_container(mk, Path::new("/a/x.mkv")).as_deref(), Some("mkv"));
    assert_eq!(parse_container(mk, Path::new("/a/x.webm")).as_deref(), Some("webm"));
    // 认不出来时 None,不编一个
    let odd = "Input #0, mpegts, from 'x.ts':";
    assert_eq!(parse_container(odd, Path::new("/a/x.ts")), None);
    assert_eq!(parse_container("no input line at all", Path::new("/a/x.mp4")), None);
}

#[test]
fn maps_container_to_muxer_and_extension() {
    // **muxer 名与扩展名不是同一个词** —— mkv 的 muxer 叫 matroska。
    // 写成 -f mkv 会直接失败。
    assert_eq!(container_muxer("mkv"), Some(("matroska", "mkv")));
    assert_eq!(container_muxer("webm"), Some(("webm", "webm")));
    assert_eq!(container_muxer("mp4"), Some(("mp4", "mp4")));
    assert_eq!(container_muxer("nope"), None);
}
```

- [ ] **Step 2: 跑测试看它失败**

Run: `rtk proxy cargo test --test videoops_media_info_test 2>&1 | grep -E "^error" | head -3`
Expected: `unresolved import ... parse_video_codec`（或 `MediaInfo` 少字段导致的构造错误）

- [ ] **Step 3: 写最小实现**

`src/core/videoops/media_info.rs`：`MediaInfo` 加两个字段，`parse_media_info` 改成收 `path`：

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub duration_secs: Option<f64>,
    pub has_video: bool,
    /// 视频编码，形如 "h264" / "hevc" / "vp9"。取不到时 None。
    pub video_codec: Option<String>,
    /// 容器，形如 "mp4" / "mkv" / "webm" / "mov"。取不到时 None。
    pub container: Option<String>,
    pub size_bytes: u64,
}

/// 从 `Stream #0:0…: Video: h264 (High) …` 里取编码名。
pub fn parse_video_codec(stderr: &str) -> Option<String> {
    let line = stderr
        .lines()
        .find(|l| l.trim_start().starts_with("Stream #") && l.contains("Video:"))?;
    let rest = line.split("Video:").nth(1)?.trim();
    let codec = rest.split_whitespace().next()?.trim();
    if codec.is_empty() {
        None
    } else {
        Some(codec.to_string())
    }
}

/// ffmpeg 打印的 demuxer 列表 → 容器名。
///
/// **mkv 与 webm 在 ffmpeg 眼里是同一个 demuxer**（`matroska,webm`），
/// 光靠 stderr 分不开 —— 所以要看输入文件的扩展名来定夺。
pub fn parse_container(stderr: &str, path: &Path) -> Option<String> {
    let line = stderr.lines().find(|l| l.trim_start().starts_with("Input #"))?;
    // `Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':`
    let list: Vec<&str> = line
        .split("Input #")
        .nth(1)?
        .split(',')
        .skip(1)
        .take_while(|s| !s.contains("from"))
        .map(str::trim)
        .collect();
    if list.iter().any(|s| matches!(*s, "mp4" | "mov" | "m4a" | "3gp")) {
        // mov 与 mp4 共用 demuxer；用扩展名把 .mov 区分出来
        return Some(if ext_is(path, "mov") { "mov".into() } else { "mp4".into() });
    }
    if list.iter().any(|s| matches!(*s, "matroska" | "webm")) {
        return Some(if ext_is(path, "webm") { "webm".into() } else { "mkv".into() });
    }
    None
}

fn ext_is(path: &Path, want: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(want))
        .unwrap_or(false)
}

/// 容器名 → `(ffmpeg 的 -f 名, 产物扩展名)`。
///
/// **muxer 名与扩展名不是同一个词**：mkv 的 muxer 叫 `matroska`，写成 `-f mkv`
/// 会直接失败。这张表是唯一的转换点。
pub fn container_muxer(c: &str) -> Option<(&'static str, &'static str)> {
    match c {
        "mp4" => Some(("mp4", "mp4")),
        "mov" => Some(("mov", "mov")),
        "mkv" => Some(("matroska", "mkv")),
        "webm" => Some(("webm", "webm")),
        _ => None,
    }
}

pub fn parse_media_info(stderr: &str, path: &Path) -> MediaInfo {
    MediaInfo {
        duration_secs: parse_duration(stderr),
        has_video: parse_has_video(stderr),
        video_codec: parse_video_codec(stderr),
        container: parse_container(stderr, path),
        size_bytes: 0, // 由命令层填（见 Task 5）
    }
}

pub async fn inspect(ffmpeg: &Path, input: &Path) -> anyhow::Result<MediaInfo> {
    let out = tokio::process::Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .output()
        .await
        .context("run ffmpeg -i")?;
    Ok(parse_media_info(&String::from_utf8_lossy(&out.stderr), input))
}
```

**两个已有调用点会被这次签名/结构变更打破，必须一起改**：

1. `tests/videoops_media_info_test.rs` 里既有的 `parse_media_info(stderr)` 调用要补上路径参数，
   并补 `video_codec: …` / `container: …` 两个字段的期望值。
2. `tests/videoops_compress_test.rs` 里 Task 1 新增的 `src_mp4()` 辅助函数只构造了三个字段
   （因为那时 `MediaInfo` 还只有三个），要补上：
   `video_codec: Some("h264".into()), container: Some("mp4".into())`。

- [ ] **Step 4: 跑测试看它通过**

Run: `rtk proxy cargo test --test videoops_media_info_test 2>&1 | tail -4`
Expected: `test result: ok. 9 passed`（原 6 条 + 新增 3 条）

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): MediaInfo 扩出容器与视频编码

转码页必须能显示"源：H.264 · mp4" —— 否则用户选"转码 → MP4·H.264"时看不出
这是在把同一个格式重编一遍(白掉一次画质),而 spec 的验收清单第 4 条要的就是这个。

容器解析里有一处必须写清楚:**mkv 与 webm 在 ffmpeg 眼里是同一个 demuxer**
(matroska,webm),光靠 stderr 分不开,要看扩展名定夺。另外 muxer 名与扩展名
不是同一个词 —— mkv 的 muxer 叫 matroska,写成 -f mkv 会直接失败,所以有一张
唯一的转换表 container_muxer。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 3: 转码 —— 四种目标格式

**Files:**
- Create: `src/core/videoops/transcode.rs`
- Modify: `src/core/videoops/params.rs`（加 `JobParams::Transcode` 变体）
- Modify: `src/core/videoops/mod.rs`（`pub mod transcode;`）
- Test: `tests/videoops_transcode_test.rs`

**Interfaces:**
- Consumes: `compress::CompressPreset`、`media_info::MediaInfo`、`params::part_path_for`
- Produces:
  - `pub enum TargetFormat { Mp4H264, Mp4H265, WebmVp9, MkvH264 }`，`FromStr` 接受 `"mp4h264" | "mp4h265" | "webmvp9" | "mkvh264"`
  - `pub struct TranscodeParams { pub target: TargetFormat, pub preset: CompressPreset }`
  - `pub fn transcode_args(input: &Path, output: &Path, p: &TranscodeParams) -> Vec<String>`
  - `pub fn target_label(t: TargetFormat) -> &'static str`
  - `pub fn target_extension(t: TargetFormat) -> &'static str`

- [ ] **Step 1: 写失败的测试**

`tests/videoops_transcode_test.rs`：

```rust
use easydownvideo::core::videoops::compress::CompressPreset;
use easydownvideo::core::videoops::transcode::{
    target_extension, target_label, transcode_args, TargetFormat, TranscodeParams,
};
use std::path::Path;

fn run(p: &TranscodeParams) -> Vec<String> {
    transcode_args(Path::new("/in/a.mp4"), Path::new("/out/a-转码.webm"), p)
}

fn has(args: &[String], needle: &str) -> bool {
    args.iter().any(|a| a == needle)
}

fn arg_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let i = args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("没有 {flag}: {args:?}"));
    &args[i + 1]
}

/// **WebM 只收 VP8/VP9/AV1 + Vorbis/Opus。** 硬塞 aac 进去是
/// `Could not write header` 的硬失败（实测），所以这条是"非法组合"的回归测试：
/// 目标格式是整组打包的，音频编码必须跟着容器走。
#[test]
fn webm_uses_opus_and_never_aac() {
    let args = run(&TranscodeParams { target: TargetFormat::WebmVp9, preset: CompressPreset::Balanced });
    assert_eq!(arg_after(&args, "-c:a"), "libopus");
    assert!(
        !has(&args, "aac"),
        "WebM 收不了 aac，写进去是硬失败。实际: {args:?}"
    );
    assert_eq!(arg_after(&args, "-c:v"), "libvpx-vp9");
    assert_eq!(arg_after(&args, "-f"), "webm");
}

/// VP9 的恒定质量模式要 `-b:v 0` 配 `-crf`；只给 crf 会退化成默认码率模式，
/// 出来的体积与预期差很远。
#[test]
fn vp9_pairs_crf_with_zero_bitrate() {
    let args = run(&TranscodeParams { target: TargetFormat::WebmVp9, preset: CompressPreset::Balanced });
    assert!(has(&args, "-crf"));
    assert_eq!(arg_after(&args, "-b:v"), "0");
}

#[test]
fn h265_uses_libx265_and_mp4() {
    let args = run(&TranscodeParams { target: TargetFormat::Mp4H265, preset: CompressPreset::Balanced });
    assert_eq!(arg_after(&args, "-c:v"), "libx265");
    assert_eq!(arg_after(&args, "-c:a"), "aac");
    assert_eq!(arg_after(&args, "-f"), "mp4");
}

#[test]
fn mkv_target_uses_the_matroska_muxer_not_the_word_mkv() {
    let args = run(&TranscodeParams { target: TargetFormat::MkvH264, preset: CompressPreset::Balanced });
    // -f mkv 是错的，ffmpeg 认的是 matroska
    assert_eq!(arg_after(&args, "-f"), "matroska");
    assert!(!has(&args, "mkv"));
}

/// 三档质量在每个编码器下都要给出**不同**的 CRF —— 跨编码器照抄刻度会让某一档
/// 产出比源文件还大（上一轮 VT 的 q:v 就是这么错的）。
#[test]
fn every_encoder_has_its_own_three_crf_values() {
    let crf_of = |t: TargetFormat, preset: CompressPreset| -> u8 {
        let args = run(&TranscodeParams { target: t, preset });
        arg_after(&args, "-crf").parse().unwrap()
    };
    for t in [TargetFormat::Mp4H264, TargetFormat::Mp4H265, TargetFormat::WebmVp9, TargetFormat::MkvH264] {
        let small = crf_of(t, CompressPreset::Small);
        let balanced = crf_of(t, CompressPreset::Balanced);
        let high = crf_of(t, CompressPreset::High);
        assert!(small > balanced && balanced > high, "{t:?} 的 CRF 应递减: {small}/{balanced}/{high}");
        assert!(high >= 10 && small <= 45, "{t:?} 的 CRF 落在荒唐的区间: {small}..{high}");
    }
    // 同一档位下，不同编码器的数值不该全部相同（那说明用了同一张表）
    let per_encoder: Vec<u8> = [TargetFormat::Mp4H264, TargetFormat::Mp4H265, TargetFormat::WebmVp9]
        .iter().map(|t| crf_of(*t, CompressPreset::Balanced)).collect();
    assert!(per_encoder.iter().any(|c| *c != per_encoder[0]), "各编码器应有各自的刻度: {per_encoder:?}");
}

#[test]
fn label_and_extension_follow_the_target() {
    assert_eq!(target_label(TargetFormat::Mp4H264), "MP4 · H.264 + AAC");
    assert_eq!(target_label(TargetFormat::WebmVp9), "WebM · VP9 + Opus");
    assert_eq!(target_extension(TargetFormat::WebmVp9), "webm");
    assert_eq!(target_extension(TargetFormat::MkvH264), "mkv");
    assert_eq!(target_extension(TargetFormat::Mp4H264), "mp4");
}

#[test]
fn targets_parse_from_the_wire_strings() {
    assert_eq!("webmvp9".parse::<TargetFormat>().unwrap(), TargetFormat::WebmVp9);
    assert_eq!("mp4h265".parse::<TargetFormat>().unwrap(), TargetFormat::Mp4H265);
    assert!("mpeg2".parse::<TargetFormat>().is_err());
}

/// 命令行形状：不碰 stdin、静默进度、显式 -f、输出在最后。
#[test]
fn argv_shape_is_well_formed() {
    let args = run(&TranscodeParams { target: TargetFormat::Mp4H264, preset: CompressPreset::Balanced });
    assert!(has(&args, "-nostdin"));
    assert!(has(&args, "-hide_banner"));
    assert!(has(&args, "-progress"));
    assert!(has(&args, "-y"));
    assert!(args.last().unwrap().ends_with(".part"), "产物先写临时名: {}", args.last().unwrap());
    assert!(args.iter().position(|a| a == "-f").unwrap() < args.len() - 1, "-f 必须在输出之前");
}
```

- [ ] **Step 2: 跑测试看它失败**

Run: `rtk proxy cargo test --test videoops_transcode_test 2>&1 | grep -E "^error" | head -3`
Expected: `unresolved import ... videoops::transcode`

- [ ] **Step 3: 写最小实现**

`src/core/videoops/transcode.rs`：

```rust
use std::path::Path;
use std::str::FromStr;

use crate::core::videoops::compress::CompressPreset;
use crate::core::videoops::params::part_path_for;

/// 目标格式是**整组打包**的：容器 + 视频编码 + 音频编码三样绑死。
///
/// 不做成三根自由的轴 —— 那等于允许用户拼出跑不通的组合（webm + aac 是硬失败），
/// 而且要到任务失败才告诉他。这与 `-crf`/`-q:v` 是同一类错误：给出互相矛盾的旋钮。
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
/// **不能跨编码器照抄刻度**：x265 与 VP9 的 CRF 语义与 x264 不同。数值见下方
/// 校准注释（实施时按 Task 3 Step 5 实测填写）。
fn crf_for(t: TargetFormat, preset: CompressPreset) -> u8 {
    let (small, balanced, high) = match t {
        TargetFormat::Mp4H264 | TargetFormat::MkvH264 => (28, 24, 20),
        TargetFormat::Mp4H265 => (30, 28, 24),
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
    // 而且 mkv 的 muxer 名是 matroska，不是 mkv。
    a.push("-f".into());
    a.push(muxer.into());
    a.push(part_path_for(output).to_string_lossy().into_owned());
    a
}
```

`src/core/videoops/mod.rs` 加 `pub mod transcode;`。
`src/core/videoops/params.rs` 加变体：

```rust
use crate::core::videoops::transcode::{target_extension, target_label, transcode_args, TranscodeParams};

pub enum JobParams {
    Compress(CompressParams),
    Transcode(TranscodeParams),
}
```

四个方法各加一个 arm：

```rust
            JobParams::Transcode(p) => format!("转码 · {}", target_label(p.target)),
```
```rust
            JobParams::Transcode(_) => "-转码",
```
```rust
            // 转码的扩展名由目标格式决定，与源无关
            JobParams::Transcode(p) => target_extension(p.target).to_string(),
```
```rust
            JobParams::Transcode(p) => transcode_args(input, output, p),
```

- [ ] **Step 4: 跑测试看它通过**

Run: `rtk proxy cargo test --test videoops_transcode_test 2>&1 | tail -4`
Expected: `test result: ok. 7 passed`

- [ ] **Step 5: 校准 H.265 与 VP9 的三档 CRF**

`target_extension` 里那些数值是按经验起的**起点值**，必须实测核准。判据（沿用上一轮校准
VideoToolbox `q:v` 的做法）：**同档位的产物体积落在 x264 同档产物的 0.3–1.5 倍之间，
且任意一档都不得大于源文件。**（后半条才是真正会伤到用户的：他要的是转码，拿到一个更大的
文件就是白干。）

Run:

```bash
F=vendor/ffmpeg/macos-aarch64/ffmpeg
rm -rf /tmp/vtc && mkdir -p /tmp/vtc && cd /tmp/vtc
# testsrc2 细节多，纯色块会让 CRF 完全失真
$F -v error -y -f lavfi -i "testsrc2=duration=10:size=1280x720:rate=30" \
   -f lavfi -i "sine=frequency=440:duration=10" -c:v libx264 -preset veryfast -crf 18 \
   -c:a aac -shortest src.mp4
echo "src: $(wc -c < src.mp4)"
for crf in 28 24 20; do
  $F -v error -y -i src.mp4 -c:v libx264 -preset veryfast -crf $crf -c:a aac -b:a 128k -f mp4 x264_$crf.mp4
  echo "x264 crf=$crf -> $(wc -c < x264_$crf.mp4)"
done
for crf in 30 28 24; do
  $F -v error -y -i src.mp4 -c:v libx265 -crf $crf -c:a aac -b:a 128k -f mp4 x265_$crf.mp4 2>/dev/null
  echo "x265 crf=$crf -> $(wc -c < x265_$crf.mp4)"
done
for crf in 40 34 30; do
  $F -v error -y -i src.mp4 -c:v libvpx-vp9 -crf $crf -b:v 0 -c:a libopus -b:a 128k -f webm vp9_$crf.webm 2>/dev/null
  echo "vp9  crf=$crf -> $(wc -c < vp9_$crf.webm)"
done
```

把落在判据之外的档位就地调整 `crf_for` 并重跑，然后把实测体积写进 `crf_for` 的文档注释
（照 `compress.rs` 里 `preset_defaults` 的写法）。**只调数值，不改结构。**

- [ ] **Step 6: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): 转码 — 四种目标格式整组打包

不做成"容器 + 视频编码 + 音频编码"三根自由的轴:webm 只收 VP8/VP9/AV1 +
Vorbis/Opus,硬塞 aac 进去是 Could not write header 的硬失败(实测),而自由的轴
等于允许用户拼出这种组合、要到任务失败才告诉他。配了一条专门的回归测试。

VP9 的恒定质量模式必须 -crf 配 -b:v 0,否则退回默认码率模式、出来的体积与
"档位"完全对不上。

mkv 的 muxer 名是 matroska 而不是 mkv,-f mkv 会直接失败,这条也有测试钉住。

H.265 与 VP9 的三档 CRF 是实测校准过的(判据:同档体积落在 x264 同档的
0.3–1.5 倍,且任意一档不得大于源文件 —— 后半条是真正会伤到用户的那条)。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 4: 裁剪视频 —— 时间解析与两档

**Files:**
- Create: `src/core/videoops/trim.rs`
- Modify: `src/core/videoops/params.rs`（加 `JobParams::Trim` 变体）
- Modify: `src/core/videoops/mod.rs`（`pub mod trim;`）
- Test: `tests/videoops_trim_test.rs`

**Interfaces:**
- Consumes: `compress::CompressPreset`、`media_info::{MediaInfo, container_muxer}`、`params::part_path_for`
- Produces:
  - `pub enum TrimMode { Fast, Accurate }`，`FromStr` 接受 `"fast" | "accurate"`
  - `pub struct TrimParams { pub start: f64, pub end: Option<f64>, pub mode: TrimMode }`
  - `pub fn parse_time(s: &str) -> Option<f64>`
  - `pub fn trim_args(input: &Path, output: &Path, p: &TrimParams, src: &MediaInfo) -> Vec<String>`
  - `pub fn trim_extension(p: &TrimParams, src: &MediaInfo) -> String`
  - `pub fn trim_muxer(p: &TrimParams, src: &MediaInfo) -> &'static str`
  - `pub fn validate_trim(p: &TrimParams, duration_secs: Option<f64>) -> anyhow::Result<()>`

- [ ] **Step 1: 写失败的测试**

`tests/videoops_trim_test.rs`：

```rust
use easydownvideo::core::videoops::compress::CompressPreset;
use easydownvideo::core::videoops::media_info::MediaInfo;
use easydownvideo::core::videoops::trim::{
    parse_time, trim_args, trim_extension, trim_muxer, validate_trim, TrimMode, TrimParams,
};
use std::path::Path;

fn src(container: Option<&str>, dur: Option<f64>) -> MediaInfo {
    MediaInfo {
        duration_secs: dur,
        has_video: true,
        video_codec: Some("h264".into()),
        container: container.map(|c| c.to_string()),
        size_bytes: 1000,
    }
}

fn p(start: f64, end: Option<f64>, mode: TrimMode) -> TrimParams {
    TrimParams { start, end, mode }
}

fn args_of(t: &TrimParams, s: &MediaInfo) -> Vec<String> {
    trim_args(Path::new("/in/a.mkv"), Path::new("/out/a-裁剪.mkv"), t, s)
}

fn has(args: &[String], n: &str) -> bool {
    args.iter().any(|a| a == n)
}

fn arg_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let i = args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("没有 {flag}: {args:?}"));
    &args[i + 1]
}

/// 人有三种写法习惯，只认一种会被骂。**纯数字一律按秒**（90 是 90 秒）。
#[test]
fn parses_the_three_habits() {
    assert_eq!(parse_time("12"), Some(12.0));
    assert_eq!(parse_time("1:30"), Some(90.0));
    assert_eq!(parse_time("01:30"), Some(90.0));
    assert_eq!(parse_time("0:00:12"), Some(12.0));
    assert_eq!(parse_time("1:02:03"), Some(3723.0));
    assert_eq!(parse_time("12.5"), Some(12.5));
    // 前后空白要容忍（用户从别处复制粘贴）
    assert_eq!(parse_time("  1:30  "), Some(90.0));
}

#[test]
fn rejects_garbage_times() {
    for bad in ["", "  ", "abc", "1:2:3:4", "-5", "1:", ":30", "1:xx", "9:99"] {
        assert_eq!(parse_time(bad), None, "应该拒绝: {bad:?}");
    }
}

/// 快速档用 copy，**容器必须沿用源** —— copy 到别的容器常常直接失败。
#[test]
fn fast_mode_keeps_the_source_container() {
    let t = p(12.0, Some(20.0), TrimMode::Fast);
    let args = args_of(&t, &src(Some("mkv"), Some(30.0)));
    assert!(has(&args, "-c"), "快速档必须 -c copy");
    assert_eq!(arg_after(&args, "-c"), "copy");
    assert_eq!(arg_after(&args, "-f"), "matroska"); // 不是 "mkv"
    assert!(!has(&args, "-crf"), "快速档不该出现重编码参数");
    assert_eq!(trim_extension(&t, &src(Some("mkv"), Some(30.0))), "mkv");
    assert_eq!(trim_muxer(&t, &src(Some("mkv"), Some(30.0))), "matroska");
}

/// **源容器认不出来时退到 matroska**（它什么都装得下，copy 一定成功），
/// 并且这个退化会体现在产物名里（外框的预览会显示 .mkv）—— 不是悄悄改扩展名。
#[test]
fn fast_mode_falls_back_to_matroska_when_the_container_is_unknown() {
    let t = p(1.0, Some(2.0), TrimMode::Fast);
    let s = src(None, Some(30.0));
    assert_eq!(trim_muxer(&t, &s), "matroska");
    assert_eq!(trim_extension(&t, &s), "mkv");
}

/// 精确档重编码成 h264+aac，mp4 一定装得下 —— 与压缩页同一句话。
#[test]
fn accurate_mode_encodes_to_mp4() {
    let t = p(12.0, Some(20.0), TrimMode::Accurate);
    let s = src(Some("mkv"), Some(30.0));
    let args = args_of(&t, &s);
    assert_eq!(arg_after(&args, "-c:v"), "libx264");
    assert_eq!(arg_after(&args, "-c:a"), "aac");
    assert_eq!(arg_after(&args, "-f"), "mp4");
    assert_eq!(trim_extension(&t, &s), "mp4");
    assert_eq!(trim_muxer(&t, &s), "mp4");
}

/// `-ss`/`-to` 都放在 `-i` **之前**（输入选项）—— mp4 的索引能给出精确寻址，
/// spec §2 的实测就是按这个写法做的。
#[test]
fn seek_options_come_before_the_input() {
    let args = args_of(&p(12.0, Some(20.0), TrimMode::Fast), &src(Some("mp4"), Some(30.0)));
    let i = args.iter().position(|a| a == "-i").unwrap();
    assert!(args.iter().position(|a| a == "-ss").unwrap() < i);
    assert!(args.iter().position(|a| a == "-to").unwrap() < i);
    assert_eq!(arg_after(&args, "-ss"), "12");
    assert_eq!(arg_after(&args, "-to"), "20");
}

/// `end` 留空 = 到结尾（"从 1:30 到结束"是常见需求），此时不该出现 -to。
#[test]
fn no_end_means_to_the_end() {
    let args = args_of(&p(90.0, None, TrimMode::Fast), &src(Some("mp4"), Some(300.0)));
    assert!(!has(&args, "-to"), "没给结束时间就不该有 -to: {args:?}");
    assert_eq!(arg_after(&args, "-ss"), "90");
}

#[test]
fn validate_rejects_impossible_ranges() {
    let d = Some(30.0);
    // start 超出源时长
    assert!(validate_trim(&p(31.0, None, TrimMode::Fast), d).is_err());
    // start >= end
    assert!(validate_trim(&p(20.0, Some(20.0), TrimMode::Fast), d).is_err());
    assert!(validate_trim(&p(20.0, Some(10.0), TrimMode::Fast), d).is_err());
    // 负数
    assert!(validate_trim(&p(-1.0, Some(10.0), TrimMode::Fast), d).is_err());
    // 合法
    assert!(validate_trim(&p(0.0, Some(30.0), TrimMode::Fast), d).is_ok());
    assert!(validate_trim(&p(0.0, None, TrimMode::Fast), d).is_ok());
    // 时长未知时不拦 start（读不出时长不等于 clip 不合理）
    assert!(validate_trim(&p(9999.0, None, TrimMode::Fast), None).is_ok());
    // end 超过源时长允许（容器时长常常略短于实际），但不允许 start >= end
    assert!(validate_trim(&p(28.0, Some(31.0), TrimMode::Fast), d).is_ok());
}

#[test]
fn modes_parse_from_the_wire_strings() {
    assert_eq!("fast".parse::<TrimMode>().unwrap(), TrimMode::Fast);
    assert_eq!("accurate".parse::<TrimMode>().unwrap(), TrimMode::Accurate);
    assert!("quick".parse::<TrimMode>().is_err());
}
```

- [ ] **Step 2: 跑测试看它失败**

Run: `rtk proxy cargo test --test videoops_trim_test 2>&1 | grep -E "^error" | head -3`
Expected: `unresolved import ... videoops::trim`

- [ ] **Step 3: 写最小实现**

`src/core/videoops/trim.rs`：

```rust
use std::path::Path;
use std::str::FromStr;

use crate::core::videoops::compress::CompressPreset;
use crate::core::videoops::media_info::{container_muxer, MediaInfo};
use crate::core::videoops::params::part_path_for;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimMode {
    /// `-c copy`：秒完、画质一点不丢。
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrimParams {
    pub start: f64,
    /// `None` = 到结尾。
    pub end: Option<f64>,
    pub mode: TrimMode,
}

/// 解析用户填的时间。接受 `mm:ss`、`hh:mm:ss`、纯数字三种写法。
///
/// **纯数字一律按秒解释**（`90` 是 90 秒，不是 90 分）。接受前后空白 ——
/// 用户常从别处复制粘贴。
///
/// 非法输入一律 `None`：调用方据此报错，而不是拿一个 0 去裁出个空文件。
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
            let sec: f64 = parse_secs(s)?;
            Some(m as f64 * 60.0 + sec)
        }
        [h, m, s] => {
            let h: u64 = h.trim().parse().ok()?;
            let m: u64 = m.trim().parse().ok()?;
            let sec: f64 = parse_secs(s)?;
            Some(h as f64 * 3600.0 + m as f64 * 60.0 + sec)
        }
        _ => None,
    }
}

/// 秒那一格：必须是 0–59.999…。`9:99` 这种要拒掉 —— 它不是"99 秒"，是打错了。
fn parse_secs(s: &str) -> Option<f64> {
    let v: f64 = s.trim().parse().ok()?;
    (0.0..60.0).contains(&v).then_some(v)
}

/// 入队前的校验。**时长读不出来时不拦 start** —— 读不出时长不等于这段区间不合理。
///
/// `end` 允许略超源时长：容器的 Duration 常常略短于实际可播长度，
/// 卡死在这上面会让用户裁不掉最后一点。但 `start >= end` 一律拒绝。
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
            anyhow::bail!(
                "起始时间超出了视频长度（这个视频只有 {}）",
                format_secs(d)
            );
        }
    }
    Ok(())
}

fn format_secs(s: f64) -> String {
    let total = s.max(0.0).round() as u64;
    let (h, m, sec) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// 产物的 muxer 名。**快速档必须沿用源容器**（copy 到别的容器常常直接失败），
/// 认不出来时退到 matroska —— 它什么都装得下，copy 一定成功；而这个退化会体现
/// 在产物名里（预览会显示 .mkv），不是悄悄改扩展名。
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

/// 精确档沿用压缩页的质量档位。
const ACCURATE_CRF: u8 = 24; // = CompressPreset::Balanced

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
    // spec §2 的实测就是按这个写法做的。
    a.push("-ss".into());
    a.push(fmt_secs(p.start).into());
    if let Some(end) = p.end {
        a.push("-to".into());
        a.push(fmt_secs(end).into());
    }
    a.push("-i".into());
    a.push(input.to_string_lossy().into_owned());

    match p.mode {
        TrimMode::Fast => {
            a.push("-c".into());
            a.push("copy".into());
        }
        TrimMode::Accurate => {
            // 逐个 push 而不是塞进一个数组：`ACCURATE_CRF.to_string()` 是个临时值，
            // 放进数组会在借用检查上炸掉，而绑一个局部变量只是为了绕开编译器。
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

/// 秒 → ffmpeg 能吃的字符串。**固定 6 位小数**，不用逗号（有的地区设置会把它写成本地化数字）。
fn fmt_secs(s: f64) -> String {
    format!("{:.6}", s.max(0.0))
}
```

`params.rs` 加变体：

```rust
    JobParams::Trim(p) => format!("裁剪 · {}", mode_label(p.mode)),
```
```rust
    JobParams::Trim(_) => "-裁剪",
```
```rust
    JobParams::Trim(p) => trim_extension(p, src),
```
```rust
    JobParams::Trim(p) => trim_args(input, output, p, src),
```
```rust
    JobParams::Trim(p) => validate_trim(p, src.duration_secs),
```

（`mode_label`：`Fast => "快速"`、`Accurate => "精确"`，写在 `trim.rs` 里。
`validate_trim` 已经在 `trim.rs` 里定义好了，这里只是把它接进 `JobParams::validate` ——
**没有这一步的话，测试全绿而校验根本没生效**，那是最坏的一种"完成"。）

- [ ] **Step 4: 跑测试看它通过**

Run: `rtk proxy cargo test --test videoops_trim_test 2>&1 | tail -4`
Expected: `test result: ok. 9 passed`

- [ ] **Step 5: 端到端真跑（两条：两档各一次 + 切口精度回归）**

在 `tests/videoops_trim_test.rs` 追加：

```rust
use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::ffmpeg_job::{run_job, JobOutcome};
use easydownvideo::core::videoops::media_info::inspect;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 造一段 30 秒、**强制 GOP=10 秒**的素材。GOP 必须可控，否则切口精度那条测不出东西。
async fn make_gop10(ffmpeg: &Path, path: &Path) {
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i",
               "testsrc2=duration=30:size=640x360:rate=30",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30",
               "-g", "300", "-keyint_min", "300", "-sc_threshold", "0"])
        .arg(path).output().await.unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// **把 spec §2 的实测钉成回归测试。**
///
/// 流传的说法是 `-c copy` 会把切口吸附到关键帧（于是起点提前、产物更长）。
/// 我们在这台 ffmpeg 9.0.2 上复现不出来：10 秒 GOP 的素材、切 12.5→20，
/// 产物首帧与源在 12.5s 的帧**逐字节相同**。这条测试守住这个行为 ——
/// 哪天它变了（换 ffmpeg 版本），界面文案就该跟着改。
///
/// 这正是"为什么界面上把偏移写成**可能**而不是**会**"的依据。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fast_trim_cut_is_frame_accurate_on_a_sparse_gop_source() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src_path = dir.path().join("gop10.mp4");
    make_gop10(&ffmpeg, &src_path).await;

    let info = inspect(&ffmpeg, &src_path).await.unwrap();
    assert_eq!(info.container.as_deref(), Some("mp4"));

    let out = dir.path().join("cut.mp4");
    let t = TrimParams { start: 12.5, end: Some(20.0), mode: TrimMode::Fast };
    let args = trim_args(&src_path, &out, &t, &info);
    let outcome = run_job(&ffmpeg, &args, &out, Some(30.0), Arc::new(|_| {}),
                          &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);

    // 产物的首帧，与源在 12.5s 的那一帧比 MD5
    let first_frame = |p: &Path, at: Option<f64>| {
        let p = p.to_path_buf();
        let ff = ffmpeg.clone();
        async move {
            let mut cmd = tokio::process::Command::new(&ff);
            cmd.args(["-v", "error"]);
            if let Some(t) = at {
                cmd.args(["-ss", &t.to_string()]);
            }
            cmd.args(["-i"]).arg(&p).args(["-frames:v", "1", "-f", "md5", "-"]);
            let out = cmd.output().await.unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
    };
    let got = first_frame(&out, None).await;
    let want = first_frame(&src_path, Some(12.5)).await;
    assert_eq!(got, want, "快速切的首帧应与源在请求时刻的帧完全相同（吸附的话会差一整个 GOP）");

    // 时长也要对（吸附了会是 10 秒而不是 7.5 秒）
    let cut = inspect(&ffmpeg, &out).await.unwrap();
    let d = cut.duration_secs.expect("产物时长");
    assert!((d - 7.5).abs() < 0.2, "产物应约 7.5 秒（吸附了会变成 10 秒），实际: {d}");
}

/// 精确档真跑一次，断言时长也对。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accurate_trim_runs_and_has_the_right_duration() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src_path = dir.path().join("src.mp4");
    make_gop10(&ffmpeg, &src_path).await;
    let info = inspect(&ffmpeg, &src_path).await.unwrap();

    let out = dir.path().join("cut.mp4");
    let t = TrimParams { start: 5.0, end: Some(9.0), mode: TrimMode::Accurate };
    let args = trim_args(&src_path, &out, &t, &info);
    let outcome = run_job(&ffmpeg, &args, &out, Some(30.0), Arc::new(|_| {}),
                          &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);
    assert!(out.exists());

    let cut = inspect(&ffmpeg, &out).await.unwrap();
    let d = cut.duration_secs.expect("产物时长");
    assert!((d - 4.0).abs() < 0.3, "产物应约 4 秒，实际: {d}");
    assert_eq!(cut.video_codec.as_deref(), Some("h264"), "精确档重编码成 h264");
}
```

Run: `rtk proxy cargo test --test videoops_trim_test 2>&1 | tail -4`
Expected: `test result: ok. 11 passed`

- [ ] **Step 6: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): 裁剪视频 — 快速与精确两档

快速档用 -c copy,**容器必须沿用源** —— copy 到别的容器常常直接失败。
源容器认不出来时退到 matroska(它什么都装得下),而这个退化会体现在产物名里
(预览显示 .mkv),不是悄悄改扩展名。

精确档重编码成 h264+aac 走 mp4 —— 与压缩页同一句话:mp4 一定装得下。

-ss/-to 放在 -i 之前(输入选项):mp4 的索引能给出精确寻址。

时间接受 mm:ss / hh:mm:ss / 纯数字三种写法,纯数字一律按秒解释;
秒那格必须落在 0–60,`9:99` 这种是打错而不是"99 秒",要拒掉。

**一条回归测试把 spec 的实测钉住了**:流传的说法是 -c copy 会把切口吸附到
关键帧,但我们用强制 GOP=10 秒的素材实测复现不出来 —— 切 12.5→20 得到的产物
首帧与源在 12.5s 的帧逐字节相同、时长也精确 7.5 秒。这条测试守住该行为;
哪天它变了,界面文案(把偏移写成"可能"而不是"会")就该跟着改。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 5: 命令面 —— `add_video_job` + `plan_output`

**Files:**
- Modify: `src-tauri/src/dto.rs`（`JobParamsDto`、`VideoInfoDto` 补字段）
- Modify: `src-tauri/src/commands.rs`（删 `add_compress_job`，加 `add_video_job` 与 `plan_output`）
- Modify: `src-tauri/src/main.rs`（`invoke_handler!`）
- Modify: `src/core/videoops/mod.rs`（`plan_output` 的公开入口）
- Test: `tests/videoops_engine_test.rs`（命名一致性）、`src-tauri/src/dto.rs` 的 `mod tests`（DTO 形状）

**Interfaces:**
- Consumes: `videoops::{JobParams, VideoOps}`、`transcode::{TargetFormat, TranscodeParams}`、`trim::{TrimMode, TrimParams}`、`compress::{CompressParams, CompressPreset, Encoder}`
- Produces:
  - `dto::JobParamsDto`（带 `op` tag 的枚举）与 `impl TryFrom<JobParamsDto> for JobParams`（**不是 `From`** —— 里面的 `parse()` 会失败，得把错误交出去）
  - `commands::add_video_job(input, output_dir, params) -> Result<u64, String>`
  - `commands::plan_output(input, output_dir, params) -> Result<String, String>`
  - `VideoOps::plan_output(&self, input: PathBuf, output_dir: PathBuf, params: JobParams) -> anyhow::Result<String>`

- [ ] **Step 1: 写失败的测试**

先在 `tests/videoops_engine_test.rs` 追加一条 —— 它守的是 spec §9 里的"命名一致性"：

```rust
/// **产物名预览与实际落盘必须一致。** 它们走的是同一个 `plan_output_path`，
/// 这条测试守住它：哪天有人给其中一边加了特例（比如预览忘了算在途任务的占位），
/// 这里就红 —— 而那个分叉正是产物名预览存在的意义所在。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plan_output_matches_the_path_add_job_actually_takes() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    make_src(&ffmpeg, &src, 20).await; // 慢到第一个任务跑不完
    let out_dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let ops = VideoOps::new(rt, dir.path().to_path_buf());

    // 还没有任何任务时的预览
    let planned = ops
        .plan_output(src.clone(), out_dir.path().to_path_buf(), params())
        .await
        .unwrap();
    assert!(planned.ends_with("src-压缩.mp4"), "实际: {planned}");

    // 入队，比对它真正用的路径
    let id = ops
        .add_job(src.clone(), out_dir.path().to_path_buf(), params())
        .await
        .unwrap();
    let real = ops.list_jobs().iter().find(|j| j.id == id).unwrap().output.clone();
    assert_eq!(std::path::PathBuf::from(&planned), real, "预览与实际必须一致");

    // 再预览一次：这次要绕开刚入队那个（**在途任务也要占位**），而实际入队也会走同一个名字
    let planned2 = ops
        .plan_output(src.clone(), out_dir.path().to_path_buf(), params())
        .await
        .unwrap();
    assert!(
        planned2.ends_with("src-压缩 (1).mp4"),
        "在途任务必须占位，否则第二行会覆盖第一行的产物。实际: {planned2}"
    );

    ops.cancel(id);
}
```

然后在 `src-tauri/src/dto.rs` 的 `mod tests` 里追加：

```rust
    /// 带 tag 的参数枚举：wire 形状必须与 TS 镜像逐字一致。
    /// 这是 `add_video_job` 唯一的入参，错了就是运行时的 "invalid args"。
    #[test]
    fn job_params_dto_pins_the_tagged_shape() {
        let d: JobParamsDto = serde_json::from_str(
            r#"{"op":"compress","preset":"balanced","encoder":"auto","maxHeight":null,
                "crf":null,"qv":null,"audioKbps":null}"#,
        )
        .unwrap();
        match JobParams::try_from(d).unwrap() {
            JobParams::Compress(p) => {
                assert_eq!(p.preset, CompressPreset::Balanced);
                assert_eq!(p.encoder, Encoder::Auto);
                assert_eq!(p.max_height, None);
            }
            other => panic!("应转成 Compress，得到 {other:?}"),
        }

        let d: JobParamsDto =
            serde_json::from_str(r#"{"op":"transcode","target":"webmvp9","preset":"high"}"#).unwrap();
        match JobParams::try_from(d).unwrap() {
            JobParams::Transcode(p) => {
                assert_eq!(p.target, TargetFormat::WebmVp9);
                assert_eq!(p.preset, CompressPreset::High);
            }
            other => panic!("应转成 Transcode，得到 {other:?}"),
        }

        // endSecs 缺省 = 到结尾
        let d: JobParamsDto =
            serde_json::from_str(r#"{"op":"trim","startSecs":12.5,"mode":"fast"}"#).unwrap();
        match JobParams::try_from(d).unwrap() {
            JobParams::Trim(p) => {
                assert_eq!(p.start, 12.5);
                assert_eq!(p.end, None);
                assert_eq!(p.mode, TrimMode::Fast);
            }
            other => panic!("应转成 Trim，得到 {other:?}"),
        }
    }

    /// 未知的 op / 未知的枚举值要在**反序列化**时就失败，
    /// 而不是放进队列到 worker 里才炸。
    #[test]
    fn job_params_dto_rejects_unknown_values() {
        assert!(serde_json::from_str::<JobParamsDto>(r#"{"op":"explode"}"#).is_err());
        assert!(serde_json::from_str::<JobParamsDto>(
            r#"{"op":"transcode","target":"mpeg2","preset":"high"}"#
        ).is_err());
        assert!(serde_json::from_str::<JobParamsDto>(
            r#"{"op":"trim","startSecs":1,"mode":"quick"}"#
        ).is_err());
    }

    /// `VideoInfoDto` 补了容器与编码 —— 转码页要靠它们显示"源：H.264 · mp4"。
    #[test]
    fn video_info_dto_carries_container_and_codec() {
        let v = VideoInfoDto {
            duration_secs: Some(30.0),
            has_video: true,
            video_codec: Some("h264".into()),
            container: Some("mp4".into()),
            size_bytes: 1000,
        };
        assert_eq!(
            serde_json::to_value(&v).unwrap(),
            serde_json::json!({
                "durationSecs": 30.0,
                "hasVideo": true,
                "videoCodec": "h264",
                "container": "mp4",
                "sizeBytes": 1000
            })
        );
    }
```

- [ ] **Step 2: 跑测试看它失败**

Run: `cd src-tauri && rtk proxy cargo test 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find type JobParamsDto`

- [ ] **Step 3: 写最小实现**

`src-tauri/src/dto.rs` 追加：

```rust
// ===== 视频操作的入参 =====
//
// 一条命令收一个带 tag 的枚举，而不是给每个操作开一条命令：后者要重复
// input/output_dir 几百遍，而八个位置参数那股味道在上一个任务里已经尝过了。
// 用枚举仍然保住编译期类型检查（`set_choice` 已经在用这个模式）。

use easydownvideo::core::videoops::compress::{CompressParams, CompressPreset, Encoder};
use easydownvideo::core::videoops::params::JobParams;
use easydownvideo::core::videoops::transcode::{TargetFormat, TranscodeParams};
use easydownvideo::core::videoops::trim::{TrimMode, TrimParams};

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum JobParamsDto {
    Compress {
        preset: String,
        encoder: String,
        max_height: Option<u32>,
        crf: Option<u8>,
        qv: Option<u8>,
        audio_kbps: Option<u32>,
    },
    Transcode {
        target: String,
        preset: String,
    },
    Trim {
        start_secs: f64,
        #[serde(default)]
        end_secs: Option<f64>,
        mode: String,
    },
}

impl TryFrom<JobParamsDto> for JobParams {
    type Error = anyhow::Error;
    fn try_from(d: JobParamsDto) -> Result<Self, Self::Error> {
        Ok(match d {
            JobParamsDto::Compress { preset, encoder, max_height, crf, qv, audio_kbps } => {
                JobParams::Compress(CompressParams {
                    preset: preset.parse()?,
                    encoder: encoder.parse()?,
                    max_height,
                    crf,
                    qv,
                    audio_kbps,
                })
            }
            JobParamsDto::Transcode { target, preset } => JobParams::Transcode(TranscodeParams {
                target: target.parse()?,
                preset: preset.parse()?,
            }),
            JobParamsDto::Trim { start_secs, end_secs, mode } => JobParams::Trim(TrimParams {
                start: start_secs,
                end: end_secs,
                mode: mode.parse()?,
            }),
        })
    }
}
```

`VideoInfoDto` 加两个字段：

```rust
pub struct VideoInfoDto {
    pub duration_secs: Option<f64>,
    pub has_video: bool,
    /// 视频编码，形如 "h264"。取不到时 None，界面显示"编码未知"。
    pub video_codec: Option<String>,
    /// 容器，形如 "mp4"。取不到时 None。
    pub container: Option<String>,
    pub size_bytes: u64,
}
```

`src/core/videoops/mod.rs` 加公开入口（与 `add_job` 共用 `plan_output_path`）：

```rust
    /// 只算产物名，不入队。**与 `add_job` 走同一个取名函数** —— 两处各写一遍的话，
    /// 预览与实际落盘会在某个边角上分叉（比如"在途任务占位"漏了一边），
    /// 而那正是预览存在的意义。
    pub async fn plan_output(
        &self,
        input: PathBuf,
        output_dir: PathBuf,
        params: JobParams,
    ) -> anyhow::Result<String> {
        if !input.is_file() {
            anyhow::bail!("找不到输入文件：{}", input.display());
        }
        let ffmpeg = self.ffmpeg()?;
        let src = media_info::inspect(&ffmpeg, &input).await?;
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video")
            .to_string();
        let jobs = self.inner.jobs.lock().unwrap();
        Ok(Self::plan_output_path(
            &jobs,
            &output_dir,
            &stem,
            params.suffix(),
            &params.extension(&src),
        )
        .to_string_lossy()
        .into_owned())
    }
```

`src-tauri/src/commands.rs`：**删掉 `add_compress_job`**，加两条：

```rust
#[tauri::command]
pub async fn add_video_job(
    state: State<'_, AppState>,
    input: String,
    output_dir: String,
    params: JobParamsDto,
) -> R<u64> {
    let params = JobParams::try_from(params).map_err(|e| format!("{e:#}"))?;
    let ops = state.videoops.clone();
    ops.add_job(PathBuf::from(input), PathBuf::from(output_dir), params)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// 只算产物名给界面预览。**与 add_video_job 走同一个取名函数** —— 见 core 那边的说明。
#[tauri::command]
pub async fn plan_output(
    state: State<'_, AppState>,
    input: String,
    output_dir: String,
    params: JobParamsDto,
) -> R<String> {
    let params = JobParams::try_from(params).map_err(|e| format!("{e:#}"))?;
    let ops = state.videoops.clone();
    ops.plan_output(PathBuf::from(input), PathBuf::from(output_dir), params)
        .await
        .map_err(|e| format!("{e:#}"))
}
```

`probe_video` 的返回补两个字段：

```rust
    Ok(VideoInfoDto {
        duration_secs: info.duration_secs,
        has_video: info.has_video,
        video_codec: info.video_codec,
        container: info.container,
        size_bytes,
    })
```

`main.rs` 的 `invoke_handler!`：把 `commands::add_compress_job` 换成
`commands::add_video_job` 与 `commands::plan_output`。

- [ ] **Step 4: 跑测试看它通过**

Run:

```bash
cd src-tauri && rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "src-tauri:",s,"passed,",f,"failed"}'
cd .. && rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"passed,",f,"failed"}'
```

Expected: `src-tauri: 20 passed`（原 17 + 新增 3）、root 全绿

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): 命令面收敛成 add_video_job + plan_output

一条命令收一个带 tag 的枚举,而不是给每个操作开一条命令:后者要把 input/
output_dir 重复三遍,而"八个位置参数"那股味道上一个任务已经尝过了。用枚举
仍然保住编译期类型检查(set_choice 已经在用这个模式)。add_compress_job 删掉,
不留两个入口。

plan_output 与 add_job **走同一个取名函数** —— 两处各写一遍的话,产物名预览
与实际落盘会在某个边角上分叉(比如"在途任务占位"只加了一边),而那正是预览
存在的意义。

未知的 op / 未知的枚举值在**反序列化**时就失败,不会放进队列到 worker 里才炸。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 6: 前端数据层

**Files:**
- Modify: `ui/src/lib/types.ts`
- Modify: `ui/src/lib/ipc.ts`
- Test: `ui/src/lib/videoOps.test.ts`（改 ready 守卫）

**Interfaces:**
- Consumes: Task 5 的命令名与参数形状
- Produces:
  - `VideoInfoDto`（含 `videoCodec` / `container`）
  - `JobParamsDto`（TS 判别联合）
  - `ipc.addVideoJob(args)`、`ipc.planOutput(args)`
  - `VIDEO_OPS` 里压缩/转码/裁剪 `ready: true`

- [ ] **Step 1: 写失败的测试**

把 `ui/src/lib/videoOps.test.ts` 的第一条改成：

```ts
  it("本轮 ready 的是压缩、转码、裁剪视频", () => {
    // 这条守卫"不做假按钮"：未实现的必须是 ready: false，界面据此灰显。
    // 全标成 true 的话，用户点上去不会有任何反应 —— 那个界面比没有入口更糟。
    expect(VIDEO_OPS.filter((o) => o.ready).map((o) => o.op)).toEqual([
      "compress",
      "transcode",
      "trim",
    ]);
    // 合并与画面裁剪仍是未实现——各自带一个独立的设计问题，留着以后
    expect(VIDEO_OPS.filter((o) => !o.ready).map((o) => o.op)).toEqual(["merge", "crop"]);
  });
```

- [ ] **Step 2: 跑测试看它失败**

Run: `cd ui && rtk proxy npx vitest run src/lib/videoOps.test.ts 2>&1 | grep -E "FAIL|Tests "`
Expected: FAIL —— 实际只 `["compress"]`

- [ ] **Step 3: 写最小实现**

`ui/src/lib/videoOps.ts` 的两处 `ready` 改成 `true`：

```ts
  { op: "transcode", label: "转码", ready: true },
  { op: "trim", label: "裁剪视频", ready: true },
```

`ui/src/lib/types.ts` 追加：

```ts
/** `probe_video` 的返回。`videoCodec` / `container` 取不到时为 null —— 界面显示"未知"，不编。 */
export interface VideoInfoDto {
  durationSecs: number | null;
  hasVideo: boolean;
  videoCodec: string | null;
  container: string | null;
  sizeBytes: number;
}

/**
 * 视频操作的入参。**带 `op` 判别标签**，与 Rust 侧的 `JobParamsDto` 一一对应
 * （`#[serde(tag = "op", rename_all_fields = "camelCase")]`）。
 * 加一个操作就在这里加一个成员，`switch` 会替你找齐所有该改的地方。
 */
export type JobParamsDto =
  | {
      op: "compress";
      preset: string;
      encoder: string;
      maxHeight: number | null;
      crf: number | null;
      qv: number | null;
      audioKbps: number | null;
    }
  | { op: "transcode"; target: string; preset: string }
  | { op: "trim"; startSecs: number; endSecs: number | null; mode: string };
```

`ui/src/lib/ipc.ts`：删掉 `addCompressJob`，加两条：

```ts
export const addVideoJob = (input: string, outputDir: string, params: JobParamsDto) =>
  invoke<number>("add_video_job", { input, outputDir, params });

/** 只算产物名给界面预览。与 addVideoJob 走 core 里同一个取名函数。 */
export const planOutput = (input: string, outputDir: string, params: JobParamsDto) =>
  invoke<string>("plan_output", { input, outputDir, params });
```

`probeVideo` 的返回类型改成 `VideoInfoDto`：

```ts
export const probeVideo = (path: string) => invoke<VideoInfoDto>("probe_video", { path });
```

（`VideoInfoDto` 加进文件顶部的 `import type { ... } from "./types"` 列表。）

- [ ] **Step 4: 跑测试看它通过**

Run: `cd ui && npx tsc --noEmit 2>&1 | tail -3; rtk proxy npx vitest run 2>&1 | grep -E "Test Files|Tests "`
Expected: tsc 无错误（会先在 `VideoOpPage.tsx` 报 `addCompressJob` 不存在，
**这是预期的** —— 下一个任务会改它；如果 tsc 因此红了，先只跑 vitest 确认测试绿，
tsc 留到 Task 7 之后一起看）

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(ui): 视频操作的前端数据层跟上新命令面

JobParamsDto 做成带 op 判别标签的联合,与 Rust 侧一一对应 —— 加一个操作就是
加一个成员,switch 会替你找齐所有该改的地方。

VIDEO_OPS 里转码与裁剪改成 ready:true,那条"不做假按钮"的守卫测试跟着改成
"这三项 ready、那两项不是"。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 7: 前端拆成外框 + 三个表单

**Files:**
- Create: `ui/src/pages/ops/VideoOpShell.tsx`
- Create: `ui/src/pages/ops/CompressForm.tsx`
- Create: `ui/src/pages/ops/TranscodeForm.tsx`
- Create: `ui/src/pages/ops/TrimForm.tsx`
- Modify: `ui/src/pages/VideoOpPage.tsx`（变成只做分派）
- Create: `ui/src/lib/timecode.ts` + `ui/src/lib/timecode.test.ts`
- Create: `ui/src/lib/opDefaults.ts`

**Interfaces:**
- Consumes: `ipc.{addVideoJob, planOutput, probeVideo, pickVideoFile}`、`useSettings`、`JobList`
- Produces:
  - `ui/src/lib/timecode.ts`：`export function parseTimeInput(s: string): number | null`、`export function formatTimecode(secs: number): string`
  - 每个表单的 props：`{ value: JobParamsDto; onChange: (p: JobParamsDto) => void }`
  - `VideoOpShell` 的 props：`{ op: VideoOp; params: JobParamsDto | null; sourceOk: boolean; children: ReactNode }`

- [ ] **Step 1: 写时间解析的测试**

`ui/src/lib/timecode.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { formatTimecode, parseTimeInput } from "./timecode";

describe("parseTimeInput", () => {
  it("接受 mm:ss / hh:mm:ss / 纯秒数三种写法", () => {
    expect(parseTimeInput("12")).toBe(12);
    expect(parseTimeInput("1:30")).toBe(90);
    expect(parseTimeInput("0:00:12")).toBe(12);
    expect(parseTimeInput("1:02:03")).toBe(3723);
    expect(parseTimeInput("12.5")).toBe(12.5);
  });

  it("容忍前后空白（用户常从别处复制粘贴）", () => {
    expect(parseTimeInput("  1:30  ")).toBe(90);
  });

  it("空与非法一律 null —— 界面据此把开始按钮禁掉，而不是拿 0 去裁出个空文件", () => {
    for (const bad of ["", "  ", "abc", "1:2:3:4", "-5", "1:", ":30", "1:xx", "9:99"]) {
      expect(parseTimeInput(bad)).toBeNull();
    }
  });

  it("**秒那一格必须落在 0–60**：9:99 是打错，不是 99 秒", () => {
    expect(parseTimeInput("1:60")).toBeNull();
    expect(parseTimeInput("1:59")).toBe(59 + 60);
  });
});

describe("formatTimecode", () => {
  it("小给 m:ss，大给 h:mm:ss", () => {
    expect(formatTimecode(8)).toBe("0:08");
    expect(formatTimecode(90)).toBe("1:30");
    expect(formatTimecode(3723)).toBe("1:02:03");
  });
});
```

- [ ] **Step 2: 跑测试看它失败**

Run: `cd ui && rtk proxy npx vitest run src/lib/timecode.test.ts 2>&1 | grep -E "Failed to load|Tests "`
Expected: `Failed to load url ./timecode`

- [ ] **Step 3: 写最小实现**

`ui/src/lib/timecode.ts`：

```ts
/**
 * 解析用户填的时间。**与 core 的 `trim::parse_time` 是同一套规则** ——
 * 两边都实现是因为前端要在输入时就把开始按钮禁掉（不能让用户提交一个明显越界的值
 * 再等 core 报错）。规则变了要一起改，测试两边都有。
 *
 * 纯数字一律按秒解释（`90` 是 90 秒）。秒那一格必须落在 0–60，
 * 所以 `9:99` 是 null —— 那是打错，不是"99 秒"。
 */
export function parseTimeInput(s: string): number | null {
  const t = s.trim();
  if (t === "") return null;
  const parts = t.split(":");
  const secs = (x: string): number | null => {
    const v = Number(x.trim());
    if (x.trim() === "" || Number.isNaN(v) || v < 0 || v >= 60) return null;
    return v;
  };
  if (parts.length === 1) {
    const v = Number(t);
    return Number.isNaN(v) || v < 0 ? null : v;
  }
  if (parts.length === 2) {
    const m = Number(parts[0].trim());
    const s = secs(parts[1]);
    if (!Number.isInteger(m) || m < 0 || s === null) return null;
    return m * 60 + s;
  }
  if (parts.length === 3) {
    const h = Number(parts[0].trim());
    const m = Number(parts[1].trim());
    const s = secs(parts[2]);
    if (!Number.isInteger(h) || h < 0 || !Number.isInteger(m) || m < 0 || s === null) return null;
    return h * 3600 + m * 60 + s;
  }
  return null;
}

export function formatTimecode(secs: number): string {
  const total = Math.max(0, Math.floor(secs));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
    : `${m}:${String(s).padStart(2, "0")}`;
}
```

`ui/src/lib/opDefaults.ts`：

```ts
import type { JobParamsDto } from "./types";
import type { VideoOp } from "./videoOps";

/**
 * 每个操作的初始参数。放在一处，是因为外框要用它来**重置**状态：
 * 切操作时参数必须跟着换，否则从转码切到裁剪会带着一个 `target` 字段，
 * 提交时被 core 拒掉（反序列化失败）。
 */
export function defaultParams(op: VideoOp): JobParamsDto {
  switch (op) {
    case "compress":
      return { op: "compress", preset: "balanced", encoder: "auto",
               maxHeight: null, crf: null, qv: null, audioKbps: null };
    case "transcode":
      return { op: "transcode", target: "mp4h264", preset: "balanced" };
    case "trim":
      return { op: "trim", startSecs: 0, endSecs: null, mode: "fast" };
    default:
      // merge / crop 还没做，走不到这里（侧边栏不给导航）；给一个合法的占位
      return { op: "compress", preset: "balanced", encoder: "auto",
               maxHeight: null, crf: null, qv: null, audioKbps: null };
  }
}
```

`ui/src/pages/ops/VideoOpShell.tsx`（外框：选文件 / probe 信息 / 输出目录 / 产物名预览 /
开始按钮 / 任务列表）：

```tsx
import { useEffect, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import JobList from "../../components/JobList";
import * as ipc from "../../lib/ipc";
import { formatSize } from "../../lib/format";
import { formatTimecode } from "../../lib/timecode";
import { run, useSettings } from "../../store/store";
import type { JobParamsDto, VideoInfoDto } from "../../lib/types";
import { opLabel, type VideoOp } from "../../lib/videoOps";

export default function VideoOpShell({
  op, params, children,
}: {
  op: VideoOp;
  /** 为 null 表示参数还不合法（比如裁剪的起止填错了），开始按钮要禁掉。 */
  params: JobParamsDto | null;
  children: ReactNode;
}) {
  const settings = useSettings();
  const [input, setInput] = useState<string | null>(null);
  const [info, setInfo] = useState<VideoInfoDto | null>(null);
  const [outputDir, setOutputDir] = useState<string | null>(null);
  const [planned, setPlanned] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (settings && outputDir === null) setOutputDir(settings.downloadDir);
  }, [settings, outputDir]);

  async function pick() {
    const p = await run(ipc.pickVideoFile());
    if (!p) return;
    setInput(p);
    setInfo(null);
    setPlanned(null);
    const i = await run(ipc.probeVideo(p));
    if (i) setInfo(i);
  }

  // **只在影响产物名的东西变化时重算**：源文件、操作、以及参数里决定扩展名的那两项
  // （转码的 target、裁剪的 mode）。时间与画质不影响名字，所以改它们不会触发 ——
  // 否则用户每敲一个数字就要跑一次 ffmpeg -i。
  const namer = params === null ? null : JSON.stringify([
    input, op,
    params.op === "transcode" ? params.target : null,
    params.op === "trim" ? params.mode : null,
  ]);
  useEffect(() => {
    if (!input || !outputDir || params === null) return;
    let alive = true;
    void run(ipc.planOutput(input, outputDir, params)).then((v) => {
      if (alive && v !== undefined) setPlanned(v);
    });
    return () => { alive = false; };
    // 依赖刻意只放这个拼出来的字符串（`namer`），不是 [input, outputDir, params] ——
    // 后者每一帧都是新对象，会把 effect 变成死循环。
  }, [namer]);

  async function start() {
    if (!input || !outputDir || params === null || busy) return;
    setBusy(true);
    try {
      const id = await run(ipc.addVideoJob(input, outputDir, params));
      // 入队成功才清空选择：失败时把文件留在框里，用户要重试的就是它
      if (id !== undefined) { setInput(null); setInfo(null); setPlanned(null); }
    } finally {
      setBusy(false);
    }
  }

  const srcLine = info
    ? `源文件 ${formatSize(info.sizeBytes)} · ${
        info.durationSecs != null ? formatTimecode(info.durationSecs) : "时长未知"
      } · ${info.videoCodec ?? "编码未知"} · ${info.container ?? "容器未知"}`
    : null;

  return (
    <>
      <div className="flex items-center gap-2 py-4">
        <Input readOnly value={input ?? ""} placeholder="选一个视频文件…" />
        <Button onClick={() => void pick()}>选择文件</Button>
      </div>

      {srcLine && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-muted)" }}>
          {srcLine}
          {info && !info.hasVideo && " · 这个文件里没有视频流"}
        </p>
      )}

      {children}

      <label className="block mb-5">
        <span style={{ color: "var(--fg-muted)" }}>保存到</span>
        <Input className="mt-1.5" value={outputDir ?? ""}
               onChange={(e) => setOutputDir(e.target.value)} />
      </label>

      {planned && (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-faint)" }}>
          产物：{planned}
        </p>
      )}

      <Button
        variant="primary"
        onClick={() => void start()}
        disabled={busy || !input || !outputDir || params === null || info?.hasVideo === false}
      >
        {busy ? "提交中…" : `开始${opLabel(op)}`}
      </Button>

      <div className="mt-6 pb-8"><JobList /></div>
    </>
  );
}
```

三个表单各只写自己的参数区。`CompressForm` 就是把现在 `VideoOpPage.tsx` 里的「质量 / 高级」那两段**原样搬过来**
（是搬家不是重写：现有那几段是上一个任务逐条验过的，改动越少越好），`onChange` 里把值
合进 `params`；`TranscodeForm` 是「目标格式 / 质量」两个 Select（目标格式的选项来自
一张与 core 的 `TargetFormat` 对应的常量表，含四种格式的中文名）；`TrimForm` 是
「起始 / 结束 / 档位」，其中档位那段带 §6.3 的文案（见下）。

`TrimForm` 的档位选择带着 **spec §6.3 明写的文案**（措辞是有要求的，不是随便写的）：

```tsx
      <div className="mb-5">
        <div style={{ color: "var(--fg-muted)" }}>切法</div>
        <div className="flex gap-1 mt-1.5">
          {[
            { v: "fast", label: "快速" },
            { v: "accurate", label: "精确" },
          ].map((m) => (
            <button
              key={m.v}
              onClick={() => setMode(m.v)}
              aria-pressed={mode === m.v}
              className="h-8 px-3 rounded-md text-[13px] border transition-colors"
              style={{
                borderColor: "var(--line-strong)",
                background: mode === m.v ? "var(--nav-sel)" : "transparent",
                color: "var(--fg)",
                fontWeight: mode === m.v ? 500 : 400,
              }}
            >
              {m.label}
            </button>
          ))}
        </div>
        {/* **文案刻意不对称**：快速档把偏移写成"可能"—— 我们用 10 秒 GOP 的素材实测
            复现不出吸附（见 spec §2），所以不能把没验证的事写成确定的承诺；
            精确档把"多一次质量损失"写成确定，因为那是重编码的必然。 */}
        <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
          {mode === "fast"
            ? "不重编码：秒完，画质一点不丢。个别源上切口可能略有偏移。"
            : "重新编码：慢一些，会多一次质量损失。快速切的结果不对时用它。"}
        </span>
      </div>
```

`TrimForm` 的关键逻辑（其余两个表单直白，照上面结构写）：

```tsx
  // 起止时间**在输入时**就解析：非法时把 params 交 null，外框据此禁掉开始按钮。
  // 这样用户不会先提交、再等 core 报错。
  const start = parseTimeInput(startText);
  const end = endText.trim() === "" ? null : parseTimeInput(endText);
  const bad = start === null || (endText.trim() !== "" && end === null) ||
              (start !== null && end !== null && end <= start);
  useEffect(() => {
    onChange(bad ? null : { op: "trim", startSecs: start!, endSecs: end, mode });
  }, [startText, endText, mode]);
```

`VideoOpPage.tsx` 变成分派：

```tsx
export default function VideoOpPage({ op }: { op: VideoOp }) {
  const [params, setParams] = useState<JobParamsDto | null>(() => defaultParams(op));
  // 切操作时参数必须**整个换掉**，不能留着上一个操作的字段 ——
  // 从转码切到裁剪时带着 target 提交，会被 core 的反序列化拒掉。
  useEffect(() => { setParams(defaultParams(op)); }, [op]);

  if (op === "merge" || op === "crop") {
    return (
      <div className="h-full flex items-center justify-center" style={{ color: "var(--fg-muted)" }}>
        {opLabel(op)}还没做
      </div>
    );
  }

  return (
    <VideoOpShell op={op} params={params}>
      {op === "compress" && <CompressForm value={params} onChange={setParams} />}
      {op === "transcode" && <TranscodeForm value={params} onChange={setParams} />}
      {op === "trim" && <TrimForm value={params} onChange={setParams} />}
    </VideoOpShell>
  );
}
```

（三个表单的 `value` 是 `JobParamsDto | null`，它们在内部自己按 `op` 收窄；
`null` 时按默认值渲染但把 `onChange` 交出去的仍是 null 或修正后的值。）

- [ ] **Step 4: 跑测试、类型检查、构建**

Run:

```bash
cd ui && npx tsc --noEmit 2>&1 | tail -3
rtk proxy npx vitest run 2>&1 | grep -E "Test Files|Tests "
rtk proxy npx vite build 2>&1 | tail -2
```

Expected: tsc 无错误、测试全绿、构建成功

- [ ] **Step 5: 在真实浏览器里逐条验（这是本轮的主要交付，值得实测）**

```bash
cd ui && (npx vite --port 5173 > /tmp/vite3.log 2>&1 &) ; sleep 4
```

用浏览器工具连接 `http://localhost:5173`，逐条确认：

1. 侧边栏「转码」「裁剪视频」不再灰显（`aria-disabled` 为 false、`cursor` 是 pointer），
   合并与画面裁剪仍是灰显 + `title="还没做"`。
2. 点进「转码」→ 标题是「转码」；「目标格式」下拉里有四项；选 WebM 时质量档位仍在。
3. 点进「裁剪」→ 有起始/结束/档位三处；填 `1:30` 与 `abc` 时开始按钮禁用
   （非法输入必须挡住）。
4. 切「转码」→「裁剪」→ 参数被整个换掉（页面上不该还留着"目标格式"那一栏）。

（`planOutput` 在浏览器里会失败——没有 Tauri 后端，`invoke` 会 reject。那属于预期：
产物名预览那一条要等真 app 里验，写在 Task 8。）

- [ ] **Step 6: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
refactor(ui): 视频操作页拆成外框 + 三个表单

VideoOpPage 原本全是压缩的 UI,再塞两个操作就是一个巨型文件。拆成:
外框(选文件/probe 信息/输出目录/产物名预览/开始按钮/任务列表)三个表单共用,
每个表单只写自己的参数区。

切操作时参数**整个换掉**而不是合并:从转码切到裁剪还带着 target 字段提交,
会被 core 的反序列化直接拒掉。

裁剪的起止时间在**输入时**就解析,非法时把参数交 null、外框据此禁掉开始按钮 ——
不让用户先提交再等 core 报错。时间规则与 core 的 trim::parse_time 是同一套,
两边各有一份测试。

产物名预览只在**影响名字**的东西变化时重算(源文件/操作/决定扩展名的 target 与
mode)。时间与画质不影响名字,改它们不会触发 —— 否则用户每敲一个数字就要跑一次
ffmpeg -i。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 8: 全量验证与验收清单

**Files:** 无（只验证）

- [ ] **Step 1: 跑全量套件**

```bash
cd /Users/xuwenqiang/Desktop/easyDownVideo
rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"passed,",f,"failed"}'
(cd src-tauri && rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "src-tauri:",s,"passed,",f,"failed"}')
(cd ui && npx tsc --noEmit 2>&1 | tail -2; rtk proxy npx vitest run 2>&1 | grep -E "^ +Tests "; rtk proxy npx vite build 2>&1 | grep -E "built in")
rtk proxy cargo build --workspace 2>&1 | grep -cE "^warning"
```

Expected: 全绿、tsc 无错误、构建成功、**零告警**

- [ ] **Step 2: 跨边界机械校验（命令名与参数键）**

这一层没有测试钉得住（`ipc.ts` 自己的注释就说靠人盯着），所以用一段一次性脚本核一遍：

```bash
python3 - <<'PY'
import re, pathlib
ipc = pathlib.Path("ui/src/lib/ipc.ts").read_text()
mainrs = pathlib.Path("src-tauri/src/main.rs").read_text()
cmds = pathlib.Path("src-tauri/src/commands.rs").read_text()
invoked = set(re.findall(r'invoke<[^>]*>\("([a-z_]+)"', ipc)) | set(re.findall(r'invoke\("([a-z_]+)"', ipc))
handler = set(re.findall(r'commands::([a-z_]+)', mainrs))
print("ipc.ts 调用但未注册:", sorted(invoked - handler) or "无")
print("已注册但 ipc.ts 没调:", sorted(handler - invoked) or "无")
# 多词形参：Rust snake_case → JS camelCase
def camel(s):
    head, *rest = s.split("_"); return head + "".join(w.capitalize() for w in rest)
m = re.search(r'pub async fn add_video_job\((.*?)\n\)', cmds, re.S)
print("add_video_job 期望的 JS 键:", [camel(p.split(":")[0].strip())
      for p in m.group(1).split(",") if ":" in p and "state" not in p.split(":")[0]])
PY
```

Expected: 两边都"无"；形参是 `input, outputDir, params` —— 与 `ipc.ts` 逐字一致。

- [ ] **Step 3: 真 app 里过一遍 spec 第 10 节的验收清单**

`npm run dev`（或 `cd src-tauri && cargo tauri dev`）起真 app —— `planOutput` 与
`addVideoJob` 都需要真的 Tauri 后端，浏览器里验不了产物名预览那几条。逐条过：

1. 侧边栏「转码」「裁剪视频」不再灰显，点了能进。
2. 转码页显示源格式（如"H.264 · mp4"）；四种目标格式可选；质量三档可切。
3. 转码成 WebM → 跑完、产物是 `.webm`、能播。
4. 把源转成它已有的格式 → 不阻拦，但能从"源：…"那行看出来。
5. 裁剪页显示源时长；`0:12` / `12` / `00:00:12` 三种写法都能解析。
6. 选「快速」→ 秒完；选「精确」→ 慢一些；两次产物时长都约等于请求区间。
7. 「产物：xxx」预览与实际落盘的名字一致；同名时预览里就已经带上了 ` (1)`。
8. 快速档裁一个 mkv → 产物 `.mkv`；精确档裁同一个 → 产物 `.mp4`。
9. `start` 超出源时长 → 入队时就报错，不排进队列。
10. 合并与画面裁剪仍灰显、悬停「还没做」、点了不导航。

- [ ] **Step 4: 把验收结果逐条报给 human partner**

十项逐条写清"过了 / 没过 / 没验"，**不要把没验的写成过了**。任何偏差都要写出来，
包括"预期之外但可接受"的那种（那种最容易在复述时被美化掉，而它往往是下一个 bug 的线索）。

不需要为空提交一次 —— 验收结果是人看的，不是 git 看的。

---

## 收尾

```bash
cd /Users/xuwenqiang/Desktop/easyDownVideo
rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"/",f}'
(cd src-tauri && rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "tauri:",s,"/",f}')
(cd ui && rtk proxy npx vitest run 2>&1 | grep -E "^ +Tests ")
```

**已知的、故意留着的空缺**（不要当成 bug）：

- 合并与画面裁剪仍灰显 —— 各自带一个独立的设计问题（多文件输入、抽帧预览界面）。
- 转码页没有编码器/分辨率选项，只有质量档位 —— 那两个属于压缩页，两个页面都摆全套
  只会让用户分不清该进哪个。
- 裁剪没有可拖拽的时间轴与预览播放器。
- 快速档裁一个容器认不出来的源时产物是 `.mkv`（matroska 什么都装得下）——
  这个退化会显示在产物名预览里，不是悄悄发生的。
- 硬件编码不参与转码（VideoToolbox 只出 h264，转码的价值在换格式）。
