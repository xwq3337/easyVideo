# EasyDownVideo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建 EasyDownVideo——Rust 跨平台桌面视频下载器（egui GUI + tokio 多线程下载引擎），支持直链 mp4/webm/mkv 多线程分块下载与 HLS(m3u8) 分片下载（AES-128 解密 + 内嵌 ffmpeg 无损转封装 mp4），可选 yt-dlp 网页解析。

**Architecture:** 单 crate 双层：GUI 层（app.rs，eframe）薄壳，每帧从 broadcast channel 拉取引擎事件；core 层（probe/direct/hls/ffmpeg/ytdlp/task/engine）全部纯逻辑不依赖 egui，TDD 开发。下载全程流式写盘（64KB buffer），并发用 tokio Semaphore 限制（默认 6 路/任务），进度通过原子计数器快照传递，内存占用与视频大小解耦。ffmpeg 静态二进制以 `include_bytes!` 按编译 target 内嵌，首次运行释放到应用数据目录。

**Tech Stack:** Rust 2021 edition · eframe 0.30 (egui) · tokio 1 (rt-multi-thread) + tokio-util (CancellationToken) · reqwest 0.12 (rustls-tls, stream) · m3u8-rs 0.10 · aes 0.8 + cbc 0.1 · serde/serde_json 1 · directories 5 · tracing 0.3 · url 2 · sha2 0.10 · rfd 0.15 (目录选择) · arboard 3 (剪贴板) · wiremock 0.6 + tempfile 3 (测试)

**Spec:** `docs/superpowers/specs/2026-10-14-easydownvideo-design.md`

## Global Constraints

- 禁止全量读入内存：core 中禁止 `response.bytes()` / `response.text()` 下载响应体；一律 `bytes_stream()` + 64KB buffer 流式写盘
- 并发默认值：单任务分块/分片并发 6（设置可调 1–16）；同时活跃任务数 2（可调）
- 分块最小 4MB（`MIN_BLOCK_SIZE = 4 * 1024 * 1024`）
- 输出文件名冲突时自动 `name (1).ext` 递增，不覆盖已有文件
- 下载目录默认 `~/Downloads/easydownvideo`；设置更改只影响之后新建的任务
- 所有 git commit 末尾追加：`Co-Authored-By: Claude Code <noreply@anthropic.com>`
- **提交时只暂存本任务改动的文件（`git add <路径...>`）。下方各步骤里写的 `git add -A` 是早期草稿，
  一律按本条覆盖。** 多个任务/修复轮可能同时在跑，`-A` 会把别的任务的半成品一起卷进本任务的提交，
  使提交与评审边界失效。
- m3u8 只支持 AES-128 与 fMP4 基础变体；遇到 SAMPLE-AES 等明确报错
- 首次编译 egui+reqwest 需 3–6 分钟属正常；迭代用 `cargo check`，跑测试用 `cargo test`

## Review Focus

1. **URL 无可用文件名**（路径以 `/` 结尾、纯 query、末尾无扩展名）→ 任务必须给出可用默认文件名（`video.mp4`），不得 panic 或空名 —— Task 4 probe 测试 + Task 13 引擎端到端覆盖
2. **协议相对 URL**（`//cdn.example.com/seg.ts`）与相对路径混用的 m3u8 → 分片地址解析必须全部正确 —— Task 5 解析测试
3. **服务器不支持 HEAD / 忽略 Range**（返回 200 全量）→ 探测与下载必须降级为单流，且探测不读完整个 body —— Task 4 probe 测试
4. **目标文件已存在** → 自动改名 `name (1).mp4`，绝不覆盖用户已有文件 —— Task 7 `unique_path` 测试 + Task 13 端到端
5. **多任务下一个失败** → 其余任务照常下载完成（队列不被阻塞）—— Task 13 引擎测试

---

### Task 1: 项目脚手架与依赖

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `src/main.rs`（临时最小窗口）
- Create: `src/app.rs`（空壳）
- Create: `src/settings.rs`（空壳）
- Create: `src/core/mod.rs`、`src/core/events.rs`、`src/core/probe.rs`、`src/core/direct.rs`、`src/core/ffmpeg.rs`、`src/core/ytdlp.rs`、`src/core/task.rs`、`src/core/engine.rs`（全部空壳 `pub` 模块）

**Interfaces:**
- Produces: crate 名 `easydownvideo`，全部后续任务在此结构上开发

- [ ] **Step 1: 初始化 Cargo 项目并写入依赖**

```bash
cd /Users/xuwenqiang/Desktop/easyDownVideo
cargo init --name easydownvideo
```

`Cargo.toml`（在 cargo init 生成基础上替换 `[dependencies]` 段）：

```toml
[package]
name = "easydownvideo"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
thiserror = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "fs", "io-util", "sync", "time"] }
tokio-util = "0.7"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "stream"] }
futures-util = "0.3"
m3u8-rs = "6"
aes = "0.8"
cbc = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
directories = "5"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
url = "2"
sha2 = "0.10"
eframe = "0.30"
rfd = "0.15"
arboard = "3"

[dev-dependencies]
wiremock = "0.6"
tempfile = "3"
```

`.gitignore`：

```
/target
.DS_Store
```

- [ ] **Step 2: 最小 main.rs + 模块空壳（先让编译通过）**

`src/main.rs`：

```rust
mod app;
mod core;
mod settings;

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "EasyDownVideo",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(app::App::default()))),
    )
}
```

`src/app.rs`：

```rust
use eframe::egui;

#[derive(Default)]
pub struct App;

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("EasyDownVideo");
            ui.label("scaffold");
        });
    }
}
```

`src/core/mod.rs`：

```rust
pub mod direct;
pub mod engine;
pub mod events;
pub mod ffmpeg;
pub mod hls;
pub mod probe;
pub mod task;
pub mod ytdlp;
```

并创建 `src/core/hls/mod.rs`、`src/core/hls/parse.rs`、`src/core/hls/crypto.rs`（均空文件）、以及其余模块文件（各含一行注释占位）。`src/settings.rs` 空文件。

- [ ] **Step 3: 编译验证**

Run: `cargo check`
Expected: 编译通过（无警告报错；首次拉依赖较慢）

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: 项目脚手架与依赖

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 2: settings 模块（配置加载/保存）

**Files:**
- Create: `src/lib.rs`
- Create: `tests/settings_test.rs`
- Modify: `src/settings.rs`（当前为空文件）
- Modify: `src/main.rs`（移除 `mod core;` 与 `mod settings;`）

**Interfaces:**
- Produces:
  - `pub struct Settings { pub download_dir: PathBuf, pub segment_concurrency: usize, pub max_active_tasks: usize, pub ytdlp_enabled: bool, pub ffmpeg_path_override: Option<PathBuf> }`
  - `impl Default for Settings`（download_dir = `~/Downloads/easydownvideo`，segment_concurrency=6，max_active_tasks=2，ytdlp_enabled=true，ffmpeg_path_override=None）
  - `Settings::load() -> Self` / `Settings::save(&self) -> anyhow::Result<()>`（走固定 config 目录）
  - `Settings::load_from(&Path) -> Option<Self>` / `Settings::save_to(&self, &Path) -> anyhow::Result<()>`（`pub`，供测试注入临时路径）
  - `Settings::config_dir() -> PathBuf`

- [ ] **Step 1: 写失败测试**

`tests/settings_test.rs`（这是最终版本，只写这一版）：

```rust
use easydownvideo::settings::Settings;
use std::path::Path;

#[test]
fn defaults_are_spec_values() {
    let s = Settings::default();
    assert_eq!(s.segment_concurrency, 6);
    assert_eq!(s.max_active_tasks, 2);
    assert!(s.ytdlp_enabled);
    assert!(s.ffmpeg_path_override.is_none());
    assert!(s.download_dir.ends_with("easydownvideo"));
}

#[test]
fn save_load_roundtrip_via_tempdir() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("settings.json");
    let mut s = Settings::default();
    s.segment_concurrency = 9;
    s.ytdlp_enabled = false;
    s.save_to(&cfg).unwrap();
    let loaded = Settings::load_from(&cfg).unwrap();
    assert_eq!(loaded.segment_concurrency, 9);
    assert!(!loaded.ytdlp_enabled);
}

#[test]
fn load_from_missing_file_returns_none() {
    assert!(Settings::load_from(Path::new("/nonexistent/settings.json")).is_none());
}

#[test]
fn load_from_corrupt_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("settings.json");
    std::fs::write(&cfg, "{ not valid json").unwrap();
    assert!(Settings::load_from(&cfg).is_none());
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test settings_test`
Expected: FAIL（`Settings` 未定义）

- [ ] **Step 3: 实现**

`src/settings.rs`：

```rust
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub segment_concurrency: usize,
    pub max_active_tasks: usize,
    pub ytdlp_enabled: bool,
    pub ffmpeg_path_override: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        let download_dir = directories::UserDirs::new()
            .map(|d| d.download_dir().map(|p| p.to_path_buf()))
            .flatten()
            .unwrap_or_else(|| std::env::temp_dir())
            .join("easydownvideo");
        Self {
            download_dir,
            segment_concurrency: 6,
            max_active_tasks: 2,
            ytdlp_enabled: true,
            ffmpeg_path_override: None,
        }
    }
}

impl Settings {
    pub fn config_dir() -> PathBuf {
        let dir = directories::ProjectDirs::from("dev", "easydownvideo", "easydownvideo")
            .expect("home directory must exist")
            .config_dir()
            .to_path_buf();
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    pub fn load() -> Self {
        let path = Self::config_dir().join("settings.json");
        Self::load_from(&path).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_dir().join("settings.json");
        self.save_to(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create config dir")?;
        }
        let text = serde_json::to_string_pretty(self).context("serialize settings")?;
        std::fs::write(path, text).context("write settings")
    }
}
```

`src/main.rs` 顶部模块声明已含 `mod settings;`，但 `main.rs` 的模块是 binary crate 私有；测试要访问需 `lib.rs`。创建 `src/lib.rs`：

```rust
pub mod core;
pub mod settings;
```

并把 `src/main.rs` 改为：

```rust
use easydownvideo::settings::Settings;

mod app; // 只有 app 属于 binary；core / settings 一律走 lib crate

fn main() -> eframe::Result<()> {
    let _settings = Settings::load();
    eframe::run_native(
        "EasyDownVideo",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(app::App::default()))),
    )
}
```

**关键：删除 `src/main.rs` 里的 `mod core;` 和 `mod settings;`。** 只保留 `mod app;`。

原因（务必理解，否则后续任务会踩坑）：`mod core;` 同时出现在 `lib.rs` 和 `main.rs` 里会编译出**两套互相独立的 core 模块树**。那样 `easydownvideo::core::events::TaskState`（来自 lib）与 `crate::core::events::TaskState`（来自 binary）是**两个不同的类型**，彼此不能赋值、不能匹配。Task 13/14 的 `app.rs` 用的是 `easydownvideo::core::...` 路径，一旦 binary 里还有自己的 `mod core;`，就会出现"expected `TaskState`, found `TaskState`"这种极难看懂的类型错误。

所以规则是：**core 与 settings 的唯一定义在 lib crate；binary 只声明 `mod app;`，app.rs 内一律用 `easydownvideo::core::...` / `easydownvideo::settings::...` 绝对路径。**

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test settings_test`
Expected: 4 PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: settings 配置加载/保存

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 3: events 模块（任务状态、引擎事件、进度句柄）

**Files:**
- Create: `tests/events_test.rs`
- Modify: `src/core/events.rs`
- Modify: `src/core/hls/parse.rs`（定义 `VariantChoice`）、`src/core/ytdlp.rs`（定义 `YtdlpFormat`）——仅结构体定义，后续任务填充实现

**Interfaces:**
- Produces:
  - `pub type TaskId = u64`
  - `pub enum TaskState { Queued, Probing, AwaitingChoice, Downloading, Merging, Paused, Done, Failed(String), Canceled }`
  - `pub enum EngineEvent { StateChanged { id: TaskId, state: TaskState }, Progress { id: TaskId, downloaded: u64, total: u64, unit: ProgressUnit }, ChoicesOffered { id: TaskId, kind: ChoiceKind } }`
  - `pub enum ChoiceKind { HlsVariants(Vec<VariantChoice>), YtdlpFormats(Vec<YtdlpFormat>) }`
  - `pub enum Choice { Variant(usize), YtdlpFormat(usize) }`
  - `pub struct ProgressHandle`：`new()`、`add(u64)`、`sub(u64)`、`set_total(u64)`、`set_unit(ProgressUnit)`、`unit() -> ProgressUnit`、`snapshot() -> (u64, u64)`、`reset()`
    - `sub` 是 Task 7 分块重试回滚所必需的方法（重试失败要撤销该次尝试已累加的字节）
  - `pub enum ProgressUnit { Bytes, Segments }`（HLS 任务按分片计数，GUI 据此决定格式化方式）
  - `pub struct VariantChoice { pub uri: String, pub bandwidth: u64, pub resolution: Option<(u64, u64)> }`
  - `pub struct YtdlpFormat { pub id: String, pub ext: String, pub note: String, pub filesize: Option<u64>, pub url: String }`

- [ ] **Step 1: 写失败测试**

`tests/events_test.rs`：

```rust
use easydownvideo::core::events::ProgressHandle;

#[test]
fn progress_handle_tracks_counts() {
    let p = ProgressHandle::new();
    assert_eq!(p.snapshot(), (0, 0));
    p.set_total(1000);
    p.add(250);
    p.add(250);
    assert_eq!(p.snapshot(), (500, 1000));
    p.reset();
    assert_eq!(p.snapshot(), (0, 1000));
}

#[test]
fn progress_handle_unit_defaults_to_bytes() {
    use easydownvideo::core::events::ProgressUnit;
    let p = ProgressHandle::new();
    assert_eq!(p.unit(), ProgressUnit::Bytes);
    p.set_unit(ProgressUnit::Segments);
    assert_eq!(p.unit(), ProgressUnit::Segments);
}

#[test]
fn sub_rolls_back_without_underflow() {
    // 分块重试回滚用：撤销一次失败尝试已计入的字节
    let p = ProgressHandle::new();
    p.set_total(1000);
    p.add(300);
    p.sub(300); // 一次失败尝试的回滚
    assert_eq!(p.snapshot(), (0, 1000));
    p.add(50);
    p.sub(999); // 防御：减去超过已计数的量，必须饱和到 0 而不是回绕成天文数字
    assert_eq!(p.snapshot(), (0, 1000));
}

#[test]
fn task_state_variants_exist() {
    use easydownvideo::core::events::TaskState;
    let states = [
        TaskState::Queued,
        TaskState::Probing,
        TaskState::AwaitingChoice,
        TaskState::Downloading,
        TaskState::Merging,
        TaskState::Paused,
        TaskState::Done,
        TaskState::Failed("x".into()),
        TaskState::Canceled,
    ];
    assert_eq!(states.len(), 9);
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test events_test`
Expected: FAIL（`events` 模块无内容）

- [ ] **Step 3: 实现**

`src/core/events.rs`：

```rust
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

use crate::core::hls::parse::VariantChoice;
use crate::core::ytdlp::YtdlpFormat;

pub type TaskId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Probing,
    AwaitingChoice,
    Downloading,
    Merging,
    Paused,
    Done,
    Failed(String),
    Canceled,
}

#[derive(Clone, Debug)]
pub enum EngineEvent {
    StateChanged { id: TaskId, state: TaskState },
    Progress { id: TaskId, downloaded: u64, total: u64, unit: ProgressUnit },
    ChoicesOffered { id: TaskId, kind: ChoiceKind },
}

#[derive(Clone, Debug)]
pub enum ChoiceKind {
    HlsVariants(Vec<VariantChoice>),
    YtdlpFormats(Vec<YtdlpFormat>),
}

#[derive(Clone, Copy, Debug)]
pub enum Choice {
    Variant(usize),
    YtdlpFormat(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProgressUnit {
    #[default]
    Bytes,
    Segments,
}

#[derive(Clone, Default)]
pub struct ProgressHandle {
    downloaded: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    unit: Arc<AtomicU8>,
}

impl ProgressHandle {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&self, n: u64) {
        self.downloaded.fetch_add(n, Ordering::Relaxed);
    }
    /// 回退已计入的字节数。分块重试失败时用来撤销该次尝试已累加的进度
    /// （否则重试会把同一段字节重复累加）。饱和减，绝不回绕成天文数字。
    pub fn sub(&self, n: u64) {
        let _ = self.downloaded.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |cur| Some(cur.saturating_sub(n)),
        );
    }
    pub fn set_total(&self, n: u64) {
        self.total.store(n, Ordering::Relaxed);
    }
    pub fn set_unit(&self, unit: ProgressUnit) {
        let v = match unit {
            ProgressUnit::Bytes => 0,
            ProgressUnit::Segments => 1,
        };
        self.unit.store(v, Ordering::Relaxed);
    }
    pub fn unit(&self) -> ProgressUnit {
        match self.unit.load(Ordering::Relaxed) {
            1 => ProgressUnit::Segments,
            _ => ProgressUnit::Bytes,
        }
    }
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.downloaded.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
    pub fn reset(&self) {
        self.downloaded.store(0, Ordering::Relaxed);
    }
}
```

`src/core/hls/parse.rs`：

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantChoice {
    pub uri: String,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
}
```

`src/core/ytdlp.rs`：

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YtdlpFormat {
    pub id: String,
    pub ext: String,
    pub note: String,
    pub filesize: Option<u64>,
    pub url: String,
}
```

`src/core/hls/mod.rs`：

```rust
pub mod crypto;
pub mod parse;
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test events_test`
Expected: 3 PASS（Task 7 会为 `sub()` 追加第 4 个测试，届时本文件为 4 PASS）

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 事件与进度类型定义

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 4: probe 模块（URL 嗅探）

**Files:**
- Create: `tests/probe_test.rs`
- Modify: `src/core/probe.rs`

**Interfaces:**
- Consumes: `reqwest::Client`（由调用方构造，带默认 headers/超时）
- Produces:
  - `pub enum ProbeOutcome { Direct { url: String, size: Option<u64>, accepts_ranges: bool, filename: String }, Hls { url: String }, WebPage }`
  - `pub async fn probe(client: &reqwest::Client, url: &str) -> anyhow::Result<ProbeOutcome>`
  - `pub fn filename_from_url(url: &str) -> String` —— **全项目唯一一份文件名推导逻辑**，Task 13 的 engine 与 Task 14 的 GUI 都复用它，不得各自再抄一份

**实现要点：** GET 带 `Range: bytes=0-63`；只读响应流第一帧（≤64 字节）判断 `#EXTM3U` 前缀，随后丢弃连接；size 取自 `content-range` 的 `/total` 或 `content-length`；`206` + `content-range` 视为支持 Range；`text/html` → WebPage；URL 无有效文件名时回退 `video.mp4`。

- [ ] **Step 1: 写失败测试**

`tests/probe_test.rs`：

```rust
use easydownvideo::core::probe::{probe, ProbeOutcome};
use wiremock::{matchers::{method, path}, Mock, MockServer, ResponseTemplate};

fn client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detects_direct_video_with_range() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/a/b/video.mp4"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/1048576")
                .set_body_bytes(vec![0u8; 64]),
        )
        .mount(&server)
        .await;
    let url = format!("{}/a/b/video.mp4", server.uri());
    match probe(&client(), &url).await.unwrap() {
        ProbeOutcome::Direct { size, accepts_ranges, filename, .. } => {
            assert_eq!(size, Some(1048576));
            assert!(accepts_ranges);
            assert_eq!(filename, "video.mp4");
        }
        other => panic!("expected Direct, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detects_hls_by_body_prefix() {
    let server = MockServer::start().await;
    let body = b"#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:4.0,\nseg0.ts\n";
    Mock::given(method("GET")).and(path("/index.m3u8"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/octet-stream")
                .set_body_bytes(body.to_vec()),
        )
        .mount(&server)
        .await;
    let url = format!("{}/index.m3u8", server.uri());
    match probe(&client(), &url).await.unwrap() {
        ProbeOutcome::Hls { .. } => {}
        other => panic!("expected Hls, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detects_webpage_by_content_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/page"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string("<html></html>"),
        )
        .mount(&server)
        .await;
    let url = format!("{}/page", server.uri());
    match probe(&client(), &url).await.unwrap() {
        ProbeOutcome::WebPage => {}
        other => panic!("expected WebPage, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ignores_ignored_range_and_falls_back() {
    // 服务器忽略 Range 返回 200 全量：仍应判为 Direct，不报错
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/file.mkv"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "video/x-matroska")
                .insert_header("content-length", "2048")
                .set_body_bytes(vec![1u8; 2048]),
        )
        .mount(&server)
        .await;
    let url = format!("{}/file.mkv", server.uri());
    match probe(&client(), &url).await.unwrap() {
        ProbeOutcome::Direct { size, accepts_ranges, .. } => {
            assert_eq!(size, Some(2048));
            assert!(!accepts_ranges);
        }
        other => panic!("expected Direct, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn url_without_filename_uses_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/999")
                .set_body_bytes(vec![0u8; 64]),
        )
        .mount(&server)
        .await;
    let outcome = probe(&client(), &server.uri()).await.unwrap();
    match outcome {
        ProbeOutcome::Direct { filename, .. } => assert_eq!(filename, "video.mp4"),
        other => panic!("expected Direct, got {:?}", other),
    }
}
```

再补一个纯函数测试，覆盖文件名推导的边界（这是全项目唯一一份该逻辑，engine 与 GUI 都复用它）：

```rust
#[test]
fn filename_from_url_decodes_and_falls_back() {
    use easydownvideo::core::probe::filename_from_url;
    // 百分号解码：URL 里带空格的文件名很常见
    assert_eq!(filename_from_url("https://host/a/My%20Video.mp4"), "My Video.mp4");
    // 查询串不参与文件名
    assert_eq!(filename_from_url("https://host/a/video.mp4?token=abc"), "video.mp4");
    // 无可用文件名 → 回退
    assert_eq!(filename_from_url("https://host/download?id=123"), "video.mp4");
    assert_eq!(filename_from_url("https://host/video"), "video.mp4");
    assert_eq!(filename_from_url("https://host/"), "video.mp4");
    assert_eq!(filename_from_url("not a url"), "video.mp4");
    // 非法转义原样保留，不 panic
    assert_eq!(filename_from_url("https://h/a%ZZb.mp4"), "a%ZZb.mp4");
    // 大写扩展名保留原样
    assert_eq!(filename_from_url("https://h/VIDEO.MP4"), "VIDEO.MP4");
    // 末尾截断的转义符也不 panic
    assert_eq!(filename_from_url("https://h/x%"), "video.mp4");
    assert_eq!(filename_from_url("https://h/x%2"), "video.mp4");
}

#[test]
fn filename_from_url_never_escapes_the_download_dir() {
    use easydownvideo::core::probe::filename_from_url;
    // 百分号解码会把 %2F 变成 '/'、%5C 变成 '\'。返回值绝不允许含路径分隔符，
    // 否则下游 dir.join(filename) 会让写入逃出下载目录。
    for url in [
        "https://h/a%2Fb.mp4",
        "https://h/a%5Cb.mp4",
        "https://h/%2e%2e%2fsecret.mp4",
        "https://h/..%2Fsecret.mp4",
        "https://h/%2e%2e",
        "https://h/..",
        "https://h/.",
    ] {
        let name = filename_from_url(url);
        assert!(
            !name.contains('/') && !name.contains('\\'),
            "{url} 产出含分隔符的文件名: {name:?}"
        );
        assert_ne!(name, "..", "{url} 产出父目录引用");
        assert_ne!(name, ".", "{url} 产出当前目录引用");
        assert!(!name.is_empty(), "{url} 产出空文件名");
    }
    // 具体期望：取最后一个路径分量，保留可用后缀
    assert_eq!(filename_from_url("https://h/a%2Fb.mp4"), "b.mp4");
    assert_eq!(filename_from_url("https://h/%2e%2e%2fsecret.mp4"), "secret.mp4");
    // 纯 `..` 无扩展名，回退默认名
    assert_eq!(filename_from_url("https://h/%2e%2e"), "video.mp4");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test probe_test`
