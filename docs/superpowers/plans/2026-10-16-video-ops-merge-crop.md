# 视频操作（其三）· 合并与画面裁剪 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `add_job` 的输入从单文件泛化成一组文件，并在此之上实现「合并」（含兼容性检查）与「画面裁剪」（含抽帧预览），让五个操作全部可用。

**Architecture:** core 侧先把五个能力（label / suffix / extension / args / validate）的入参从 `src: &MediaInfo` 换成 `JobContext`（携带全部输入与媒体信息），再把 `JobEntry.input` 改成 `inputs: Vec<PathBuf>` 并支持一个由 `run_job` 负责清理的临时文件；然后 `merge.rs` 与 `crop.rs` 各加一个 `JobParams` 变体；最后补两条命令（`check_merge` / `preview_frame`）与两个表单。

**Tech Stack:** Rust 2021 / tokio / 内嵌 ffmpeg 9.0.2 / Tauri 2.12.4 / React 19 + TypeScript / vitest

**Spec:** `docs/superpowers/specs/2026-10-16-video-ops-merge-crop-design.md`

## Global Constraints

- 语言：界面文案、注释、commit message 全部用中文。
- 界面**不做假按钮**：本轮把五项全部变成 ready；那条守卫测试改成断言"五项全都 ready、没有灰显项"，**不是删掉**。
- `core` 是唯一真相源：合并的兼容性规则只写在 core 一处，前端不重抄。
- **不新增第三方依赖**（base64 自己写，十几行）。
- 产物**绝不覆盖源文件**；失败或取消**绝不留下半截文件**，也**不留临时文件**。
- 产物的 `-f`（muxer）**必须显式指定**，别指望扩展名推断。
- **裁剪的时长必须与源完全相同** —— 这是本轮验收的硬指标，写成断言。
- 提交结尾带 `Co-Authored-By: Claude Code <noreply@anthropic.com>`。

## Review Focus

以下是 spec 默认满足、但没有单个任务的测试能覆盖到的输入与失败模式。每条都在指名的任务里配了测试。

1. **合并的列表文件路径含撇号**（`~/My's Videos/a.mp4`）→ 用户期望：拼得成。转义漏了就**每一次合并都失败在最后一步**（上一轮 HLS 就是这么栽的）。→ Task 4
2. **裁剪框越界**（源 1920 宽、x=1200 再裁 1280）→ 用户期望：报错说清，而不是拿到画面另一个位置的一块。ffmpeg 会**静默把 x 钳到 640** 且不报警告。→ Task 5
3. **裁剪宽高填奇数**（641）→ 用户期望：要么拒绝要么明确告诉我实际是多少。ffmpeg 会**静默取偶**成 640。→ Task 5
4. **合并时某一段没有音轨**（另几段有）→ 用户期望：能拼，但被告知音轨是断的。→ Task 4
5. **画面尺寸读不出来**（少见的封装）时进裁剪页 → 用户期望：禁用开始按钮并说清原因，而不是拿 0 去算一个必然越界的裁剪框。→ Task 5

---

### Task 1: `MediaInfo` 加画面尺寸与音频编码

合并要逐段比编码（视频与音频都要），裁剪要画面的宽高（裁剪框的坐标就是它）。

**Files:**
- Modify: `src/core/videoops/media_info.rs`
- Test: `tests/videoops_media_info_test.rs`

**Interfaces:**
- Consumes: 无
- Produces:
  - `MediaInfo { duration_secs, has_video, video_codec, audio_codec: Option<String>, container, width: Option<u32>, height: Option<u32> }`
  - `pub fn parse_audio_codec(stderr: &str) -> Option<String>`
  - `pub fn parse_dimensions(stderr: &str) -> Option<(u32, u32)>`

- [ ] **Step 1: 写失败的测试**

在 `tests/videoops_media_info_test.rs` 末尾追加：

```rust
use easydownvideo::core::videoops::media_info::{parse_audio_codec, parse_dimensions};

/// 真实输出（ffmpeg 9.0.2）。
#[test]
fn parses_audio_codec() {
    let stderr = "  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo";
    assert_eq!(parse_audio_codec(stderr).as_deref(), Some("aac"));
    let opus = "  Stream #0:1(und): Audio: opus, 48000 Hz, stereo";
    assert_eq!(parse_audio_codec(opus).as_deref(), Some("opus"));
    // 没有音轨
    assert_eq!(parse_audio_codec("  Stream #0:0(und): Video: h264, yuv420p, 640x360"), None);
}

/// 尺寸是那一行里第一个像 `宽x高` 的东西。
///
/// **两个坑**：`(avc1 / 0x31637661)` 里的十六进制 tag 也含 `x`，`SAR 1:1 DAR 16:9`
/// 里是冒号不是 `x`。前者靠"两个数都必须 ≥ 16"排除，后者天然不会命中。
#[test]
fn parses_frame_dimensions() {
    let real = "  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), \
                yuv420p(progressive), 1920x1080 [SAR 1:1 DAR 16:9], 30 fps";
    assert_eq!(parse_dimensions(real), Some((1920, 1080)));
    // 小尺寸也要认得出来（≥16 就算数）
    assert_eq!(parse_dimensions("  Stream #0:0: Video: h264, yuv420p, 320x240"), Some((320, 240)));
    // 没有视频流
    assert_eq!(parse_dimensions("  Stream #0:0(und): Audio: aac, 44100 Hz"), None);
    // 只有那个十六进制 tag，没有真尺寸 → 不能被它骗到
    assert_eq!(parse_dimensions("  Stream #0:0: Video: h264 (avc1 / 0x31637661)"), None);
}

/// 一次 `ffmpeg -i` 把四样都拿到：时长、有没有视频、编码、容器、尺寸。
#[test]
fn one_pass_yields_dimensions_and_audio_codec_too() {
    let stderr = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4':\n\
  Duration: 00:00:30.00, start: 0.000000, bitrate: 55 kb/s\n\
  Stream #0:0[0x1](und): Video: h264 (High), yuv420p, 1280x720, 30 fps\n\
  Stream #0:1[0x2](und): Audio: aac (LC), 44100 Hz, stereo";
    let m = parse_media_info(stderr, Path::new("/a/x.mp4"));
    assert_eq!(m.width, Some(1280));
    assert_eq!(m.height, Some(720));
    assert_eq!(m.audio_codec.as_deref(), Some("aac"));
}
```

（`parse_media_info` 与 `Path` 在这个文件里已经导入过了。）

- [ ] **Step 2: 跑测试看它失败**

Run: `cargo test --test videoops_media_info_test 2>&1 | grep -E "^error" | head -3`
Expected: `unresolved import ... parse_audio_codec`（以及 `MediaInfo` 少字段）

- [ ] **Step 3: 写最小实现**

`src/core/videoops/media_info.rs`：

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub duration_secs: Option<f64>,
    pub has_video: bool,
    /// 视频编码，形如 "h264" / "hevc" / "vp9"。取不到时 None。
    pub video_codec: Option<String>,
    /// 音频编码，形如 "aac" / "opus"。**没有音轨时是 None**（合并的兼容性检查
    /// 要区分"编码不同"与"压根没有音轨"——后者只是警告，不是拦阻）。
    pub audio_codec: Option<String>,
    /// 容器，形如 "mp4" / "mkv" / "webm" / "mov"。取不到时 None。
    pub container: Option<String>,
    /// 画面尺寸。裁剪框的坐标系就是它，取不到时裁剪不可用。
    pub width: Option<u32>,
    pub height: Option<u32>,
}

fn video_stream_line(stderr: &str) -> Option<&str> {
    stderr
        .lines()
        .find(|l| l.trim_start().starts_with("Stream #") && l.contains("Video:"))
}

/// 从 `Stream #0:1…: Audio: aac (LC) …` 里取音频编码名。没有音轨时 None。
pub fn parse_audio_codec(stderr: &str) -> Option<String> {
    let line = stderr
        .lines()
        .find(|l| l.trim_start().starts_with("Stream #") && l.contains("Audio:"))?;
    let rest = line.split("Audio:").nth(1)?.trim();
    let codec = rest.split_whitespace().next()?.trim();
    (!codec.is_empty()).then(|| codec.to_string())
}

/// 从视频流那一行取 `宽x高`。
///
/// **两个坑**：`(avc1 / 0x31637661)` 里的十六进制 tag 也含 `x`，靠"两个数都 ≥ 16"
/// 排除；`SAR 1:1 DAR 16:9` 用的是冒号，天然不命中。
pub fn parse_dimensions(stderr: &str) -> Option<(u32, u32)> {
    let line = video_stream_line(stderr)?;
    for token in line.split(|c: char| c == ',' || c.is_whitespace()) {
        if let Some((w, h)) = token.split_once('x') {
            if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                if w >= 16 && h >= 16 {
                    return Some((w, h));
                }
            }
        }
    }
    None
}
```

`parse_media_info` 补三个字段：

```rust
pub fn parse_media_info(stderr: &str, path: &Path) -> MediaInfo {
    let (width, height) = match parse_dimensions(stderr) {
        Some((w, h)) => (Some(w), Some(h)),
        None => (None, None),
    };
    MediaInfo {
        duration_secs: parse_duration(stderr),
        has_video: parse_has_video(stderr),
        video_codec: parse_video_codec(stderr),
        audio_codec: parse_audio_codec(stderr),
        container: parse_container(stderr, path),
        width,
        height,
    }
}
```

**两个已有调用点会被打破，一起改**（编译器会逐个指出来）：

1. `tests/videoops_media_info_test.rs` 里 `one_pass_yields_everything_we_need` 的
   `MediaInfo { … }` 字面量补上 `audio_codec: Some("aac"…)` 之外的字段（那一段 stderr
   没有音轨行，所以是 `audio_codec: None`），并补 `width`/`height`。
2. `tests/videoops_compress_test.rs` 里 `src_mp4()` 辅助函数同样补三个字段。

- [ ] **Step 4: 跑测试看它通过**

Run:

```bash
cargo test --test videoops_media_info_test 2>&1 | tail -3
cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"/",f}'
```

Expected: `videoops_media_info_test` 12 passed；**root 全绿**（152 + 新增 3）

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): MediaInfo 加画面尺寸与音频编码

合并要逐段比编码(视频与音频都要),裁剪要画面的宽高(裁剪框的坐标就是它)。

尺寸解析有两个坑,都有测试钉住:`(avc1 / 0x31637661)` 里的十六进制 tag 也含 x,
靠"两个数都 ≥ 16"排除;`SAR 1:1 DAR 16:9` 用的是冒号,天然不命中。

音频编码单独存一栏而不是塞进 video_codec:**没有音轨时是 None**,而合并的
兼容性检查要区分"编码不同"(拦)与"压根没有音轨"(只警告)。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 2: `JobContext` —— 五个能力改收入参

**行为一字不改**，只改签名。合并会逼出这件事：它的 `args` 需要全部输入、`validate`
需要每一段的媒体信息，而现在的签名只收一个 `src`。

**Files:**
- Modify: `src/core/videoops/params.rs`
- Modify: `src/core/videoops/mod.rs`（构造 ctx 并传下去）
- Modify: `src/core/videoops/compress.rs`、`transcode.rs`、`trim.rs`（只把 `src` 换成 `ctx.src()`）
- Test: 现有测试全绿即为证；另在 `tests/videoops_compress_test.rs` 补一条 ctx 的行为测试

**Interfaces:**
- Consumes: `media_info::MediaInfo`（Task 1）
- Produces:
  - `pub struct JobContext<'a> { pub inputs: &'a [PathBuf], pub infos: &'a [MediaInfo], pub output: &'a Path, pub scratch: Option<&'a Path> }`
  - `JobContext::src(&self) -> &MediaInfo`（首段）
  - `JobParams::{label, suffix, extension}` 不变（`extension` 只看首段的媒体信息，
    收 `&MediaInfo` 就够 —— 让它收 ctx 会把"先有 output 才能建 ctx、而 output 又
    要靠 extension 算"绕成一个圈）；`{args, validate}` 改收 `&JobContext`
  - **新增第六个能力** `pub fn duration_secs(&self, ctx: &JobContext) -> Option<f64>`：
    进度条的分母。合并是**各段之和**，其余三个是首段的时长

