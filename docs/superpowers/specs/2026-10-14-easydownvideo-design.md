# EasyDownVideo 设计文档

- 日期：2026-10-14
- 状态：待评审
- 定位：个人使用的跨平台桌面视频下载器

## 1. 背景与目标

用 Rust 编写一个跨平台（Windows / macOS / Linux）桌面视频下载器，支持直链视频文件（mp4、webm、mkv 等）和 HLS（m3u8）流媒体下载。

**已确认的需求与约束：**

- 个人使用工具：不做代码签名、自动更新、安装包、多语言
- 视频来源：通用 URL 识别（直链 + m3u8）；可选调用 yt-dlp 解析网页
- m3u8 合并：ffmpeg 二进制以 `include_bytes!` 内嵌，输出统一为 mp4
- GUI 框架：egui/eframe（方案 A，纯 Rust）
- 核心质量要求：内存占用低、运行流畅
- **多线程下载**：每个任务内部多线程分块/分片并行；支持随时新增任务
- **任务可视**：每个任务实时显示进度条、文件名、速度、ETA
- **保存位置可配置**：设置中可随时更改下载目录

**成功标准：**

- 下载任意大小的视频时，应用内存占用稳定在 ~100MB 以内，与视频大小无关
- 粘贴 URL 即可下载；m3u8 可选择清晰度；输出为可播放的 mp4
- 支持断点续传：关闭应用后重开可恢复未完成任务
- `cargo build --release` 三个平台各自产出单文件可执行程序

**非目标（YAGNI）：**

- 不针对具体视频网站写解析器（由可选 yt-dlp 承担）
- 不做视频格式转换 / 编码（ffmpeg 只用 `-c copy` 无损转封装）
- 不做下载限速、做种/P2P、浏览器插件
- 不做安装包：单二进制 / .app 目录直接分发

## 2. 技术栈

| 用途 | 选择 | 说明 |
|---|---|---|
| GUI | egui / eframe | 即时模式，纯 Rust，跨平台 |
| 异步运行时 | tokio | 下载并发基础设施 |
| HTTP | reqwest（rustls-tls，stream） | 不用 OpenSSL，简化跨平台编译；自动走系统代理 |
| m3u8 解析 | m3u8-rs | 支持 master / media playlist |
| AES-128 解密 | aes + cbc crates | HLS 标准加密，纯 Rust |
| ffmpeg | `include_bytes!` 内嵌静态二进制 | vendor 目录随仓库管理，见 §5.5 |
| yt-dlp | 可选，检测 PATH | 子进程方式调用，不绑定 crate |
| 配置/历史 | serde + serde_json + directories | 个人工具不引入数据库 |
| 日志 | tracing + tracing-subscriber | 写文件 + GUI 内查看 |

## 3. 项目结构

单 crate，二进制 + 内部模块（后续若 core 膨胀再拆 workspace）：

```
easydownvideo/
├── Cargo.toml
├── vendor/
│   └── ffmpeg/
│       ├── macos-aarch64/ffmpeg     # 静态构建，~40MB
│       └── windows-x86_64/ffmpeg.exe
├── src/
│   ├── main.rs          # eframe 入口
│   ├── app.rs           # GUI 状态机，每帧从 channel 拉进度
│   ├── settings.rs      # 配置（下载目录、并发数、yt-dlp 开关、ffmpeg 路径覆盖）
│   └── core/            # 全部纯逻辑，不依赖 egui，可独立测试
│       ├── probe.rs     # URL 嗅探
│       ├── direct.rs    # 直链多线程 Range 分块下载
│       ├── hls/
│       │   ├── mod.rs   # HLS 任务编排
│       │   ├── parse.rs # m3u8 解析（基于 m3u8-rs 的适配）
│       │   └── crypto.rs# AES-128 解密
│       ├── ffmpeg.rs    # 内嵌 ffmpeg 落盘、转封装调用
│       ├── ytdlp.rs     # 可选 yt-dlp 解析
│       ├── task.rs      # 任务状态机 + 断点续传 manifest
│       └── events.rs    # 进度事件类型（tokio → GUI channel）
└── tests/
    ├── fixtures/        # 样例 m3u8、加密向量等
    └── ...              # core 集成测试
```

**模块边界原则：** GUI（app.rs）只持有任务 ID 和进度快照，所有网络、文件、解析逻辑都在 core；core 不 import egui / eframe。

## 4. 低内存设计（核心要求）