Expected: FAIL（`probe` 未实现）

- [ ] **Step 3: 实现**

`src/core/probe.rs`：

```rust
use anyhow::Context;
use futures_util::StreamExt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    Direct {
        url: String,
        size: Option<u64>,
        accepts_ranges: bool,
        filename: String,
    },
    Hls {
        url: String,
    },
    WebPage,
}

/// 从 URL 推导保存用的文件名。**这是全项目唯一一处该逻辑**（engine 与 GUI 都调用它，
/// 不要各自再抄一份——重复的逻辑块会被审查判为重要缺陷）。
///
/// 行为：取路径最后一段，做百分号解码（`My%20Video.mp4` → `My Video.mp4`），
/// 再取解码结果中最后一个路径分量并校验。无可用段（以 `/` 结尾、无点、裸主机、
/// 段为 `.`/`..`）时回退 `video.mp4`。**返回值保证不含路径分隔符、不为空、不含 `.`/`..`。**
pub fn filename_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .as_ref()
        .and_then(|u| u.path_segments())
        .and_then(|mut segs| segs.next_back())
        .and_then(sanitize_segment)
        .filter(|s| s.contains('.'))
        .unwrap_or_else(|| "video.mp4".to_string())
}

/// 把 URL 的一个路径段变成安全的文件名。返回 `None` 表示该段不可用，调用方回退默认名。
///
/// **安全要点**：百分号解码会把 `%2F` 变成 `/`、`%5C` 变成 `\`。若原样返回，
/// `https://host/%2e%2e%2fsecret.mp4` 就会得到 `../secret.mp4`，而下游用
/// `dir.join(filename)` 落盘——写入会逃出下载目录。所以解码后只取最后一个路径分量，
/// 并显式拒绝 `.` 与 `..`。
fn sanitize_segment(seg: &str) -> Option<String> {
    let decoded = percent_decode(seg);
    let last = decoded.rsplit(['/', '\\']).next().unwrap_or("");
    let trimmed = last.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }
    Some(trimmed.to_string())
}

/// 极简百分号解码（不引入额外依赖）：`%XX` → 字节，其余原样，最后按 UTF-8 宽松解码。
/// 非法转义（如 `%ZZ`、截断的 `%A`）原样保留，不报错。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub async fn probe(client: &reqwest::Client, url: &str) -> anyhow::Result<ProbeOutcome> {
    let resp = client
        .get(url)
        .header("Range", "bytes=0-63")
        .send()
        .await
        .context("probe request failed")?;

    let status = resp.status();
    // 先克隆 headers：entry 之后 resp 会被 move 进 bytes_stream()
    let headers = resp.headers().clone();
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    // 读取首帧（≤64 字节）用于 #EXTM3U 嗅探，随后立即丢弃响应体与连接
    let mut first_frame = Vec::new();
    let mut stream = resp.bytes_stream();
    if let Some(chunk) = stream.next().await {
        let chunk = chunk.context("probe body read failed")?;
        first_frame = chunk.iter().take(64).copied().collect();
    }
    drop(stream);

    if first_frame.starts_with(b"#EXTM3U")
        || content_type.contains("mpegurl")
        || url.split('?').next().unwrap_or("").to_lowercase().ends_with(".m3u8")
    {
        return Ok(ProbeOutcome::Hls { url: url.to_string() });
    }

    let is_video = content_type.starts_with("video/")
        || content_type.contains("octet-stream")
        || ["mp4", "webm", "mkv", "mov", "ts"]
            .iter()
            .any(|ext| {
                url.split('?').next().unwrap_or("").to_lowercase().ends_with(&format!(".{ext}"))
            });
    if !is_video || content_type.contains("text/html") {
        return Ok(ProbeOutcome::WebPage);
    }

    let accepts_ranges = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let size = if accepts_ranges {
        parse_total_from_content_range(&headers)
    } else {
        headers
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    };

    Ok(ProbeOutcome::Direct {
        url: url.to_string(),
        size,
        accepts_ranges,
        filename: filename_from_url(url),
    })
}

fn parse_total_from_content_range(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let val = headers.get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    // "bytes 0-63/1048576" 或 "bytes 0-63/*"
    val.split('/').next_back()?.parse::<u64>().ok()
}
```

注意：上述实现中 `resp` 被部分消费（bytes_stream 拿走 ownership）——若流被消费则后续 header 读取需先取 headers 快照。实现时先 `let headers = resp.headers().clone();` 再分流。执行时按此修正（编译器会提示 move 错误）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test probe_test`
Expected: 7 PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: URL 探测（直链/HLS/网页）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 5: hls::parse 模块（m3u8 解析与 URL 解析）

**Files:**
- Create: `tests/fixtures/master.m3u8`、`tests/fixtures/media_aes.m3u8`、`tests/fixtures/media_fmp4.m3u8`
- Create: `tests/hls_parse_test.rs`
- Modify: `src/core/hls/parse.rs`

**Interfaces:**
- Produces:
  - （Task 3 已建 `VariantChoice`，本任务添加其余类型）
  - `pub struct KeyRef { pub uri: String, pub iv: Option<[u8; 16]> }`
  - `pub struct SegmentRef { pub uri: String, pub key: Option<KeyRef>, pub is_init: bool }`
  - `pub struct MediaInfo { pub segments: Vec<SegmentRef>, pub media_sequence: u64, pub is_fmp4: bool }`
  - `pub enum ParsedPlaylist { Master { variants: Vec<VariantChoice> }, Media(MediaInfo) }`
  - `pub fn parse_playlist(base_url: &str, content: &[u8]) -> Result<ParsedPlaylist, HlsParseError>`
  - `pub fn resolve_url(base: &str, relative: &str) -> Option<String>`
  - `pub enum HlsParseError { UnsupportedKeyMethod(String), Parse(String) }`（thiserror）

- [ ] **Step 1: 准备 fixtures**

`tests/fixtures/master.m3u8`：

```
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360
low/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720
high/index.m3u8
```

`tests/fixtures/media_aes.m3u8`：

```
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXT-X-MEDIA-SEQUENCE:7
#EXT-X-KEY:METHOD=AES-128,URI="https://keys.example.com/k.bin",IV=0x00000000000000000000000000000005
#EXTINF:9.976,
seg1.ts
#EXTINF:9.976,
seg2.ts
#EXT-X-ENDLIST
```

`tests/fixtures/media_fmp4.m3u8`：

```
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
s1.m4s
#EXTINF:6.0,
s2.m4s
#EXT-X-ENDLIST
```

- [ ] **Step 2: 写失败测试**

`tests/hls_parse_test.rs`：

```rust
use easydownvideo::core::hls::parse::{
    parse_playlist, patch_iv_less_method_none, resolve_url, ParsedPlaylist,
};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)).unwrap()
}

#[test]
fn parses_master_variants() {
    let p = parse_playlist("https://cdn.example.com/live/master.m3u8", fixture("master.m3u8").as_bytes()).unwrap();
    match p {
        ParsedPlaylist::Master { variants } => {
            assert_eq!(variants.len(), 2);
            assert_eq!(variants[0].resolution, Some((640, 360)));
            assert_eq!(variants[1].bandwidth, 2800000);
            assert_eq!(variants[1].uri, "https://cdn.example.com/live/high/index.m3u8");
        }
        _ => panic!("expected Master"),
    }
}

#[test]
fn parses_media_with_aes_key() {
    let p = parse_playlist("https://cdn.example.com/a/media.m3u8", fixture("media_aes.m3u8").as_bytes()).unwrap();
    match p {
        ParsedPlaylist::Media(m) => {
            assert_eq!(m.media_sequence, 7);
            assert!(!m.is_fmp4);
            assert_eq!(m.segments.len(), 2);
            let key = m.segments[0].key.as_ref().unwrap();
            assert_eq!(key.uri, "https://keys.example.com/k.bin");
            let mut expected_iv = [0u8; 16];
            expected_iv[15] = 5; // IV=0x...0005
            assert_eq!(key.iv, Some(expected_iv));
            assert!(!m.segments[0].is_init);
            // 关键：同一个 KEY 必须对**后面所有**分片继续生效。
            // m3u8-rs 只把 key 挂在紧随其后的那一个分片上，这里断了就等于后面全不解密。
            assert!(
                m.segments[1].key.is_some(),
                "第二个分片必须继承同一个 KEY（RFC 8216 §4.3.2.4）"
            );
            assert_eq!(m.segments[1].key.as_ref().unwrap().uri, "https://keys.example.com/k.bin");
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn key_applies_to_every_following_segment() {
    // 实测 m3u8-rs 6.0.1 只给第一个分片带 key：一个 KEY + 3 个分片时只有 seg[0] 有 key。
    // 我们要按 RFC 补齐：KEY 对其后所有分片生效，直到下一个 KEY 标签。
    let content = b"#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=0x00000000000000000000000000000009\n\
#EXTINF:9.9,\na.ts\n#EXTINF:9.9,\nb.ts\n#EXTINF:9.9,\nc.ts\n#EXT-X-ENDLIST\n";
    match parse_playlist("https://c/m.m3u8", content).unwrap() {
        ParsedPlaylist::Media(m) => {
            assert_eq!(m.segments.len(), 3);
            for (i, seg) in m.segments.iter().enumerate() {
                let k = seg.key.as_ref().unwrap_or_else(|| {
                    panic!("分片 {i} 丢了 key —— 真实播放列表里这些分片将不被解密")
                });
                assert_eq!(k.uri, "https://k/x", "分片 {i} 的 key 应沿用同一个");
            }
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn new_key_replaces_previous_for_later_segments() {
    let content = b"#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/1.bin\"\n#EXTINF:9.9,\na.ts\n#EXTINF:9.9,\nb.ts\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/2.bin\"\n#EXTINF:9.9,\nc.ts\n#EXT-X-ENDLIST\n";
    match parse_playlist("https://c/m.m3u8", content).unwrap() {
        ParsedPlaylist::Media(m) => {
            let uri = |i: usize| m.segments[i].key.as_ref().map(|k| k.uri.clone());
            assert_eq!(uri(0).as_deref(), Some("https://k/1.bin"));
            assert_eq!(uri(1).as_deref(), Some("https://k/1.bin"), "b 应沿用第一个 key");
            assert_eq!(uri(2).as_deref(), Some("https://k/2.bin"), "新 KEY 应替换旧的");
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn method_none_clears_the_key_for_later_segments() {
    // METHOD=NONE 表示"自此不再加密"，不能被当成"沿用上一个 key"。
    // 这条 playlist 的 NONE 行不带 IV —— 实测 m3u8-rs 会把不带 IV 的 METHOD=NONE
    // 整行丢弃（其守卫写反），所以才需要 patch_iv_less_method_none 先补占位 IV。
    let content = b"#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\"\n#EXTINF:9.9,\na.ts\n\
#EXT-X-KEY:METHOD=NONE\n#EXTINF:9.9,\nb.ts\n#EXT-X-ENDLIST\n";
    match parse_playlist("https://c/m.m3u8", content).unwrap() {
        ParsedPlaylist::Media(m) => {
            assert!(m.segments[0].key.is_some(), "第一个分片应加密");
            assert!(m.segments[1].key.is_none(), "METHOD=NONE 之后不应再解密");
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn patch_leaves_normal_encrypted_playlists_untouched() {
    // 改写只允许命中"含 METHOD=NONE 且不含 IV=" 的 KEY 行。
    // 真实加密播放列表的 KEY 行不该被碰：给 AES-128 行补个 IV 会误导解密。
    let content = b"#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=0x00000000000000000000000000000007\n\
#EXTINF:9.9,\na.ts\n#EXT-X-ENDLIST\n";
    let patched = String::from_utf8(patch_iv_less_method_none(content)).unwrap();
    assert_eq!(patched, String::from_utf8_lossy(content), "正常的 AES KEY 行不应被改写");

    // 不带 IV 的 AES KEY 行同样不该被改写（它本来就能被 m3u8-rs 正常解析）
    let aes_no_iv = b"#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\"\n#EXTINF:9.9,\na.ts\n";
    let p2 = String::from_utf8(patch_iv_less_method_none(aes_no_iv)).unwrap();
    assert_eq!(p2, String::from_utf8_lossy(aes_no_iv), "AES 缺 IV 的行不应被加 IV");

    // 带 IV 的 METHOD=NONE 已是合法行，不该被重复追加
    let none_with_iv =
        b"#EXTM3U\n#EXT-X-KEY:METHOD=NONE,IV=0x00000000000000000000000000000000\n#EXTINF:1,\na.ts\n";
    let p3 = String::from_utf8(patch_iv_less_method_none(none_with_iv)).unwrap();
    assert_eq!(p3, String::from_utf8_lossy(none_with_iv));

    // 保留末行无换行符的形态
    let no_trailing_nl = b"#EXTM3U\n#EXT-X-KEY:METHOD=NONE\nseg.ts";
    let p4 = String::from_utf8(patch_iv_less_method_none(no_trailing_nl)).unwrap();
    assert_eq!(p4, "#EXTM3U\n#EXT-X-KEY:METHOD=NONE,IV=0x00000000000000000000000000000000\nseg.ts");
}

#[test]
fn parses_fmp4_with_init_segment() {
    let p = parse_playlist("https://cdn.example.com/f/", fixture("media_fmp4.m3u8").as_bytes()).unwrap();
    match p {
        ParsedPlaylist::Media(m) => {
            assert!(m.is_fmp4);
            assert_eq!(m.segments.len(), 3); // init + 2 fragments
            assert!(m.segments[0].is_init);
            assert_eq!(m.segments[0].uri, "https://cdn.example.com/f/init.mp4");
            assert_eq!(m.segments[2].uri, "https://cdn.example.com/f/s2.m4s");
        }
        _ => panic!("expected Media"),
    }
}

#[test]
fn resolve_url_handles_relative_and_protocol_relative() {
    let base = "https://cdn.example.com/a/b/index.m3u8";
    assert_eq!(resolve_url(base, "seg.ts").as_deref(), Some("https://cdn.example.com/a/b/seg.ts"));
    assert_eq!(resolve_url(base, "../seg.ts").as_deref(), Some("https://cdn.example.com/a/seg.ts"));
    assert_eq!(resolve_url(base, "//other.com/seg.ts").as_deref(), Some("https://other.com/seg.ts"));
    assert_eq!(resolve_url(base, "https://x.com/y.ts").as_deref(), Some("https://x.com/y.ts"));
}

#[test]
fn rejects_sample_aes_explicitly() {
    let content = b"#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://k/x\"\n#EXTINF:1,\na.ts\n";
    let err = parse_playlist("https://e.com/m.m3u8", content).unwrap_err();
    assert!(err.to_string().contains("SAMPLE-AES"));
}

#[test]
fn malformed_iv_errors_instead_of_panicking() {
    // 32 字节但含多字节 UTF-8：按字节切分不会 panic，必须返回 Err
    let mut content = String::from("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=");
    content.push_str(&"€".repeat(10)); // 3 字节 × 10 = 30 字节，再加 2 字节
    content.push_str("xy"); // 共 32 字节
    content.push_str("\n#EXTINF:1,\na.ts\n");
    let err = parse_playlist("https://e.com/m.m3u8", content.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("bad IV"), "实际错误: {err}");

    // 长度不对
    let short = b"#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=0x1234\n#EXTINF:1,\na.ts\n";
    assert!(parse_playlist("https://e.com/m.m3u8", short).is_err());
}

#[test]
fn encrypted_fmp4_init_segment_inherits_key() {
    // fMP4 + AES-128：EXT-X-MAP 的初始化段本身也被加密，必须挂上 key 且要求有 IV。
    // KEY 写在 MAP 之前（规范里最常见的顺序）。
    let with_iv = b"#EXTM3U\n#EXT-X-VERSION:7\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\",IV=0x00000000000000000000000000000001\n\
#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.0,\ns0.m4s\n#EXT-X-ENDLIST\n";
    match parse_playlist("https://c/f/i.m3u8", with_iv).unwrap() {
        ParsedPlaylist::Media(m) => {
            assert!(m.is_fmp4);
            let init = &m.segments[0];
            assert!(init.is_init);
            let k = init.key.as_ref().expect("init 段必须继承 key");
            assert_eq!(k.uri, "https://k/x");
            let mut expect_iv = [0u8; 16];
            expect_iv[15] = 1;
            assert_eq!(k.iv, Some(expect_iv));
        }
        _ => panic!("expected Media"),
    }

    // 缺 IV：按 RFC 8216 §4.3.2.5 必须明确报错，绝不能按媒体序号推导 IV 解出错的内容
    let no_iv = b"#EXTM3U\n#EXT-X-VERSION:7\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/x\"\n\
#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.0,\ns0.m4s\n#EXT-X-ENDLIST\n";
    let err = parse_playlist("https://c/f/i.m3u8", no_iv).unwrap_err();
    assert!(err.to_string().contains("IV"), "实际错误: {err}");
}

#[test]
fn rejects_unknown_key_method() {
    let content = b"#EXTM3U\n#EXT-X-KEY:METHOD=AES-256,URI=\"https://k/x\"\n#EXTINF:1,\na.ts\n";
    let err = parse_playlist("https://e.com/m.m3u8", content).unwrap_err();
    assert!(err.to_string().contains("AES-256"), "实际错误: {err}");
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test --test hls_parse_test`
Expected: FAIL（函数未实现）

- [ ] **Step 4: 实现**

`src/core/hls/parse.rs`：

```rust
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantChoice {
    pub uri: String,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRef {
    pub uri: String,
    pub iv: Option<[u8; 16]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentRef {
    pub uri: String,
    pub key: Option<KeyRef>,
    pub is_init: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaInfo {
    pub segments: Vec<SegmentRef>,
    pub media_sequence: u64,
    pub is_fmp4: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedPlaylist {
    Master { variants: Vec<VariantChoice> },
    Media(MediaInfo),
}

#[derive(Error, Debug)]
pub enum HlsParseError {
    #[error("unsupported key method: {0}")]
    UnsupportedKeyMethod(String),
    #[error("parse error: {0}")]
    Parse(String),
}

pub fn resolve_url(base: &str, relative: &str) -> Option<String> {
    let base_url = url::Url::parse(base).ok()?;
    base_url.join(relative).ok().map(|u| u.to_string())
}

/// 给不带 IV 的 `#EXT-X-KEY:METHOD=NONE` 补一个占位 IV，仅用于越过 m3u8-rs 的一处缺陷。
///
/// **实测确认（m3u8-rs 6.0.1）**：`Key::from_hashmap`（playlist.rs:979）的守卫写反了——
/// 它在 `method == None && iv.is_none()` 时报错，而错误信息自述的语义是
/// "IV is required unless METHOD is NONE"。结果是不带 IV 的 `METHOD=NONE` 整行被丢弃：
/// ```text
/// #EXT-X-KEY:METHOD=NONE            ->  seg.key = None, 且标签落入 seg.unknown_tags
/// #EXT-X-KEY:METHOD=NONE,IV=0x00..  ->  seg.key = Some(Key{method: None})  成功
/// ```
/// 注意这个围栏必须标注语言（`text`）：裸 ``` 会被 rustdoc 当成 Rust doctest 去编译，
/// 上面这些内容不是合法 Rust，`cargo test` 会连带 doctest 一起失败。
///
/// 被丢弃后我们既不知道"这里曾经有过一个 NONE"、也不知道它出现在哪个分片之前，于是
/// `METHOD=NONE` 之后的分片会被错误地继续解密——又是一次静默产出乱码。
///
/// 之所以敢直接改文本：占位 IV 对我们是**可证无用**的——`parse_key` 对
/// `KeyMethod::None` 立刻返回 `None`，从不读 iv；URI 等其余属性也不受影响。
/// 且该改写只命中"含 METHOD=NONE 且不含 IV=" 的 `#EXT-X-KEY:` 行：真实加密播放列表的
/// KEY 行是 `METHOD=AES-128,...,IV=0x..`，一次都不会被碰到（空转）。
///
/// 设为 `pub` 仅为让集成测试能直接验证"改写范围足够窄"这一性质。
///
/// 按**字节**处理而非 `str`：这样对 LF / CRLF / 无末尾换行 / 非 UTF-8 都能逐字节保真，
/// 不引入任何编码假设。CR 单独摘出再补回，保证占位 IV 插在 CR 之前（否则 IV 会被 CR
/// 截成新的一行）。
pub fn patch_iv_less_method_none(content: &[u8]) -> Vec<u8> {
    const PLACEHOLDER_IV: &[u8] = b",IV=0x00000000000000000000000000000000";
    let mut out: Vec<u8> = Vec::with_capacity(content.len());
    for (i, raw_line) in content.split(|&b| b == b'\n').enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        let (line, had_cr) = match raw_line.strip_suffix(b"\r") {
            Some(l) => (l, true),
            None => (raw_line, false),
        };
        out.extend_from_slice(line);
        let is_iv_less_none_key = line.starts_with(b"#EXT-X-KEY:")
            && contains_subslice(line, b"METHOD=NONE")
            && !contains_subslice(line, b"IV=");
        if is_iv_less_none_key {
            out.extend_from_slice(PLACEHOLDER_IV);
        }
        if had_cr {
            out.push(b'\r');
        }
    }
    out
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