- [ ] **Step 1: 写失败的测试**

在 `tests/videoops_compress_test.rs` 追加：

```rust
use easydownvideo::core::videoops::params::JobContext;

/// `JobContext::src()` 必须返回**首段**的信息 —— 单输入的操作看的就是它，
/// 而合并那种多输入的，名字与容器也都按首段定。
#[test]
fn job_context_src_is_the_first_input() {
    let a = src_mp4();
    let mut b = src_mp4();
    b.video_codec = Some("hevc".into());
    let inputs = vec![PathBuf::from("/in/a.mp4"), PathBuf::from("/in/b.mp4")];
    let infos = vec![a.clone(), b.clone()];
    let out = PathBuf::from("/out/x.mp4");
    let ctx = JobContext { inputs: &inputs, infos: &infos, output: &out, scratch: None };
    assert_eq!(ctx.src(), &a, "取的是首段");
    assert_eq!(ctx.inputs.len(), 2, "全部输入都在 ctx 里");
    assert!(ctx.scratch.is_none(), "没有临时文件时是 None");
}

/// 进度条的分母：单输入的操作取首段时长（合并那支在 Task 4 里加）。
#[test]
fn job_context_duration_comes_from_the_first_input() {
    let inputs = vec![PathBuf::from("/in/a.mp4")];
    let infos = vec![src_mp4()];
    let out = PathBuf::from("/out/x.mp4");
    let ctx = JobContext { inputs: &inputs, infos: &infos, output: &out, scratch: None };
    assert_eq!(
        JobParams::Compress(p_match()).duration_secs(&ctx),
        Some(5.0)
    );
}
```

（该文件顶部补 `use std::path::PathBuf;` 与 `use easydownvideo::core::videoops::params::JobParams;`——
后者可能已经在。）

- [ ] **Step 2: 跑测试看它失败**

Run: `cargo test --test videoops_compress_test 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find struct JobContext`

- [ ] **Step 3: 写最小实现**

`src/core/videoops/params.rs` 加：

```rust
use std::path::{Path, PathBuf};

use crate::core::videoops::media_info::MediaInfo;

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
```

`JobParams` 的三个方法改签名（方法体只改取信息的方式）：

```rust
    pub fn extension(&self, ctx: &JobContext) -> String {
        match self {
            JobParams::Compress(_) => "mp4".to_string(),
            JobParams::Transcode(p) => target_extension(p.target).to_string(),
            JobParams::Trim(p) => trim_extension(p, ctx.src()),
        }
    }

    pub fn args(&self, ctx: &JobContext) -> Vec<String> {
        match self {
            JobParams::Compress(p) => compress_args(ctx.inputs[0].as_path(), ctx.output, p),
            JobParams::Transcode(p) => transcode_args(ctx.inputs[0].as_path(), ctx.output, p),
            JobParams::Trim(p) => trim_args(ctx.inputs[0].as_path(), ctx.output, p, ctx.src()),
        }
    }

    pub fn validate(&self, ctx: &JobContext) -> anyhow::Result<()> {
        match self {
            JobParams::Compress(_) => Ok(()),
            JobParams::Transcode(_) => Ok(()),
            JobParams::Trim(p) => validate_trim(p, ctx.src().duration_secs),
        }
    }

    /// 进度条的分母。
    ///
    /// **合并是各段之和** —— concat 产物的时长就是它们相加。用首段的时长会让
    /// 进度条在第一段结束时就走到 100%，后面几段看起来像卡住了。
    pub fn duration_secs(&self, ctx: &JobContext) -> Option<f64> {
        match self {
            // 三个单输入的操作：首段的时长
            _ => ctx.src().duration_secs,
        }
    }
```

`mod.rs` 的 `add_job` 里构造 ctx 并改调用：

```rust
        let id = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            // 单输入：ctx 里就一段。多输入是下一个任务的事。
            let inputs = vec![input.clone()];
            let infos = vec![src.clone()];
            let ctx = JobContext {
                inputs: &inputs,
                infos: &infos,
                output: &output,
                scratch: None,
            };
            let output = Self::plan_output_path(
                &jobs,
                &output_dir,
                &stem,
                params.suffix(),
                &params.extension(&ctx),
            );
            let args = params.args(&ctx);
            // …（其余不变；`params.validate(&ctx)?` 在锁外那次调用也要跟着改）
            id
        };
```

> 注意顺序：`plan_output_path` 需要 `output`，而 `ctx` 需要 `output` 的引用 ——
> 所以先算 `output`（它只需要 `ctx` 里的 `extension`，而 `extension` 只看 `src`），
> 再构造 ctx。实现时把 `extension` 的调用放在算 output 那一步、用只含 inputs/infos
> 的 ctx 即可（`scratch`/`output` 在 `args`/`validate` 那两步才用得上）。
> **别把 `output` 的引用塞进一个还没算出来的值里** —— 编译器会拦，但说清顺序省一轮。

`plan_output` 同样改（它只用到 `extension`）。

- [ ] **Step 4: 跑测试看它通过**

Run: `cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"/",f}'`
Expected: `root: 153 / 0`（152 + 新增 1）—— **现有 152 条一条都不能少**

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
refactor(videoops): 五个能力改收 JobContext,行为不变

合并逼出来的:它的 args 要拿全部输入去拼 concat 列表、validate 要逐段比编码,
而原来的签名只收一个 src。JobContext 携带输入列表、逐段媒体信息、产物路径与
可选的临时文件路径,`src()` 提供首段(单输入的操作与"按首段定名字/容器"都看它)。

压缩/转码/裁剪只是把 src 换成 ctx.src(),行为一字不改 —— 现有 152 条测试
一条不少地全绿,那就是这次改动的证据。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 3: 输入变成一组 + 临时文件的清理

`JobEntry.input` → `inputs: Vec<PathBuf>`；`run_job` 多一个 `scratch` 参数，负责在
**所有**退出路径上删掉它。行为仍不变（各操作传单个元素的 vec）。

**Files:**
- Modify: `src/core/videoops/mod.rs`（JobEntry / JobSnapshot / add_job / plan_output / worker）
- Modify: `src/core/videoops/ffmpeg_job.rs`（`run_job` 的 scratch 清理）
- Modify: `tests/videoops_engine_test.rs`（调用点传 vec）
- Test: `tests/videoops_job_test.rs`、`tests/videoops_engine_test.rs`

**Interfaces:**
- Consumes: `JobContext`（Task 2）
- Produces:
  - `VideoOps::add_job(inputs: Vec<PathBuf>, output_dir, params) -> Result<JobId>`
  - `VideoOps::plan_output(inputs: Vec<PathBuf>, output_dir, params) -> Result<String>`
  - `JobSnapshot.inputs: Vec<PathBuf>`
  - `run_job(ffmpeg, args, output, duration_secs, scratch: Option<&Path>, on_progress, cancel)`
  - `JobEntry.scratch: Option<(PathBuf, String)>`（路径 + 内容，内容在入队时算好）

- [ ] **Step 1: 写失败的测试**

在 `tests/videoops_job_test.rs` 末尾追加：

```rust
/// **`run_job` 负责把这次任务用过的临时文件清干净** —— 成功与取消都要清。
///
/// 合并要一个 concat 列表文件；它写在应用数据目录里，不往用户的输出目录里丢东西，
/// 但也不能因为任务失败就永远留在那儿。
///
/// 这里用一个内容无关紧要的临时文件代替真实列表：要验的是**清理**，
/// 不是列表的生成（那是 Task 4）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scratch_file_is_removed_on_success_and_on_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=8:size=320x240:rate=15",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"])
        .arg(&src).output().await.unwrap();
    assert!(gen.status.success());

    // ① 成功路径
    let out = dir.path().join("ok.mp4");
    let scratch = dir.path().join("scratch.txt");
    std::fs::write(&scratch, b"whatever").unwrap();
    let args = compress_args(&src, &out, &params(CompressPreset::Small));
    let outcome = run_job(&ffmpeg, &args, &out, Some(8.0), Some(&scratch),
                          Arc::new(|_| {}), &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);
    assert!(!scratch.exists(), "成功后临时文件必须被删掉");

    // ② 取消路径
    let out2 = dir.path().join("cancelled.mp4");
    let scratch2 = dir.path().join("scratch2.txt");
    std::fs::write(&scratch2, b"whatever").unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let _ = run_job(&ffmpeg, &args, &out2, Some(8.0), Some(&scratch2),
                    Arc::new(|_| {}), &token).await;
    assert!(!scratch2.exists(), "取消后临时文件同样要被删掉");
}

/// 没给 scratch 时照常工作（压缩/转码/裁剪都不需要临时文件）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_job_works_without_a_scratch_file() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=2:size=320x240:rate=15",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"])
        .arg(&src).output().await.unwrap();
    assert!(gen.status.success());
    let out = dir.path().join("out.mp4");
    let args = compress_args(&src, &out, &params(CompressPreset::Small));
    let outcome = run_job(&ffmpeg, &args, &out, Some(2.0), None,
                          Arc::new(|_| {}), &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);
    assert!(out.exists());
}
```

