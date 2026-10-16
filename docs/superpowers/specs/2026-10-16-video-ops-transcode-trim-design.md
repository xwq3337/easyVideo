# EasyDownVideo · 视频操作（其二）· 转码与裁剪设计

- 日期：2026-10-16
- 状态：待评审
- 前置：`docs/superpowers/specs/2026-10-16-video-ops-design.md`（子系统、事件桥、压缩通路，仍然有效）
- 定位：把「视频操作」从"只有压缩能用"扩到三个可用。本轮含一次**引擎泛化**

## 1. 背景与目标

上一轮立起了视频操作的完整通路（选文件 → 调参数 → 排队 → 进度 → 取消 → 产物落盘），
但只实现了一个操作。侧边栏里另外四项照实灰显着。

**目标**：让「转码」与「裁剪视频」真能用，并把引擎接口泛化到能给任意操作复用。
**合并**与**画面裁剪**仍不做 —— 它们各自带一个独立的设计问题（多文件输入；抽帧预览界面），
混进这一轮会把两边都做浅。

**成功标准**：拿一个真实视频，转码成 WebM 能播、裁剪出的片段起止正确；三个操作共享同一套
排队/进度/取消/命名行为；合并与画面裁剪仍然灰显且点了不导航。

## 2. 家底与实测结论

上轮的结论（内嵌 ffmpeg 9.0.2 是完整版、没有 ffprobe、`out_time_ms` 也是微秒、
VideoToolbox 静默忽略 `-crf`）仍然有效。本轮新增三条，都是实测的：

**容器与编码器不是自由组合。**

```
webm + aac  → 硬失败：[webm] Only VP8 or VP9 or AV1 video and Vorbis or Opus audio
              ... are supported for WebM / Could not write header
webm + opus → 正常
h265 + mp4  → 正常
```

可用封装器：`matroska` / `mp4` / `webm` / `mov`。可用音频编码器：`aac` / `libmp3lame` /
`libopus` / `libvorbis`。

**所以界面不能把"容器"和"编码器"做成两根独立的轴** —— 那等于允许用户拼出跑不通的组合，
而且要到任务失败才告诉他。这与 `-crf`/`-q:v` 是同一类错误：给出互相矛盾的旋钮。

**`-c copy` 裁剪实测是准的，与流传的说法不符。**

用一段强制 GOP = 10 秒的素材（关键帧实测在 0 / 10 / 20 秒）：

| 请求区间 | 产物时长 | 产物首帧 |
|---|---|---|
| `-ss 12 -to 20 -c copy` | 8.00 秒 | 与源在 12.0s 的帧**完全相同**（MD5 一致） |
| `-ss 12.5 -to 20 -c copy` | 7.50 秒 | 与源在 12.5s 的帧**完全相同** |

如果它吸附到关键帧，两次都会是 10 秒长。所以在这台 ffmpeg 上，"快速切会提前到关键帧"
**复现不出来**。界面上不许把这件事写成确定的承诺 —— 文案按**实测到的差别**写
（快 + 无损 vs 慢 + 多一次重编码），偏移只作为"个别源上可能出现"的提示。

**探针的教训（方法学）**：验证关键帧位置时用
`-v info -i x.mp4 -vf "select='eq(pict_type\,I)',showinfo" -fps_mode passthrough -f null -`
再 grep `pts_time`。**不能用 `-v error`** —— 那会把 `showinfo` 的输出一并掐掉（它走 info 级），
于是 30 秒的片子会"数出 899 个关键帧"，得到完全错误的结论。`-skip_frame nokey` + 导出图片
那条路本轮实测也是不可靠的。

## 3. 范围

**本轮做**：引擎泛化（`JobParams` 分派、产物扩展名随格式走）、转码（四种目标格式）、
裁剪视频（快速 / 精确两档）、命令面收敛为一条、前端拆成外框 + 三个表单。

**本轮不做**（继续灰显、点了不导航）：合并、画面裁剪。

## 4. 引擎泛化

### 4.1 参数枚举取代写死的压缩

```rust
// src/core/videoops/params.rs
pub enum JobParams {
    Compress(CompressParams),
    Transcode(TranscodeParams),
    Trim(TrimParams),
}

impl JobParams {
    /// 队列那一行显示的标签，形如 "压缩 · 均衡"。
    pub fn label(&self) -> String;
    /// 拼出完整的 ffmpeg 参数。
    pub fn args(&self, input: &Path, output: &Path) -> Vec<String>;
    /// 产物名的后缀，形如 "-压缩"。
    pub fn suffix(&self) -> &'static str;
    /// 产物的扩展名。**不是常量** —— 见下。
    pub fn extension(&self, src: &MediaInfo) -> String;
}
```