pub fn parse_playlist(base_url: &str, content: &[u8]) -> Result<ParsedPlaylist, HlsParseError> {
    let normalized = patch_iv_less_method_none(content);
    let playlist = m3u8_rs::parse_playlist_res(&normalized)
        .map_err(|e| HlsParseError::Parse(format!("{e:?}")))?;
    match playlist {
        m3u8_rs::Playlist::MasterPlaylist(mp) => {
            let variants = mp
                .variants
                .into_iter()
                .map(|v| VariantChoice {
                    uri: resolve_url(base_url, &v.uri).unwrap_or(v.uri),
                    bandwidth: v.bandwidth,
                    resolution: v.resolution.map(|r| (r.width, r.height)),
                })
                .collect();
            Ok(ParsedPlaylist::Master { variants })
        }
        m3u8_rs::Playlist::MediaPlaylist(mp) => {
            let media_sequence = mp.media_sequence;
            let is_fmp4 = mp.segments.iter().any(|s| s.map.is_some());

            // 按 RFC 8216 §4.3.2.4 实现 EXT-X-KEY 的作用域：一个 KEY 标签对其后**所有**
            // 分片生效，直到出现下一个 KEY 标签。
            //
            // 这件事必须自己做，不能直接用 m3u8-rs 给出的 `segment.key`。**实测确认**：
            // m3u8-rs 6.0.1 只把 key 挂在紧随其后的那一个分片上（它的 parser 在每个 media
            // URI 之后会把 encryption_key 重置为 None）：
            //     一个 KEY + 3 个分片 → 只有 seg[0] 带 key，seg[1]/seg[2] 都是 None
            // 若不在这里补齐，真实播放列表（一个 KEY + 几百个分片）里除第一个分片外全部
            // 不会被解密——下载"成功"，产出的 mp4 却从约第 10 秒起全是乱码，且不报任何错误。
            let mut effective: Vec<Option<KeyRef>> = Vec::with_capacity(mp.segments.len());
            let mut current: Option<KeyRef> = None;
            for seg in mp.segments.iter() {
                if let Some(k) = seg.key.clone() {
                    // 显式设置：METHOD=NONE 经 parse_key 得到 None，表示"自此不再加密"
                    current = parse_key(base_url, k)?;
                }
                // seg.key 为 None 表示沿用上一个 KEY（而非"无加密"）
                effective.push(current.clone());
            }

            let mut segments: Vec<SegmentRef> = Vec::new();

            // init 段（EXT-X-MAP）：AES-128 + fMP4 下初始化段本身也被加密，必须挂上生效的
            // key，否则下载端跳过解密、静默产出损坏文件。RFC 8216 §4.3.2.5 规定加密的初始
            // 化段要求 EXT-X-KEY 带 IV——缺 IV 明确报错，因为按媒体序号推导 IV 对 init 段是错的。
            if let Some(map) = mp.segments.iter().find_map(|s| s.map.clone()) {
                let inherited = effective.iter().find_map(|k| k.clone());
                if inherited.as_ref().is_some_and(|kr| kr.iv.is_none()) {
                    return Err(HlsParseError::Parse(
                        "AES-128 加密的 fMP4 初始化段缺少 IV 属性（RFC 8216 §4.3.2.5）".into(),
                    ));
                }
                segments.push(SegmentRef {
                    uri: resolve_url(base_url, &map.uri).unwrap_or(map.uri),
                    key: inherited,
                    is_init: true,
                });
            }

            for (seg, key) in mp.segments.into_iter().zip(effective) {
                segments.push(SegmentRef {
                    uri: resolve_url(base_url, &seg.uri).unwrap_or(seg.uri),
                    key,
                    is_init: false,
                });
            }
            Ok(ParsedPlaylist::Media(MediaInfo {
                segments,
                media_sequence,
                is_fmp4,
            }))
        }
    }
}

/// 把 m3u8-rs 的 `Key` 转成本项目的 `KeyRef`。
/// 不支持的方法（SAMPLE-AES、未知方法）一律返回错误——绝不静默降级为"无加密"。
fn parse_key(base_url: &str, k: m3u8_rs::Key) -> Result<Option<KeyRef>, HlsParseError> {
    match k.method {
        m3u8_rs::KeyMethod::None => Ok(None),
        m3u8_rs::KeyMethod::AES128 => {
            let uri = k
                .uri
                .ok_or_else(|| HlsParseError::Parse("AES-128 key missing URI".into()))?;
            let iv = match &k.iv {
                Some(hexstr) => Some(parse_iv(hexstr)?),
                None => None,
            };
            Ok(Some(KeyRef {
                uri: resolve_url(base_url, &uri).unwrap_or(uri),
                iv,
            }))
        }
        m3u8_rs::KeyMethod::SampleAES => {
            Err(HlsParseError::UnsupportedKeyMethod("SAMPLE-AES".into()))
        }
        m3u8_rs::KeyMethod::Other(s) => Err(HlsParseError::UnsupportedKeyMethod(s)),
    }
}

