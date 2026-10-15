# EasyDownVideo · Tauri GUI 重写设计

- 日期：2026-10-15
- 状态：待评审
- 前置：`docs/superpowers/specs/2026-10-14-easydownvideo-design.md`（core 的设计与需求，仍然有效）
- 定位：把桌面界面从 egui 换成 Tauri；**core 原样复用**

## 1. 背景与目标

core（`src/core/**` + `src/settings.rs`）已经实现并加固完毕：probe、直链分块下载、HLS（AES-128 解密、
fMP4、ffmpeg 转封装）、yt-dlp 集成、任务清单与断点恢复、`Engine` 的并发与取消语义，共 **78 个测试**
（其中 1 个属于 egui 层，见下）。
界面层（`src/app.rs` 的 egui 实现）被放弃，改用 Tauri + WebView 前端。

**为什么换**：egui 自带字体没有 CJK 字形，整个中文界面是一屏方框；而界面层要处理的东西（字体、模态框
Id、每帧落盘、事件丢失后重同步）全都是"框架没有、得自己造"的部分。WebView 把这一层全部免费给到，
且样式能做到 egui 做不到的程度。

**这一轮的目标：功能对等，不加新功能。** 界面要承载的能力与 egui 版逐条对应，不增不减。

**复用**：`src/core/**`（全部，连同它在 `tests/` 下的 77 个测试）、`src/settings.rs`、`vendor/ffmpeg/`。

**丢弃**：`src/app.rs`、`src/main.rs`、`src/ui_font.rs`、**`tests/ui_font_test.rs`**，以及
`eframe` / `rfd` / `arboard` 三个依赖。`src/lib.rs` 去掉 `pub mod ui_font;` 一行，其余保留。

**测试数的变化要说清楚**：当前 `cargo test` 是 **78 passed**，其中 1 个是 `tests/ui_font_test.rs`
（验证 egui 能否从系统加载中文字体）。它随 egui 层一起删除，**之后应是 77 passed**。
WebView 自带系统中文字体，那个测试要守的问题在新架构里根本不存在——但这也意味着**"中文不出方框"
这件事从"有测试守着"变成"靠手动验收"**，见 §8 清单里那一条。

**成功标准**：

- `cargo test` 在仓库根目录仍然只跑 core 的 77 个测试，且全绿（开发快循环不受新栈影响）
- 应用能启动、完成一次直链下载与一次 HLS 下载、可暂停/继续/续传（见 §8 验收清单）
- 界面上没有任何方框字
- core 中**已有代码零改动**（唯一新增见 §4.4）

## 2. 视觉方向（已定稿）

依次选定，交互稿留在 `.superpowers/brainstorm/73746-1792039839/content/`：

| 维度 | 选择 | 说明 |
|---|---|---|
| 风格 | **浅色极简** | 白底、发丝线分隔、无阴影、几乎只有黑白两色，靠字重与留白建立层次。参照 Vercel / Stripe 后台 |
| 主题 | **浅色 + 深色，跟随系统，设置里可强制** | 深色版是"墨与纸对调"，不引入彩色，两套共用同一份 CSS 变量 |
| 窗口 | **无边框，工具条与标题栏合一** | 省掉一条高度；macOS 用 `titleBarStyle: Overlay` 保留红绿灯 |

深色版的强调色只有"正在合并"这一处绿，主按钮靠明暗反转而不是加彩色——两套主题因此共用一套变量，
差别只在 `:root` 与 `[data-theme="dark"]` 两组取值。

## 3. 结构

```
easyDownVideo/
├─ Cargo.toml          # 仍是 lib crate（easydownvideo），同时成为 workspace 根
│                      #   [workspace] members = ["src-tauri"]
├─ src/                # core/** 与 settings.rs 原样不动；
│                      #   lib.rs 去掉 `pub mod ui_font;` 一行；删除 app.rs / main.rs / ui_font.rs
├─ tests/              # core 的 77 个测试；删除 ui_font_test.rs
│                      #   fixtures/ 与其余 *_test.rs 不动
├─ src-tauri/          # 新：Tauri 应用（Rust）
│   ├─ Cargo.toml      #   easydownvideo = { path = ".." }
│   ├─ build.rs、tauri.conf.json
│   └─ src/{main.rs, commands.rs, bridge.rs, dto.rs}
├─ ui/                 # 新：Vite + React + TypeScript + Tailwind
└─ vendor/ffmpeg/      # 不动
```

**workspace 的取舍**：`members` 只列 `src-tauri`，根包即默认成员——所以在根目录敲 `cargo test` **仍然
只编译并运行 core**，冷启动时间不受 Tauri 那几百个依赖影响。要连 src-tauri 一起查用
`cargo test --workspace` / `cargo check --workspace`。

## 4. Rust 侧

### 4.1 运行时归属