用**一个枚举 + match**，不引入 `Op` trait：只有三个实现，trait 只会多一层间接，
而分派集中在一处反而更易读。四个能力（label / args / suffix / extension）一个不少。

`VideoOps::add_job(input, output_dir, params: JobParams)` 取代 `add_compress_job`。

### 4.2 `extension()` 为什么不能是常量

上一轮容器写死 mp4、产物名写死 `.mp4`，所以后缀与扩展名可以合并成一个概念。现在不行：

- 转码到 WebM，产物必须是 `.webm`；
- **裁剪用快速档时必须沿用源容器**（copy 到别的容器常常直接失败），所以扩展名要看源文件；
- 裁剪用精确档则输出 mp4（我们重编码成 h264+aac，mp4 一定装得下 —— 与压缩页同一句话）。

于是同一个裁剪，选「快速」产物可能是 `.mkv`、选「精确」是 `.mp4`。**这个反直觉之处必须在
界面上显式说出来**，做法见 §7 的"产物名预览"。

### 4.3 模块重组

```
src/core/videoops/
  mod.rs           VideoOps：任务表、队列、并发、取消、入队校验
  params.rs        JobParams 枚举与四个能力（新增）
  compress.rs      原 presets.rs 改名 —— 现在有三个操作了，"presets" 不再描述它
  transcode.rs     目标格式表与参数构造（新增）
  trim.rs          时间解析、两档参数构造（新增）
  ffmpeg_job.rs    跑一次 ffmpeg（不变）
  media_info.rs    媒体信息（扩两个字段）
```

改名是纯机械的（`mod.rs` 与三个测试文件的 import），但不做的话，一个装着压缩参数的
文件叫 `presets.rs`、隔壁叫 `transcode.rs`，下一个读代码的人会先困惑一轮。

### 4.4 `MediaInfo` 扩两个字段

```rust
pub struct MediaInfo {
    pub duration_secs: Option<f64>,
    pub has_video: bool,
    /// 视频编码，形如 "h264" / "hevc" / "vp9"。取不到时 None。
    pub video_codec: Option<String>,
    /// 容器，形如 "mp4" / "matroska" / "webm"。取不到时 None。
    pub container: Option<String>,
    pub size_bytes: u64,
}
```

容器从 `Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'x.mp4'` 那行的格式串里取（取第一个能对上
已知表的），编码从 `Stream #0:… Video: h264 …` 那行取。解析器 `parse_media_info` 已经有了，
加两个字段与对应测试。

**为什么必须加**：转码页要显示"源：H.264 · mp4"，否则用户选"转码 → MP4·H.264"时
看不出这是在把同一个格式重编一遍（白掉一次画质）。

## 5. 转码的行为

### 5.1 四种目标格式（整组打包）

| 目标格式 | 视频编码 | 音频编码 | 容器 | 说明 |
|---|---|---|---|---|
| MP4 · H.264 + AAC | libx264 | aac | mp4 | 兼容性最好，默认 |
| MP4 · H.265 + AAC | libx265 | aac | mp4 | 同画质体积更小，老设备可能不支持 |
| WebM · VP9 + Opus | libvpx-vp9 | libopus | webm | 网页友好 |
| MKV · H.264 + AAC | libx264 | aac | matroska | 容器最宽容 |

**非法组合从构造上不存在** —— 界面给的是这四组，不是"容器 + 视频编码 + 音频编码"三根轴。

VP9 编码明显更慢（软件编，且 `-b:v 0 -crf N` 这一套与 x264 的 `-crf` 不是一回事），
界面上要写一句提示；不拦。

### 5.2 质量三档，但每个编码器一套 CRF

沿用「小体积 / 均衡 / 高画质」三档，但 **CRF 数值不能跨编码器照抄** —— x265 与 VP9 的
CRF 刻度与 x264 不同。

判据可测量（沿用上一轮校准 q:v 的做法）：**同档位的产物体积，应落在 x264 同档产物的
0.3–1.5 倍之间**，且**任意一档都不得大于源文件**（后半条是真正会伤到用户的那条：他要的是
转码，拿到一个更大的文件就是白干）。实施计划里排一步实测校准，结论写进 `transcode.rs`
的常量并附上校准依据。

### 5.3 高级项

转码页**只有质量档位**，不给编码器/分辨率。那两个留给压缩页 —— 两个页面都摆全套只会让用户
分不清该进哪个（它们本来就容易混，见 §5.1 的"转码 = 换格式"定位）。

源格式已经等于目标格式时不拦，但界面上显示源格式，让用户自己看得出来。