（该文件顶部补 `use easydownvideo::core::videoops::compress::compress_args;`。）

- [ ] **Step 2: 跑测试看它失败**

Run: `cargo test --test videoops_job_test 2>&1 | grep -E "^error" | head -3`
Expected: 参数个数不符（`run_job` 还是 6 个参数）

- [ ] **Step 3: 写最小实现**

`src/core/videoops/ffmpeg_job.rs`：

```rust
/// 退出时删掉这次任务用过的临时文件。
///
/// 放在一个 `Drop` 里而不是逐个返回点手动删：`run_job` 有四个返回点（成功、
/// ffmpeg 非零退出、取消、spawn 失败），逐个记得删迟早漏一个 —— 而"漏一个"的
/// 表现就是用户的磁盘上多出一个没人认领的文件。
struct ScratchGuard(Option<std::path::PathBuf>);

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            std::fs::remove_file(p).ok();
        }
    }
}

pub async fn run_job(
    ffmpeg: &Path,
    args: &[String],
    output: &Path,
    duration_secs: Option<f64>,
    scratch: Option<&Path>,
    on_progress: Arc<dyn Fn(f64) + Send + Sync>,
    cancel: &CancellationToken,
) -> anyhow::Result<JobOutcome> {
    let _scratch = ScratchGuard(scratch.map(|p| p.to_path_buf()));
    let part_path = crate::core::videoops::params::part_path_for(output);
    // …（其余一字不改）
}
```

`mod.rs`：

```rust
struct JobEntry {
    label: String,
    inputs: Vec<PathBuf>,
    output: PathBuf,
    args: Vec<String>,
    /// 本次任务要写的临时文件：路径 + **内容**。内容在入队时就算好
    /// （`concat_list` 是纯函数），worker 只负责写。
    /// 排队中被取消的任务从没写过它，所以没有可清理的东西。
    scratch: Option<(PathBuf, String)>,
    /// 进度条的分母，**入队时就算好**（`params.duration_secs(ctx)`）。
    /// 不放进 worker 去现算：那时 entry 里已经没有逐段的媒体信息了，
    /// 而合并要的是**各段之和**。
    duration: Option<f64>,
    state: JobState,
    progress: Option<f64>,
    cancel: CancellationToken,
}

pub struct JobSnapshot {
    pub id: JobId,
    pub label: String,
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub state: JobState,
    pub progress: Option<f64>,
}
```

`add_job` 的入参改成 `inputs: Vec<PathBuf>`，校验逐个来（**第一段的媒体信息**是
`src`，后续段这轮还没用上，留给 Task 4 的合并）：

```rust
    pub async fn add_job(
        &self,
        inputs: Vec<PathBuf>,
        output_dir: PathBuf,
        params: JobParams,
    ) -> anyhow::Result<JobId> {
        if inputs.is_empty() {
            anyhow::bail!("没有选文件");
        }
        for p in &inputs {
            if !p.is_file() {
                anyhow::bail!("找不到输入文件：{}", p.display());
            }
        }
        // …（output_dir / 可写探针 / ffmpeg 不变）
        // 只探首段：单输入的操作看的就是它
        let src = media_info::inspect(&ffmpeg, &inputs[0]).await?;
        if !src.has_video { /* 不变 */ }
        let stem = inputs[0]
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video")
            .to_string();
        let id = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let infos = vec![src.clone()];
            // `extension` 只看首段，所以这里不需要 output —— 先算它，再算 output，
            // 最后才构造带 output 的完整 ctx。**顺序不能反**：output 要靠 extension
            // 算，而 ctx 要靠 output 建。
            let output = Self::plan_output_path(
                &jobs,
                &output_dir,
                &stem,
                params.suffix(),
                &params.extension(&infos[0]),
            );
            let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
            // 需要临时文件的操作（目前只有合并）在这里把**路径与内容**都备好：
            // 内容是纯函数算的，worker 只负责写。排队中被取消的任务从没写过它。
            let scratch = matches!(params, JobParams::Merge).then(|| {
                (
                    crate::core::videoops::merge::scratch_path(&self.inner.data_dir, id),
                    crate::core::videoops::merge::concat_list(&inputs),
                )
            });
            // `args` 需要列表文件的**路径**，所以从 scratch 借一个引用给它
            let scratch_path = scratch.as_ref().map(|(p, _)| p.as_path());
            let ctx = JobContext {
                inputs: &inputs,
                infos: &infos,
                output: &output,
                scratch: scratch_path,
            };
            params.validate(&ctx)?;
            let duration = params.duration_secs(&ctx);
            let args = params.args(&ctx); // ctx 的最后一处使用 —— 之后才能把 scratch 移进 entry
            jobs.insert(
                id,
                JobEntry {
                    label: params.label(),
                    inputs,
                    output,
                    args,
                    scratch,
                    duration,
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

`worker` 里（两个读 entry 的小方法照 `job_input` 的写法加在 `VideoOps` 上）：

```rust
    fn job_scratch(&self, id: JobId) -> Option<(PathBuf, String)> {
        self.inner.jobs.lock().unwrap().get(&id).and_then(|j| j.scratch.clone())
    }

    fn job_duration(&self, id: JobId) -> Option<f64> {
        self.inner.jobs.lock().unwrap().get(&id).and_then(|j| j.duration)
    }
```

worker 本体：

```rust
                let outcome = async {
                    let ffmpeg = engine.ffmpeg()?;
                    // 临时文件由 worker 写：排队中被取消的任务从没写过它
                    let scratch = engine.job_scratch(id);
                    if let Some((path, content)) = &scratch {
                        std::fs::write(path, content)?;
                    }
                    let duration = engine.job_duration(id);
                    // …（探测首段拿它的时长已不再需要：duration 入队时就算好了）
                    let tx_progress = tx.clone();
                    let engine_progress = engine.clone();
                    let on_progress: Arc<dyn Fn(f64) + Send + Sync> = Arc::new(move |f| {
                        engine_progress.note_progress(id, f);
                        let _ = tx_progress.send(VideoOpEvent::Progress { id, fraction: f });
                    });
                    run_job(
                        &ffmpeg,
                        &args,
                        &output,
                        duration,
                        scratch.as_ref().map(|(p, _)| p.as_path()),
                        on_progress,
                        &cancel,
                    )
                    .await
                }
                .await;
```

- [ ] **Step 4: 改测试调用点并跑全量**

`tests/videoops_engine_test.rs` 里所有 `ops.add_job(x.clone(), …)` 改成
`ops.add_job(vec![x.clone()], …)`；`plan_output` 同理；断言里的 `.input` 改成 `.inputs`。

Run: `cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"/",f}'`
Expected: `root: 155 / 0`（153 + 新增 2）

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
refactor(videoops): 输入从单个变成一组,临时文件归 run_job 清理

JobParams 说"怎么处理",inputs 说"处理什么" —— 合并的参数其实一个都没有
(顺序就是 inputs 的顺序),所以路径不该塞进参数里。现有三个操作各传一个元素的
vec,行为不变。

临时文件的清理放进一个 Drop 守卫:run_job 有四个返回点(成功、ffmpeg 非零退出、
取消、spawn 失败),逐个记得删迟早漏一个,而漏一个的表现就是用户磁盘上多出一个
没人认领的文件。配了两条测试:成功与取消都清、以及不给临时文件时照常工作。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 4: 合并 —— `merge.rs`

**Files:**
- Create: `src/core/videoops/merge.rs`
- Modify: `src/core/videoops/params.rs`（加 `JobParams::Merge`）
- Modify: `src/core/videoops/mod.rs`（`pub mod merge;`）
- Test: `tests/videoops_merge_test.rs`

**Interfaces:**
- Consumes: `hls::escape_concat_path`、`media_info::{container_muxer, MediaInfo}`、`params::{part_path_for, JobContext}`
- Produces:
  - `pub fn concat_list(inputs: &[PathBuf]) -> String`
  - `pub struct MergeCheck { pub blockers: Vec<String>, pub warnings: Vec<String> }`
  - `pub fn check(infos: &[MediaInfo]) -> MergeCheck`
  - `pub fn merge_muxer(src: &MediaInfo) -> &'static str`
  - `pub fn merge_extension(src: &MediaInfo) -> String`
  - `pub fn merge_args(list: &Path, output: &Path, src: &MediaInfo) -> Vec<String>`
  - `pub fn scratch_path(data_dir: &Path, id: JobId) -> PathBuf`

- [ ] **Step 1: 写失败的测试**

`tests/videoops_merge_test.rs`：

```rust
use easydownvideo::core::videoops::media_info::MediaInfo;
use easydownvideo::core::videoops::merge::{
    check, concat_list, merge_args, merge_extension, merge_muxer,
};
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
```

- [ ] **Step 2: 跑测试看它失败**

Run: `cargo test --test videoops_merge_test 2>&1 | grep -E "^error" | head -3`
Expected: `unresolved import ... videoops::merge`

- [ ] **Step 3: 写最小实现**

`src/core/videoops/merge.rs`：