fn parse_iv(hexstr: &str) -> Result<[u8; 16], HlsParseError> {
    let s = hexstr.strip_prefix("0x").unwrap_or(hexstr);
    // 先按字节校验长度与字符集，再切分。
    // 注意不能用 `s.len()` 判断"32 个十六进制字符"：`len()` 是字节数，若串里混入多字节
    // UTF-8（畸形 playlist 可以做到），`&s[2*i..2*i+2]` 会落在字符边界之外直接 panic。
    // 按字节遍历 chunks_exact 则完全不涉及 str 边界。
    let bytes = s.as_bytes();
    if bytes.len() != 32 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return Err(HlsParseError::Parse(format!("bad IV: {hexstr}")));
    }
    let mut out = [0u8; 16];
    for (i, pair) in bytes.chunks_exact(2).enumerate() {
        let hex = std::str::from_utf8(pair)
            .map_err(|_| HlsParseError::Parse(format!("bad IV hex: {hexstr}")))?;
        out[i] = u8::from_str_radix(hex, 16)
            .map_err(|_| HlsParseError::Parse(format!("bad IV hex: {hexstr}")))?;
    }
    Ok(out)
}
```

注（重要）：`m3u8-rs` 锁定 **6.x**（当前 6.0.1），上面的代码已按 6.0.1 的真实 API 核对过（源码见 `~/.cargo/registry/src/*/m3u8-rs-6.0.1/src/playlist.rs`）。已确认的事实：

- 顶层枚举是 `Playlist::MasterPlaylist(MasterPlaylist)` / `Playlist::MediaPlaylist(MediaPlaylist)`（**不是** `Master`/`Media`）。
- `MasterPlaylist.variants: Vec<VariantStream>`；`VariantStream { uri: String, bandwidth: u64, average_bandwidth: Option<u64>, resolution: Option<Resolution>, frame_rate: Option<f64> }`；`Resolution { width: u64, height: u64 }` —— 故 `VariantChoice` 的 `bandwidth`/`resolution` 用 `u64`，不要用 u32。
- `MediaPlaylist { version, target_duration, media_sequence, segments: Vec<MediaSegment>, .. }`，**没有** `title` 字段。
- `MediaSegment { uri: String, duration: f32, title: Option<String>, byte_range, discontinuity, key: Option<Key>, map: Option<Map>, .. }`。
- `Key { method: KeyMethod, uri: Option<String>, iv: Option<String>, keyformat, keyformatversions }`，其中 **`KeyMethod` 是枚举 `{ None, AES128, SampleAES, Other(String) }`，不是 String**，必须按变体匹配（`KeyMethod::SampleAES` 的 Debug 名是 `SampleAES`，直接 `format!("{k:?}")` 得不到测试期望的 `SAMPLE-AES`，所以要像上面那样显式写成字符串）。
- `Map { uri: String, byte_range: Option<ByteRange>, .. }`。
- 用 `parse_playlist_res` 拿 `Result`，不要引入 nom 去解 `IResult`。

**不要为了迁就示例代码而降级 crate 版本。** 若实际 API 与此处描述仍有出入，以 crate 源码为准并记录差异。

- [ ] **Step 5: 运行确认通过**

Run: `cargo test --test hls_parse_test`
Expected: 12 PASS

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: m3u8 解析（master/media/AES-128/fMP4）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 6: hls::crypto 模块（AES-128 解密）

**Files:**
- Create: `tests/hls_crypto_test.rs`
- Modify: `src/core/hls/crypto.rs`

**Interfaces:**
- Produces:
  - `pub fn decrypt_aes128(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Result<Vec<u8>, HlsCryptoError>`
  - `pub enum HlsCryptoError { InvalidLength, Decrypt(String) }`（thiserror）
- 语义：AES-128-CBC 解密并做 PKCS7 去填充。**去填充失败即报 `BadPadding` 错误，不做"按无填充返回"的宽容回退** —— 详见实现处的说明：HLS AES-128 无 MAC，PKCS7 是唯一的完整性信号，宽容化会把可检测的损坏变成静默的坏视频。

- [ ] **Step 1: 写失败测试**

先用 openssl CLI（macOS 自带）生成两组测试向量，把 hex 输出硬编码进测试：

```bash
printf '0123456789abcdef' > /tmp/pt.bin

# 向量 A：带 PKCS7 填充（明文 16 字节 → 填充后 32 字节密文）
openssl enc -aes-128-cbc -K 00112233445566778899aabbccddeeff -iv 0102030405060708090a0b0c0d0e0f10 \
  -in /tmp/pt.bin -out /tmp/ct_pad.bin
xxd -p /tmp/ct_pad.bin

# 向量 B：无填充（恰好 16 字节密文，用于走 PKCS7 去填充失败的回退分支）
openssl enc -aes-128-cbc -nopad -K 00112233445566778899aabbccddeeff -iv 0102030405060708090a0b0c0d0e0f10 \
  -in /tmp/pt.bin -out /tmp/ct_nopad.bin
xxd -p /tmp/ct_nopad.bin
```

两组向量测的是**不同的代码路径**：向量 A 走 PKCS7 成功去填充（密文 32 字节 → 明文 16 字节）；向量 B 走 PKCS7 失败后回退到 NoPadding（密文 16 字节 → 明文 16 字节）。

`tests/hls_crypto_test.rs`：

```rust
use easydownvideo::core::hls::crypto::{decrypt_aes128, HlsCryptoError};

const KEY: [u8; 16] = *b"\x00\x11\x22\x33\x44\x55\x66\x77\x88\x99\xaa\xbb\xcc\xdd\xee\xff";
const IV: [u8; 16] = *b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10";

// 向量 A：带 PKCS7 填充的密文（32 字节），来自上面的 openssl 命令
const CT_PADDED_HEX: &str = "<向量 A 的 hex>";
// 向量 B：无填充的密文（16 字节，-nopad）
const CT_UNPADDED_HEX: &str = "<向量 B 的 hex>";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

#[test]
fn decrypts_pkcs7_padded_segment() {
    let ct = unhex(CT_PADDED_HEX);
    assert_eq!(ct.len(), 32, "向量 A 应为 32 字节（16 字节明文 + 16 字节 PKCS7 填充）");
    let pt = decrypt_aes128(&KEY, &IV, &ct).unwrap();
    assert_eq!(pt, b"0123456789abcdef");
    assert_eq!(pt.len(), 16, "PKCS7 填充应被去除");
}

#[test]
fn rejects_bad_padding_instead_of_returning_garbage() {
    // 向量 B：无填充密文，末尾解出 0x66(>16)，不是合法 PKCS7 填充。
    // HLS AES-128 没有 MAC，PKCS7 校验是**唯一**的完整性信号——
    // 这里必须报错，绝不能"宽容地"把明文（连同填充）返回出去：
    // 那等于把每一次可检测的损坏变成静默的坏视频。
    let ct = unhex(CT_UNPADDED_HEX);
    assert_eq!(ct.len(), 16);
    let err = decrypt_aes128(&KEY, &IV, &ct).unwrap_err();
    assert!(
        matches!(err, HlsCryptoError::BadPadding),
        "期望 BadPadding，实际: {err}"
    );
}

#[test]
fn wrong_key_errors_rather_than_returning_garbage() {
    // 用错误密钥解出来的整段都是乱码；PKCS7 校验必须把它拦住。
    let ct = unhex(CT_PADDED_HEX);
    let wrong_key = [0u8; 16];
    let r = decrypt_aes128(&wrong_key, &IV, &ct);
    assert!(r.is_err(), "错误密钥必须报错，而不是产出看似成功的乱码");
}

#[test]
fn rejects_non_blocksize_input() {
    let err = decrypt_aes128(&KEY, &IV, b"not-a-multiple!").unwrap_err();
    assert!(matches!(err, HlsCryptoError::InvalidLength(15)), "实际: {err}");
}

#[test]
fn rejects_empty_input() {
    let err = decrypt_aes128(&KEY, &IV, b"").unwrap_err();
    // 空输入必须走 Empty 而不是 InvalidLength：0 本身是 16 的倍数，
    // 若不单独判空会落到 InvalidLength 并给出"长度 0 不是 16 的倍数"这种错误信息。
    assert!(matches!(err, HlsCryptoError::Empty), "实际: {err}");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test hls_crypto_test`
Expected: FAIL（函数未实现）

- [ ] **Step 3: 实现**

`src/core/hls/crypto.rs`：

```rust
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use thiserror::Error;

type Aes128CbcDecrypt = cbc::Decryptor<aes::Aes128>;

#[derive(Error, Debug)]
pub enum HlsCryptoError {
    #[error("ciphertext is empty")]
    Empty,
    #[error("ciphertext length {0} is not a multiple of 16")]
    InvalidLength(usize),
    #[error(
        "PKCS7 去填充失败：分片已损坏，或密钥/IV 不匹配。\
HLS AES-128 不带 MAC，PKCS7 校验是它唯一的完整性信号，因此这里必须报错"
    )]
    BadPadding,
}

pub fn decrypt_aes128(
    key: &[u8; 16],
    iv: &[u8; 16],
    data: &[u8],
) -> Result<Vec<u8>, HlsCryptoError> {
    if data.is_empty() {
        return Err(HlsCryptoError::Empty);
    }
    if data.len() % 16 != 0 {
        return Err(HlsCryptoError::InvalidLength(data.len()));
    }
    // 用原地解密 API 而非 `decrypt_padded_vec_mut`：后者需要 cipher 的 `alloc` feature，
    // 而当前解析到的 cipher v0.4.4 未启用（启用它等于改依赖）。
    //
    // **不做"去填充失败就按无填充返回"的宽容回退**。原因：
    // 1) 密钥/IV 正确时，解出的最后一个分组本来就是原始 PKCS7 填充，去填充必然成功。
    //    因此失败只可能意味着密钥/IV 错或数据损坏 —— 此时解出的**整段**都是乱码，
    //    不只是尾部，返回它没有任何好处。
    // 2) HLS AES-128 没有 MAC：PKCS7 校验是格式提供的**唯一**完整性信号。
    //    把它降级成"宽容通过"，等于把每一次本可检测的损坏变成静默的坏视频。
    let mut buf = data.to_vec();
    let decryptor = Aes128CbcDecrypt::new(key.into(), iv.into());
    match decryptor.decrypt_padded_mut::<Pkcs7>(&mut buf) {
        Ok(pt) => Ok(pt.to_vec()),
        Err(_) => Err(HlsCryptoError::BadPadding),
    }
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test hls_crypto_test`
Expected: 5 PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: HLS AES-128 分片解密

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 7: direct 模块（多线程分块下载 + 拼接 + unique_path）

**Files:**
- Create: `tests/direct_test.rs`
- Modify: `src/core/direct.rs`
- Modify: `src/core/events.rs`（追加 `ProgressHandle::sub` —— 本任务的重试回滚需要它）
- Modify: `tests/events_test.rs`（追加 `sub_rolls_back_without_underflow`，该文件变为 4 个测试）

**Interfaces:**
- Consumes: `ProgressHandle`（Task 3）、`tokio_util::sync::CancellationToken`（Task 3）
- 本任务向 `ProgressHandle` 追加 `sub(u64)`：分块重试失败时撤销该次尝试已累加的进度
- Produces:
  - `pub const MIN_BLOCK_SIZE: u64 = 4 * 1024 * 1024`
  - `pub fn plan_blocks(total: u64, concurrency: usize, min_block: u64) -> Vec<(u64, u64)>`（左闭右开区间，块数 ≤ concurrency）
  - `pub fn unique_path(dir: &Path, filename: &str) -> PathBuf`（重名自动 `(1)` 递增；返回完整目标路径）
  - `pub async fn download_direct(client: &Client, url: &str, final_path: &Path, task_dir: &Path, size: Option<u64>, accepts_ranges: bool, concurrency: usize, min_block: u64, progress: &ProgressHandle, cancel: CancellationToken) -> anyhow::Result<()>`
    - `final_path` 由调用方经 `unique_path` 计算；`.part-N` 文件写 `task_dir` 下；完成后流式拼接进 `final_path`；已存在且大小匹配的 `.part-N` 跳过（断点续传）

- [ ] **Step 1: 写失败测试**

`tests/direct_test.rs`：

```rust
use easydownvideo::core::direct::{download_direct, plan_blocks, unique_path, MIN_BLOCK_SIZE};
use easydownvideo::core::events::ProgressHandle;
use tokio_util::sync::CancellationToken;
use wiremock::{matchers::{header, method, path}, Mock, MockServer, ResponseTemplate};

#[test]
fn plan_blocks_respects_min_block_and_concurrency() {
    // 100 / 4，min=1 → 每块 25，正好 4 块
    assert_eq!(plan_blocks(100, 4, 1), vec![(0, 25), (25, 50), (50, 75), (75, 100)]);
    // 小文件：min_block 主导，块数 < concurrency
    assert_eq!(plan_blocks(10, 4, 4), vec![(0, 4), (4, 8), (8, 10)]);
    // 恰好整除
    assert_eq!(plan_blocks(8, 4, 1), vec![(0, 2), (2, 4), (4, 6), (6, 8)]);
    assert!(MIN_BLOCK_SIZE >= 4 * 1024 * 1024);
}

#[test]
fn unique_path_increments_on_conflict() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(unique_path(dir.path(), "a.mp4"), dir.path().join("a.mp4"));
    std::fs::write(dir.path().join("a.mp4"), b"x").unwrap();
    assert_eq!(unique_path(dir.path(), "a.mp4"), dir.path().join("a (1).mp4"));
    std::fs::write(dir.path().join("a (1).mp4"), b"x").unwrap();
    assert_eq!(unique_path(dir.path(), "a.mp4"), dir.path().join("a (2).mp4"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_in_blocks_and_merges() {
    let data: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/f.bin")).and(header("range", "bytes=0-511"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-range", "bytes 0-511/1024")
            .set_body_bytes(data[0..512].to_vec()))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/f.bin")).and(header("range", "bytes=512-1023"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-range", "bytes 512-1023/1024")
            .set_body_bytes(data[512..].to_vec()))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("f.bin");
    let progress = ProgressHandle::new();
    download_direct(
        &reqwest::Client::new(),
        &format!("{}/f.bin", server.uri()),
        &final_path,
        dir.path(),
        Some(1024),
        true,
        2,
        512, // 测试用小 min_block
        &progress,
        CancellationToken::new(),
    ).await.unwrap();

    assert_eq!(std::fs::read(&final_path).unwrap(), data);
    assert_eq!(progress.snapshot(), (1024, 1024));
    assert!(std::fs::read_dir(dir.path()).unwrap().count() == 1); // .part 已清理
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumes_completed_parts() {
    let data: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
    let server = MockServer::start().await;
    // 只有第二个 range 有 mock：第一个 part 若被重新请求则 404 → 测试证明跳过了它
    Mock::given(method("GET")).and(path("/f.bin")).and(header("range", "bytes=512-1023"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-range", "bytes 512-1023/1024")
            .set_body_bytes(data[512..].to_vec()))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    // 预置已完成的第一块
    std::fs::write(dir.path().join("f.part-0"), &data[0..512]).unwrap();

    let final_path = dir.path().join("f.bin");
    download_direct(
        &reqwest::Client::new(),
        &format!("{}/f.bin", server.uri()),
        &final_path,
        dir.path(),
        Some(1024),
        true,
        2,
        512,
        &ProgressHandle::new(),
        CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(std::fs::read(&final_path).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_stops_download() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/slow.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 4096]))
        .mount(&server).await;
    let dir = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let r = download_direct(
        &reqwest::Client::new(),
        &format!("{}/slow.bin", server.uri()),
        &dir.path().join("slow.bin"),
        dir.path(),
        Some(4096),
        false,
        1,
        MIN_BLOCK_SIZE,
        &ProgressHandle::new(),
        token,
    ).await;
    assert!(r.is_err());
}
```

注：`plan_blocks(10, 4, 4)` → chunk = max(ceil(10/4)=3, 4) = 4 → [(0,4),(4,8),(8,10)]，3 块 ≤ 4。`plan_blocks(100,4,1)` → chunk = max(25, 1) = 25 → 4 块。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test direct_test`
Expected: FAIL

- [ ] **Step 3: 实现**

`src/core/direct.rs`：

```rust
use anyhow::{anyhow, Context};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::core::events::ProgressHandle;

pub const MIN_BLOCK_SIZE: u64 = 4 * 1024 * 1024;

pub fn plan_blocks(total: u64, concurrency: usize, min_block: u64) -> Vec<(u64, u64)> {
    if total == 0 {
        return vec![];
    }
    let n = concurrency.max(1) as u64;
    let chunk = ((total + n - 1) / n).max(min_block);
    let mut out = Vec::new();
    let mut start = 0u64;
    while start < total {
        let end = (start + chunk).min(total);
        out.push((start, end));
        start = end;
    }
    out
}

pub fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .to_string();
    let ext = Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    for i in 1.. {
        let p = dir.join(format!("{stem} ({i}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

pub async fn download_direct(
    client: &reqwest::Client,
    url: &str,
    final_path: &Path,
    task_dir: &Path,
    size: Option<u64>,
    accepts_ranges: bool,
    concurrency: usize,
    min_block: u64,
    progress: &ProgressHandle,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(task_dir).context("create task dir")?;
    let blocks = match (size, accepts_ranges) {
        (Some(total), true) if total > 0 => plan_blocks(total, concurrency, min_block),
        _ => vec![(0, u64::MAX)], // 单流
    };
    if let Some(total) = size {
        progress.set_total(total);
    }

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut handles = Vec::new();
    for (i, (start, end)) in blocks.iter().enumerate() {
        let client = client.clone();
        let url = url.to_string();
        let part_path = task_dir.join(format!("f.part-{i}"));
        let expected = *end - *start;
        let (start, end) = (*start, *end);
        let progress = progress.clone();
        let cancel = cancel.clone();
        let permit = sem.clone().acquire_owned().await?;
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            // 断点续传：已完成则跳过
            if let Ok(meta) = std::fs::metadata(&part_path) {
                if meta.len() == expected {
                    progress.add(expected);
                    return Ok(());
                }
            }
            let req = client.get(&url);
            let req = if end != u64::MAX {
                req.header("Range", format!("bytes={start}-{}", end - 1))
            } else {
                req
            };
            // 分块级重试：指数退避 3 次（1s/2s/4s）
            let mut last_err = None;
            for attempt in 0..3 {
                match download_one_block(&req, &part_path, &progress, &cancel).await {
                    Ok(()) => { last_err = None; break; }
                    Err(e) => {
                        last_err = Some(e);
                        if cancel.is_cancelled() { break; }
                        tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                    }
                }
            }
            match last_err {
                Some(e) => Err(e),
                None => Ok::<_, anyhow::Error>(()),
            }
        }));
    }
    for h in handles {
        h.await??;
    }

    // 流式拼接
    let mut out = tokio::fs::File::create(final_path).await?;
    for i in 0..blocks.len() {
        let part_path = task_dir.join(format!("f.part-{i}"));
        let mut input = tokio::fs::File::open(&part_path).await?;
        tokio::io::copy(&mut input, &mut out).await?;
        std::fs::remove_file(&part_path).ok();
    }
    out.flush().await?;
    Ok(())
}

async fn download_one_block(
    req: &reqwest::RequestBuilder,
    part_path: &Path,
    progress: &ProgressHandle,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    // 进度回滚守卫：本次尝试计入的字节先记在守卫里，若这次尝试失败（守卫未被解除），
    // 退出时把已计入的字节减回去。不做这件事的话，重试会把同一段字节重复累加——
    // 三次重试后同一块被计三倍，进度超过总数，速度与 ETA 同时失真。
    let mut rollback = ProgressRollback::new(progress);
    let resp = req.try_clone().context("request not cloneable")?.send().await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(part_path).await?;
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(anyhow!("canceled")); // 守卫在此 drop，自动回滚
        }
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        rollback.add(chunk.len() as u64);
    }
    file.flush().await?;
    rollback.disarm();
    Ok(())
}

/// 见 `download_one_block` 中的说明：为一次下载尝试的进度累加提供失败回滚。
struct ProgressRollback<'a> {
    progress: &'a ProgressHandle,
    counted: u64,
    armed: bool,
}

impl<'a> ProgressRollback<'a> {
    fn new(progress: &'a ProgressHandle) -> Self {
        Self { progress, counted: 0, armed: true }
    }
    fn add(&mut self, n: u64) {
        self.progress.add(n);
        self.counted += n;
    }
    /// 尝试成功：保留已计入的字节，撤销回滚。
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProgressRollback<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.progress.sub(self.counted);
        }
    }
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test direct_test`
Expected: 5 PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 直链多线程分块下载、拼接与断点续传

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 8: hls 下载编排（分片调度、解密、落盘、concat 计划）

**Files:**
- Create: `tests/hls_download_test.rs`
- Modify: `src/core/hls/mod.rs`

**Interfaces:**
- Consumes: `parse.rs`（`MediaInfo`/`SegmentRef`/`KeyRef`）、`crypto.rs`（`decrypt_aes128`）、`events::ProgressHandle`
- Produces:
  - `pub struct ConcatPlan { pub list_file: PathBuf, pub segment_files: Vec<PathBuf> }`（**没有** `is_ts`：转封装参数与容器类型无关，见 Task 9）
  - `pub async fn download_media(client: &Client, media: &MediaInfo, task_dir: &Path, concurrency: usize, progress: &ProgressHandle, cancel: CancellationToken) -> anyhow::Result<ConcatPlan>`
    - 分片先写 `task_dir/seg-XXXXX.<ext>.part`，成功后原子改名为 `seg-XXXXX.<ext>`；
      **只有改好名的文件才视为已完成**（`exists()` 即等价于"完整"，不需要额外记录大小）。
      遗留的 `.part` 会被下次 `create` 截断覆盖，绝不当成成品
    - IV 缺省时按 HLS 规则取 `media_sequence + 分片序号` 的 64 位大端（前 8 字节补零）
    - key 按 URI 缓存，只拉一次
    - 完成后写 `task_dir/concat.txt`（ffmpeg concat 列表）并返回 `ConcatPlan`

- [ ] **Step 1: 写失败测试**

`tests/hls_download_test.rs`：

```rust
use easydownvideo::core::events::ProgressHandle;
use easydownvideo::core::hls::{download_media, plan_segment_indices};
use easydownvideo::core::hls::parse::{KeyRef, MediaInfo, SegmentRef};
use tokio_util::sync::CancellationToken;
use wiremock::{matchers::{method, path}, Mock, MockServer, ResponseTemplate};

fn media_no_crypto(seg_uris: &[String]) -> MediaInfo {
    MediaInfo {
        segments: seg_uris.iter().map(|u| SegmentRef {
            uri: u.clone(), key: None, is_init: false,
        }).collect(),
        media_sequence: 0,
        is_fmp4: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_segments_and_builds_concat_plan() {
    let server = MockServer::start().await;
    let s0 = vec![7u8; 100];
    let s1 = vec![8u8; 200];
    Mock::given(method("GET")).and(path("/seg0.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s0.clone()))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/seg1.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s1.clone()))
        .mount(&server).await;

    let media = media_no_crypto(&[
        format!("{}/seg0.ts", server.uri()),
        format!("{}/seg1.ts", server.uri()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let progress = ProgressHandle::new();

    let plan = download_media(
        &reqwest::Client::new(),
        &media,
        dir.path(),
        4,
        &progress,
        CancellationToken::new(),
    ).await.unwrap();

    assert_eq!(plan.segment_files.len(), 2);
    assert_eq!(std::fs::read(&plan.segment_files[0]).unwrap(), s0);
    assert_eq!(std::fs::read(&plan.segment_files[1]).unwrap(), s1);
    // HLS 进度以分片计数：2 个分片
    assert_eq!(progress.snapshot(), (2, 2));
    assert_eq!(progress.unit(), easydownvideo::core::events::ProgressUnit::Segments);
    // concat 列表已生成
    let list = std::fs::read_to_string(&plan.list_file).unwrap();
    assert!(list.contains("file '"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumes_existing_segments() {
    let server = MockServer::start().await;
    let s1 = vec![9u8; 50];
    // 只挂 seg1：seg0 若被重新请求会 404
    Mock::given(method("GET")).and(path("/seg1.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s1.clone()))
        .mount(&server).await;

    let media = media_no_crypto(&[
        format!("{}/seg0.ts", server.uri()),
        format!("{}/seg1.ts", server.uri()),
    ]);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("seg-00000.ts"), vec![7u8; 100]).unwrap();

    let plan = download_media(
        &reqwest::Client::new(),
        &media,
        dir.path(),
        2,
        &ProgressHandle::new(),
        CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(std::fs::read(&plan.segment_files[0]).unwrap(), vec![7u8; 100]);
    assert_eq!(std::fs::read(&plan.segment_files[1]).unwrap(), s1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_failed_segment() {
    let server = MockServer::start().await;
    let s0 = vec![7u8; 100];
    // 第一次请求 500，之后 200：验证重试逻辑
    Mock::given(method("GET")).and(path("/flaky.ts"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/flaky.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s0.clone()))
        .mount(&server).await;
    let media = media_no_crypto(&[format!("{}/flaky.ts", server.uri())]);
    let dir = tempfile::tempdir().unwrap();
    let plan = download_media(
        &reqwest::Client::new(), &media, dir.path(), 1,
        &ProgressHandle::new(), CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(std::fs::read(&plan.segment_files[0]).unwrap(), s0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decrypts_with_derived_iv_across_an_init_offset() {
    // 这是对「数组下标 vs 媒体序号」这条不变式的端到端验证，而不只是测那个纯函数。
    // 设计要点：MediaInfo 里第一个元素是 init 段，所以媒体分片的**数组下标**是 1、2，
    // 而**媒体序号**是 0、1。若实现用 idx 推 IV，第一个分片就会拿 IV1 去解，
    // PKCS7 校验必然失败（或即便碰巧通过，明文断言也会挂）→ 测试失败。
    //
    // 向量用 openssl 生成（明文 32 字节 "0123456789abcdef0123456789abcdef"，key/IV 如下）：
    //   IV0 = 0x00..00（media_sequence 0 + 媒体序号 0）
    //   IV1 = 0x00..01（media_sequence 0 + 媒体序号 1）
    //   openssl enc -aes-128-cbc -K 00112233445566778899aabbccddeeff -iv <IV> -in pt32.bin | xxd -p
    const CT_IV0: &str = "<openssl 输出（IV=00..00）>";
    const CT_IV1: &str = "<openssl 输出（IV=00..01）>";
    const KEY_HEX: &str = "00112233445566778899aabbccddeeff";
    const PLAINTEXT: &[u8] = b"0123456789abcdef0123456789abcdef";

    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/key.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(unhex(KEY_HEX)))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/init.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 16]))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/e0.m4s"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(unhex(CT_IV0)))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/e1.m4s"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(unhex(CT_IV1)))
        .mount(&server).await;

    // 注意 iv: None —— 走"按媒体序号推导 IV"这条路径，这正是要验证的分支
    let key = KeyRef { uri: format!("{}/key.bin", server.uri()), iv: None };
    let media = MediaInfo {
        segments: vec![
            SegmentRef { uri: format!("{}/init.mp4", server.uri()), key: None, is_init: true },
            SegmentRef { uri: format!("{}/e0.m4s", server.uri()), key: Some(key.clone()), is_init: false },
            SegmentRef { uri: format!("{}/e1.m4s", server.uri()), key: Some(key), is_init: false },
        ],
        media_sequence: 0,
        is_fmp4: true,
    };

    let dir = tempfile::tempdir().unwrap();
    let plan = download_media(
        &reqwest::Client::new(), &media, dir.path(), 4,
        &ProgressHandle::new(), CancellationToken::new(),
    ).await.unwrap();

    assert_eq!(plan.segment_files.len(), 3, "init + 两个媒体分片");
    assert_eq!(
        plan.segment_files[0],
        dir.path().join("seg-00000.m4s"),
        "init 段必须排在最前（文件按 segments 数组下标命名，所以是 seg-00000）"
    );
    for i in [1usize, 2usize] {
        assert_eq!(
            std::fs::read(&plan.segment_files[i]).unwrap(),
            PLAINTEXT,
            "分片 {i} 解密结果不对 —— 检查 IV 是否用了媒体序号而非数组下标"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn truncated_leftover_segment_is_not_treated_as_complete() {
    // 完成标志是"最终文件名存在"（写 .part → 原子改名）。被中断的下载留下的是 .part，
    // 因此遗留 .part 绝不能被当成完成、更不能被直接改名为成品送进 concat 列表。
    //
    // 注意种子必须是 .part（而不是最终名）：最终名存在按设计就是"已完成"，
    // 那正是 resumes_existing_segments 覆盖的行为，两者不能混为一谈。
    let server = MockServer::start().await;
    let s0 = vec![7u8; 100];
    Mock::given(method("GET")).and(path("/seg0.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s0.clone()))
        .mount(&server).await;

    let media = media_no_crypto(&[format!("{}/seg0.ts", server.uri())]);
    let dir = tempfile::tempdir().unwrap();
    // 模拟：上次 create 之后、写满/改名之前被杀，留下半截 .part
    std::fs::write(dir.path().join("seg-00000.ts.part"), vec![7u8; 40]).unwrap();

    let plan = download_media(
        &reqwest::Client::new(), &media, dir.path(), 1,
        &ProgressHandle::new(), CancellationToken::new(),
    ).await.unwrap();
    assert_eq!(
        std::fs::read(&plan.segment_files[0]).unwrap(),
        s0,
        "遗留的 .part 必须被覆盖重下，其内容绝不能进入最终分片"
    );
    // 计划里还写了一条 `!dir.path().join("seg-00000.ts.part").exists()`。已实现并复查的版本
    // 只保留了上面这条内容断言：它才是这个缺陷的实质属性（半截产物不得进入成品分片）。
    // .part 残留与否是改名逻辑的副作用，且该断言从未被实现，故不在此处追溯补写。
}

#[test]
fn escape_concat_path_escapes_single_quotes() {
    use easydownvideo::core::hls::escape_concat_path;
    // 实测：不转义时 ffmpeg 把 `file '/tmp/My's Videos/seg.ts'` 解析成 `/tmp/Mys` 并报
    // "Impossible to open"。用户把下载目录设在含撇号的路径下（如 ~/My's Videos）很常见，
    // 不转义等于**每一次 HLS 下载都在合并这一步失败**。
    assert_eq!(escape_concat_path("/tmp/a.ts"), "/tmp/a.ts", "无特殊字符时原样返回");
    assert_eq!(
        escape_concat_path("/tmp/My's Videos/a.ts"),
        "/tmp/My'\\''s Videos/a.ts",
        "路径里的单引号必须转义成 '\\''"
    );
    // 多个撇号都要转义
    assert_eq!(escape_concat_path("it's a 'test'.ts"), "it'\\''s a '\\''test'\\''.ts");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concat_list_uses_escaped_paths() {
    // 端到端确认生成的 concat.txt 真的用了转义（纯函数测试保护不了"有没有调用它"）
    let server = MockServer::start().await;
    let s0 = vec![7u8; 100];
    Mock::given(method("GET")).and(path("/seg0.ts"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s0.clone()))
        .mount(&server).await;

    let media = media_no_crypto(&[format!("{}/seg0.ts", server.uri())]);
    // 任务目录名固定为 task-<id>，但父目录来自 download_dir —— 这里用一个带撇号的临时目录
    let parent = tempfile::tempdir().unwrap();
    let weird = parent.path().join("My's Videos");
    std::fs::create_dir_all(&weird).unwrap();
    let task_dir = weird.join("task-1");

    let plan = download_media(
        &reqwest::Client::new(), &media, &task_dir, 1,
        &ProgressHandle::new(), CancellationToken::new(),
    ).await.unwrap();

    let list = std::fs::read_to_string(&plan.list_file).unwrap();
    assert!(
        list.contains("'\\''"),
        "concat 列表里的单引号必须已转义，实际内容: {list:?}"
    );
    assert!(
        !list.contains("Videos/task-1/"),
        "未转义的路径形态不应出现（那说明转义没生效）: {list:?}"
    );
}

#[test]
fn init_segment_does_not_shift_media_indices() {
    // IV 由 `media_sequence + 媒体序号` 推导。init 段（fMP4 的 EXT-X-MAP）不占媒体序号，
    // 若按 segments 数组下标推导 IV，init 之后的每个分片序号都会多 1，解出乱码。
    let segs = vec![
        SegmentRef { uri: "init.mp4".into(), key: None, is_init: true },
        SegmentRef { uri: "s0.m4s".into(), key: None, is_init: false },
        SegmentRef { uri: "s1.m4s".into(), key: None, is_init: false },
        SegmentRef { uri: "s2.m4s".into(), key: None, is_init: false },
    ];
    assert_eq!(
        plan_segment_indices(&segs),
        vec![(0, 0), (1, 0), (2, 1), (3, 2)],
        "第一个媒体分片的媒体序号必须是 0，而不是它在数组里的下标 1"
    );

    // 无 init 段时，下标即媒体序号
    let plain = vec![
        SegmentRef { uri: "a.ts".into(), key: None, is_init: false },
        SegmentRef { uri: "b.ts".into(), key: None, is_init: false },
    ];
    assert_eq!(plan_segment_indices(&plain), vec![(0, 0), (1, 1)]);
}
```

注：`ConcatPlan` 只含 `list_file` 与 `segment_files` —— 转封装参数与容器类型无关，见 Task 9。
分片字节数是下载时累计的（`progress.add(1)` 计数的是分片个数，故 `ProgressUnit::Segments`）。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test hls_download_test`
Expected: FAIL

- [ ] **Step 3: 实现**

`src/core/hls/mod.rs`：

```rust
pub mod crypto;
pub mod parse;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::core::events::ProgressHandle;
use crypto::decrypt_aes128;
use parse::{KeyRef, MediaInfo, SegmentRef};

#[derive(Clone, Debug)]
pub struct ConcatPlan {
    pub list_file: PathBuf,
    pub segment_files: Vec<PathBuf>,
}

/// 转义 ffmpeg concat 列表里的路径。
///
/// ffmpeg 的 concat 解析器用单引号界定路径，路径里的 `'` 必须写成 `'\''`，否则会被当成
/// 引号结束。**实测**：`file '/tmp/My's Videos/seg.ts'` 被解析成 `/tmp/Mys` 并报
/// "Impossible to open" —— 用户只要把下载目录设在含撇号的路径下（如 `~/My's Videos`），
/// **每一次 HLS 下载都会在最后合并这一步失败**。
///
/// 实测有效的两种写法是 `'\''` 与全反斜杠转义；双引号包裹无效。这里取前者。
///
/// 抽成公开的纯函数是为了能直接单测：转义逻辑曾被我放在 ffmpeg 模块一个**没有任何调用方**
/// 的函数里，导致真正的 bug 一直存在而"修复"落在死代码上。逻辑在哪落地，测试就盯哪。
pub fn escape_concat_path(path: &str) -> String {
    path.replace('\'', "'\\''")
}

pub async fn download_media(
    client: &reqwest::Client,
    media: &MediaInfo,
    task_dir: &Path,
    concurrency: usize,
    progress: &ProgressHandle,
    cancel: CancellationToken,
) -> anyhow::Result<ConcatPlan> {
    use std::sync::Arc;
    std::fs::create_dir_all(task_dir).context("create task dir")?;

    // HLS 进度按"已下分片数/总分片数"计数，GUI 用 ProgressUnit 决定展示格式
    progress.set_unit(crate::core::events::ProgressUnit::Segments);
    progress.set_total(media.segments.len() as u64);

    let ext = if media.is_fmp4 { "m4s" } else { "ts" };
    let keys = Arc::new(Mutex::new(HashMap::new()));
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let media_sequence = media.media_sequence;

    let mut handles = Vec::new();
    // (segments 下标, 媒体序号偏移)：init 段不占媒体序号，见 plan_segment_indices 的说明
    for (idx, media_index) in plan_segment_indices(&media.segments) {
        let seg = &media.segments[idx];
        let file_path = task_dir.join(format!("seg-{idx:05}.{ext}"));
        // 先写临时名，成功后改名。"已完成"的唯一标志是**改名后的文件存在**。
        // 不能只用 exists() 判续传：写入路径是 create(截断) + write_all，
        // 若进程在两步之间被杀，会留下半截文件；下一轮会把它当成完成，
        // concat 列表照样列它，ffmpeg 产出损坏输出且**全程不报错**。
        // rename 在同一文件系统内是原子的，所以"存在"就等价于"完整"。
        let tmp_path = task_dir.join(format!("seg-{idx:05}.{ext}.part"));
        let client = client.clone();
        let seg_uri = seg.uri.clone();
        let key = seg.key.clone();
        let progress = progress.clone();
        let cancel = cancel.clone();
        let keys = keys.clone();
        let permit = sem.clone().acquire_owned().await?;
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            // 续传：只认已改名完成的文件；遗留的 .part 会被下面的 create 覆盖
            if file_path.exists() {
                progress.add(1);
                return Ok(file_path);
            }
            if cancel.is_cancelled() {
                return Err(anyhow!("canceled"));
            }
            let ciphertext = fetch_bytes(&client, &seg_uri, &cancel).await?;
            let data = match &key {
                Some(KeyRef { uri, iv }) => {
                    let k = {
                        let mut cache = keys.lock().await;
                        if let Some(k) = cache.get(uri) {
                            *k
                        } else {
                            let raw = fetch_bytes(&client, uri, &cancel).await?;
                            let k: [u8; 16] = raw.try_into().map_err(|_| anyhow!("key length != 16"))?;
                            cache.insert(uri.clone(), k);
                            k
                        }
                    };
                    // 用媒体序号而非数组下标推导 IV
                    let effective_iv = iv
                        .unwrap_or_else(|| default_iv(media_sequence, media_index));
                    decrypt_aes128(&k, &effective_iv, &ciphertext)?
                }
                None => ciphertext,
            };
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            file.write_all(&data).await?;
            file.flush().await?;
            drop(file); // 改名之前必须关闭句柄（Windows 上尤其如此）
            tokio::fs::rename(&tmp_path, &file_path).await?;
            progress.add(1);
            Ok::<_, anyhow::Error>(file_path)
        }));
    }
    // parse 阶段已把 init segment 排在首位，handle 顺序即 concat 顺序
    let mut files = Vec::new();
    for h in handles {
        files.push(h.await??);
    }

    // 写 ffmpeg concat 列表。列表的生成属于本模块（顺序是这里决定的），所以路径转义
    // 也放在这里，不要指望调用方再处理一遍。
    let list_file = task_dir.join("concat.txt");
    let mut list = String::new();
    for f in &files {
        list.push_str(&format!("file '{}'\n", escape_concat_path(&f.to_string_lossy())));
    }
    std::fs::write(&list_file, list)?;

    Ok(ConcatPlan {
        list_file,
        segment_files: files,
    })
}

async fn fetch_bytes(
    client: &reqwest::Client,
    uri: &str,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<u8>> {
    // 重试 3 次：指数退避 1s/2s/4s
    let mut last_err = None;
    for attempt in 0..3 {
        if cancel.is_cancelled() {
            anyhow::bail!("canceled");
        }
        match fetch_once(client, uri).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                last_err = Some(e);
                // 最后一次失败后不要再睡：那是纯粹的无效等待（每段白等 4 秒）。
                // 退避期间也必须能被取消打断，否则用户点了取消仍要等满退避。
                if attempt < 2 {
                    tokio::select! {
                        _ = cancel.cancelled() => anyhow::bail!("canceled"),
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)) => {}
                    }
                }
            }
        }
    }
    Err(last_err.unwrap())
}

async fn fetch_once(client: &reqwest::Client, uri: &str) -> anyhow::Result<Vec<u8>> {
    let resp = client.get(uri).send().await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk?);
    }
    Ok(out)
}

/// 为一个 media playlist 规划每个分段的 `(segments 数组下标, 媒体序号偏移)`。
///
/// init 段（fMP4 的 `#EXT-X-MAP`）**不占媒体序号**，所以它之后的第一个媒体分片的媒体序号
/// 仍是 0。若直接拿数组下标当媒体序号去推导 IV，`#EXT-X-KEY` 未显式给 IV 时每个分片都会
/// 用错一个序号的 IV，解出乱码且不会报错。单独抽成纯函数就是为了能直接测这个不变式。
pub fn plan_segment_indices(segments: &[SegmentRef]) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(segments.len());
    let mut media_index = 0usize;
    for (idx, seg) in segments.iter().enumerate() {
        let mi = if seg.is_init {
            0 // init 段不使用 IV，占位
        } else {
            let v = media_index;
            media_index += 1;
            v
        };
        out.push((idx, mi));
    }
    out
}

fn default_iv(media_sequence: u64, segment_index: usize) -> [u8; 16] {
    let seq = media_sequence.wrapping_add(segment_index as u64);
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}
```

注（内存特性，务必知道，不要误以为本模块是 O(1)）：`fetch_bytes` 把**整个分片**读进内存再解密。这满足「内存不随**文件**大小增长」——分片有界，所以下 10GB 的文件内存也不会涨。但实际峰值是 **并发数 × 单分片大小 × 系数**（默认 6 路），系数不为 1：

- **稳态 ≈ 2×**：加密分片的 `ciphertext` 与解密结果 `data` 会同时存活。
- **解密瞬时 ≈ 3×**：`decrypt_aes128` 内部先 `data.to_vec()` 再返回 `pt.to_vec()`，短时间三份并存。

| 场景 | 单分片 | 稳态 | 解密瞬时 |
|---|---|---|---|
| 1080p，6s 分片，5 Mbps | ≈3.8MB | ≈46MB | ≈68MB |
| 4K，6s 分片，25 Mbps | ≈19MB | ≈228MB | **≈342MB（严重超预算）** |

即：1080p 已逼近 spec 的 ~100MB 预算，4K 远超。这不影响正确性，但确实与「内存占用不高」的目标有张力。

**决定：先不做优化，按代价从低到高留两级台阶。** 理由：这是性能预算问题而非正确性问题，且 Task 15 有实测 RSS 的验收项；没有测量数据就动手，等于在代码里最需要克制的地方引入复杂度（本模块同类问题已产出 3 个静默损坏缺陷）。

若 Task 15 实测超标，优化顺序如下（**先做 1，只有 1 不够才做 2**）：

1. **廉价：原地解密** —— 加 `decrypt_aes128_in_place(key, iv, buf: &mut Vec<u8>)`，用 `decrypt_padded_mut` 原地解、再 `buf.truncate(明文长度)`。一次分配都不多，直接把系数从 2×/3× 降到 **1×**。约 10 行。
2. **昂贵：流式解密** —— 逐分组解密、只滞留最后一个分组做 PKCS7 校验，做到真正 O(1)。需处理 `&mut [Block]` 转换与边界，约 60 行。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test hls_download_test`
Expected: 8 PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: HLS 分片并发下载、解密与 concat 计划

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 9: ffmpeg 模块（内嵌释放、concat 列表、转封装命令）

**Files:**
- Create: `tests/ffmpeg_test.rs`
- Modify: `src/core/ffmpeg.rs`
- Modify: `Cargo.toml` —— 接口里出现 `tokio::process::Command`，而 `tokio::process` 在 `process` feature 之后；原稿的 feature 列表没有它，模块无法编译。加 `"process"` 即可（唯一允许的依赖改动，`Cargo.lock` 随之 +1 行）

**Interfaces:**
- Produces:
  - `pub fn ensure_ffmpeg(data_dir: &Path) -> anyhow::Result<PathBuf>`（内嵌二进制 → `data_dir/ffmpeg[.exe]`，SHA256 校验，已存在且校验通过则跳过；Unix 加 755 权限）
  - `pub fn build_remux(ffmpeg: &Path, list: &Path, output: &Path) -> tokio::process::Command`
  - `pub async fn remux_to_mp4(ffmpeg: &Path, plan: &crate::core::hls::ConcatPlan, output: &Path) -> anyhow::Result<()>`
- **不产出** `write_concat_list`：concat 列表由 Task 8 生成（顺序在那里决定，转义也在那里），
  本模块只消费 `plan.list_file`。原计划里的 `write_concat_list` 无任何调用方，是死代码，
  而且会让"转义"出现两个可能不同步的实现位置。

- [ ] **Step 1: 写失败测试**

`tests/ffmpeg_test.rs`：

```rust
use easydownvideo::core::ffmpeg::{build_remux, ensure_ffmpeg};
use std::path::PathBuf;

#[test]
fn ensure_extracts_and_reuses() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = ensure_ffmpeg(dir.path()).unwrap();
    assert!(p1.exists());
    let m1 = std::fs::metadata(&p1).unwrap().modified().unwrap();
    let p2 = ensure_ffmpeg(dir.path()).unwrap();
    assert_eq!(p1, p2);
    // 二次调用不重写（修改时间不变即复用）
    assert_eq!(m1, std::fs::metadata(&p2).unwrap().modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p1).unwrap().permissions().mode() & 0o111, 0o111);
    }
}

#[test]
fn build_remux_args_are_container_agnostic() {
    let ffmpeg = PathBuf::from("/usr/bin/ffmpeg");
    let list = PathBuf::from("/tmp/concat.txt");
    let out = PathBuf::from("/tmp/o.mp4");
    let cmd = build_remux(&ffmpeg, &list, &out);
    let args: Vec<String> = cmd
        .as_std()
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let joined = args.join(" ");
    assert!(joined.contains("-f concat"));
    assert!(joined.contains("-safe 0"));
    assert!(joined.contains("-c copy"), "必须是无损转封装");
    assert!(args.iter().any(|a| a == "/tmp/o.mp4"), "输出路径要作为最后一个参数");
    assert!(args.iter().any(|a| a == "-y"), "需覆盖已存在的输出文件");
    // 显式 aac_adtstoasc 会让非 AAC 音频（AC-3 等）的 TS 合并直接失败；
    // 交给 ffmpeg 按需自动插入。见 build_remux 中的实测说明。
    assert!(
        !joined.contains("aac_adtstoasc"),
        "不应硬编码 aac_adtstoasc，否则 AC-3 音轨的 TS 会合并失败"
    );
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test ffmpeg_test`
Expected: FAIL

- [ ] **Step 3: 实现（先放占位字节，真二进制 Task 10 替换）**

`src/core/ffmpeg.rs`：

```rust
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use sha2::Digest;

// ===== 内嵌二进制（Task 10 用真实文件替换占位内容）=====
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const FFMPEG_BYTES: &[u8] = include_bytes!("../../vendor/ffmpeg/macos-aarch64/ffmpeg");
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const FFMPEG_BYTES: &[u8] = include_bytes!("../../vendor/ffmpeg/windows-x86_64/ffmpeg.exe");
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const FFMPEG_BYTES: &[u8] = b"placeholder";

fn vendored_sha256() -> String {
    let mut h = sha2::Sha256::new();
    h.update(FFMPEG_BYTES);
    format!("{:x}", h.finalize())
}

pub fn ensure_ffmpeg(data_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(data_dir).context("create data dir")?;
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let target = data_dir.join(exe);
    let sha = vendored_sha256();
    if target.exists() {
        let mut h = sha2::Sha256::new();
        h.update(&std::fs::read(&target)?);
        if format!("{:x}", h.finalize()) == sha {
            return Ok(target);
        }
    }
    std::fs::write(&target, FFMPEG_BYTES)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&target)?.permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&target, perm)?;
    }
    Ok(target)
}

// 注意：**没有** `write_concat_list`。concat 列表由 Task 8 的 `download_media` 生成
// （分片顺序是在那里决定的），转义也在那里完成。本模块只负责把已有的 `plan.list_file`
// 交给 ffmpeg。曾计划在此提供一个 `write_concat_list`，但它从头到尾没有调用方——
// 属于死代码，且会让"转义"这件事出现两个可能不同步的实现位置。

pub fn build_remux(
    ffmpeg: &Path,
    list: &Path,
    output: &Path,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(ffmpeg);
    cmd.arg("-y")
        .arg("-f").arg("concat")
        .arg("-safe").arg("0")
        .arg("-i").arg(list)
        .arg("-c").arg("copy");
    // 不要显式加 `-bsf:a aac_adtstoasc`。实测（ffmpeg 7.1.1）：
    //   AAC 的 TS 不加也能正确输出 h264+aac —— ffmpeg 会按需自动插入该滤镜；
    //   而 AC-3 的 TS 加了会直接失败："Codec 'ac3' is not supported by the bitstream
    //   filter 'aac_adtstoasc'"，整个合并就此中断。
    // 也就是说显式加它会让所有非 AAC 音频（AC-3/E-AC-3/MP3/Opus）的 HLS 流下载后合并失败，
    // 而不加则两种都正常。让 ffmpeg 自己决定。
    cmd.arg(output);
    // 用户取消任务时，`remux_to_mp4` 的 future 会被 drop，而 `cmd.output()` 内部持有的 Child
    // 一并被 drop。**必须开 `kill_on_drop`**：否则 ffmpeg 子进程会变成孤儿继续跑，而引擎在取消路径上
    // 会清掉任务目录（"取消是不要了"），于是孤儿进程还在往一个已被删除的目录里写。
    // 设置在 `build_remux` 而不是调用点：Task 13 调的是高层 `remux_to_mp4`，拿不到 Command，
    // 没有别的地方能设置它 —— 这是唯一正确的归属点。
    //
    // 故意不写测试：`tokio::process::Command` 没有读取该标志的 getter，无法断言。
    // 这是有据可查的"未测试"而非"忘了测"。
    cmd.kill_on_drop(true);
    cmd
}

pub async fn remux_to_mp4(
    ffmpeg: &Path,
    plan: &crate::core::hls::ConcatPlan,
    output: &Path,
) -> anyhow::Result<()> {
    let out = output.to_path_buf();
    // 必须 `mut`：`Command::output` 取 `&mut self`（原稿写 `let cmd` 编译不过）
    let mut cmd = build_remux(ffmpeg, &plan.list_file, &out);
    let output_status = cmd.output().await.context("spawn ffmpeg")?;
    if !output_status.status.success() {
        return Err(anyhow!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output_status.stderr)
        ));
    }
    Ok(())
}
```

注意：`include_bytes!` 路径相对源文件，Task 10 前需先创建 `vendor/ffmpeg/macos-aarch64/ffmpeg` 占位文件（任意内容），否则编译失败。

- [ ] **Step 4: 放置占位文件并运行测试**

```bash
mkdir -p vendor/ffmpeg/macos-aarch64 vendor/ffmpeg/windows-x86_64
printf 'fake-ffmpeg' > vendor/ffmpeg/macos-aarch64/ffmpeg
printf 'fake-ffmpeg' > vendor/ffmpeg/windows-x86_64/ffmpeg.exe
```

Run: `cargo test --test ffmpeg_test`
Expected: 2 PASS（占位文件满足提取/复用/权限逻辑测试；`build_remux` 与二进制无关。**原写 3 PASS 是陈旧计数**：DEFECT-21 删掉死函数 `write_concat_list` 时一并删了它的测试，但计数没跟着降；本任务的实际测试块始终只有 2 个）

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: ffmpeg 内嵌释放与转封装命令构造

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 10: vendor 真实 ffmpeg 静态二进制

**Files:**
- Modify: `vendor/ffmpeg/macos-aarch64/ffmpeg`（真实二进制）
- Modify: `vendor/ffmpeg/windows-x86_64/ffmpeg.exe`（真实二进制，如无法在本机交叉获取则记录下载命令与预期 SHA256 校验步骤）

**Interfaces:**
- Produces: 可执行 `ffmpeg -version` 的 vendored 二进制（至少本机 target 必须真实；其余 target 记录获取命令）

- [ ] **Step 1: 下载 macOS arm64 静态构建**

**注意（控制器已实测）**：该端点对 **HEAD 返回 404，只有 GET 正常**。所以不要用 `curl -I` 自检——你会看到误导性的 404 而以为链接失效。用下面的 GET：
已验证：`302 → /download/macos/arm64/<ts>_<ver>/ffmpeg.zip`，`200`，`content-length: 28395699`（≈28MB），且支持 Range（206）。

**该镜像慢且不稳定（控制器复测）**：1 字节 Range 请求**第一次 25 秒超时收了 0 字节**，重试才拿到
`206 content-range: bytes 0-0/28395699`。链接是活的，但 28MB 的下载不能用 25 秒的盖子——所以下面显式
加大 `--connect-timeout`/`--max-time`，失败就重试，而不是换源。

```bash
cd vendor/ffmpeg/macos-aarch64
curl -fSL --retry 5 --retry-all-errors --connect-timeout 30 --max-time 900 \
  -o ffmpeg.zip "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip"
unzip -o ffmpeg.zip
chmod +x ffmpeg
./ffmpeg -version | head -2
rm ffmpeg.zip
```

若 zip 内不止 `ffmpeg` 一个文件（可能含 `ffprobe`），只保留 `ffmpeg`，其余删除——`include_bytes!` 只引用这一个路径。

Expected: `ffmpeg version ...` 正常输出。实测解压后 **66332880 字节（≈63.3 MiB）**，低于原先估计的
70–120MB —— 该区间只是估计值，**以实测大小与 SHA256 为准**（见 `vendor/ffmpeg/README.md` 的表），
别因为"比预期小"就以为下载不全。

- [ ] **Step 2: 下载 Windows x86_64 构建（跨平台二进制，本机可存不可运行）**

**内部结构（控制器已实测）**：该 zip 为 `114768076` 字节，顶层目录名形如
`ffmpeg-9.0.2-essentials_build/`，可执行文件在 `ffmpeg-<ver>-essentials_build/bin/ffmpeg.exe`。
所以下面的 `"*/bin/ffmpeg.exe"` 通配是成立的：