## 6. 裁剪的行为

### 6.1 参数

```rust
pub struct TrimParams {
    pub start: f64,
    pub end: Option<f64>,   // None = 到结尾
    pub mode: TrimMode,
}
pub enum TrimMode { Fast, Accurate }
```

`end` 留空 = 到结尾（"从 1:30 到结束"是常见需求）。校验 `0 ≤ start`、`start < end`（给了 end 时）。

### 6.2 两档的命令行与容器

| 档位 | 命令行 | 产物容器 |
|---|---|---|
| 快速 | `-ss <start> -to <end> -i in -c copy` | **沿用源容器**（copy 到别的容器常常直接失败） |
| 精确 | `-ss <start> -to <end> -i in -c:v libx264 -preset veryfast -crf <档位> -c:a aac` | mp4 |

精确档复用压缩页的质量档位（默认「均衡」）。`-ss`/`-to` 都放在 `-i` **之前**（输入选项），
这样 mp4 的索引能给出精确寻址 —— §2 的实测就是按这个写法做的。

### 6.3 界面文案（按实测写，不按 lore 写）

- **快速（不重编码）**：秒完，画质一点不丢。个别源上切口可能略有偏移。
- **精确（重新编码）**：慢，会重新编码一次（多一次质量损失）。快速切的结果不对时用它。

措辞刻意不对称：快速档把"偏移"写成**可能**而不是"会"，因为我们在 §2 的实测里复现不出来；
精确档把"多一次质量损失"写成确定，因为那是重编码的必然。

### 6.4 时间输入

接受 `mm:ss`、`hh:mm:ss`、纯数字三种写法；**纯数字一律按秒解释**（`90` 是 90 秒，不是 90 分）。解析是纯函数、单测覆盖（含非法输入与越界）。
界面上显示源时长，并把当前选择翻成人话：`0:12 → 0:20 · 共 8 秒`。

## 7. 命令面与前端结构

### 7.1 一条命令，带 tag 的参数

八位置参数那股味道要收掉，同时保住编译期类型检查（`set_choice` 已经在用这个模式）：

```rust
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum JobParamsDto {
    Compress { preset: String, encoder: String, max_height: Option<u32>,
               crf: Option<u8>, qv: Option<u8>, audio_kbps: Option<u32> },
    Transcode { target: String, preset: String },
    Trim { start_secs: f64, end_secs: Option<f64>, mode: String },
}
```

命令：`add_video_job(input: String, output_dir: String, params: JobParamsDto) -> Result<u64, String>`。
原有的 `add_compress_job` **删掉**（前端同步改），不留两个入口。

### 7.2 前端拆成外框 + 三个表单

`VideoOpPage.tsx` 现在全是压缩的 UI，再塞两个操作会变成一个巨型文件。

```
pages/VideoOpPage.tsx          只做分派：按 op 选表单，套进外框
pages/ops/VideoOpShell.tsx     选文件 / probe 信息 / 输出目录 / 产物名预览 / 开始按钮 / 任务列表
pages/ops/CompressForm.tsx     只有压缩的参数区
pages/ops/TranscodeForm.tsx
pages/ops/TrimForm.tsx
```

三个表单共用的是"外框"里那些；每个表单只负责自己的参数区并把结果交给外框提交。

**产物名预览**：外框显示 `产物：第一集-裁剪.mkv`。它同时解决三件事 —— 让路命名（` (1)`）
提前可见、扩展名会随模式/格式变化这件事被显性化、以及"源文件不会被覆盖"这句承诺有了落点。

新增一条命令：

```rust
plan_output(input: String, output_dir: String, params: JobParamsDto) -> Result<String, String>
```

**它必须与 `add_video_job` 走同一个取名函数**，否则预览与实际落盘会在某个边角上分叉
（而那正是预览存在的意义）。为此把取名那一段抽成 `VideoOps` 上的一个私有方法，
两个入口都调它。代价是它会再跑一次 `ffmpeg -i`（只为拿源容器算扩展名）—— 参数变化时
才调用，可接受。

前端不自己拼名字：`unique_path_avoiding` 的让路规则、在途任务的占位、以及后缀与扩展名
的组合都在 core 里。

### 7.3 `VIDEO_OPS` 的 ready

压缩 / 转码 / 裁剪 三项 `ready: true`；合并与画面裁剪仍 `false`（灰显 + 悬停「还没做」）。
那条"本轮只有压缩是 ready"的守卫测试要跟着改成"这三项是 ready、那两项不是"。

## 8. 错误处理

沿用上一轮的全部规则（入队校验、失败删半截产物、改名失败也清理、stderr 取尾部 20 行/2 KB）。
本轮新增两条：