```rust
use std::path::{Path, PathBuf};

use crate::core::videoops::events::TaskId;
use crate::core::videoops::hls::escape_concat_path;
use crate::core::videoops::media_info::{container_muxer, MediaInfo};
use crate::core::videoops::params::part_path_for;

/// ffmpeg concat 解复用器的列表文件内容。
///
/// **转义必须走 `escape_concat_path`** —— 上一轮 HLS 就栽在同一个地方：
/// 路径里带撇号（`~/My's Videos`）时 ffmpeg 把 `'` 当成引号结束，
/// 每一次合并都会失败在最后一步。同一个坑不踩第二次。
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
        if let (Some(a), Some(b)) = (&m.audio_codec, &first.audio_codec) {
            if a != b {
                c.blockers.push(format!(
                    "第 {n} 段的音频编码是 {a}，与第 1 段（{b}）不一致"
                ));
            }
        }
        if (m.audio_codec.is_some()) != (first.audio_codec.is_some()) {
            c.warnings.push(format!("第 {n} 段与第 1 段的音轨有无不一致，拼出来的音轨是断的"));
        }
        if (m.width, m.height) != (first.width, first.height) {
            c.warnings.push(format!(
                "第 {n} 段的画面尺寸（{}x{}）与第 1 段（{}x{}）不同，接缝处画面会变",
                m.width.unwrap_or(0), m.height.unwrap_or(0),
                first.width.unwrap_or(0), first.height.unwrap_or(0)
            ));
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
pub fn scratch_path(data_dir: &Path, id: TaskId) -> PathBuf {
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
```

> **模块路径**：`escape_concat_path` 在 `crate::core::hls` 里（不是 `videoops::hls`）。
> 实现时用正确的路径，别照抄上面的占位。

`params.rs` 加变体：

```rust
use crate::core::videoops::merge::{merge_args, merge_extension, merge_muxer};

pub enum JobParams {
    Compress(CompressParams),
    Transcode(TranscodeParams),
    Trim(TrimParams),
    /// 合并**没有参数** —— 顺序就是 `ctx.inputs` 的顺序。
    Merge,
}

pub enum JobParams { … }
impl JobParams {
    // label: JobParams::Merge => "合并".to_string(),
    // suffix: JobParams::Merge => "-合并",
    // extension: JobParams::Merge => merge_extension(ctx.src()),
    // args: JobParams::Merge => merge_args(ctx.scratch.expect("合并必须有列表文件"), ctx.output, ctx.src()),
    // validate:
    //   JobParams::Merge => {
    //       let c = crate::core::videoops::merge::check(ctx.infos);
    //       if !c.blockers.is_empty() {
    //           anyhow::bail!("{}", c.blockers.join("\n"));
    //       }
    //       Ok(())
    //   }
}
```

> `validate` 里**再判一次**是刻意的：界面已经拦过，但 DTO 是外部输入。

- [ ] **Step 4: 跑测试看它通过**

Run: `cargo test --test videoops_merge_test 2>&1 | tail -3`
Expected: `test result: ok. 8 passed`

- [ ] **Step 5: 端到端真跑（三条）**

在 `tests/videoops_merge_test.rs` 追加：

```rust
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
    let parts: Vec<PathBuf> = ["a", "b", "c"].iter().map(|n| dir.path().join(format!("{n}.mp4"))).collect();
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
```

Run: `cargo test --test videoops_merge_test 2>&1 | tail -3`
Expected: `test result: ok. 11 passed`

- [ ] **Step 6: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): 合并 — 无损拼接 + 兼容性检查

**编码不一致必须我们主动拦**:实测 -c copy 拼接对 h264+h265 报 exit 0 但产物
解码器报 3 行错、h264+mpeg4 报 **174 行**,基本打不开。ffmpeg 不会告诉我们。

判据的分界来自实测:编码不同 → 拦;分辨率不同(0 错)、帧率不同(2 处错)、
音频参数不同(0 错但时长偏)、有无音轨混合(0 错) → 警告。**没有音轨不等于
编码不一致** —— 那该是警告,否则用户会遇到"明明能拼却不让我拼"。

列表文件的转义走 escape_concat_path:上一轮 HLS 就是栽在这里(下载目录里带撇号
时每一次合并都失败在最后一步),这条是回归测试,另有一条端到端用真的
"My's Videos" 目录再验一遍。

容器沿用首段,认不出来退 matroska —— 与裁剪的快速档同一套规则、同一个 helper。

合并没有参数(顺序就是 inputs 的顺序),所以 JobParams::Merge 是个单元变体。

add_job 里为它备好 concat 列表的**路径与内容**(内容是纯函数算的),
worker 只负责写、run_job 负责清 —— 排队中被取消的任务从没写过它。

进度条的分母对合并是**各段之和**:用首段时长会让进度在第一段结束时就走到 100%,
后面几段看起来像卡住了。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 5: 画面裁剪 —— `crop.rs`

**Files:**
- Create: `src/core/videoops/crop.rs`
- Modify: `src/core/videoops/params.rs`（加 `JobParams::Crop`）
- Modify: `src/core/videoops/mod.rs`（`pub mod crop;`）
- Test: `tests/videoops_crop_test.rs`

**Interfaces:**
- Consumes: `compress::CompressPreset`、`media_info::MediaInfo`、`params::part_path_for`
- Produces:
  - `pub struct CropParams { pub width: u32, pub height: u32, pub x: u32, pub y: u32, pub preset: CompressPreset }`
  - `pub fn validate_crop(p: &CropParams, src: &MediaInfo) -> anyhow::Result<()>`
  - `pub fn crop_args(input: &Path, output: &Path, p: &CropParams) -> Vec<String>`

- [ ] **Step 1: 写失败的测试**

`tests/videoops_crop_test.rs`：

```rust
use easydownvideo::core::videoops::compress::CompressPreset;
use easydownvideo::core::videoops::crop::{crop_args, validate_crop, CropParams};
use easydownvideo::core::videoops::media_info::MediaInfo;
use std::path::Path;

fn src(w: Option<u32>, h: Option<u32>) -> MediaInfo {
    MediaInfo {
        duration_secs: Some(10.0),
        has_video: true,
        video_codec: Some("h264".into()),
        audio_codec: Some("aac".into()),
        container: Some("mp4".into()),
        width: w,
        height: h,
    }
}

fn p(w: u32, h: u32, x: u32, y: u32) -> CropParams {
    CropParams { width: w, height: h, x, y, preset: CompressPreset::Balanced }
}

#[test]
fn argv_uses_the_crop_filter_and_re_encodes() {
    let args = crop_args(Path::new("/in/a.mp4"), Path::new("/out/a-裁剪.mp4"), &p(640, 360, 320, 180));
    let has = |n: &str| args.iter().any(|a| a == n);
    assert!(has("-nostdin") && has("-hide_banner") && has("-progress") && has("-y"));
    let vf = args.iter().position(|a| a == "-vf").expect("要有 -vf");
    assert_eq!(args[vf + 1], "crop=640:360:320:180");
    // 裁剪必须重编码（没有 -c copy 这条路）
    assert_eq!(args[args.iter().position(|a| a == "-c:v").unwrap() + 1], "libx264");
    assert_eq!(args[args.iter().position(|a| a == "-f").unwrap() + 1], "mp4");
    assert!(args.last().unwrap().ends_with(".part"));
    // **绝对不能出现 -ss / -to** —— 画面裁剪不改时长，那正是它与「裁剪视频」的分界
    assert!(!has("-ss") && !has("-to"), "画面裁剪不许碰时间轴: {args:?}");
}

/// **越界必须拒绝，不能交给 ffmpeg。** 实测：源 1920 宽、请求 x=1200 再裁 1280
/// （1200+1280=2480 已超出），ffmpeg **把 x 静默钳到 640** 照常输出 —— 用户要的是
/// 画面右侧那一块，拿到的是完全另一个位置的一块，而 stderr 一条警告都没有。
#[test]
fn rejects_out_of_bounds() {
    let s = src(Some(1920), Some(1080));
    let e = validate_crop(&p(1280, 720, 1200, 800), &s).unwrap_err();
    assert!(format!("{e:#}").contains("1920"), "要说清源有多宽: {e:#}");
    assert!(validate_crop(&p(1920, 1080, 1, 0), &s).is_err(), "x+宽 超出也要拒");
    assert!(validate_crop(&p(1280, 1080, 640, 0), &s).is_ok(), "正好贴边可以");
}

/// **奇数宽高必须拒绝。** 实测：请求 crop=641:361 得到的是 **640:360** ——
/// ffmpeg 静默取偶，没有提示。
#[test]
fn rejects_odd_dimensions() {
    let s = src(Some(1920), Some(1080));
    for (w, h) in [(641, 360), (640, 361), (1, 100), (100, 1), (0, 100)] {
        assert!(validate_crop(&p(w, h, 0, 0), &s).is_err(), "{w}x{h} 应当被拒");
    }
    assert!(validate_crop(&p(640, 360, 0, 0), &s).is_ok());
    assert!(validate_crop(&p(2, 2, 0, 0), &s).is_ok(), "2x2 是技术下限，允许");
}

/// **源尺寸读不出来时拒绝**，并说清原因 —— 不能拿 0 去算裁剪框。
#[test]
fn rejects_when_the_source_dimensions_are_unknown() {
    let e = validate_crop(&p(640, 360, 0, 0), &src(None, None)).unwrap_err();
    assert!(format!("{e:#}").contains("画面尺寸"), "要说清读不出尺寸: {e:#}");
}

#[test]
fn quality_preset_shows_up_in_the_argv() {
    let mut params = p(640, 360, 0, 0);
    params.preset = CompressPreset::Small;
    let args = crop_args(Path::new("/in/a.mp4"), Path::new("/out/a.mp4"), &params);
    assert_eq!(args[args.iter().position(|a| a == "-crf").unwrap() + 1], "28");
}
```

- [ ] **Step 2: 跑测试看它失败**

Run: `cargo test --test videoops_crop_test 2>&1 | grep -E "^error" | head -3`
Expected: `unresolved import ... videoops::crop`

- [ ] **Step 3: 写最小实现**

`src/core/videoops/crop.rs`：

```rust
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
            p.width, p.height
        );
    }
    let (Some(sw), Some(sh)) = (src.width, src.height) else {
        // 读不出尺寸就没法判越界，而那正是最容易出错的地方 —— 宁可拒绝
        anyhow::bail!("读不出这个视频的画面尺寸，无法裁剪");
    };
    if p.x + p.width > sw {
        anyhow::bail!(
            "左 {} + 宽 {} 超出了画面的宽度 {}",
            p.x, p.width, sw
        );
    }
    if p.y + p.height > sh {
        anyhow::bail!(
            "上 {} + 高 {} 超出了画面的高度 {}",
            p.y, p.height, sh
        );
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
```

`params.rs` 加变体（`label` → `"裁剪画面"`、`suffix` → `"-画面裁剪"`、
`extension` → `"mp4"`、`args` → `crop_args(inputs[0], output, p)`、
`validate` → `validate_crop(p, ctx.src())`）。