`Engine::new` 需要一个 `tokio::runtime::Handle`。**我们自己创建这个 Runtime**（在 `main` 里、
`tauri::Builder` 之前，进程存活期内一直持有），不依赖 Tauri 内部用的是什么运行时、能否取到 handle。

### 4.2 命令面

每个命令都是 `Engine` 对应方法的一层薄封装，**不含业务逻辑**：

| 命令 | 背后 |
|---|---|
| `get_tasks() -> Vec<TaskDto>` | `list_tasks()` |
| `add_url(url) -> TaskId` | `add_url` |
| `pause / unpause / retry / cancel(id)` | 同名方法 |
| `set_filename(id, name)` | `set_filename` |
| `pending_choice(id) -> Option<ChoiceKindDto>` | `pending_choice`（§4.4 新增） |
| `set_choice(id, choice)` | `set_choice` |
| `get_settings() -> SettingsDto` / `update_settings(SettingsDto)` | `Settings` |
| `pick_download_dir() -> Option<PathBuf>` | `tauri-plugin-dialog`（异步，不阻塞 WebView） |

粘贴板**不引入插件**：用 WebView 原生的 `paste` 事件读取文本即可。egui 版需要 `arboard` 是因为它没有
"浏览器"这一层。

### 4.3 事件桥

一个常驻任务，把 core 的事件原样搬到前端：

```rust
let mut rx = engine.events();
loop {
    match rx.recv().await {
        Ok(ev) => app.emit("engine://event", dto(ev)),
        Err(broadcast::error::RecvError::Lagged(n)) => app.emit("engine://resync", n),
        Err(broadcast::error::RecvError::Closed) => break,
    }
}
```

`Lagged` 分支是这个项目已经付过学费的教训在架构层落地（DEFECT-54：egui 版把 `Lagged` 当成"队列空"，
任务行会**永久**停在错误状态）。这里**不是"希望事件别丢"，而是明确声明丢了之后的恢复路径**。

core 的事件类型**不加 serde**；wire DTO 全部定义在 `src-tauri/src/dto.rs`，core 对 Tauri 一无所知。

### 4.4 core 的唯一改动

`Engine` 目前没有读取 `pending_choice` 的公开方法（`task_info(id)` 只返回 `(url, filename)`）。新增一个
只读方法：

```rust
pub fn pending_choice(&self, id: TaskId) -> Option<ChoiceKind>
```

**为什么需要它**：`ChoicesOffered` 是一次性事件。前端一旦没收到（或重同步后没有本地副本），那个"选清晰度"
模态框就再也弹不出来——任务停在 `AwaitingChoice`，界面上没有任何按钮可点，只能删掉重加。
`TaskSnapshot.pending_choice` 是另一种做法（改已有公开结构体的形状、让 `list_tasks` 顺带带上），
**选定的是新增方法**：不动任何已有类型的形状，`list_tasks` 也不为一个极少用的字段变胖；代价是前端在
"重同步且该行处于 AwaitingChoice 且本地无候选"这一种情况下多一次往返。

除此之外 core 一行不改。

## 5. 前端

**栈**：Vite + React + TypeScript + Tailwind CSS。

**状态模型**：不引状态库。一个模块级 store（`Map<TaskId, TaskView>` + `Settings` + UI 局部态）
配 `useSyncExternalStore`。

**组件树**：

```
<App>                       布局骨架：拖拽区 / 工具条 / 内容 / 状态栏
├─ <Toolbar>                URL 输入 + 下载 + 设置齿轮      ← 无边框窗口的拖拽区
├─ <TaskList> → <TaskRow>   可编辑文件名 · 进度条 · meta · 按状态出现的操作
├─ <StatusBar>              总速度 · 活跃任务数 · 版本号
├─ <SettingsDialog>         下载目录 · 并发数 · 同时任务数 · yt-dlp 开关 · ffmpeg 覆盖 · 主题
├─ <ChoiceDialog>           选清晰度 / 选格式 —— 同一个组件的两种数据
└─ <WindowControls>         仅 Windows/Linux 自绘的最小化/最大化/关闭
```

**数据流（四条规则，覆盖全部情况）**：

1. **启动**：`get_tasks()` 水合 → 订阅 `engine://event`。
2. **事件**：`state_changed` 改状态；`progress` 改进度（每活跃任务 300ms 一条，不节流）；
   `choices_offered` 存候选并打开模态框。
3. **未知 id ⇒ 触发一次重同步。** 这一条把 `add_url` 的竞态、事件丢失、新任务的首帧都归到同一条恢复
   路径上——因为 `add_url` 返回 id 之前，core 已经在发这个任务的事件了。
4. **`engine://resync` ⇒ `get_tasks()` 整体替换**，并对处于 `AwaitingChoice` 且本地无候选的行调用
   `pending_choice(id)` 补齐。