1. **全程流式写盘**：所有 HTTP 响应用 `bytes_stream()` + 固定 64KB buffer 消费；代码评审红线——core 中禁止 `.bytes()` / `text()` 全量读取。
2. **分片落盘再合并**：HLS 每个分片下载后立即写临时文件；合并用 ffmpeg concat demuxer（传文件列表文件），分片永不读回内存。
3. **并发受限**：直链分块与 HLS 分片均用 tokio Semaphore 限流，默认 6 路，可在设置中调整（1–16）。每个任务内部独立并行；多任务并发执行互不影响。
4. **进度零拷贝**：后台任务只更新共享的原子计数器（已下字节 / 总字节），GUI 每帧读取快照；事件 channel 只传小型枚举（状态转换、错误信息），不携带数据体。

**验收方式**：下载一个 ≥2GB 的直链视频和一个 ≥500 分片的 m3u8，观察 Activity Monitor / 任务管理器中 RSS，应稳定在 ~100MB 以内。

## 5. 核心流程

### 5.1 URL 探测（probe.rs）

输入 URL，输出 `ProbeResult`：

| 判定 | 依据 |
|---|---|
| `Direct { size, range_supported }` | Content-Type 为 `video/*` / `application/octet-stream`，或路径后缀为 .mp4/.webm/.mkv/.mov/.ts |
| `Hls` | 响应体以 `#EXTM3U` 开头（GET 前 64 字节即可判定，或 Content-Type `application/vnd.apple.mpegurl` / `application/x-mpegURL`） |
| `WebPage` | 其余情况 |

重定向自动跟随。`WebPage` 且 yt-dlp 可用 → 提示走 yt-dlp 解析；yt-dlp 不可用 → 报错并给出安装提示。

### 5.2 直链下载（direct.rs）

1. `HEAD` 获取大小与 `Accept-Ranges`；
2. 支持 Range 且大小已知：切 N 块（每块 ≥4MB，上限 = 并发数），各块独立 `.part-<i>` 文件并行下载；
3. 不支持 Range：单流写 `.part`；
4. 全部完成后按块序流式拼接（读 64KB 写 64KB）为目标文件，删除 `.part-*`。

### 5.3 HLS 下载（hls/）

1. 拉取 m3u8；master playlist 则解析出所有 variant（分辨率、码率），GUI 弹出清晰度选择，默认最高；
2. 解析 media playlist：分片绝对 URL（相对路径按 m3u8 URL 解析）、`#EXT-X-KEY`（METHOD=AES-128 时取 URI 与 IV）、`#EXT-X-MAP`（fMP4 的 init segment）、 discontinuity 标记；
3. Semaphore 6 路并发下载分片；AES-128 分片先取 key（按 key URI 缓存，不重复请求）再解密；fMP4 先下 init segment；
4. 全部分片写入任务临时目录；生成 ffmpeg concat 列表，调用内嵌 ffmpeg `-c copy -bsf:a aac_adtstoasc`（ts 时）转封装为 mp4；
5. 删除临时目录。

### 5.4 yt-dlp 集成（ytdlp.rs）

- 设置中开启后，`WebPage` 探测结果提供"用 yt-dlp 解析"入口；
- 调 `yt-dlp -J <url>` 拿元数据 JSON，解析格式列表（format_id、分辨率、ext、filesize）供用户选择；
- 选定格式的 URL 交回 §5.2 / §5.3 路径下载（本地 HTTP 直链按直链处理；m3u8 按 HLS 处理）。

### 5.5 ffmpeg 内嵌（ffmpeg.rs）

- 静态二进制存放于 `vendor/ffmpeg/<target>/`，随仓库管理（不入 cargo 编译，运行期由 `include_bytes!` 按编译 target 条件编译嵌入）；
- 首次需要合并时释放到应用数据目录（`directories` crate 定位），加可执行权限，校验 SHA256 后复用；若用户环境变量/设置里指定了自定义 ffmpeg 路径则优先使用；
- 对外只暴露 `remux_to_mp4(inputs: &[PathBuf], output: &Path)`，内部拼接参数并子进程调用，解析 stderr 报告进度。

## 6. 任务管理（task.rs）

**状态机：**

```
Queued → Probing → Downloading → Merging → Done
                    ↘ Failed / Canceled（任意阶段可达）
```