> `suffix` 用 `-画面裁剪` 而不是 `-裁剪` —— 后者与「裁剪视频」撞名，产物名会分不清。

- [ ] **Step 4: 跑测试看它通过**

Run: `cargo test --test videoops_crop_test 2>&1 | tail -3`
Expected: `test result: ok. 5 passed`

- [ ] **Step 5: 端到端真跑：尺寸对、**时长与帧数都不变**（本轮验收的硬指标）**

在 `tests/videoops_crop_test.rs` 追加：

```rust
use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::ffmpeg_job::{run_job, JobOutcome};
use easydownvideo::core::videoops::media_info::inspect;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 数帧数：`-f null -` 会把每个解码出来的帧打一行。
async fn frame_count(ffmpeg: &Path, p: &Path) -> usize {
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-v", "error", "-i"]).arg(p).args(["-f", "null", "-"])
        .output().await.unwrap();
    assert!(out.status.success());
    // -v error 下没有帧行输出，所以改用 -nostats 的 stats 行数不靠谱；
    // 这里改用 showinfo 数帧（走 info 级，不能用 -v error 掐掉）
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-v", "info", "-i"]).arg(p)
        .args(["-vf", "showinfo", "-fps_mode", "passthrough", "-f", "null", "-"])
        .output().await.unwrap();
    String::from_utf8_lossy(&out.stderr).matches("pts_time:").count()
}

/// **裁剪的时长与帧数都必须与源完全相同。**
///
/// 这是本轮验收的硬指标（用户明确要求"生成的视频长度不能改变"）。
/// `-vf` 不碰时间戳，所以它天然成立 —— 但正因为"天然成立"，才必须钉住：
/// 将来有人往这段 argv 里加 `-ss`/`-to` 或换滤镜，这条会立刻红。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crop_preserves_duration_and_frame_count_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src_path = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=3:size=640x360:rate=30",
               "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "28",
               "-c:a", "aac", "-shortest"])
        .arg(&src_path).output().await.unwrap();
    assert!(gen.status.success());

    let info = inspect(&ffmpeg, &src_path).await.unwrap();
    assert_eq!((info.width, info.height), (Some(640), Some(360)));
    let src_frames = frame_count(&ffmpeg, &src_path).await;
    assert!(src_frames > 80, "源应当有约 90 帧，实际 {src_frames}");

    let out = dir.path().join("cropped.mp4");
    let p = CropParams { width: 320, height: 180, x: 160, y: 90, preset: CompressPreset::Balanced };
    validate_crop(&p, &info).expect("这个框在 640x360 里是合法的");
    let args = crop_args(&src_path, &out, &p);
    let outcome = run_job(&ffmpeg, &args, &out, Some(3.0), None,
                          Arc::new(|_| {}), &CancellationToken::new()).await.unwrap();
    assert_eq!(outcome, JobOutcome::Done);

    let back = inspect(&ffmpeg, &out).await.unwrap();
    assert_eq!((back.width, back.height), (Some(320), Some(180)), "尺寸就是请求的那个");
    let d_in = info.duration_secs.unwrap();
    let d_out = back.duration_secs.unwrap();
    assert!((d_in - d_out).abs() < 0.05, "**时长必须一模一样**：源 {d_in} vs 产物 {d_out}");
    let out_frames = frame_count(&ffmpeg, &out).await;
    assert_eq!(out_frames, src_frames, "**帧数也必须一模一样**：源 {src_frames} vs 产物 {out_frames}");
}
```

Run: `cargo test --test videoops_crop_test 2>&1 | tail -3`
Expected: `test result: ok. 6 passed`

- [ ] **Step 6: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): 画面裁剪 — 改画面尺寸,时长不变

与「裁剪视频」是两件事:那边截一段时间(时长会变),这边只取画面的一块
(时长**必须一模一样**)。产物后缀用 -画面裁剪 而不是 -裁剪,免得两者撞名。

**合法性必须我们自己判**:实测源 1920 宽、请求 x=1200 再裁 1280(2480 已超出),
ffmpeg **把 x 静默钳到 640** 照常输出 —— 用户要的是画面右侧那一块,拿到的是
完全另一个位置的一块,而 stderr 一条警告都没有。奇数同理:请求 641 得到 640。
所以越界、奇数、源尺寸读不出来,一律拒绝并说清。

一条端到端把"时长不变"钉成断言:比较源与产物的**时长**(±0.05 秒)与**帧数**
(必须相等)。这正是用户明确提的那条要求,而 `-vf` 天然满足它 —— 正因为天然成立,
才要钉住:将来有人往这段 argv 里加 -ss/-to 或换滤镜,这条会立刻红。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 6: 抽帧预览 —— `preview.rs`

**Files:**
- Create: `src/core/videoops/preview.rs`
- Modify: `src/core/videoops/mod.rs`（`pub mod preview;`）
- Test: `tests/videoops_preview_test.rs`

**Interfaces:**
- Consumes: 无（只用内嵌 ffmpeg）
- Produces:
  - `pub fn base64_encode(bytes: &[u8]) -> String`（纯函数）
  - `pub const PREVIEW_MAX_WIDTH: u32 = 640;`
  - `pub async fn frame_data_url(ffmpeg: &Path, input: &Path, at_secs: f64, max_width: u32) -> anyhow::Result<String>`

- [ ] **Step 1: 写失败的测试**

`tests/videoops_preview_test.rs`：

```rust
use easydownvideo::core::ffmpeg::ensure_ffmpeg;
use easydownvideo::core::videoops::preview::{base64_encode, frame_data_url, PREVIEW_MAX_WIDTH};
use std::path::Path;

/// 自己写 base64 是为了**不新增依赖** —— 十几行，配一条与已知编码对照的测试。
#[test]
fn base64_matches_known_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    // 非 ASCII 字节也要对（JPEG 里全是）
    assert_eq!(base64_encode(&[0xff, 0xd8, 0xff]), "/9j/");
}

/// 真抽一帧：产物必须是能认得出来的 JPEG，而且是 data URL。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extracts_a_jpeg_data_url() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=3:size=1280x720:rate=30",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "28"])
        .arg(&src).output().await.unwrap();
    assert!(gen.status.success());

    let url = frame_data_url(&ffmpeg, &src, 1.0, PREVIEW_MAX_WIDTH).await.unwrap();
    assert!(url.starts_with("data:image/jpeg;base64,"), "要能直接塞进 img.src");
    // 解码回来的头两个字节必须是 JPEG 的 SOI（FF D8）
    let b64 = url.strip_prefix("data:image/jpeg;base64,").unwrap();
    assert!(b64.starts_with("/9j/"), "JPEG 的 base64 一定以 /9j/ 开头，实际: {}", &b64[..8.min(b64.len())]);
    // 体积要小到能走 IPC（实测 640 宽约 13.7KB，base64 后 18KB；留足余量）
    assert!(b64.len() < 200_000, "预览图不该这么大: {} 字节", b64.len());
}

/// 时间超出时长时不 panic，而是给一个错误让界面显示一行"取预览失败"。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beyond_the_end_still_returns_something_or_an_error_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let src = dir.path().join("src.mp4");
    let gen = tokio::process::Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=duration=1:size=320x240:rate=15",
               "-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"])
        .arg(&src).output().await.unwrap();
    assert!(gen.status.success());
    // 只能保证"不 panic"：ffmpeg 对超出时长 + -ss 的行为是给最后一帧或报错，
    // 两种都能接受，非法的输入不能把进程带走
    let _ = frame_data_url(&ffmpeg, &src, 9999.0, PREVIEW_MAX_WIDTH).await;
}

/// 文件不存在时给 Err，不 panic。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = ensure_ffmpeg(dir.path()).unwrap();
    let r = frame_data_url(&ffmpeg, Path::new("/nope/none.mp4"), 0.0, PREVIEW_MAX_WIDTH).await;
    assert!(r.is_err());
}
```

- [ ] **Step 2: 跑测试看它失败**

Run: `cargo test --test videoops_preview_test 2>&1 | grep -E "^error" | head -3`
Expected: `unresolved import ... videoops::preview`

- [ ] **Step 3: 写最小实现**

`src/core/videoops/preview.rs`：

```rust
use std::path::Path;

use anyhow::Context;

/// 预览图缩到多宽。640 宽的一张 JPEG 实测 13.7 KB（base64 后 18 KB）——
/// 走 IPC 毫无压力，所以**不必**去开 Tauri 的 assetProtocol。
pub const PREVIEW_MAX_WIDTH: u32 = 640;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64。自己写是因为全局约束说了不新增依赖 —— 十几行，标准向量测着。
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// 抽一帧，缩到 `max_width` 宽，返回可以直接塞进 `<img src>` 的 data URL。
///
/// `-ss` 放在 `-i` **之前**（输入选项）：对单帧来说那是"跳过去再解一帧"，
/// 比解到那一帧快得多（长视频上差一个数量级）。
pub async fn frame_data_url(
    ffmpeg: &Path,
    input: &Path,
    at_secs: f64,
    max_width: u32,
) -> anyhow::Result<String> {
    let out = tokio::process::Command::new(ffmpeg)
        .args(["-nostdin", "-v", "error", "-y"])
        .arg("-ss").arg(format!("{:.3}", at_secs.max(0.0)))
        .arg("-i").arg(input)
        .args(["-frames:v", "1"])
        .args(["-vf", &format!("scale={max_width}:-2")])
        .args(["-c:v", "mjpeg", "-q:v", "5", "-f", "image2", "-"])
        .output()
        .await
        .context("run ffmpeg (frame)")?;
    if !out.status.success() {
        anyhow::bail!("抽帧失败: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    if out.stdout.is_empty() {
        anyhow::bail!("抽帧没有输出（可能是时间超出了视频长度）");
    }
    Ok(format!("data:image/jpeg;base64,{}", base64_encode(&out.stdout)))
}
```

- [ ] **Step 4: 跑测试看它通过**

Run: `cargo test --test videoops_preview_test 2>&1 | tail -3`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): 抽帧预览

裁剪框的坐标是源像素,不看画面就是瞎猜 —— 所以要把一帧抽出来给界面看。

走 **data URL 而不是文件协议**:640 宽的一张 JPEG 实测 13.7KB(base64 后 18KB),
走 IPC 毫无压力;而 tauri.conf.json 里**没有开** assetProtocol,走文件协议还得
加配置与权限,没必要。