- **裁剪的时间越界**：`start` 超出源时长、或 `start ≥ end` → 入队时就报错，不排进去。
- **快速档裁一个容器与源不同的目标**：不可能发生（扩展名由 `extension(&src)` 算出来），
  但 `-f` 要显式用源容器的 muxer 名，别指望扩展名推断 —— 那条坑上一轮已经踩过
  （`.part` 后缀让 ffmpeg 推断失败）。

## 9. 测试策略

| 层 | 内容 |
|---|---|
| `trim::parse_time` | `mm:ss` / `hh:mm:ss` / 纯秒数三种写法；空、负数、`1:2:3:4`、非数字 → `None` |
| `trim::trim_args` | 两档各自的参数；**快速档必须带源容器的 `-f`**；`end` 为 None 时不出现 `-to` |
| `transcode::transcode_args` | 四种格式各自的编码器/音频编码/`-f`；**专门一条**断言 WebM 那组用的是 `libopus` 而不是 `aac`（这是"非法组合"的回归测试） |
| `transcode` 的 CRF 三档 | 每个编码器一套数值，断言互不相同且方向一致 |
| 媒体信息解析 | `video_codec` / `container` 从真实 stderr 片段里取；取不到时 `None` 而不 panic |
| **端到端真跑（转码）** | 造一段 H.264/mp4 → 转成 WebM → 断言产物存在、能被 ffmpeg 读回、**编码是 vp9 而音频是 opus** |
| **端到端真跑（裁剪）** | 两档各跑一次，断言产物时长 ≈ 请求区间 |
| **裁剪切口精度** | 用强制大 GOP 的素材，`-c copy` 裁出来的产物**首帧与源在请求时刻的帧 MD5 一致**（把 §2 的实测钉成回归测试） |
| **命名一致性** | `plan_output` 与 `add_video_job` 对同一组输入给出同一个路径（同一 helper），含让路场景 |
| 前端 | `lib/` 下的纯逻辑（时间格式化、档位与格式的映射）配 vitest；组件不做 DOM 测试 |

端到端那几条都**不需要网络**（素材现造、ffmpeg 内嵌），能进常规套件。

## 10. 验收清单（手动，逐项过）

1. 侧边栏「转码」「裁剪视频」不再灰显，点了能进。
2. 转码页显示源格式（如"H.264 · mp4"）；四种目标格式可选；质量三档可切。
3. 转码成 WebM → 任务跑完、产物是 `.webm`、能播。
4. 把源转成它已有的格式 → 不阻拦，但用户能从"源：…"那行看出来。
5. 裁剪页显示源时长；`0:12` 与 `12` 与 `00:00:12` 三种写法都能解析出 12 秒。
6. 选「快速」→ 秒完；选「精确」→ 慢一些；两次的产物时长都约等于请求区间。
7. 「产物：xxx」预览与实际落盘的名字一致；同名时预览里就已经带上了 ` (1)`。
8. 快速档裁一个 mkv → 产物是 `.mkv`；精确档裁同一个 → 产物是 `.mp4`。
9. `start` 超出源时长 → 入队时就报错，不排进队列。
10. 合并与画面裁剪仍然灰显，悬停提示「还没做」，点了不导航。

## 11. 风险与对策

| 风险 | 对策 |
|---|---|
| 转码的 CRF 跨编码器照抄 → 某档产物比源文件还大 | §5.2 的可测量判据 + 实施计划里的校准步骤；判据含"不得大于源文件" |
| 把"快速切会提前"写成确定承诺，而实测复现不出来 | §6.3 的措辞不对称：偏移写"可能"，重编码的损失写"确定" |
| 非法容器/编码组合流到 ffmpeg 才失败 | 目标格式整组打包（§5.1），并配 WebM/opus 的回归测试 |
| 产物扩展名随模式变化让用户困惑 | §7.2 的产物名预览 |
| 拆前端时把共用的"外框"逻辑抄三遍 | 外框是一个组件，三个表单只交参数 |
| 改 `presets.rs` 名字波及面大 | 纯机械改动，import 由编译器逐个揪出来；放在第一个任务做，做完立刻跑全量 |

## 12. 非目标（YAGNI）

- 合并、画面裁剪（各自独立一轮）
- 转码的三根自由轴（容器/视频编码/音频编码）
- 转码的自定义码率、两遍编码、目标体积逼近
- 裁剪的可拖拽时间轴与预览播放器
- 裁剪的批量（一次裁出多段）
- 硬件编码的转码（VideoToolbox 只出 h264，转码的价值在换格式）
- 产物名预览之外的"输出设置记忆"