```bash
cd ../windows-x86_64
curl -fSL --retry 5 --retry-all-errors --connect-timeout 30 --max-time 900 \
  -o ffmpeg.zip "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
unzip -o ffmpeg.zip "*/bin/ffmpeg.exe" -d tmp
mv tmp/*/bin/ffmpeg.exe ./ffmpeg.exe
rm -rf tmp ffmpeg.zip
ls -lh ffmpeg.exe  # 应为 70-120MB
```

控制器复测：该端点 `302 → /packages/ffmpeg-9.0.2-essentials_build.zip`，响应很快。当前版本是 **9.0.2**，
与下面假定的顶层目录名一致，`*/bin/ffmpeg.exe` 通配成立。114MB，同样需要足够的 `--max-time`。

若 gyan.dev 不可达，备选：`https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip`（取 `bin/ffmpeg.exe`）。

- [ ] **Step 3: 验证 include_bytes 路径与编译**

Run: `cargo check`
Expected: 通过（真实二进制替换占位，路径不变）

- [ ] **Step 4: 端到端验证转封装（用真实 ffmpeg 做一次 remux 冒烟）**

```bash
# 造两个假 ts 分片不行（非法 TS 会被 ffmpeg 拒绝）；直接验证二进制可运行即可：
./vendor/ffmpeg/macos-aarch64/ffmpeg -hide_banner -f lavfi -i testsrc=duration=1:size=320x240:rate=10 -f mpegts /tmp/t.ts
ls -lh /tmp/t.ts
```

Expected: 生成非空 /tmp/t.ts。

- [ ] **Step 5: Commit（大文件入库）**

```bash
cd /Users/xuwenqiang/Desktop/easyDownVideo
git add vendor/
git commit -m "chore: vendor ffmpeg 静态二进制（macOS arm64 / Windows x86_64）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

注：实测提交体积 **约 172MB**（macOS 66.3MB + Windows 105.4MB，原写"约 150MB"是估计值），属 spec 已
接受的权衡。校验值与各平台获取命令记录在 `vendor/ffmpeg/README.md`：

```markdown
# ffmpeg 静态构建来源
- macOS arm64: https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip
- Windows x86_64: https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip (取 bin/ffmpeg.exe)
- Linux amd64: https://ffmpeg.martin-riedl.de/redirect/latest/linux/amd64/release/ffmpeg.zip
- Linux arm64: https://ffmpeg.martin-riedl.de/redirect/latest/linux/arm64/release/ffmpeg.zip
- macOS x86_64: https://evermeet.cx/ffmpeg/ （下载后解压取 ffmpeg）
下载后放本目录对应 target 子目录，chmod +x（非 Windows），并 `ffmpeg -version` 验证。
```

---

### Task 11: task 模块（manifest 与断点恢复扫描）

**Files:**
- Create: `tests/task_manifest_test.rs`
- Modify: `src/core/task.rs`

**Interfaces:**
- Consumes: `events::TaskId`
- Produces:
  - `pub struct TaskManifest { pub id: TaskId, pub url: String, pub filename: String, pub download_dir: PathBuf, pub kind: ManifestKind }`
  - `pub enum ManifestKind { Direct { size: Option<u64> }, Hls { playlist_url: String } }`
  - `pub fn task_dir(download_root: &Path, id: TaskId) -> PathBuf`（`download_root/task-<id>/`）
  - `pub fn write_manifest(task_dir: &Path, m: &TaskManifest) -> anyhow::Result<()>`
  - `pub fn scan_resume(download_root: &Path) -> Vec<TaskManifest>`（遍历 `task-*/manifest.json`，能解析出的全部返回，由引擎决定哪些可续传）

- [ ] **Step 1: 写失败测试**

`tests/task_manifest_test.rs`：

```rust
use easydownvideo::core::task::{scan_resume, task_dir, write_manifest, ManifestKind, TaskManifest};

#[test]
fn manifest_roundtrips_and_scans() {
    let root = tempfile::tempdir().unwrap();
    let dir = task_dir(root.path(), 42);
    std::fs::create_dir_all(&dir).unwrap();
    let m = TaskManifest {
        id: 42,
        url: "https://example.com/v.mp4".into(),
        filename: "v.mp4".into(),
        download_dir: root.path().join("done"),
        kind: ManifestKind::Direct { size: Some(123) },
    };
    write_manifest(&dir, &m).unwrap();

    let found = scan_resume(root.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, 42);
    assert_eq!(found[0].filename, "v.mp4");
    match found[0].kind {
        ManifestKind::Direct { size } => assert_eq!(size, Some(123)),
        _ => panic!("expected Direct"),
    }
}

#[test]
fn scan_ignores_broken_and_done_dirs() {
    let root = tempfile::tempdir().unwrap();
    let d = task_dir(root.path(), 1);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("manifest.json"), "not json").unwrap();
    // 已完成任务目录里没有 manifest（引擎完成后删除），此处用空目录模拟
    std::fs::create_dir_all(task_dir(root.path(), 2)).unwrap();
    assert!(scan_resume(root.path()).is_empty());
}

#[test]
fn scan_ignores_dirs_without_task_prefix() {
    // 下载目录可以由用户改成 ~/Documents 这类地方，而 `manifest.json` 是极常见的文件名
    // （Web App Manifest / Chrome 扩展 / VS Code）。没有前缀过滤，这些目录会被当成可续传任务。
    let root = tempfile::tempdir().unwrap();

    // 一个**合法但非任务**的目录：manifest.json 内容完全能解析成 TaskManifest
    let alien = root.path().join("some-web-app");
    std::fs::create_dir_all(&alien).unwrap();
    std::fs::write(
        alien.join("manifest.json"),
        r#"{"id":7,"url":"https://x/v.mp4","filename":"v.mp4","download_dir":"/tmp","kind":{"Direct":{"size":null}}}"#,
    )
    .unwrap();

    // 同时放一个真任务，确认过滤没有把真任务也滤掉
    let real = task_dir(root.path(), 3);
    std::fs::create_dir_all(&real).unwrap();
    write_manifest(
        &real,
        &TaskManifest {
            id: 3,
            url: "https://example.com/v.mp4".into(),
            filename: "v.mp4".into(),
            download_dir: root.path().join("done"),
            kind: ManifestKind::Direct { size: None },
        },
    )
    .unwrap();

    let found = scan_resume(root.path());
    assert_eq!(
        found.len(),
        1,
        "只应认出 task-3；`some-web-app` 里的 manifest.json 再合法也不是任务。实际: {found:?}"
    );
    assert_eq!(found[0].id, 3);
}

#[test]
fn manifest_roundtrips_hls_kind() {
    // HLS 续传是核心功能（Task 8 就是为它写的），它的 kind 变体必须真的能往返，
    // 否则 serde 一旦标签写错，HLS 任务的断点恢复会静默失效 —— 而单测只测 Direct 是看不出来的。
    let root = tempfile::tempdir().unwrap();
    let dir = task_dir(root.path(), 9);
    std::fs::create_dir_all(&dir).unwrap();
    let m = TaskManifest {
        id: 9,
        url: "https://example.com/page".into(),
        filename: "video.mp4".into(),
        download_dir: root.path().join("done"),
        kind: ManifestKind::Hls { playlist_url: "https://cdn.example.com/master.m3u8".into() },
    };
    write_manifest(&dir, &m).unwrap();

    let found = scan_resume(root.path());
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].kind,
        ManifestKind::Hls { playlist_url: "https://cdn.example.com/master.m3u8".into() },
        "Hls 变体必须原样往返"
    );
    assert_eq!(found[0].kind, m.kind, "整体必须相等");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test task_manifest_test`
Expected: FAIL

- [ ] **Step 3: 实现**

`src/core/task.rs`：

```rust
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::events::TaskId;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskManifest {
    pub id: TaskId,
    pub url: String,
    pub filename: String,
    pub download_dir: PathBuf,
    pub kind: ManifestKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ManifestKind {
    Direct { size: Option<u64> },
    Hls { playlist_url: String },
}

pub fn task_dir(download_root: &Path, id: TaskId) -> PathBuf {
    download_root.join(format!("task-{id}"))
}

pub fn write_manifest(dir: &Path, m: &TaskManifest) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(m)?;
    std::fs::write(dir.join("manifest.json"), text)?;
    Ok(())
}

pub fn scan_resume(download_root: &Path) -> Vec<TaskManifest> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(download_root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // **必须过滤 `task-<id>` 前缀**。下载目录是**用户可配置**的（设置里可以改成 ~/Documents 之类），
        // 而 `manifest.json` 是个极常见的文件名 —— Web App Manifest、Chrome 扩展、VS Code 都在用。
        // 不按前缀过滤的话，用户目录下任何一个含 manifest.json 的子目录都会被当成"可续传任务"，
        // 于是在启动时凭空冒出一堆假任务，甚至去"续传"一个根本不是下载的目录。
        // 仅靠"JSON 能解析成 TaskManifest"挡不住：外部 manifest 可能恰好字段同名而解析通过。
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("task-") {
            continue;
        }
        let manifest_path = path.join("manifest.json");
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        if let Ok(m) = serde_json::from_str::<TaskManifest>(&text) {
            out.push(m);
        }
    }
    out.sort_by_key(|m| m.id);
    out
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test task_manifest_test`
Expected: 4 PASS（原有 2 个 + 本轮补的 `scan_ignores_dirs_without_task_prefix`、`manifest_roundtrips_hls_kind`）

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 任务 manifest 与断点恢复扫描

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 12: ytdlp 模块（检测与格式解析）

**Files:**
- Create: `tests/fixtures/ytdlp_dump.json`
- Create: `tests/ytdlp_test.rs`
- Modify: `src/core/ytdlp.rs`

**Interfaces:**
- Produces:
  - `pub fn find_ytdlp() -> Option<PathBuf>`（取 `PATH` 环境变量并交 `find_ytdlp_in`；Unix 依次找 `yt-dlp`、`youtube-dl`，Windows 依次找 `yt-dlp.exe`、`youtube-dl.exe`、`yt-dlp`、`youtube-dl` —— **以 Step 4 代码为准**，此前的散文描述与代码顺序不一致）
  - `pub fn find_ytdlp_in(path_var: impl AsRef<std::ffi::OsStr>) -> Option<PathBuf>`（纯函数，扫描传入的 PATH 字符串；拆出来是为了可测，避免测试改全局环境变量）
  - `pub fn parse_formats(json: &serde_json::Value) -> Vec<YtdlpFormat>`（只保留带 `url` 且有视频流的格式，`note` 取 `format_note` 或 `resolution`）
  - `pub async fn fetch_formats(exe: &Path, page_url: &str) -> anyhow::Result<Vec<YtdlpFormat>>`（`exe -J url`，超时 120s）

- [ ] **Step 1: fixture（`-J` 输出摘录）**

`tests/fixtures/ytdlp_dump.json`：

> 字段是**刻意**这么摆的，不要"整理"掉：22 号格式用 `filesize: null` + `filesize_approx`（真实 yt-dlp
> 的常见形态，用来钉住 null-vs-缺失 的回退），hls-1 **不给** `format_note`（用来钉住回退到 `resolution`）。
> 少了这两处，两个回退分支就都没有测试覆盖 —— 而这正是缺陷藏身之处。

```json
{
  "id": "abc123",
  "title": "demo",
  "formats": [
    { "format_id": "18", "ext": "mp4", "resolution": "640x360", "format_note": "360p", "filesize": 1000000, "vcodec": "avc1.42001E", "url": "https://cdn.example.com/v/18.mp4" },
    { "format_id": "22", "ext": "mp4", "resolution": "1280x720", "format_note": "720p", "filesize": null, "filesize_approx": 2000000, "vcodec": "avc1.64001F", "url": "https://cdn.example.com/v/22.mp4" },
    { "format_id": "139", "ext": "m4a", "resolution": null, "format_note": "audio", "filesize": 100000, "vcodec": "none", "url": "https://cdn.example.com/a/139.m4a" },
    { "format_id": "hls-1", "ext": "mp4", "resolution": "1920x1080", "protocol": "m3u8_native", "vcodec": "avc1.640028", "url": "https://cdn.example.com/hls/master.m3u8" }
  ]
}
```

- [ ] **Step 2: 写失败测试**

`tests/ytdlp_test.rs`：

```rust
use easydownvideo::core::ytdlp::{find_ytdlp_in, parse_formats};