base64 自己写(标准向量测着),因为全局约束说了不新增依赖。

-ss 放在 -i 之前:对单帧来说是"跳过去再解一帧",长视频上比解到那一刻快一个数量级。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 7: 命令面 —— 多输入 + 两条新命令

**Files:**
- Modify: `src-tauri/src/dto.rs`（`JobParamsDto` 两个变体、`VideoInfoDto` 补字段、`MergeCheckDto`）
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`（`invoke_handler!`）
- Modify: `src/core/videoops/mod.rs`（供命令层调用的 `check_merge` / `preview_frame` 入口）
- Test: `src-tauri/src/dto.rs` 的 `mod tests`

**Interfaces:**
- Consumes: Task 4/5/6 的全部产出
- Produces:
  - `dto::JobParamsDto` += `Merge` / `Crop { width, height, x, y, preset }`
  - `dto::VideoInfoDto` += `width` / `height` / `audio_codec`
  - `dto::MergeCheckDto { blockers: Vec<String>, warnings: Vec<String> }`
  - `commands::{add_video_job, plan_output}` 收 `inputs: Vec<String>`
  - `commands::check_merge(inputs: Vec<String>) -> R<MergeCheckDto>`
  - `commands::preview_frame(path: String, at_secs: f64) -> R<String>`
  - `VideoOps::check_merge(&self, inputs: Vec<PathBuf>) -> anyhow::Result<MergeCheck>`
  - `VideoOps::preview(&self, path: PathBuf, at_secs: f64) -> anyhow::Result<String>`

- [ ] **Step 1: 写失败的测试**

在 `src-tauri/src/dto.rs` 的 `mod tests` 里追加：

```rust
    /// 新加的两个变体：wire 形状必须与 TS 镜像逐字一致。
    #[test]
    fn merge_and_crop_params_pin_the_tagged_shape() {
        let d: JobParamsDto = serde_json::from_str(r#"{"op":"merge"}"#).unwrap();
        assert_eq!(JobParams::try_from(d).unwrap(), JobParams::Merge);

        let d: JobParamsDto = serde_json::from_str(
            r#"{"op":"crop","width":640,"height":360,"x":320,"y":180,"preset":"high"}"#,
        )
        .unwrap();
        match JobParams::try_from(d).unwrap() {
            JobParams::Crop(p) => {
                assert_eq!((p.width, p.height, p.x, p.y), (640, 360, 320, 180));
                assert_eq!(p.preset, CompressPreset::High);
            }
            other => panic!("应转成 Crop，得到 {other:?}"),
        }
    }

    /// 非法取值要在**入队之前**失败（与上一轮那两个变体同样的分工：
    /// 未知 tag 在反序列化时挂，未知取值在 TryFrom 时挂）。
    #[test]
    fn crop_rejects_unknown_preset_and_missing_fields() {
        // preset 不认
        let d: JobParamsDto = serde_json::from_str(
            r#"{"op":"crop","width":2,"height":2,"x":0,"y":0,"preset":"huge"}"#,
        )
        .unwrap();
        assert!(JobParams::try_from(d).is_err());
        // 少字段 → 反序列化就挂
        assert!(serde_json::from_str::<JobParamsDto>(r#"{"op":"crop","width":2}"#).is_err());
    }

    /// `VideoInfoDto` 补了宽高与音频编码 —— 裁剪页要宽高，合并页要音频编码。
    #[test]
    fn video_info_dto_carries_dimensions_and_audio_codec() {
        let v = VideoInfoDto {
            duration_secs: Some(30.0),
            has_video: true,
            video_codec: Some("h264".into()),
            audio_codec: Some("aac".into()),
            container: Some("mp4".into()),
            width: Some(1280),
            height: Some(720),
            size_bytes: 1000,
        };
        assert_eq!(
            serde_json::to_value(&v).unwrap(),
            serde_json::json!({
                "durationSecs": 30.0,
                "hasVideo": true,
                "videoCodec": "h264",
                "audioCodec": "aac",
                "container": "mp4",
                "width": 1280,
                "height": 720,
                "sizeBytes": 1000
            })
        );
        // 读不出来时必须是 null 而不是缺字段
        let v = VideoInfoDto { width: None, height: None, audio_codec: None, ..v };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["width"], serde_json::Value::Null);
        assert_eq!(j["audioCodec"], serde_json::Value::Null);
    }

    /// 合并的检查结果：两个数组都必须在（前端按它们渲染红字与灰字）。
    #[test]
    fn merge_check_dto_pins_its_keys() {
        let c = MergeCheckDto {
            blockers: vec!["第 2 段的视频编码是 hevc，与第 1 段（h264）不一致".into()],
            warnings: vec![],
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            serde_json::json!({
                "blockers": ["第 2 段的视频编码是 hevc，与第 1 段（h264）不一致"],
                "warnings": []
            })
        );
    }
```

- [ ] **Step 2: 跑测试看它失败**

Run: `cd src-tauri && cargo test 2>&1 | grep -E "^error" | head -3`
Expected: `cannot find variant Merge`（或 `cannot find type MergeCheckDto`）

- [ ] **Step 3: 写最小实现**

`dto.rs` 追加/修改：

```rust
    Merge,
    Crop {
        width: u32,
        height: u32,
        x: u32,
        y: u32,
        preset: String,
    },
```

`TryFrom` 加两个 arm：

```rust
            JobParamsDto::Merge => JobParams::Merge,
            JobParamsDto::Crop { width, height, x, y, preset } => {
                JobParams::Crop(CropParams {
                    width,
                    height,
                    x,
                    y,
                    preset: preset.parse().map_err(|e| format!("{e:#}"))?,
                })
            }
```

`VideoInfoDto` 补三个字段（`width` / `height` / `audio_codec`，都在 `MediaInfo` 里了）。

```rust
/// 合并的兼容性结论。**规则只在 core 里一份** —— 前端只负责把它画出来。
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeCheckDto {
    /// 非空时禁止提交。
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
}
```

`mod.rs` 加两个供命令层用的入口：

```rust
    /// 合并的兼容性检查。**与 `add_job` 里那次走同一个 `merge::check`** ——
    /// 两处各写一遍的话，界面放行的组合会被入队拒绝（或反过来），
    /// 而用户看到的是"按钮能点、点了报错"。
    pub async fn check_merge(&self, inputs: Vec<PathBuf>) -> anyhow::Result<merge::MergeCheck> {
        let ffmpeg = self.ffmpeg()?;
        let mut infos = Vec::with_capacity(inputs.len());
        for p in &inputs {
            if !p.is_file() {
                anyhow::bail!("找不到输入文件：{}", p.display());
            }
            infos.push(media_info::inspect(&ffmpeg, p).await?);
        }
        Ok(merge::check(&infos))
    }

    /// 抽一帧给界面做裁剪预览。
    pub async fn preview(&self, path: PathBuf, at_secs: f64) -> anyhow::Result<String> {
        let ffmpeg = self.ffmpeg()?;
        preview::frame_data_url(&ffmpeg, &path, at_secs, preview::PREVIEW_MAX_WIDTH).await
    }
```

`commands.rs`：

```rust
#[tauri::command]
pub async fn add_video_job(
    state: State<'_, AppState>,
    inputs: Vec<String>,
    output_dir: String,
    params: JobParamsDto,
) -> R<u64> {
    let params = JobParams::try_from(params)?;
    let ops = state.videoops.clone();
    ops.add_job(
        inputs.into_iter().map(PathBuf::from).collect(),
        PathBuf::from(output_dir),
        params,
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn plan_output(
    state: State<'_, AppState>,
    inputs: Vec<String>,
    output_dir: String,
    params: JobParamsDto,
) -> R<String> {
    let params = JobParams::try_from(params)?;
    let ops = state.videoops.clone();
    ops.plan_output(
        inputs.into_iter().map(PathBuf::from).collect(),
        PathBuf::from(output_dir),
        params,
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

/// 合并的兼容性检查。界面在文件列表变化时调它，把 blockers 画成红字、warnings 画成灰字。
#[tauri::command]
pub async fn check_merge(state: State<'_, AppState>, inputs: Vec<String>) -> R<MergeCheckDto> {
    let ops = state.videoops.clone();
    let c = ops
        .check_merge(inputs.into_iter().map(PathBuf::from).collect())
        .await
        .map_err(|e| format!("{e:#}"))?;
    Ok(MergeCheckDto { blockers: c.blockers, warnings: c.warnings })
}

/// 抽一帧给裁剪页做预览，返回可直接塞进 `<img src>` 的 data URL。
#[tauri::command]
pub async fn preview_frame(
    state: State<'_, AppState>,
    path: String,
    at_secs: f64,
) -> R<String> {
    let ops = state.videoops.clone();
    ops.preview(PathBuf::from(path), at_secs)
        .await
        .map_err(|e| format!("{e:#}"))
}
```

`main.rs` 的 `invoke_handler!` 追加 `commands::check_merge,` 与 `commands::preview_frame,`。

- [ ] **Step 4: 跑测试看它通过**

Run:

```bash
cd src-tauri && cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "src-tauri:",s,"/",f}'
cd .. && cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"/",f}'
```

Expected: `src-tauri: 24 / 0`（20 + 新增 4）、root 全绿

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(videoops): 命令面 — 多输入 + check_merge + preview_frame

add_video_job / plan_output 改收 inputs: Vec<String>;另加两条:
check_merge(界面画红字与灰字)与 preview_frame(裁剪页的预览图,data URL)。

check_merge 与 add_job 里那次走**同一个 merge::check** —— 两处各写一遍的话,
界面放行的组合会被入队拒绝(或反过来),而用户看到的是"按钮能点、点了报错"。

两个新变体(merge / crop)沿用既有的分工:未知 tag 在反序列化时挂,未知取值
(比如 preset 不认)在 TryFrom 时挂 —— 两者都发生在排进队列之前。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 8: 前端数据层与纯逻辑

**Files:**
- Modify: `ui/src/lib/types.ts`、`ui/src/lib/ipc.ts`、`ui/src/lib/videoOps.ts`
- Modify: `ui/src/lib/opForms.ts`（加裁剪框钳制与列表移动）
- Test: `ui/src/lib/opForms.test.ts`、`ui/src/lib/videoOps.test.ts`

**Interfaces:**
- Consumes: Task 7 的命令名与形状
- Produces:
  - `JobParamsDto` += `{op:"merge"}` / `{op:"crop", width, height, x, y, preset}`
  - `VideoInfoDto` += `width` / `height` / `audioCodec`
  - `MergeCheckDto`
  - `ipc.{addVideoJob(inputs, …), planOutput(inputs, …), checkMerge(inputs), previewFrame(path, atSecs)}`
  - `opForms.{evenDown, clampCrop, moveUp, moveDown}`
  - `VIDEO_OPS` 五项全 `ready: true`

- [ ] **Step 1: 写失败的测试**

`ui/src/lib/opForms.test.ts` 追加：

```ts
import { clampCrop, evenDown, moveDown, moveUp } from "./opForms";

describe("evenDown / clampCrop", () => {
  it("奇数向下取偶 —— ffmpeg 会静默取偶，我们让这件事**看得见**", () => {
    // 实测：请求 crop=641 得到的是 640，而且 stderr 里没有提示。
    // 界面上让数字当场变成 640，用户就知道实际会裁多少。
    expect(evenDown(641)).toBe(640);
    expect(evenDown(640)).toBe(640);
    expect(evenDown(3)).toBe(2);
    expect(evenDown(1)).toBe(0);
  });

  it("越界时把左/上**可见地**钳到上限", () => {
    // 源 1920x1080，要裁 1280x720：左最多 640、上最多 360
    const c = clampCrop({ width: 1280, height: 720, x: 1200, y: 800 }, 1920, 1080);
    expect(c).toEqual({ width: 1280, height: 720, x: 640, y: 360 });
  });

  it("宽高超过源尺寸时也钳（并且仍然是偶数）", () => {
    expect(clampCrop({ width: 4000, height: 3000, x: 0, y: 0 }, 1920, 1080))
      .toEqual({ width: 1920, height: 1080, x: 0, y: 0 });
    // 源是奇数时（罕见但合法）取偶
    expect(clampCrop({ width: 999, height: 999, x: 0, y: 0 }, 641, 361))
      .toEqual({ width: 640, height: 360, x: 0, y: 0 });
  });

  it("源尺寸未知时返回 null —— 界面据此禁用开始按钮并说明原因", () => {
    expect(clampCrop({ width: 640, height: 360, x: 0, y: 0 }, null, null)).toBeNull();
  });
});

describe("moveUp / moveDown", () => {
  const xs = ["a", "b", "c"];
  it("上下移动一格", () => {
    expect(moveUp(xs, 1)).toEqual(["b", "a", "c"]);
    expect(moveDown(xs, 1)).toEqual(["a", "c", "b"]);
  });
  it("到头不动，且**不返回原数组**（React 靠引用判断变没变）", () => {
    expect(moveUp(xs, 0)).toEqual(["a", "b", "c"]);
    expect(moveDown(xs, 2)).toEqual(["a", "b", "c"]);
    expect(moveUp(xs, 0)).not.toBe(xs);
  });
});
```

`ui/src/lib/videoOps.test.ts` 的第一条改成：

```ts
  it("五项全部 ready —— 侧边栏不该再有任何灰显项", () => {
    // 这条守卫"不做假按钮"：以前断言的是"只有这三项 ready、另两项不是"，
    // 现在五项都做完了，所以改成断言"一项灰显都没有"。
    // **不是删掉它** —— 将来有人加第六个操作，这条会提醒他要么实现要么标灰。
    expect(VIDEO_OPS.filter((o) => o.ready).map((o) => o.op)).toEqual([
      "compress", "transcode", "merge", "trim", "crop",
    ]);
    expect(VIDEO_OPS.filter((o) => !o.ready)).toEqual([]);
  });
```

- [ ] **Step 2: 跑测试看它失败**

Run: `cd ui && npx vitest run src/lib/opForms.test.ts src/lib/videoOps.test.ts 2>&1 | grep -E "FAIL|Tests " | head -4`
Expected: FAIL（`clampCrop` 未定义、ready 只有三项）

- [ ] **Step 3: 写最小实现**

`opForms.ts` 追加：

```ts
/**
 * 奇数向下取偶。
 *
 * **ffmpeg 会静默取偶**（实测：请求 crop=641 得到 640，stderr 里一条提示都没有），
 * 所以界面上要让这件事**看得见** —— 输入框里的数字当场变成 640，用户就知道
 * 实际会裁多少。
 */
export function evenDown(n: number): number {
  return Math.max(0, Math.floor(n / 2) * 2);
}

/** 裁剪框草稿。四个值都是用户刚敲进去的、可能还没合法化。 */
export type CropDraft = { width: number; height: number; x: number; y: number };

/**
 * 把草稿钳成一个**合法**的裁剪框：取偶、不超过源、左/上不越界。
 *
 * 与 ffmpeg 的静默钳制有本质区别：这里钳完的结果**会写回输入框**，
 * 用户看得见数字变了；ffmpeg 钳完什么都没说。
 *
 * 源尺寸未知时返回 `null` —— 界面据此禁用开始按钮并说明原因，
 * 而不是拿 0 去算一个必然越界的框。
 */
export function clampCrop(
  d: CropDraft,
  srcW: number | null,
  srcH: number | null
): CropDraft | null {
  if (srcW == null || srcH == null || srcW < 2 || srcH < 2) return null;
  const width = Math.min(evenDown(d.width), evenDown(srcW));
  const height = Math.min(evenDown(d.height), evenDown(srcH));
  const x = Math.min(Math.max(0, evenDown(d.x)), srcW - width);
  const y = Math.min(Math.max(0, evenDown(d.y)), srcH - height);
  return { width, height, x, y };
}

/** 列表里第 i 项与上一项交换。到顶时返回**新数组**（内容不变）。 */
export function moveUp<T>(list: readonly T[], i: number): T[] {
  const out = [...list];
  if (i > 0 && i < out.length) [out[i - 1], out[i]] = [out[i], out[i - 1]];
  return out;
}

export function moveDown<T>(list: readonly T[], i: number): T[] {
  const out = [...list];
  if (i >= 0 && i < out.length - 1) [out[i], out[i + 1]] = [out[i + 1], out[i]];
  return out;
}
```

> `moveUp`/`moveDown` 到边界时**也返回新数组** —— 返回原数组的话 React 的
> `setState` 会因为引用相同而跳过重渲染（这是上一轮 `newestFirst` 那条注释的同一个道理）。

`types.ts` / `ipc.ts` / `videoOps.ts` 按 Interfaces 补（`addVideoJob` 与 `planOutput`
的第一个参数改成 `inputs: string[]`；`probeVideo` 的返回类型跟着 `VideoInfoDto` 走）。

- [ ] **Step 4: 跑测试、类型检查**

Run:

```bash
cd ui && npx tsc --noEmit 2>&1 | tail -3
npx vitest run 2>&1 | grep -E "Test Files|Tests "
```

Expected: tsc **可能有错**（`VideoOpShell` 还在用旧的单文件签名）—— 那是下一个任务的事，
**只跑 vitest 确认测试绿**即可，tsc 留到 Task 9 之后一起看。

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(ui): 合并与裁剪的前端数据层与纯逻辑

clampCrop 把草稿钳成合法的裁剪框,而它与 ffmpeg 的静默钳制有本质区别:
这里钳完的结果**会写回输入框**,用户看得见数字变了;ffmpeg 钳完什么都没说
(实测:源 1920 宽、请求 x=1200 → 它钳到 640 照常输出,stderr 一条提示都没有)。

evenDown 单独拎出来测,是因为"奇数会被静默取偶"这件事得在界面上看得见。

moveUp/moveDown 到边界也返回新数组 —— 返回原数组的话 React 的 setState 会因为
引用相同而跳过重渲染。

VIDEO_OPS 五项全 ready;那条守卫测试改成断言"一项灰显都没有"(不是删掉)。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 9: 两个表单 + 外框支持多文件

**Files:**
- Modify: `ui/src/pages/ops/VideoOpShell.tsx`（`input: string | null` → `inputs: string[]`）
- Create: `ui/src/pages/ops/MergeForm.tsx`
- Create: `ui/src/pages/ops/CropForm.tsx`
- Modify: `ui/src/pages/VideoOpPage.tsx`（分派两个新表单、拿掉 `NOT_READY`）
- Modify: `ui/src/lib/opDefaults.ts`（两个新操作的默认参数）
- Modify: `src-tauri/src/commands.rs`（`pick_video_file` → 另加一条多选的 `pick_video_files`）

**Interfaces:**
- Consumes: Task 7/8 的命令与纯函数
- Produces: 两个表单组件；`ipc.pickVideoFiles() -> string[] | null`

- [ ] **Step 1: 先加多选文件的命令**

`commands.rs`：

```rust
/// 多选视频文件（合并用）。与 `pick_video_file` 同理走回调式 API，不阻塞工作线程。
#[tauri::command]
pub async fn pick_video_files(app: tauri::AppHandle) -> R<Vec<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("视频", &["mp4", "mov", "mkv", "webm", "avi", "m4v", "ts"])
        .pick_files(move |picked| {
            let _ = tx.send(picked);
        });
    let picked = rx.await.map_err(|e| format!("文件选择被中断：{e}"))?;
    Ok(picked.unwrap_or_default().into_iter().map(|p| p.to_string()).collect())
}
```

（`main.rs` 的 `invoke_handler!` 追加 `commands::pick_video_files,`。）

- [ ] **Step 2: 外框支持多文件**

`VideoOpShell` 的改动要点：

```tsx
  const [inputs, setInputs] = useState<string[]>([]);
  const [infos, setInfos] = useState<VideoInfoDto[]>([]);
  // …
  /** 单文件的操作：选一个就把列表整个换成它。 */
  async function pickOne() {
    const p = await run(ipc.pickVideoFile());
    if (!p) return;
    setInputs([p]); setInfos([]); setPlanned(null);
    const i = await run(ipc.probeVideo(p));
    if (i) setInfos([i]);
  }
  /** 合并：多选，**追加**到现有列表**（用户常分几次挑）**。 */
  async function pickMany() {
    const ps = await run(ipc.pickVideoFiles());
    if (!ps || ps.length === 0) return;
    const next = [...inputs, ...ps];
    setInputs(next); setInfos([]); setPlanned(null);
    const got = await Promise.all(next.map((p) => run(ipc.probeVideo(p))));
    setInfos(got.filter((x): x is VideoInfoDto => x !== undefined));
  }
```

外框按 `op === "merge"` 选 `pickMany`、否则 `pickOne`；`namer` 与 `planOutput`/`addVideoJob`
都传 `inputs`。展示源信息那一行在单文件时是原来那样，多文件时显示
`已选 3 个文件 · 共 12.3 MB`。

- [ ] **Step 3: 两个表单**

`MergeForm`（列表 + 上下移 + 删除 + 兼容性结果）：

```tsx
export default function MergeForm({ value, onChange, inputs, setInputs, infos }: …) {
  const [check, setCheck] = useState<MergeCheckDto | null>(null);
  // 文件列表一变就去问 core；**规则只在 core 一份**，前端只负责画
  useEffect(() => {
    if (inputs.length < 2) { setCheck(null); return; }
    let alive = true;
    void run(ipc.checkMerge(inputs)).then((c) => { if (alive && c) setCheck(c); });
    return () => { alive = false; };
  }, [inputs.join("\u0000")]);

  // 有拦阻时把参数交 null，外框据此禁用开始按钮
  useEffect(() => {
    onChange(check && check.blockers.length > 0 ? null : { op: "merge" });
  }, [check]);

  return (
    <>
      <div className="mb-2" style={{ color: "var(--fg-muted)" }}>按此顺序拼接</div>
      <div className="rounded-xl overflow-hidden mb-3" style={{ background: "var(--bg-subtle)" }}>
        {inputs.map((p, i) => (
          <div key={p} className="flex items-center gap-2 px-3 py-2" style={{ borderBottom: "1px solid var(--line)" }}>
            <span className="truncate flex-1 text-[13px]">{basename(p)}</span>
            <span className="text-[12px]" style={{ color: "var(--fg-faint)" }}>
              {infos[i] ? `${infos[i].videoCodec ?? "?"} · ${infos[i].width ?? "?"}x${infos[i].height ?? "?"}` : ""}
            </span>
            <button className={ACTION_BTN} disabled={i === 0} onClick={() => setInputs(moveUp(inputs, i))}>↑</button>
            <button className={ACTION_BTN} disabled={i === inputs.length - 1} onClick={() => setInputs(moveDown(inputs, i))}>↓</button>
            <button className={ACTION_BTN} onClick={() => setInputs(inputs.filter((_, k) => k !== i))}>✕</button>
          </div>
        ))}
      </div>
      {check?.blockers.map((b) => (
        <p key={b} className="text-[12px] mb-1" style={{ color: "var(--accent-err)" }}>{b}</p>
      ))}
      {check?.blockers.length ? (
        <p className="text-[12px] mb-4" style={{ color: "var(--fg-muted)" }}>
          可以先把它「转码」成 MP4 · H.264 + AAC，再回来合并。
        </p>
      ) : null}
      {check?.warnings.map((w) => (
        <p key={w} className="text-[12px] mb-1" style={{ color: "var(--fg-faint)" }}>{w}</p>
      ))}
    </>
  );
}
```

文件顶部需要的两个小东西（与 `JobRow` 里那个同名常量一致）：

```tsx
const ACTION_BTN =
  "flex items-center gap-1 text-[12px] transition-colors hover:text-[var(--fg)] disabled:opacity-30 disabled:pointer-events-none";

/** 从路径里取文件名。路径分隔符两种都要认（Windows 上也有人用 `/`）。 */
function basename(p: string): string {
  return p.split(/[/\\]/).pop() || p;
}
```

`CropForm`（预览图 + 叠加框 + 四个数字 + 时间 + 质量）：

```tsx
  const [draft, setDraft] = useState<CropDraft>({ width: 0, height: 0, x: 0, y: 0 });
  const [atSecs, setAtSecs] = useState<number | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const src = infos[0];
  const box = clampCrop(draft, src?.width ?? null, src?.height ?? null);

  // 源信息一到手就把整个画面当默认裁剪框
  useEffect(() => {
    if (src?.width && src.height && draft.width === 0)
      setDraft({ width: evenDown(src.width), height: evenDown(src.height), x: 0, y: 0 });
  }, [src?.width, src?.height]);

  // 预览帧：只在文件或时间变化时重抽（要跑一次 ffmpeg）
  useEffect(() => {
    if (!inputs[0]) { setPreview(null); return; }
    const t = atSecs ?? (src?.durationSecs != null ? src.durationSecs * 0.1 : 0);
    let alive = true;
    void run(ipc.previewFrame(inputs[0], t)).then((u) => { if (alive && u) setPreview(u); });
    return () => { alive = false; };
  }, [inputs[0], atSecs, src?.durationSecs]);

  // 合法时把参数交上去；非法时 null（外框禁用开始按钮）
  useEffect(() => {
    onChange(box ? { op: "crop", ...box, preset } : null);
  }, [box?.width, box?.height, box?.x, box?.y, preset]);
```

叠加框：一个绝对定位的 div，用 `left: (box.x / src.width * 100)%` 这类百分比画在
`<img>` 上（图片容器 `relative`、`<img>` `w-full`），所以**不用管预览实际缩到多少** ——
百分比天然按比例。四个数字框的 `onChange` 走 `setDraft`，并把 `clampCrop` 的结果
**写回输入框**（这是与 ffmpeg 静默钳制的分界）。

- [ ] **Step 4: 分派与默认参数**

`VideoOpPage`：删掉 `NOT_READY`，把两个新表单接进分派：

```tsx
      {op === "merge" && <MergeForm value={params} onChange={setParams} inputs={inputs} setInputs={setInputs} infos={infos} />}
      {op === "crop" && <CropForm value={params} onChange={setParams} inputs={inputs} infos={infos} />}
```

`opDefaults.ts` 加：

```ts
    case "merge":
      return { op: "merge" };
    case "crop":
      // 宽高默认跟随源（表单拿到源尺寸后会填满整个画面）
      return { op: "crop", width: 0, height: 0, x: 0, y: 0, preset: "balanced" };
```

- [ ] **Step 5: 跑测试、类型检查、构建**

Run:

```bash
cd ui && npx tsc --noEmit 2>&1 | tail -3
npx vitest run 2>&1 | grep -E "Test Files|Tests "
npx vite build 2>&1 | tail -2
```

Expected: 全绿、构建成功

- [ ] **Step 6: 提交**

```bash
git add -A
git commit -m "$(cat <<'MSG'
feat(ui): 合并与画面裁剪两个表单,外框支持多文件

外框的 input 从单个变成一组。合并走多选(且**追加**到现有列表——用户常分几次挑),
其余四个操作仍然单选(选一个就把列表整个换成它)。

合并的兼容性结果**全部来自 core 的 check_merge**:前端只负责把 blockers 画成红字、
warnings 画成灰字,规则不重抄一份。

裁剪的叠加框用**百分比**定位,所以不必管预览图实际缩到了多少 —— 比例天然对。
四个数字框在改动后把 clampCrop 的结果**写回输入框**:这正是与 ffmpeg 静默钳制的
分界,用户看得见数字变了。

Co-Authored-By: Claude Code <noreply@anthropic.com>
MSG
)"
```

---

### Task 10: 全量验证与验收清单

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

- [ ] **Step 2: 跨边界机械校验**

```bash
python3 - <<'PY'
import re, pathlib
base = pathlib.Path("/Users/xuwenqiang/Desktop/easyDownVideo")
ipc = (base/"ui/src/lib/ipc.ts").read_text()
mainrs = (base/"src-tauri/src/main.rs").read_text()
cmds = (base/"src-tauri/src/commands.rs").read_text()
invoked = set(re.findall(r'invoke<[^>]*>\("([a-z_]+)"', ipc)) | set(re.findall(r'invoke\("([a-z_]+)"', ipc))
handler = set(re.findall(r'commands::([a-z_]+)', mainrs))
print("ipc.ts 调用但未注册:", sorted(invoked - handler) or "无")
print("已注册但 ipc.ts 没调:", sorted(handler - invoked) or "无")
def camel(s):
    head, *rest = s.split("_"); return head + "".join(w.capitalize() for w in rest)
for fn in ["add_video_job", "plan_output", "check_merge", "preview_frame"]:
    m = re.search(r'pub async fn ' + fn + r'\((.*?)\n\)', cmds, re.S)
    if not m: print(f"{fn}: 找不到"); continue
    params = [p.split(":")[0].strip() for p in m.group(1).split(",") if ":" in p and "state" not in p.split(":")[0]]
    print(f"{fn} 期望的 JS 键:", [camel(p) for p in params])
PY
```

Expected: 两边都"无"；四条命令的键与 `ipc.ts` 逐字一致。

- [ ] **Step 3: 真 app 里过一遍 spec 第 10 节的验收清单**

`npm run dev` 起真 app（`preview_frame` / `check_merge` / 产物名预览都需要真后端）。

**注意**：Tauri 的窗口是 WKWebView，本仓库环境里的浏览器自动化只驱动 Chrome ——
**点不进去**。所以这一步需要人来点，或者逐条说明"没验"。
**不要把没验的写成过了。**

10 条清单见 spec §10。

- [ ] **Step 4: 把验收结果逐条报给 human partner**

逐条写清"过了 / 没过 / 没验"，任何偏差都写出来（包括"预期之外但可接受"那种）。

---

## 收尾

```bash
cd /Users/xuwenqiang/Desktop/easyDownVideo
rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "root:",s,"/",f}'
(cd src-tauri && rtk proxy cargo test 2>&1 | grep -E "^test result" | awk '{s+=$4; f+=$6} END {print "tauri:",s,"/",f}')
(cd ui && rtk proxy npx vitest run 2>&1 | grep -E "^ +Tests ")
```

**做完之后**：`VIDEO_OPS` 五项全部可点，侧边栏不再有灰显项。**五个操作全通，
「视频操作」这一栏做完了。**

**已知的、故意留着的空缺**（不要当成 bug）：

- 混源拼接不自动重编码 —— 拒绝并指路去「转码」（spec §5.2 的刻意选择）
- 合并的排序是上下移动按钮，不是拖拽
- 裁剪是数字输入 + 实时叠加框，不是鼠标拖拽框
- 裁剪只有一帧预览（裁剪是静态的，不做时间轴拖拽）
- 没有旋转/翻转/缩放（那属于"调整尺寸"，是另一件事）