- **暂停**（Downloading 阶段）：取消 in-flight 请求，保留已下载的 `.part` / 分片；
- **断点续传**：每个任务目录存 `manifest.json`（任务 ID、URL、探测结果、块/分片清单及各自完成状态、目标路径）；应用启动时扫描下载目录恢复未完成任务到列表；
- **删除**：清理该任务所有 `.part` 文件、临时目录与 manifest；
- 同时下载任务数默认 2（队列内其余排队），可配置；
- **随时新增**：任何时刻（包括其他任务正在下载时）粘贴新 URL 都会立即创建任务进入队列，无需等待现有任务完成；
- 文件名初始取 URL 路径最后一段或 m3u8 的 title / yt-dlp 元数据，任务创建后在列表中可直接重命名（作用于最终输出文件，不改临时文件）。

## 7. 错误处理

- 分片/分块级重试：指数退避（1s → 2s → 4s），最多 3 次；最终失败把该块标 failed；
- 任务失败时在 GUI 展示失败块清单，支持"仅重试失败块"；
- probe 失败、HTTP 4xx/5xx、ffmpeg 非零退出等，均映射为面向用户的简短中文错误 + 详情折叠（完整错误写入日志文件）；
- 网络中断恢复后，续传逻辑自动接管（已完成块不重下）。

## 8. 数据与存储

- 应用数据目录（`directories::ProjectDirs`）：
  - `ffmpeg`（释放的内嵌二进制）
  - `settings.json`（下载目录、并发数、yt-dlp 开关）
  - `logs/easydownvideo.log`
- 下载目录默认 `~/Downloads/easydownvideo`，设置中可随时更改（目录选择对话框）。
- **更改保存位置的语义**：只影响之后新建的任务；已在进行中的任务沿用创建时确定的目录，避免跨目录搬移半成品文件。
- 每个任务一个子目录（含 `.parts/` 与 `manifest.json`），完成后只留最终 mp4。

## 9. GUI 设计（app.rs）

- 主界面单窗口：顶部 URL 输入框 + "下载"按钮（支持粘贴板自动嗅探），**任何时刻提交即新建任务**，已有任务照常下载；中部任务列表（每条：可编辑文件名、实时进度条、百分比、速度、ETA、状态、暂停/继续/删除按钮）；底部状态栏（全局总速度、活跃任务数、版本号）；
- m3u8 清晰度选择、yt-dlp 格式选择以模态窗口呈现；
- 设置窗口：下载目录（含"更改"目录选择对话框，立即生效于新任务）、并发数、同时任务数、yt-dlp 开关、ffmpeg 路径覆盖；
- 浅色/深色跟随系统（egui 原生支持）；
- 窗口最小尺寸 720×480，可缩放。

## 10. 测试策略

**core 走 TDD（先写测试）：**

- `hls/parse.rs`：fixture m3u8 样例（master、AES-128 加密、fMP4、相对/绝对路径混合）解析断言；
- `hls/crypto.rs`：AES-128 标准测试向量 + 已知 key/IV 的样例分片解密；
- `probe.rs`：mock HTTP 服务覆盖 Content-Type 判定、`#EXTM3U` 嗅探、重定向、Range 支持检测；
- `direct.rs`：本地 mock 服务跑通"分块下载 → 拼接 → 与源文件字节一致"；
- `task.rs`：状态机转换、manifest 序列化/恢复、失败块单独重试。

**不测的部分：** 真实网站端到端下载（手动验收）；GUI（保持薄壳，手动验收清单：下 mp4 / 下 m3u8 / 暂停 / 续传 / 删除各过一遍）。

**验收清单（手动）：**

- [ ] macOS / Windows / Linux 各平台 `cargo build --release` 通过并产出单文件
- [ ] 大文件（≥2GB）直链下载内存 ≤100MB
- [ ] m3u8（AES-128 加密、fMP4 各一）下载合并出可播放 mp4
- [ ] 中途退出应用，重启后任务可续传
- [ ] yt-dlp 未安装时入口置灰；安装后可解析页面

## 11. 风险与对策

| 风险 | 对策 |
|---|---|
| vendor 的 ffmpeg 二进制体积大（~100MB 进 git） | 个人项目可接受；也可用 Git LFS，后续按需 |
| m3u8 变种繁多（LL-HLS、SAMPLE-AES、多 key） | v1 只支持 AES-128 与 fMP4 基础变体；遇到不支持的 tag 明确报错而非静默出错 |
| 网站反爬导致 yt-dlp 解析失败 | 属于 yt-dlp 自身范畴，我们只透传错误 |
| egui 大量任务列表的绘制性能 | 任务列表用 `egui::ScrollArea` + 行高固定；进度更新走原子快照，重绘开销与任务数解耦 |