#[test]
fn parse_formats_keeps_video_entries() {
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ytdlp_dump.json")).unwrap()
    ).unwrap();
    let formats = parse_formats(&json);
    // vcodec=none 的纯音频被剔除
    assert_eq!(formats.len(), 3);
    assert_eq!(formats[0].id, "18");
    assert_eq!(formats[0].note, "360p");
    assert_eq!(formats[0].filesize, Some(1_000_000));
    // 22 号格式是 `"filesize": null` + `"filesize_approx": 2000000` —— 真实 yt-dlp 就这么输出。
    // 必须回退到 approx；写成 `assert_eq!(filesize, None)` 等于把 bug 当成期望值钉死。
    assert_eq!(
        formats[1].filesize,
        Some(2_000_000),
        "filesize 为 null 时必须回退到 filesize_approx（or_else 不会被 Some(Null) 触发）"
    );
    // hls-1 没有 format_note，note 必须回退到 resolution
    assert_eq!(formats[2].note, "1920x1080", "format_note 缺失时应回退到 resolution");
    assert!(formats.iter().all(|f| f.url.starts_with("https://")));
}

#[test]
fn parse_formats_tolerates_missing_fields() {
    let json = serde_json::json!({ "formats": [ { "format_id": "x" } ] });
    assert!(parse_formats(&json).is_empty());
}