**不做本地乐观更新**：点了暂停就等事件回来。core 是唯一真相源，乐观更新迟早会和"这次调用被拒绝"
（例如对已经 `Done` 的任务点暂停）打架。

**窗口与主题**：macOS 用 `titleBarStyle: Overlay`（红绿灯原样保留、浮在内容上）；Windows/Linux 用
`decorations: false` + 自绘按钮。拖拽区用 `data-tauri-drag-region` 挂在工具条的**背景**上——输入框
本身不挂，否则框里选不中文字。主题用两套 CSS 变量，`data-theme` 解析自 `system` / `light` / `dark`。

## 6. 错误处理

- **任务级失败**：`TaskState::Failed(String)` 已经带着 core 写好的中文简述。界面在任务行内联显示，
  完整错误另在日志文件里（沿用原规格 §7）。
- **命令级失败**：命令返回 `Result<T, String>`，前端统一进一条 toast，不打断操作。
- **事件丢失**：由 §5 的规则 3/4 兜底，不需要用户干预。
- **日志**：`tracing` 写 `Settings::data_dir()/logs/easydownvideo.log`，配置逻辑从原 `main.rs` 原样搬到
  `src-tauri`。

## 7. 测试策略

| 层 | 测什么 | 怎么跑 |
|---|---|---|
| core | 现有 77 个一个不动；新增的 `pending_choice` 按 TDD 先写失败测试 | `cargo test`（根目录，只跑 core） |
| src-tauri | DTO 映射（`EngineEvent` → wire DTO、`TaskSnapshot` → `TaskDto`）——纯函数；桥的 `Lagged` 分支（喂假 emitter，断言发出 `engine://resync`） | `cargo test --workspace` |
| ui | **只测 reducer**：事件序列 → 期望视图状态、未知 id 规则、resync 整体替换、进度合并 | `npm --prefix ui test`（vitest） |

**组件不测**，沿用原规格"GUI 保持薄壳、手动验收"的立场。逻辑尽量挤进 reducer 与 Rust，组件里只剩连线——
这也是 reducer 没有塞进组件里的原因。

## 8. 验收清单（手动，逐项过）

- [ ] `npm run tauri build` 在 macOS 出可运行产物
- [ ] 直链 mp4 下载完成，与源文件字节一致
- [ ] m3u8（AES-128）下载合并出可播放 mp4；master playlist 能选清晰度
- [ ] 下载中暂停 → 继续 → 完成；**合并期间暂停也能停住**
- [ ] 下载中途退出应用 → 重启 → 任务自动恢复到列表并继续
- [ ] 同名文件重复下载 → 自动 `name (1).mp4`
- [ ] 设置改下载目录 → 新任务落新目录，旧任务不受影响
- [ ] 主题三态（跟随系统 / 强制浅色 / 强制深色）都正确
- [ ] 无边框窗口：拖动、最小化/最大化/关闭都可用（macOS 走红绿灯）
- [ ] 界面上没有任何方框字
- [ ] 大文件下载时内存稳定（原规格的 ~100MB 目标）

## 9. 构建与运行

```
npm --prefix ui install      # 一次性
npm run tauri dev            # 开发（Vite dev server + cargo）
npm run tauri build          # 出产物（内部先 vite build 再 cargo build）
cargo test                   # core 快循环，不受新栈影响
```

**构建入口的变化**：原规格写的是 `cargo build --release` 出单文件；引入 Vite 之后变为
`npm run tauri build`。`cargo test` 不受影响。

**一次性代价**：首次构建会明显变慢（Tauri 依赖是几百个 crate）；之后增量构建回到正常。

## 10. 风险与对策

| 风险 | 对策 |
|---|---|
| `titleBarStyle: Overlay` 是 macOS 专有 | Windows/Linux 改为自绘按钮；接受三平台手感存在差异，不追求像素级一致 |
| WebView 版本差异（WebView2 / WKWebView） | 当前设计不用 `backdrop-filter` 等新特性，只依赖 CSS 变量、grid/flex、`:has()`。若后续要加，需先确认两端都支持 |
| 事件桥本身的可靠性 | `Lagged` 有明确恢复路径（§4.3、§5），不依赖"事件不会丢"这一假设 |
| 前端首次引入 Node 工具链 | 只在 `ui/` 下，`cargo test` 与 core 开发循环完全不受影响 |
| 无边框窗口的拖动/按钮在各平台行为不同 | 列入手动验收清单逐项过 |

## 11. 非目标（YAGNI）

- 不加新功能：界面能力与 egui 版**严格对等**
- 不补原规格 §2 提过但从未实现的"GUI 内查看日志"
- 不做安装包 / 签名 / 自动更新 / 多语言
- 不改 core 的任何既有行为（唯一新增是 §4.4 的只读方法）