#[test]
fn find_ytdlp_scans_path_in_order() {
    // **不能**写成 `let _ = find_ytdlp();` —— 那是断言不了任何东西的空测试。
    // PATH 扫描必须测真行为：跳过没有该文件的目录、在后面的目录里找到、找不到返回 None。
    // 把 PATH 字符串作为参数传入纯函数，测试才能在不改全局环境（会与并行测试打架）的前提下驱动它。
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let target = b.path().join("bin");
    std::fs::create_dir_all(&target).unwrap();
    let exe = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
    std::fs::write(target.join(exe), b"stub").unwrap();

    // 前面的目录里没有该文件，必须继续往后找
    let path_var = std::env::join_paths([a.path(), &target]).unwrap();
    let expected = target.join(exe);
    assert_eq!(
        find_ytdlp_in(&path_var).as_deref(),
        Some(expected.as_path()),
        "应跳过不含该文件的目录，在后面的目录里找到"
    );

    // 完全没有该文件 -> None（而不是 panic）
    let none_var = std::env::join_paths([a.path()]).unwrap();
    assert_eq!(find_ytdlp_in(&none_var), None);
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test --test ytdlp_test`
Expected: FAIL

- [ ] **Step 4: 实现**

`src/core/ytdlp.rs`（在 Task 3 结构体基础上追加）：

```rust
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YtdlpFormat {
    pub id: String,
    pub ext: String,
    pub note: String,
    pub filesize: Option<u64>,
    pub url: String,
}

pub fn find_ytdlp() -> Option<PathBuf> {
    find_ytdlp_in(std::env::var_os("PATH")?)
}

/// 纯函数：扫描给定的 PATH 字符串。拆出来是为了可测 —— 直接驱动 `find_ytdlp()` 就得改全局
/// 环境变量，那会与并行跑的其它测试互相干扰（env 是进程级的）。
pub fn find_ytdlp_in(path_var: impl AsRef<std::ffi::OsStr>) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["yt-dlp.exe", "youtube-dl.exe", "yt-dlp", "youtube-dl"]
    } else {
        &["yt-dlp", "youtube-dl"]
    };
    // 注意是 `&path_var`：`std::env::split_paths` 取 `&T`，传值会 E0308
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn parse_formats(json: &serde_json::Value) -> Vec<YtdlpFormat> {
    let mut out = Vec::new();
    let Some(formats) = json.get("formats").and_then(|f| f.as_array()) else {
        return out;
    };
    for f in formats {
        let vcodec = f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("");
        if vcodec == "none" {
            continue; // 纯音频
        }
        let Some(url) = f.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        let note = f
            .get("format_note")
            .and_then(|v| v.as_str())
            .or_else(|| f.get("resolution").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        out.push(YtdlpFormat {
            id: f.get("format_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            ext: f.get("ext").and_then(|v| v.as_str()).unwrap_or("mp4").to_string(),
            note,
            // 必须**先 `and_then` 再 `or_else`**。真实 yt-dlp 对很多格式输出的是
            // `"filesize": null`，此时 `f.get("filesize")` 返回的是 `Some(Value::Null)` 而**不是**
            // `None`，于是 `or_else` 永远不触发，`as_u64()` 在 Null 上得到 None ——
            // `filesize_approx` 被白白丢掉，而且恰好发生在最需要它的时候。
            filesize: f
                .get("filesize")
                .and_then(|v| v.as_u64())
                .or_else(|| f.get("filesize_approx").and_then(|v| v.as_u64())),
            url: url.to_string(),
        });
    }
    out
}

pub async fn fetch_formats(exe: &Path, page_url: &str) -> anyhow::Result<Vec<YtdlpFormat>> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new(exe)
            .arg("-J")
            .arg(page_url)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("yt-dlp 解析超时（120s）"))??;
    if !out.status.success() {
        return Err(anyhow::anyhow!(
            "yt-dlp 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    Ok(parse_formats(&json))
}
```

- [ ] **Step 5: 运行确认通过**

Run: `cargo test --test ytdlp_test`
Expected: 3 PASS

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: yt-dlp 检测与格式解析

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 13: engine 模块（任务队列、调度、端到端）

**Files:**
- Create: `tests/engine_test.rs`
- Create: `tests/fixtures/seg.ts`（真实 MPEG-TS 小分片，160x120 h264 1 秒约 10KB；
  `tests/fixtures/` 下已有 Task 5 的 m3u8 fixture。生成命令见 Step 1 末尾）
- Modify: `src/core/engine.rs`

**Interfaces:**
- Consumes: 以上全部（probe、direct、hls、ffmpeg、ytdlp、task、events、settings）
- Produces:
  - `pub struct Engine`（`Clone`）
  - `Engine::new(rt: tokio::runtime::Handle, settings: Settings, data_dir: PathBuf) -> Engine`
  - `add_url(&self, url: String) -> TaskId`
  - `resume(&self, m: TaskManifest)`（扫盘恢复）
  - `pause(&self, id: TaskId)` / `unpause(&self, id: TaskId)` / `retry(&self, id: TaskId)` / `cancel(&self, id: TaskId)`
    （`retry` 接受 `Failed` 状态重新入队；GUI 的"重试"按钮用它。`unpause` 只认 `Paused`，两者不能互相顶替）
  - `set_choice(&self, id: TaskId, choice: Choice)`
  - `set_filename(&self, id: TaskId, name: String)`（GUI 重命名）
  - `events(&self) -> tokio::sync::broadcast::Receiver<EngineEvent>`
  - `task_info(&self, id: TaskId) -> Option<(String url, String filename)>`
  - `list_tasks(&self) -> Vec<TaskSnapshot>`，`pub struct TaskSnapshot { pub id: TaskId, pub url: String, pub filename: String, pub state: TaskState, pub progress: ProgressHandle }`（供 GUI 启动时水合，尤其是 `resume` 恢复的任务）
  - `update_settings(&self, s: Settings)` / `current_settings(&self) -> Settings`（GUI 改设置后立即生效于新启动的任务，无需重启）

**实现要点：** `Inner` 持 `Mutex<HashMap<TaskId, TaskEntry>>`（entry 含 `TaskState`、`CancellationToken`、`pending_choice: Option<ChoiceKind>`）与 `Mutex<VecDeque<TaskId>>` 排队顺序；每次状态变化后 `pump()`：活跃数 < `max_active_tasks` 时从队首启动 worker。worker 流程：probe → Direct 直接下载；Hls 拉 playlist → Master 则 `AwaitingChoice` + `ChoicesOffered` 事件 → 收到 `set_choice` 后拉 media playlist 继续 → 下完 `Merging` → ffmpeg remux → `Done`。下载循环内每个分块/分片失败重试 3 次（退避 1s/2s/4s），最终失败才标任务失败——单任务失败不阻塞队列（worker 结束即 pump 下一个）。进度转发：worker 内起一个 300ms 定时任务读 `ProgressHandle.snapshot()` 发 `EngineEvent::Progress`。

- [ ] **Step 1: 写失败测试**

`tests/engine_test.rs`：

```rust
use easydownvideo::core::engine::Engine;
use easydownvideo::core::events::{Choice, EngineEvent, TaskState};
use easydownvideo::core::task::{ManifestKind, TaskManifest};
use easydownvideo::settings::Settings;
use wiremock::{matchers::{header, method, path}, Mock, MockServer, ResponseTemplate};

fn settings_with(dir: &std::path::Path) -> Settings {
    Settings {
        download_dir: dir.to_path_buf(),
        segment_concurrency: 2,
        max_active_tasks: 1,
        ytdlp_enabled: false,
        ffmpeg_path_override: None,
    }
}

async fn wait_failed(
    rx: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    id: u64,
    secs: u64,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let ev = tokio::time::timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        ).await.expect("timeout waiting for failure");
        if let Ok(EngineEvent::StateChanged { id: eid, state: TaskState::Failed(_) }) = ev {
            if eid == id { return; }
        }
    }
}

async fn wait_state(
    rx: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    id: u64,
    want: TaskState,
    secs: u64,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let ev = tokio::time::timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        ).await.expect("timeout waiting for state");
        match ev {
            Ok(EngineEvent::StateChanged { id: eid, state }) if eid == id => {
                if state == want { return; }
                if let TaskState::Failed(_) = state {
                    panic!("task failed while waiting for {want:?}: {state:?}");
                }
            }
            _ => continue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn end_to_end_direct_download() {
    let data: Vec<u8> = (0..2048u32).map(|i| (i % 253) as u8).collect();
    let server = MockServer::start().await;
    // probe 请求
    Mock::given(method("GET")).and(path("/v.mp4")).and(header("range", "bytes=0-63"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-type", "video/mp4")
            .insert_header("content-range", "bytes 0-63/2048")
            .set_body_bytes(data[0..64].to_vec()))
        .mount(&server).await;
    // 引擎用 MIN_BLOCK_SIZE(4MB)，2048 字节的小文件只会切出 1 个块 → 单次全量 range 请求
    Mock::given(method("GET")).and(path("/v.mp4")).and(header("range", "bytes=0-2047"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-type", "video/mp4")
            .insert_header("content-range", "bytes 0-2047/2048")
            .set_body_bytes(data.clone()))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let url = format!("{}/v.mp4", server.uri());
    let id = engine.add_url(url);

    wait_state(&mut rx, id, TaskState::Done, 30).await;
    let saved = dir.path().join("v.mp4");
    assert_eq!(std::fs::read(&saved).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failure_does_not_block_queue() {
    let server = MockServer::start().await;
    // 坏 URL 直接 404
    Mock::given(method("GET")).and(path("/missing.mp4"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server).await;
    // 好 URL 可完整下载（512 字节 → 1 个块 → 单次全量 range）
    let data: Vec<u8> = (0..512u32).map(|i| (i % 251) as u8).collect();
    Mock::given(method("GET")).and(path("/v.mp4")).and(header("range", "bytes=0-63"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-type", "video/mp4")
            .insert_header("content-range", "bytes 0-63/512")
            .set_body_bytes(data[0..64].to_vec()))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v.mp4")).and(header("range", "bytes=0-511"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-type", "video/mp4")
            .insert_header("content-range", "bytes 0-511/512")
            .set_body_bytes(data.clone()))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let bad = engine.add_url(format!("{}/missing.mp4", server.uri()));
    let good = engine.add_url(format!("{}/v.mp4", server.uri()));

    // 坏任务先失败（max_active=1，它占着槽位）
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let ev = tokio::time::timeout(
            deadline.saturating_duration_since(std::time::Instant::now()),
            rx.recv(),
        ).await.expect("timeout");
        if let Ok(EngineEvent::StateChanged { id, state: TaskState::Failed(_) }) = ev {
            if id == bad { break; }
        }
    }
    // 好任务随后照常完成，证明队列未被失败阻塞
    wait_state(&mut rx, good, TaskState::Done, 30).await;
    assert!(dir.path().join("v.mp4").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn existing_file_gets_renamed() {
    let data: Vec<u8> = (0..128u32).map(|i| i as u8).collect();
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v.mp4")).and(header("range", "bytes=0-63"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-type", "video/mp4")
            .insert_header("content-range", "bytes 0-63/128")
            .set_body_bytes(data[0..64].to_vec()))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v.mp4"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "video/mp4")
            .set_body_bytes(data.clone()))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("v.mp4"), b"existing").unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/v.mp4", server.uri()));
    wait_state(&mut rx, id, TaskState::Done, 30).await;
    assert_eq!(std::fs::read(dir.path().join("v.mp4")).unwrap(), b"existing");
    assert!(dir.path().join("v (1).mp4").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_settings_changes_concurrency() {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut s = settings_with(dir.path());
    s.segment_concurrency = 12;
    s.ytdlp_enabled = false;
    engine.update_settings(s);
    assert_eq!(engine.current_settings().segment_concurrency, 12);
    assert!(!engine.current_settings().ytdlp_enabled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_cleans_resume_artifacts() {
    // 取消是"不要了"：断点目录必须清掉，否则下次启动 scan_resume 会把它复活重下
    let server = MockServer::start().await;
    let data: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();
    Mock::given(method("GET")).and(path("/v.mp4")).and(header("range", "bytes=0-63"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-type", "video/mp4")
            .insert_header("content-range", "bytes 0-63/4096")
            .set_body_bytes(data[0..64].to_vec()))
        .mount(&server).await;
    // 全量请求延迟返回，留出取消窗口
    Mock::given(method("GET")).and(path("/v.mp4"))
        .respond_with(ResponseTemplate::new(206)
            .insert_header("content-type", "video/mp4")
            .insert_header("content-range", "bytes 0-4095/4096")
            .set_body_bytes(data.clone())
            .set_delay(std::time::Duration::from_millis(1500)))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/v.mp4", server.uri()));

    // 进入 Downloading 说明 manifest 已落盘
    wait_state(&mut rx, id, TaskState::Downloading, 10).await;
    engine.cancel(id);
    wait_state(&mut rx, id, TaskState::Canceled, 10).await;

    // 清理发生在 worker 退出路径，轮询等待（避免依赖固定 sleep 时长）
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if easydownvideo::core::task::scan_resume(dir.path()).is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "取消后仍残留可恢复的 manifest，重启会把已取消任务复活"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!easydownvideo::core::task::task_dir(dir.path(), id).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumed_ids_do_not_collide_with_new_tasks() {
    // 恢复的任务带着它原来的 id，而 add_url 从 1 开始分配。不抬高 next_id，
    // 新增任务会拿到已被占用的 id，insert_entry 直接把它覆盖 —— 恢复的任务无声消失。
    //
    // max_active_tasks = 0 让任何 worker 都不启动：这个测试只关心 id 分配，
    // 不该联网，也不该受下载时序影响。
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s = settings_with(dir.path());
    s.max_active_tasks = 0;
    let engine = Engine::new(rt, s, dir.path().join("data"));

    engine.resume(TaskManifest {
        id: 7,
        url: "https://example.com/a.mp4".into(),
        filename: "a.mp4".into(),
        download_dir: dir.path().to_path_buf(),
        kind: ManifestKind::Direct { size: Some(10) },
    });

    let fresh = engine.add_url("https://example.com/b.mp4".into());
    assert_ne!(fresh, 7, "add_url 分配到了已被恢复任务占用的 id");
    assert_eq!(
        engine.task_info(7).map(|(_, f)| f).as_deref(),
        Some("a.mp4"),
        "恢复出来的任务被新增任务覆盖了"
    );
    assert_eq!(engine.list_tasks().len(), 2, "两个任务都应存在");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_while_queued_is_not_resurrected() {
    // 取消发生在"排队期间"时，id 仍留在 VecDeque 里。pump 启动任务前必须检查状态，
    // 否则用户点了取消，任务随后自己复活并开始下载。
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s0 = settings_with(dir.path());
    s0.max_active_tasks = 0; // 先让任务停在队列里
    let engine = Engine::new(rt, s0, dir.path().join("data"));

    let id = engine.add_url("https://example.com/x.mp4".into());
    engine.cancel(id);

    // 提高并发上限触发 pump —— 这是会把它复活的那条路径
    let mut s1 = settings_with(dir.path());
    s1.max_active_tasks = 1;
    engine.update_settings(s1);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let snap = engine
        .list_tasks()
        .into_iter()
        .find(|t| t.id == id)
        .expect("任务应仍在列表中");
    assert_eq!(
        snap.state,
        TaskState::Canceled,
        "被取消的排队任务被 pump 复活了（状态变成 {:?}）",
        snap.state
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retry_requeues_a_failed_task() {
    // 失败的"重试"必须真的重新跑一遍，而不是只把状态字段改一改、或者干脆没反应。
    // 用一个恒定 404 的 URL：重试成功的话会**再次**失败，于是能等到第二个 Failed 事件。
    // （GUI 的"重试"按钮曾经调的是 unpause，而 unpause 只认 Paused —— 死按钮。）
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/missing.mp4"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/missing.mp4", server.uri()));

    wait_failed(&mut rx, id, 30).await; // 第一次失败
    engine.retry(id);
    wait_failed(&mut rx, id, 30).await; // 重试后第二次失败 —— 证明它真的重跑了
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_queued_resumed_task_removes_its_manifest() {
    // `resume()` 会**立刻**把 manifest 写盘（insert_entry 的 eager write）。若这个任务因为并发
    // 槽位已满一直排队（max_active=0），就没有任何 worker 会跑收尾清理 —— 清理原先只写在
    // worker 的错误路径里。cancel 自己不删目录的话，manifest 留在盘上，下次启动就被复活。
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s = settings_with(dir.path());
    s.max_active_tasks = 0;
    let engine = Engine::new(rt, s, dir.path().join("data"));

    let id = 5u64;
    engine.resume(TaskManifest {
        id,
        url: "https://example.com/a.mp4".into(),
        filename: "a.mp4".into(),
        download_dir: dir.path().to_path_buf(),
        kind: ManifestKind::Direct { size: Some(10) },
    });
    // **前提断言**：manifest 确实已经落盘。没有这一条，下面的断言可能因为"压根没写过"而假通过。
    assert_eq!(
        easydownvideo::core::task::scan_resume(dir.path()).len(),
        1,
        "resume 应当立刻写盘；否则这个测试不成立"
    );

    engine.cancel(id);

    assert!(
        easydownvideo::core::task::scan_resume(dir.path()).is_empty(),
        "取消排队中的恢复任务后 manifest 仍在盘上，下次启动会被 scan_resume 复活"
    );
    assert!(!easydownvideo::core::task::task_dir(dir.path(), id).exists());
}

/// 引擎的 HLS 接线（playlist → 分片 → ffmpeg 合并 → Done → 清目录）此前**完全没有测试**，
/// 只做到"能编译"。假 `.ts` 字节不行：ffmpeg 的 concat 解复用器会拒绝非法 TS，合并直接失败。
/// 所以用 `tests/fixtures/seg.ts` —— 一个真实的小分片（160x120 h264，1 秒，约 10KB）。
fn ts_fixture() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/seg.ts"))
        .expect("缺少 tests/fixtures/seg.ts")
}

const MEDIA_PLAYLIST: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:1.0,\nseg0.ts\n#EXTINF:1.0,\nseg1.ts\n#EXT-X-ENDLIST\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_media_playlist_end_to_end() {
    let ts = ts_fixture();
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/media.m3u8"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "application/vnd.apple.mpegurl")
            .set_body_string(MEDIA_PLAYLIST))
        .mount(&server).await;
    for seg in ["/seg0.ts", "/seg1.ts"] {
        Mock::given(method("GET")).and(path(seg))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("content-type", "video/mp2t")
                .set_body_bytes(ts.clone()))
            .mount(&server).await;
    }

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/media.m3u8", server.uri()));

    // 合并要跑真实 ffmpeg（首次会把内嵌二进制释放到 data_dir），给足时间
    wait_state(&mut rx, id, TaskState::Done, 120).await;

    let out = dir.path().join("media.mp4");
    let meta = std::fs::metadata(&out)
        .unwrap_or_else(|e| panic!("合并产物 {} 应当存在: {e}", out.display()));
    assert!(meta.len() > 0, "合并产物不应为空");
    assert!(
        !easydownvideo::core::task::task_dir(dir.path(), id).exists(),
        "完成后任务目录（manifest/分片/concat 列表）必须清掉，否则会被 scan_resume 复活"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_master_playlist_offers_choice_then_completes() {
    // 真实世界的 m3u8 大多是 master playlist：先停在 AwaitingChoice 等用户选码率，
    // 再经 set_choice 换 URL 重新入队。这条接线（含 set_choice）同样没有测试。
    let ts = ts_fixture();
    let server = MockServer::start().await;
    let master = "#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=160x120\nmedia.m3u8\n";
    let media = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:1.0,\nseg0.ts\n#EXT-X-ENDLIST\n";
    Mock::given(method("GET")).and(path("/master.m3u8"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "application/vnd.apple.mpegurl")
            .set_body_string(master))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/media.m3u8"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "application/vnd.apple.mpegurl")
            .set_body_string(media))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/seg0.ts"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "video/mp2t")
            .set_body_bytes(ts.clone()))
        .mount(&server).await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();
    let id = engine.add_url(format!("{}/master.m3u8", server.uri()));

    // 先停在等待选择
    wait_state(&mut rx, id, TaskState::AwaitingChoice, 30).await;
    // 选 0 号（唯一的）码率 —— 这一步走 set_choice 的重新入队
    engine.set_choice(id, Choice::Variant(0));
    wait_state(&mut rx, id, TaskState::Done, 120).await;

    assert!(
        dir.path().join("master.mp4").exists(),
        "选择码率后应当合并出 master.mp4"
    );
}
```

注（原文有误，已更正）：`failure_does_not_block_queue` 里 good 任务的双 range mock **已经在上面写好了**（文件 512 字节，`content-range: bytes 0-511/512`），不需要另行补齐；原文说"执行时补齐、大小取 2048"，与该测试实际内容矛盾，是残留草稿。

**mock 必须显式匹配，不要依赖"漏网落到兜底 mock"。** wiremock 只按你声明的 matcher 匹配；某个 range 请求若没被任何 mock 命中会 404，任务就失败在 probe 上。若实测发现某个 range 请求与预期不符，**报告实际发出的 range 值**再调整 mock —— 不要放宽断言或改成"任何请求都返回全量"来让它变绿。

**`tests/fixtures/seg.ts` 的生成命令**（控制器已用 vendor 的 ffmpeg 生成并入库，正常无需重跑；
只有在缺少该文件时才需要）：

```bash
./vendor/ffmpeg/macos-aarch64/ffmpeg -hide_banner -loglevel error \
  -f lavfi -i testsrc=duration=1:size=160x120:rate=10 \
  -c:v libx264 -pix_fmt yuv420p -f mpegts tests/fixtures/seg.ts -y
```

实测产物 10.1KB、h264 High 160x120 10fps、时长 1 秒 —— 既能被 ffmpeg 正常 concat + `-c copy` 进 mp4，
又小到可以入库。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test engine_test`
Expected: FAIL（`Engine` 未实现）

- [ ] **Step 3: 实现**

`src/core/engine.rs`（核心骨架，约 300 行；执行时补全编译细节）：

```rust
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::core::direct::{download_direct, unique_path, MIN_BLOCK_SIZE};
use crate::core::events::{Choice, ChoiceKind, EngineEvent, ProgressHandle, TaskId, TaskState};
use crate::core::ffmpeg::{ensure_ffmpeg, remux_to_mp4};
use crate::core::hls::{download_media, parse::parse_playlist, parse::ParsedPlaylist};
use crate::core::probe::{filename_from_url, probe, ProbeOutcome};
use crate::core::task::{task_dir, write_manifest, ManifestKind, TaskManifest};
use crate::core::ytdlp::{fetch_formats, find_ytdlp};
use crate::settings::Settings;

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

#[derive(Clone)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub url: String,
    pub filename: String,
    pub state: TaskState,
    pub progress: ProgressHandle,
}

struct Inner {
    rt: Handle,
    client: reqwest::Client,
    settings: Mutex<Settings>,
    tasks: Mutex<HashMap<TaskId, TaskEntry>>,
    queue: Mutex<VecDeque<TaskId>>,
    tx: broadcast::Sender<EngineEvent>,
    next_id: AtomicU64,
    data_dir: PathBuf,
}

#[derive(Clone)]
struct TaskEntry {
    url: String,
    filename: String,
    state: TaskState,
    cancel: CancellationToken,
    pending_choice: Option<ChoiceKind>,
    progress: ProgressHandle,
}

impl TaskEntry {
    fn is_active(&self) -> bool {
        matches!(
            self.state,
            TaskState::Probing
                | TaskState::Downloading
                | TaskState::Merging
                | TaskState::AwaitingChoice
        )
    }
}

impl Engine {
    pub fn new(rt: Handle, settings: Settings, data_dir: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Inner {
                rt,
                client: reqwest::Client::builder()
                    .user_agent("easydownvideo/0.1")
                    .build()
                    .expect("build http client"),
                settings: Mutex::new(settings),
                tasks: Mutex::new(HashMap::new()),
                queue: Mutex::new(VecDeque::new()),
                tx,
                next_id: AtomicU64::new(1),
                data_dir,
            }),
        }
    }

    pub fn events(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.tx.subscribe()
    }

    pub fn task_info(&self, id: TaskId) -> Option<(String, String)> {
        let tasks = self.inner.tasks.lock().unwrap();
        tasks.get(&id).map(|t| (t.url.clone(), t.filename.clone()))
    }

    pub fn update_settings(&self, s: Settings) {
        *self.inner.settings.lock().unwrap() = s;
        self.pump(); // 并发上限提高时可立即启动排队中的任务
    }

    pub fn current_settings(&self) -> Settings {
        self.inner.settings.lock().unwrap().clone()
    }

    pub fn list_tasks(&self) -> Vec<TaskSnapshot> {
        let mut out: Vec<TaskSnapshot> = self
            .inner
            .tasks
            .lock()
            .unwrap()
            .iter()
            .map(|(id, t)| TaskSnapshot {
                id: *id,
                url: t.url.clone(),
                filename: t.filename.clone(),
                state: t.state.clone(),
                progress: t.progress.clone(),
            })
            .collect();
        out.sort_by_key(|s| s.id);
        out
    }

    pub fn add_url(&self, url: String) -> TaskId {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        // 复用 probe 里的唯一实现，不要在这里再抄一份推导逻辑
        let filename = filename_from_url(&url);
        self.insert_entry(id, url, filename, None);
        id
    }

    pub fn resume(&self, m: TaskManifest) {
        // **必须抬高 next_id**：恢复的任务带着它原来的 id，而 `add_url` 是从 1 开始递增分配的。
        // 不抬高的话，重启后先 resume 出 id=1..N 的任务，用户随即新增一个 URL 又拿到 id=1，
        // `insert_entry` 的 `tasks.insert` 会把这个恢复出来的任务**直接覆盖掉** ——
        // 用户看到的是"我刚加了个任务，上一个任务凭空消失了"。
        // 这不是边角情况：它发生在每次重启后的第一次新增。
        self.inner.next_id.fetch_max(m.id + 1, Ordering::SeqCst);
        // 先把字段取出来再整体移动 m：`insert_entry(m.id, m.url, m.filename, Some(m))`
        // 会先移走 `m.url`/`m.filename` 再移动整个 `m`，触发 E0382（借用已移动的值）。
        let (id, url, filename) = (m.id, m.url.clone(), m.filename.clone());
        self.insert_entry(id, url, filename, Some(m));
    }

    pub fn set_filename(&self, id: TaskId, name: String) {
        let mut tasks = self.inner.tasks.lock().unwrap();
        if let Some(t) = tasks.get_mut(&id) {
            if !name.trim().is_empty() {
                t.filename = name.trim().to_string();
            }
        }
    }

    pub fn pause(&self, id: TaskId) {
        let entry = { self.inner.tasks.lock().unwrap().get(&id).cloned() };
        if let Some(e) = entry {
            // 排队中的任务也要能暂停（用户对着队列里等待的任务点暂停是正常操作）。
            // 标记为 Paused 后，pump 的分支闸会因为状态不是 Queued 而跳过它。
            if matches!(e.state, TaskState::Queued | TaskState::Downloading) {
                e.cancel.cancel();
                self.set_state(id, TaskState::Paused);
            }
        }
    }

    pub fn unpause(&self, id: TaskId) {
        let was_paused = {
            let mut tasks = self.inner.tasks.lock().unwrap();
            match tasks.get_mut(&id) {
                Some(t) if t.state == TaskState::Paused => {
                    t.state = TaskState::Queued;
                    t.cancel = CancellationToken::new();
                    true
                }
                _ => false,
            }
        };
        if was_paused {
            self.inner.queue.lock().unwrap().push_back(id);
            self.pump();
        }
    }

    /// 重试：把失败的任务重新排队。
    ///
    /// 与 `unpause` 的区别是它接受 `Failed` 状态 —— GUI 的"重试"按钮直接调这个方法。
    /// （原先 GUI 调的是 `unpause`，而 `unpause` 只认 `Paused`，于是失败任务的"重试"
    /// 按钮点了毫无反应：不报错、不变化，纯粹是个死按钮。）
    ///
    /// 清掉 `pending_choice`：失败可能发生在等待选择的阶段，重试应当重新探测，
    /// 而不是沿用上一次那份已经过期的候选列表。
    pub fn retry(&self, id: TaskId) {
        let ok = {
            let mut tasks = self.inner.tasks.lock().unwrap();
            match tasks.get_mut(&id) {
                Some(t) if matches!(t.state, TaskState::Failed(_)) => {
                    t.state = TaskState::Queued;
                    t.cancel = CancellationToken::new();
                    t.pending_choice = None;
                    true
                }
                _ => false,
            }
        };
        if ok {
            self.inner.queue.lock().unwrap().push_back(id);
            self.pump();
        }
    }

    pub fn cancel(&self, id: TaskId) {
        let entry = { self.inner.tasks.lock().unwrap().get(&id).cloned() };
        let Some(e) = entry else { return };
        e.cancel.cancel();
        // 是否有 worker 正在跑这个任务？只有 `run_task` 还在途中的三个状态才算。
        // `AwaitingChoice` **不算** —— 那时 `run_task` 已经返回 Ok，不会再有收尾逻辑执行。
        let worker_running = matches!(
            e.state,
            TaskState::Probing | TaskState::Downloading | TaskState::Merging
        );
        self.set_state(id, TaskState::Canceled);
        // **没有 worker 就必须在这里自己清目录。**
        //
        // 清理原先只写在 worker 的错误收尾路径里，于是"没有 worker 的任务"被取消后，
        // manifest 会留在盘上，下次启动 scan_resume 把它复活并重新下载 —— 正是全局约束
        // 明令禁止的那件事。最容易踩到的就是 `resume()` 恢复出来的任务：它在 insert_entry
        // 里**立刻**写盘，若因为并发槽位已满而一直排队，用户取消时压根没有任何 worker 见过它。
        // （`cancel_while_queued_is_not_resurrected` 覆盖不到这一条：它取消的任务来自
        //   `add_url`，manifest 是 None，盘上本来就没有东西可泄漏。）
        if !worker_running {
            let root = self.current_settings().download_dir;
            std::fs::remove_dir_all(task_dir(&root, id)).ok();
        }
    }

    fn state_of(&self, id: TaskId) -> Option<TaskState> {
        self.inner.tasks.lock().unwrap().get(&id).map(|t| t.state.clone())
    }

    pub fn set_choice(&self, id: TaskId, choice: Choice) {
        let pending = {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let Some(t) = tasks.get_mut(&id) else { return };
            t.pending_choice.take()
        };
        let Some(kind) = pending else { return };
        let url = match (kind, choice) {
            (ChoiceKind::HlsVariants(v), Choice::Variant(i)) => v.get(i).map(|v| v.uri.clone()),
            (ChoiceKind::YtdlpFormats(f), Choice::YtdlpFormat(i)) => f.get(i).map(|f| f.url.clone()),
            _ => None,
        };
        let Some(url) = url else { return };
        if let Ok(mut tasks) = self.inner.tasks.lock() {
            if let Some(t) = tasks.get_mut(&id) {
                t.url = url;
                t.state = TaskState::Queued;
            }
        }
        self.inner.queue.lock().unwrap().push_back(id);
        self.pump();
    }

    // ===== 内部 =====

    fn insert_entry(&self, id: TaskId, url: String, filename: String, manifest: Option<TaskManifest>) {
        let (state, cancel) = (TaskState::Queued, CancellationToken::new());
        let progress = ProgressHandle::new();
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            tasks.insert(id, TaskEntry { url: url.clone(), filename, state, cancel, pending_choice: None, progress });
        }
        self.inner.queue.lock().unwrap().push_back(id);
        // 启动时若带 manifest 立即写盘，保证恢复信息存在
        if let Some(m) = manifest {
            let root = self.inner.settings.lock().unwrap().download_dir.clone();
            let _ = write_manifest(&task_dir(&root, id), &m);
        }
        self.pump();
    }

    fn set_state(&self, id: TaskId, state: TaskState) {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            if let Some(t) = tasks.get_mut(&id) {
                t.state = state.clone();
            }
        }
        let _ = self.inner.tx.send(EngineEvent::StateChanged { id, state });
        self.pump();
    }

    fn pump(&self) {
        let settings = self.inner.settings.lock().unwrap().clone();
        loop {
            // **"读活跃数 → 判定上限 → 把 Queued 置为 Probing"必须在同一个锁里完成。**
            // 拆成两个锁会有两个真实后果：
            //  a) 两个线程同时 pump（worker 在 runtime 线程上收尾，GUI 同时在 add_url /
            //     update_settings）会同时读到 active = max-1，于是都启动任务，**超出并发上限**；
            //  b) 在释放锁到置 Probing 之间，cancel() 可以把任务置成 Canceled，随后又被
            //     Probing 覆盖 —— 刚被取消的任务原地复活，并且因为 token 已被换掉，
            //     收尾路径看到的不是 Canceled 而是 Failed，于是 manifest 目录也留下不走。
            // 这里是全局唯一一处 tasks 与 queue 的嵌套取锁，顺序固定 tasks → queue，不会反向。
            let started = {
                let mut tasks = self.inner.tasks.lock().unwrap();
                let active = tasks.values().filter(|t| t.is_active()).count();
                if active >= settings.max_active_tasks {
                    return;
                }
                let next = { self.inner.queue.lock().unwrap().pop_front() };
                let Some(id) = next else { return };
                match tasks.get_mut(&id) {
                    // **只有仍处于 Queued 的才允许启动。** 取消/暂停发生在"排队期间"时，id 还留在
                    // 队列里；没有这道闸，pump 会把它重新拉起来 —— 用户明明点了取消，任务却自己复活。
                    Some(t) if t.state == TaskState::Queued => {
                        t.cancel = CancellationToken::new();
                        t.state = TaskState::Probing; // 与上面的判定同锁，关掉 (b) 的窗口
                        Some((id, t.url.clone(), t.progress.clone()))
                    }
                    _ => None,
                }
            };
            let Some((id, url, progress)) = started else { continue };
            // 状态事件在锁外发。这里**不调 set_state**：它会再次进入 pump，把这条路径变成递归。
            let _ = self
                .inner
                .tx
                .send(EngineEvent::StateChanged { id, state: TaskState::Probing });
            let engine = self.clone();
            let tx = self.inner.tx.clone();
            self.inner.rt.spawn(async move {
                // 进度转发：每 300ms 把原子计数器快照发为事件
                let mut ticker = tokio::time::interval(std::time::Duration::from_millis(300));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                // `run_task` 的 future **必须在 select! 之外创建并 pin 住**，循环里只借用它（`&mut task`）。
                //
                // 原稿把 `engine.run_task(...)` 直接写在 select! 的分支里，那是**错的**：select!
                // 每轮丢弃未胜出的分支，而 ticker 每 300ms 必然胜出一次 —— 于是 run_task 每 300ms
                // 被从头重建，probe 反复重跑，任务永远到不了终止态。实测症状：一个本该在数秒内
                // 失败的 404 任务会**永久卡在 Downloading**（20 秒都不出结果）。
                // 这类 bug 编译期完全看不出来，只有跑起来才显形。
                let task = engine.run_task(id, url.clone(), progress.clone());
                tokio::pin!(task);
                let result = loop {
                    tokio::select! {
                        _ = ticker.tick() => {
                            let (d, t) = progress.snapshot();
                            let _ = tx.send(EngineEvent::Progress { id, downloaded: d, total: t, unit: progress.unit() });
                        }
                        r = &mut task => break r,
                    }
                };
                let (d, t) = progress.snapshot();
                let _ = tx.send(EngineEvent::Progress { id, downloaded: d, total: t, unit: progress.unit() });
                // 收尾：只有仍处于执行态才标记 Done；AwaitingChoice/Paused/Canceled 保持
                match result {
                    Ok(()) => {
                        if let Some(s) = engine.state_of(id) {
                            if matches!(s, TaskState::Probing | TaskState::Downloading | TaskState::Merging) {
                                engine.set_state(id, TaskState::Done);
                            }
                        }
                    }
                    Err(e) => {
                        // 用户主动取消：清掉断点目录。否则 manifest 还在，下次启动
                        // scan_resume 会把已取消的任务当未完成任务复活并重新下载。
                        if matches!(engine.state_of(id), Some(TaskState::Canceled)) {
                            let root = engine.current_settings().download_dir;
                            std::fs::remove_dir_all(task_dir(&root, id)).ok();
                        } else if matches!(engine.state_of(id), Some(TaskState::Paused)) {
                            // 暂停：保留断点与 manifest，等待用户继续
                        } else {
                            tracing::warn!("task {id} failed: {e:#}");
                            engine.set_state(id, TaskState::Failed(format!("{e:#}")));
                        }
                    }
                }
            });
        }
    }

    async fn run_task(&self, id: TaskId, url: String, progress: ProgressHandle) -> anyhow::Result<()> {
        let settings = self.inner.settings.lock().unwrap().clone();
        let outcome = probe(&self.inner.client, &url).await?;
        match outcome {
            ProbeOutcome::WebPage => {
                if settings.ytdlp_enabled {
                    if let Some(exe) = find_ytdlp() {
                        let formats = fetch_formats(&exe, &url).await?;
                        if formats.is_empty() {
                            anyhow::bail!("yt-dlp 未找到可下载格式");
                        }
                        self.set_state(id, TaskState::AwaitingChoice);
                        {
                            let mut tasks = self.inner.tasks.lock().unwrap();
                            if let Some(t) = tasks.get_mut(&id) {
                                t.pending_choice = Some(ChoiceKind::YtdlpFormats(formats));
                            }
                        }
                        let _ = self.inner.tx.send(EngineEvent::ChoicesOffered {
                            id,
                            kind: self.inner.tasks.lock().unwrap().get(&id).unwrap().pending_choice.clone().unwrap(),
                        });
                        return Ok(()); // 等待 set_choice 重新入队
                    }
                }
                anyhow::bail!("无法识别的网页地址；可安装 yt-dlp 或在设置中启用");
            }
            ProbeOutcome::Hls { url } => {
                self.run_hls(id, &url, &settings, &progress).await
            }
            ProbeOutcome::Direct { url, size, accepts_ranges, filename } => {
                // 若任务还未被用户重命名过（默认名），用 probe 得到的更准文件名
                {
                    let mut tasks = self.inner.tasks.lock().unwrap();
                    if let Some(t) = tasks.get_mut(&id) {
                        if t.filename == "video.mp4" && filename != "video.mp4" {
                            t.filename = filename;
                        }
                    }
                }
                let (final_path, tdir) = self.paths(id, &settings);
                // 任务一开始就写 manifest：中途退出应用可据扫描恢复。
                //
                // **必须在 `set_state(Downloading)` 之前落盘。** "进入 Downloading"被当作
                // "断点信息已经存在"的信号 —— `cancel_cleans_resume_artifacts` 就是等到
                // Downloading 才取消并断言 scan_resume 为空。先置状态再写盘会留下一个窗口：
                // 此刻 `scan_resume()` 什么都扫不到（目录已建但 manifest 还没写），
                // 与"任务已在下载"的状态自相矛盾。原稿就是这个顺序，实测让该测试失败。
                let manifest = TaskManifest {
                    id,
                    url: url.clone(),
                    filename: final_path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                    download_dir: settings.download_dir.clone(),
                    kind: ManifestKind::Direct { size },
                };
                let _ = write_manifest(&tdir, &manifest);
                self.set_state(id, TaskState::Downloading);
                let cancel = self.inner.tasks.lock().unwrap().get(&id).unwrap().cancel.clone();
                download_direct(
                    &self.inner.client, &url, &final_path, &tdir,
                    size, accepts_ranges, settings.segment_concurrency, MIN_BLOCK_SIZE,
                    &progress, cancel,
                ).await?;
                // 完成后整个任务目录一起清掉（含 manifest），避免重启时误恢复已完成任务
                std::fs::remove_dir_all(&tdir).ok();
                Ok(())
            }
        }
    }

    async fn run_hls(&self, id: TaskId, playlist_url: &str, settings: &Settings, progress: &ProgressHandle) -> anyhow::Result<()> {
        let body = {
            let resp = self.inner.client.get(playlist_url).send().await?.error_for_status()?;
            resp.bytes().await? // 播放列表文件很小（KB 级），全量读取可接受
        };
        let parsed = parse_playlist(playlist_url, &body)?;
        let media = match parsed {
            ParsedPlaylist::Master { variants } => {
                self.set_state(id, TaskState::AwaitingChoice);
                {
                    let mut tasks = self.inner.tasks.lock().unwrap();
                    if let Some(t) = tasks.get_mut(&id) {
                        t.pending_choice = Some(ChoiceKind::HlsVariants(variants));
                    }
                }
                let _ = self.inner.tx.send(EngineEvent::ChoicesOffered {
                    id,
                    kind: self.inner.tasks.lock().unwrap().get(&id).unwrap().pending_choice.clone().unwrap(),
                });
                return Ok(());
            }
            ParsedPlaylist::Media(m) => m,
        };
        let (final_path, tdir) = self.paths(id, settings);
        // HLS 任务同样要落 manifest：否则中途退出后无法续传，违反 spec §6。
        // 记的是 media playlist 的 URL（master 已在上面转成具体码率的 media）。
        // 与 Direct 分支同理，**落盘必须早于 `set_state(Downloading)`**。
        let _ = write_manifest(
            &tdir,
            &TaskManifest {
                id,
                url: playlist_url.to_string(),
                filename: final_path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                download_dir: settings.download_dir.clone(),
                kind: ManifestKind::Hls { playlist_url: playlist_url.to_string() },
            },
        );
        self.set_state(id, TaskState::Downloading);
        let cancel = self.inner.tasks.lock().unwrap().get(&id).unwrap().cancel.clone();
        let plan = download_media(
            &self.inner.client, &media, &tdir, settings.segment_concurrency, progress, cancel,
        ).await?;
        self.set_state(id, TaskState::Merging);
        let ffmpeg = match &settings.ffmpeg_path_override {
            Some(p) => p.clone(),
            None => ensure_ffmpeg(&self.inner.data_dir)?,
        };
        let out_mp4 = final_path.with_extension("mp4");
        remux_to_mp4(&ffmpeg, &plan, &out_mp4).await?;
        // 合并成功后整个任务目录（含 manifest、分片、concat 列表）一起清掉，
        // 避免重启时 scan_resume 把已完成任务复活
        std::fs::remove_dir_all(&tdir).ok();
        Ok(())
    }

    fn paths(&self, id: TaskId, settings: &Settings) -> (PathBuf, PathBuf) {
        let root = &settings.download_dir;
        let tdir = task_dir(root, id);
        let filename = self.inner.tasks.lock().unwrap().get(&id).unwrap().filename.clone();
        let final_path = unique_path(root, &filename);
        (final_path, tdir)
    }
}
```

注：以上为完整实现；`TaskEntry` 已派生 `Clone`，进度事件由 worker 内的 300ms 定时转发任务发出。

**关于 m3u8-rs（原文有误，已更正）：** 原注写"核对 m3u8-rs 0.10 的字段名"，两处都不对 ——
本项目 pin 的是 **m3u8-rs 6.0.1**，而且**引擎根本不接触 m3u8-rs**：它只调用 `hls::parse::parse_playlist`
和 `ParsedPlaylist`/`MediaInfo`/`VariantChoice`。这些类型在 Task 5 已实现并通过审查，形参和字段名
（`VariantChoice.uri`、`ParsedPlaylist::{Master{variants}, Media(MediaInfo)}`）都是既成事实，直接照用。
若此处出现编译错误，说明是计划写错了调用，而不是 m3u8-rs 版本问题 —— 报告它，不要去改 `hls/parse.rs`。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --test engine_test`
Expected: 11 PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 下载引擎（任务队列、调度、端到端下载）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 14: GUI 主窗口（URL 输入、任务列表、进度、重命名、控制按钮）

**Files:**
- Modify: `src/app.rs`
- Modify: `src/main.rs`（挂接 Engine、tokio Runtime、tracing）
- Modify: `src/settings.rs`（新增 `data_dir()` —— 见下面"数据目录"一节；现状里没有这个方法）
- Modify: `src/lib.rs`（加 `pub mod ui_font;`）
- Create: `src/ui_font.rs`（系统中文字体安装 —— 见下面"中文字体"一节）
- Create: `tests/ui_font_test.rs`（1 PASS）
- Modify: `tests/settings_test.rs`（为 `data_dir()` 补测试）

**数据目录（本任务新增 `Settings::data_dir()`）**

原稿写的是 `Settings::config_dir().parent().unwrap().join("data")`，**是错的**。`config_dir()` 返回的是
`directories::ProjectDirs` 给出的**应用专属**目录（macOS `~/Library/Application Support/dev.easydownvideo.easydownvideo`，
Linux `~/.config/easydownvideo`）。取它的 `parent()` 会**退回到多应用共享的那一级**：
macOS 得 `~/Library/Application Support/data`，Linux 得 `~/.config/data`。于是释放的 ffmpeg 和日志会落在
一个**与别的应用共用、且不带应用名**的目录里 —— 既污染共享位置，也可能与其他同样"取 parent 再拼 data"的
程序撞名。spec 说的是"应用数据目录（`directories` crate 定位）"，那就该用 `ProjectDirs::data_dir()`。

在 `src/settings.rs` 里加：

```rust
/// 应用数据目录（`ProjectDirs::data_dir()`）：释放出来的 ffmpeg 与日志放这里。
///
/// **不要用 `config_dir().parent()` 去凑**：`config_dir()` 已经是应用专属的那一级，
/// 取 parent 会退回到 `~/.config` / `~/Library/Application Support` 这类**多应用共享**目录，
/// 于是 ffmpeg 和日志会落到一个不带应用名的公共位置。
pub fn data_dir() -> PathBuf {
    let dir = directories::ProjectDirs::from("dev", "easydownvideo", "easydownvideo")
        .expect("home directory must exist")
        .data_dir()
        .to_path_buf();
    std::fs::create_dir_all(&dir).ok();
    dir
}
```

`tests/settings_test.rs` 补一个测试（**1 PASS**，该文件原有测试数 +1）：

```rust
#[test]
fn data_dir_is_app_scoped() {
    // 关键：数据目录必须带应用名，且**不等于**"取 config_dir 的 parent 再拼 data"那个共享位置。
    // macOS 上 data_dir 与 config_dir 恰好是同一个路径，两者都带应用名；Linux 上分别是
    // ~/.local/share/easydownvideo 与 ~/.config/easydownvideo。两种平台下本断言都成立。
    let d = Settings::data_dir();
    let c = Settings::config_dir();
    assert!(
        d.to_string_lossy().contains("easydownvideo"),
        "数据目录必须应用专属，实际: {}",
        d.display()
    );
    assert_ne!(
        d,
        c.parent().unwrap().join("data"),
        "不得退回到共享目录（config_dir 的父目录 + data）"
    );
}

**中文字体（本任务新增 `src/ui_font.rs`）**

egui 自带字体（Ubuntu-Light / Hack 等）**不含任何 CJK 字形**，而本应用界面全是中文 ——
不额外装字体，所有中文都会渲染成方框（tofu，□），界面等于不可读。实测确认：Task 14 首次
`cargo run` 出来的窗口里中文全是方框。

**选择"运行期加载系统字体"而不是内嵌字体文件**，理由：
- 不增加仓库体积（ffmpeg 已经占了 172MB）
- 避开再分发授权问题：PingFang、微软雅黑**都不可再分发**；Noto Sans SC 可以，但要额外下载+子集化
- 各平台自带的中文字体都覆盖界面用到的常用字

**可行性已核实**：egui 0.30 的 `FontData` 带 `index: u32` 字段，底层走
`ab_glyph::FontVec::try_from_vec_and_index`（见 `epaint-0.30.0/src/text/fonts.rs:119,206`），
因此**支持 `.ttc` 字体集合** —— 而这正是 macOS/Windows 上中文字体的常见打包形式
（`PingFang.ttc`、`msyh.ttc`）。若 epaint 不支持 `.ttc`，这条路走不通，必须改为内嵌字体。

`src/lib.rs` 增加一行：

```rust
pub mod ui_font;
```

`src/ui_font.rs`：

```rust
//! 中文字体安装。详见计划中"中文字体"一节的取舍说明。

use eframe::egui;

/// 各平台候选路径（按优先级）。都是系统自带字体，取第一个能读到的。
/// 第二项是 `.ttc` 集合里的字体面序号（0 = 该集合的第一个面）。
#[cfg(target_os = "macos")]
const CANDIDATES: &[(&str, u32)] = &[
    ("/System/Library/Fonts/PingFang.ttc", 0), // 苹方 SC
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    ("/Library/Fonts/Arial Unicode.ttf", 0),
];

#[cfg(target_os = "windows")]
const CANDIDATES: &[(&str, u32)] = &[
    ("C:\\Windows\\Fonts\\msyh.ttc", 0),   // 微软雅黑
    ("C:\\Windows\\Fonts\\simhei.ttf", 0), // 黑体
    ("C:\\Windows\\Fonts\\simsun.ttc", 0), // 宋体
];

#[cfg(all(unix, not(target_os = "macos")))]
const CANDIDATES: &[(&str, u32)] = &[
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf", 0),
    ("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0),
];

/// 把系统中文字体装进 egui 字体表，作为**后备**字体（追加在默认字体之后）。
///
/// 返回是否装上了 —— `false` 意味着界面上所有中文都会显示成方框。
pub fn install_cjk_font(ctx: &egui::Context) -> bool {
    for (path, index) in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else { continue };
        if bytes.is_empty() {
            continue;
        }
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "cjk".to_owned(),
            egui::FontData {
                font: bytes.into(),
                index: *index,
                tweak: Default::default(),
            }
            .into(),
        );
        // **追加到末尾**而不是插到最前：拉丁字母和数字继续用 egui 默认字体渲染
        // （字形更协调），只有默认字体缺字的字符才回退到中文字体。
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        tracing::info!("中文字体已加载: {path} (index {index})");
        return true;
    }
    tracing::warn!(
        "未找到系统中文字体，界面中文将显示为方框（□）。候选路径: {:?}",
        CANDIDATES.iter().map(|(p, _)| *p).collect::<Vec<_>>()
    );
    false
}
```

`tests/ui_font_test.rs`（**1 PASS**）：

```rust
#[test]
fn cjk_font_loads_from_the_system() {
    // 界面全是中文。这个函数返回 false，用户看到的就是满屏方框（□）。
    // 用 Context::default() 即可 —— egui 允许在无窗口环境下建 Context。
    let ctx = eframe::egui::Context::default();
    assert!(
        easydownvideo::ui_font::install_cjk_font(&ctx),
        "本机没找到任何可用的系统中文字体 —— 界面中文会渲染成方框。\
         这说明候选路径在本平台不对（或字体文件读不出来），不是无关紧要的失败。"
    );
}
```
```

**Interfaces:**
- Consumes: `Engine`（Task 13）、`Settings`（Task 2）、`EngineEvent`/`TaskState`/`ChoiceKind`/`Choice`（Task 3）
- Produces: 可运行的 GUI 主窗口（手动验收）

- [ ] **Step 1: main.rs 挂接 runtime / settings / engine / tracing**

`src/main.rs`：

```rust
mod app; // 只声明 app；core / settings 走 lib crate（不要写 `mod core;`）

use eframe::egui;
use easydownvideo::settings::Settings;

fn main() -> eframe::Result<()> {
    // 日志：应用数据目录 logs/easydownvideo.log（按天滚动）
    // 注意用 Settings::data_dir() 而不是 config_dir().parent().join("data") —— 后者会落到共享目录
    let data_dir = Settings::data_dir();
    std::fs::create_dir_all(data_dir.join("logs")).ok();
    let (file_writer, _guard) = tracing_appender::non_blocking(
        tracing_appender::rolling::daily(&data_dir.join("logs"), "easydownvideo.log"),
    );
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_writer(file_writer)
        .init();

    let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
    let _enter = rt.enter();

    let settings = Settings::load();
    let engine = app::build_engine(rt.handle().clone(), settings.clone(), data_dir);

    // 断点恢复：扫描上次未完成的任务并重新入队
    for m in easydownvideo::core::task::scan_resume(&settings.download_dir) {
        tracing::info!("resuming task {} ({})", m.id, m.filename);
        engine.resume(m);
    }

    eframe::run_native(
        "EasyDownVideo",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([860.0, 560.0])
                .with_min_inner_size([720.0, 480.0]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, engine, settings)))),
    )
}
```

- [ ] **Step 2: app.rs 完整实现**

`src/app.rs`（核心结构；约 350 行）：

```rust
use std::time::{Duration, Instant};

// 必须用 `eframe::egui`：`egui` **不是**本 crate 的直接依赖（只有 eframe），
// 直接写 `use egui::...` 会得到 E0433。原稿漏了这一行，实测 25 处编译失败。
use eframe::egui;
use easydownvideo::core::engine::Engine;
use easydownvideo::core::events::{Choice, ChoiceKind, EngineEvent, ProgressUnit, TaskId, TaskState};
use easydownvideo::settings::Settings;

pub fn build_engine(
    rt: tokio::runtime::Handle,
    settings: Settings,
    data_dir: std::path::PathBuf,
) -> Engine {
    Engine::new(rt, settings, data_dir)
}

struct UiTask {
    id: TaskId,
    url: String,
    filename: String,
    state: TaskState,
    downloaded: u64,
    total: u64,
    unit: ProgressUnit,
    speed_bps: f64,
    last_downloaded: u64,
    last_tick: Instant,
    choices: Option<ChoiceKind>,
    show_choices: bool,
}

impl UiTask {
    fn progress_text(&self) -> String {
        let pct = if self.total > 0 {
            self.downloaded as f64 / self.total as f64 * 100.0
        } else {
            0.0
        };
        match self.unit {
            ProgressUnit::Bytes => format!(
                "{} / {} ({:.1}%)",
                format_size(self.downloaded),
                format_size(self.total),
                pct
            ),
            ProgressUnit::Segments => format!(
                "分片 {} / {} ({:.1}%)",
                self.downloaded, self.total, pct
            ),
        }
    }

    fn progress_ratio(&self) -> f32 {
        if self.total > 0 {
            (self.downloaded as f32 / self.total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

pub struct App {
    engine: Engine,
    events: tokio::sync::broadcast::Receiver<EngineEvent>,
    settings: Settings,
    input_url: String,
    status: String,
    tasks: Vec<UiTask>,
    settings_open: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        engine: Engine,
        settings: Settings,
    ) -> Self {
        // 装中文字体：egui 自带字体没有 CJK 字形，界面全是中文，不装就是满屏方框。
        // 放在最前面 —— 越早装，第一帧就越不会是方框。
        if !easydownvideo::ui_font::install_cjk_font(&cc.egui_ctx) {
            // 已经 warn 过；界面会退化成方框，但程序仍可用
        }
        let events = engine.events();
        // 水合：把引擎中已存在的任务（含 scan_resume 恢复的）载入 UI 列表
        let tasks: Vec<UiTask> = engine
            .list_tasks()
            .into_iter()
            .map(|s| {
                let (downloaded, total) = s.progress.snapshot();
                UiTask {
                    id: s.id,
                    url: s.url,
                    filename: s.filename,
                    state: s.state,
                    downloaded,
                    total,
                    unit: s.progress.unit(),
                    speed_bps: 0.0,
                    last_downloaded: downloaded,
                    last_tick: Instant::now(),
                    choices: None,
                    show_choices: false,
                }
            })
            .collect();
        let mut app = Self {
            engine,
            events,
            settings,
            input_url: String::new(),
            status: String::new(),
            tasks,
            settings_open: false,
        };
        app.restore_clipboard(cc);
        app
    }

    fn restore_clipboard(&mut self, _cc: &eframe::CreationContext<'_>) {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(text) = cb.get_text() {
                if text.starts_with("http://") || text.starts_with("https://") {
                    self.input_url = text.trim().to_string();
                }
            }
        }
    }

    fn poll_events(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                EngineEvent::StateChanged { id, state } => {
                    if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                        t.state = state.clone();
                        if matches!(state, TaskState::Done | TaskState::Failed(_) | TaskState::Canceled) {
                            t.choices = None;
                            t.show_choices = false;
                        }
                    }
                }
                EngineEvent::Progress { id, downloaded, total, unit } => {
                    if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                        t.downloaded = downloaded;
                        t.total = total;
                        t.unit = unit;
                    }
                }
                EngineEvent::ChoicesOffered { id, kind } => {
                    if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                        t.choices = Some(kind);
                        t.show_choices = true;
                    }
                }
            }
        }
    }

    fn tick_speeds(&mut self) {
        for t in &mut self.tasks {
            let now = Instant::now();
            let dt = now.duration_since(t.last_tick).as_secs_f64();
            if dt >= 1.0 {
                let delta = t.downloaded.saturating_sub(t.last_downloaded);
                t.speed_bps = delta as f64 / dt;
                t.last_downloaded = t.downloaded;
                t.last_tick = now;
            }
        }
    }

    fn add_task(&mut self) {
        let url = self.input_url.trim().to_string();
        if url.is_empty() {
            return;
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.status = "请输入 http(s) 链接".into();
            return;
        }
        let id = self.engine.add_url(url.clone());
        // 同一份推导逻辑，来自 probe 模块（不要在 GUI 里再抄一遍）
        let filename = easydownvideo::core::probe::filename_from_url(&url);
        self.tasks.push(UiTask {
            id,
            url,
            filename,
            state: TaskState::Queued,
            downloaded: 0,
            total: 0,
            unit: ProgressUnit::Bytes,
            speed_bps: 0.0,
            last_downloaded: 0,
            last_tick: Instant::now(),
            choices: None,
            show_choices: false,
        });
        self.input_url.clear();
        self.status.clear();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        self.tick_speeds();
        ctx.request_repaint_after(Duration::from_millis(200));

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.input_url)
                        .desired_width(f32::INFINITY)
                        .hint_text("粘贴视频链接（mp4 / m3u8 / 网页）后回车"),
                );
                let submit = ui.button("下载").clicked()
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if submit {
                    self.add_task();
                }
                if ui.button("设置").clicked() {
                    self.settings_open = !self.settings_open;
                }
            });
            ui.add_space(6.0);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let total_speed: f64 = self.tasks.iter().map(|t| t.speed_bps).sum();
                ui.label(format!("总速度: {}", format_speed(total_speed)));
                let active = self.tasks.iter().filter(|t| {
                    matches!(t.state, TaskState::Downloading | TaskState::Probing | TaskState::Merging)
                }).count();
                ui.label(format!("活跃任务: {active}"));
                if !self.status.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.status);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut remove_ids = Vec::new();
                for i in 0..self.tasks.len() {
                    let t = &mut self.tasks[i];
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            let name_edit = ui.add(
                                egui::TextEdit::singleline(&mut t.filename)
                                    .desired_width(220.0),
                            );
                            if name_edit.lost_focus() {
                                self.engine.set_filename(t.id, t.filename.clone());
                            }
                            ui.colored_label(state_color(&t.state), state_text(&t.state));
                        });
                        ui.horizontal(|ui| {
                            let bar = egui::ProgressBar::new(t.progress_ratio())
                                .desired_width(420.0)
                                .text(t.progress_text());
                            ui.add(bar);
                            // 分片计数模式下速度无意义（单位为分片），只在字节模式下显示
                            if t.unit == ProgressUnit::Bytes {
                                ui.label(format_speed(t.speed_bps));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                match t.state {
                                    TaskState::Downloading | TaskState::Probing | TaskState::Merging => {
                                        if ui.button("暂停").clicked() {
                                            self.engine.pause(t.id);
                                        }
                                    }
                                    TaskState::Paused => {
                                        if ui.button("继续").clicked() {
                                            self.engine.unpause(t.id);
                                        }
                                    }
                                    TaskState::Failed(_) => {
                                        if ui.button("重试").clicked() {
                                            // 必须调 retry：unpause 只认 Paused，对 Failed 毫无反应，
                                            // 那样这个按钮就是个点了没任何动静的死按钮。
                                            self.engine.retry(t.id);
                                        }
                                    }
                                    _ => {}
                                }
                                if ui.button("删除").clicked() {
                                    self.engine.cancel(t.id);
                                    remove_ids.push(t.id);
                                }
                                if t.choices.is_some() && !t.show_choices {
                                    if ui.button("选择格式").clicked() {
                                        t.show_choices = true;
                                    }
                                }
                            });
                        });
                        ui.label(egui::RichText::new(&t.url).small().weak());
                    });
                    ui.add_space(4.0);

                    // 选择窗口（模态）
                    if t.show_choices {
                        if let Some(kind) = t.choices.clone() {
                            let mut open = t.show_choices;
                            let mut chosen: Option<Choice> = None;
                            egui::Window::new("选择下载格式")
                                .collapsible(false)
                                .resizable(false)
                                .open(&mut open)
                                .show(ctx, |ui| match &kind {
                                    ChoiceKind::HlsVariants(vars) => {
                                        ui.label("选择清晰度：");
                                        // 默认最高码率排最前
                                        let mut order: Vec<(usize, _)> = vars.iter().enumerate().collect();
                                        order.sort_by(|a, b| b.1.bandwidth.cmp(&a.1.bandwidth));
                                        for (idx, v) in order {
                                            let label = match v.resolution {
                                                Some((w, h)) => format!("{}x{} ({} kbps)", w, h, v.bandwidth / 1000),
                                                None => format!("{} kbps", v.bandwidth / 1000),
                                            };
                                            if ui.button(label).clicked() {
                                                chosen = Some(Choice::Variant(idx));
                                            }
                                        }
                                    }
                                    ChoiceKind::YtdlpFormats(fmt) => {
                                        ui.label("yt-dlp 可用格式：");
                                        for (i, f) in fmt.iter().enumerate() {
                                            let size = f.filesize.map(format_size).unwrap_or_else(|| "未知大小".into());
                                            if ui.button(format!("[{}] {} - {}", f.id, f.note, size)).clicked() {
                                                chosen = Some(Choice::YtdlpFormat(i));
                                            }
                                        }
                                    }
                                });
                            // **选中后必须真正关窗，而且不能无条件采纳 `open`。**
                            // 原稿在每个 arm 里置 `show_choices = false`，紧接着又无条件写
                            // `t.show_choices = open` —— 而 `open` 的初值就是 `t.show_choices`（true），
                            // 只有用户点窗口标题栏的 [x] 才会变 false。于是 arm 里刚关上的窗立刻被
                            // 覆盖回来：模态框永远关不掉，里面的按钮点起来像是没反应。
                            // `chosen` 提到 Window 外面也是为此 —— arm 内部的局部变量在外面看不见。
                            if let Some(c) = chosen {
                                self.engine.set_choice(t.id, c);
                                t.choices = None;
                                t.show_choices = false;
                            } else {
                                t.show_choices = open;
                            }
                        }
                    }
                }
                for id in remove_ids {
                    self.tasks.retain(|t| t.id != id);
                }
                if self.tasks.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("暂无任务，粘贴链接开始下载").weak());
                    });
                }
            });
        });

        // 设置窗口
        if self.settings_open {
            egui::Window::new("设置").collapsible(false).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("下载目录：");
                    ui.label(self.settings.download_dir.display().to_string());
                    if ui.button("更改…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.settings.download_dir = dir;
                            let _ = self.settings.save();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("单任务并发：");
                    ui.add(egui::Slider::new(&mut self.settings.segment_concurrency, 1..=16));
                });
                ui.horizontal(|ui| {
                    ui.label("同时下载任务数：");
                    ui.add(egui::Slider::new(&mut self.settings.max_active_tasks, 1..=8));
                });
                ui.checkbox(&mut self.settings.ytdlp_enabled, "启用 yt-dlp 网页解析");
                // 立即推给引擎（其余任务用新并发/开关，无需重启）
                self.engine.update_settings(self.settings.clone());
                if self.settings.save().is_err() {
                    ui.colored_label(egui::Color32::LIGHT_RED, "设置保存失败");
                }
            });
        }
    }
}

fn state_text(s: &TaskState) -> String {
    match s {
        TaskState::Queued => "排队中".into(),
        TaskState::Probing => "探测中".into(),
        TaskState::AwaitingChoice => "等待选择".into(),
        TaskState::Downloading => "下载中".into(),
        TaskState::Merging => "合并中".into(),
        TaskState::Paused => "已暂停".into(),
        TaskState::Done => "完成".into(),
        TaskState::Failed(e) => format!("失败: {e}"),
        TaskState::Canceled => "已取消".into(),
    }
}

fn state_color(s: &TaskState) -> egui::Color32 {
    match s {
        TaskState::Done => egui::Color32::LIGHT_GREEN,
        TaskState::Failed(_) => egui::Color32::LIGHT_RED,
        TaskState::Downloading | TaskState::Merging => egui::Color32::LIGHT_BLUE,
        TaskState::Paused | TaskState::Canceled => egui::Color32::GRAY,
        _ => egui::Color32::YELLOW,
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

fn format_speed(bps: f64) -> String {
    format!("{}/s", format_size(bps as u64))
}
```

- [ ] **Step 3: 编译 + 手动冒烟**

Run: `cargo check` → 通过后 `cargo run`
Expected: 窗口出现；粘贴直链 mp4 URL（可用本机 `python3 -m http.server` 起个含 mp4 的目录）→ 任务出现、显示文件名与进度、完成。

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: GUI 主窗口（任务列表、进度、重命名、设置窗口）

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

### Task 15: 手动验收 + 文档收尾

**Files:**
- Modify: `README.md`

- [ ] **Step 1: README**

```markdown
# EasyDownVideo

Rust 跨平台桌面视频下载器（个人工具）。

## 功能
- 直链视频（mp4/webm/mkv/mov/ts）多线程分块下载，断点续传
- HLS(m3u8) 下载：清晰度选择、AES-128 解密、内嵌 ffmpeg 无损转封装 mp4
- 可选 yt-dlp 解析网页视频
- 随时新增任务；每任务显示文件名/进度/速度；暂停/继续/删除
- 设置中更改保存位置（仅影响新任务）

## 构建
cargo build --release   # macOS / Windows / Linux 各自产出单文件二进制

## 测试
cargo test
```

- [ ] **Step 2: 手动验收清单（逐项过 spec §10）**

- [ ] macOS `cargo build --release` 产出单二进制，可运行
- [ ] 直链 mp4（≥100MB 或本机大文件）下载，Activity Monitor 观察 RSS ≤100MB
- [ ] m3u8（找一个公开 AES-128 加密测试源）下载合并出可播放 mp4
- [ ] 下载中暂停 → 继续（观察不重下已完成部分）
- [ ] 下载中途退出应用 → 重启 → 任务自动恢复到列表并继续（`main` 已接 `scan_resume` → `engine.resume`，`App::new` 用 `list_tasks` 水合）
- [ ] 同名文件重复下载 → 自动 `name (1).mp4`
- [ ] 设置改下载目录 → 新任务落新目录，旧任务不受影响
- [ ] yt-dlp 未安装时网页 URL 给出提示；安装后可解析
- [ ] Windows 交叉验证（如有条件）：`cargo build --release --target x86_64-pc-windows-msvc` 需先装 mingw/msvc target；否则记录为后续项

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "docs: README 与手动验收

Co-Authored-By: Claude Code <noreply@anthropic.com>"
```

---

## 备注（执行者须知）

- m3u8-rs / eframe 的具体字段与方法名以编译器为准微调；不要为此回退功能。
- vendor 大文件导致 `git clone` 变慢属预期；不要改用 LFS（个人工具无需）。