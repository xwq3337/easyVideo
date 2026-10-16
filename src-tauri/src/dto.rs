//! core 与前端之间的 wire 类型。
//!
//! 存在的理由：**core 不引入 serde**。这一层把 core 的类型翻译成前端能直接 switch 的形状，
//! 顺带把"core 改名"和"前端契约"解耦——core 内部怎么变，只要这里补齐翻译就不影响界面。
//! 字段一律 camelCase（前端习惯），Rust 侧用 `#[serde(rename_all = "camelCase")]`。

use easydownvideo::core::engine::TaskSnapshot;
use easydownvideo::core::events::{Choice, ChoiceKind, EngineEvent, ProgressUnit, TaskState};
use easydownvideo::settings::Settings;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StateDto {
    Queued,
    Probing,
    AwaitingChoice,
    Downloading,
    Merging,
    Paused,
    Done,
    Failed { message: String },
    Canceled,
}

impl From<&TaskState> for StateDto {
    fn from(s: &TaskState) -> Self {
        match s {
            TaskState::Queued => StateDto::Queued,
            TaskState::Probing => StateDto::Probing,
            TaskState::AwaitingChoice => StateDto::AwaitingChoice,
            TaskState::Downloading => StateDto::Downloading,
            TaskState::Merging => StateDto::Merging,
            TaskState::Paused => StateDto::Paused,
            TaskState::Done => StateDto::Done,
            TaskState::Failed(m) => StateDto::Failed { message: m.clone() },
            TaskState::Canceled => StateDto::Canceled,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UnitDto {
    Bytes,
    Segments,
}

impl From<ProgressUnit> for UnitDto {
    fn from(u: ProgressUnit) -> Self {
        match u {
            ProgressUnit::Bytes => UnitDto::Bytes,
            ProgressUnit::Segments => UnitDto::Segments,
        }
    }
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskDto {
    pub id: u64,
    pub url: String,
    pub filename: String,
    pub state: StateDto,
    pub downloaded: u64,
    pub total: u64,
    pub unit: UnitDto,
    /// 本轮运行已下字节数，与 `unit` 无关地恒为字节。界面用它算速度。
    pub bytes_downloaded: u64,
}

/// 从快照构造。`list_tasks()` 给的是**同一批** `ProgressHandle`，所以这里读到的进度
/// 与后台 worker 正在写的原子计数器是同一份，不存在快照过期的问题。
pub fn task_dto(t: TaskSnapshot) -> TaskDto {
    let (downloaded, total) = t.progress.snapshot();
    TaskDto {
        id: t.id,
        url: t.url,
        filename: t.filename,
        state: StateDto::from(&t.state),
        downloaded,
        total,
        unit: UnitDto::from(t.progress.unit()),
        bytes_downloaded: t.progress.bytes(),
    }
}

// `rename_all` 只管**变体名**，字段名要另外靠 `rename_all_fields`。这个枚举此前的字段
// 全是单词（id/state/downloaded/total/unit），所以这条一直没被需要；`bytes_downloaded`
// 是第一个多词字段，少了它发出去的就是 snake_case，TS 那边读到 undefined（速度恒为 0）。
#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum EventDto {
    StateChanged { id: u64, state: StateDto },
    Progress {
        id: u64,
        downloaded: u64,
        total: u64,
        unit: UnitDto,
        /// 永远以字节计，与 `downloaded`（单位随 `unit` 变）是两回事。界面算速度只用它。
        bytes_downloaded: u64,
    },
    ChoicesOffered { id: u64, kind: ChoiceKindDto },
    TaskRemoved { id: u64 },
}

pub fn event_dto(ev: EngineEvent) -> EventDto {
    match ev {
        EngineEvent::StateChanged { id, state } => EventDto::StateChanged {
            id,
            state: StateDto::from(&state),
        },
        EngineEvent::Progress {
            id,
            downloaded,
            total,
            unit,
            bytes_downloaded,
        } => EventDto::Progress {
            id,
            downloaded,
            total,
            unit: UnitDto::from(unit),
            bytes_downloaded,
        },
        EngineEvent::ChoicesOffered { id, kind } => EventDto::ChoicesOffered {
            id,
            kind: ChoiceKindDto::from(kind),
        },
        EngineEvent::TaskRemoved { id } => EventDto::TaskRemoved { id },
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VariantDto {
    pub uri: String,
    pub bandwidth: u64,
    pub resolution: Option<(u64, u64)>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FormatDto {
    pub id: String,
    pub ext: String,
    pub note: String,
    pub filesize: Option<u64>,
    pub url: String,
}

// **不用新类型变体**(`HlsVariants(Vec<VariantDto>)`)。serde 内部标签
// (`tag = "kind"`)不支持"新类型变体里装一个序列"——会在运行时炸:
//   cannot serialize tagged newtype variant ChoiceKindDto::HlsVariants containing a sequence
// 改成**结构体变体**就合法,而且 JSON 正好是前端 sketch 想要的
// `{"kind":"hlsVariants","hlsVariants":[...]}`(`rename_all_fields` 让内层字段也 camelCase)。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChoiceKindDto {
    HlsVariants { hls_variants: Vec<VariantDto> },
    YtdlpFormats { ytdlp_formats: Vec<FormatDto> },
}

impl From<ChoiceKind> for ChoiceKindDto {
    fn from(k: ChoiceKind) -> Self {
        match k {
            ChoiceKind::HlsVariants(v) => ChoiceKindDto::HlsVariants {
                hls_variants: v
                    .into_iter()
                    .map(|c| VariantDto {
                        uri: c.uri,
                        bandwidth: c.bandwidth,
                        resolution: c.resolution,
                    })
                    .collect(),
            },
            ChoiceKind::YtdlpFormats(f) => ChoiceKindDto::YtdlpFormats {
                ytdlp_formats: f
                    .into_iter()
                    .map(|c| FormatDto {
                        id: c.id,
                        ext: c.ext,
                        note: c.note,
                        filesize: c.filesize,
                        url: c.url,
                    })
                    .collect(),
            },
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChoiceDto {
    Variant { index: usize },
    YtdlpFormat { index: usize },
}

impl From<ChoiceDto> for Choice {
    fn from(c: ChoiceDto) -> Self {
        match c {
            ChoiceDto::Variant { index } => Choice::Variant(index),
            ChoiceDto::YtdlpFormat { index } => Choice::YtdlpFormat(index),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub download_dir: String,
    pub segment_concurrency: usize,
    pub max_active_tasks: usize,
    pub ytdlp_enabled: bool,
    pub ffmpeg_path_override: Option<String>,
    /// 从哪个浏览器读 cookie（`chrome` / `firefox` / `safari` …），`null` = 不读。
    pub cookies_from_browser: Option<String>,
    /// 是否伪装浏览器指纹（绕过 Cloudflare 的主要手段）。
    pub impersonate: bool,
}

pub fn settings_dto(s: Settings) -> SettingsDto {
    SettingsDto {
        download_dir: s.download_dir.to_string_lossy().into_owned(),
        segment_concurrency: s.segment_concurrency,
        max_active_tasks: s.max_active_tasks,
        ytdlp_enabled: s.ytdlp_enabled,
        ffmpeg_path_override: s.ffmpeg_path_override.map(|p| p.to_string_lossy().into_owned()),
        cookies_from_browser: s.cookies_from_browser,
        impersonate: s.impersonate,
    }
}

pub fn settings_from(s: SettingsDto) -> Settings {
    Settings {
        download_dir: s.download_dir.into(),
        segment_concurrency: s.segment_concurrency,
        max_active_tasks: s.max_active_tasks,
        ytdlp_enabled: s.ytdlp_enabled,
        ffmpeg_path_override: s.ffmpeg_path_override.map(Into::into),
        // 白名单在这里落闸：这个值会被拼进 yt-dlp 的命令行，而 DTO 是外部输入
        // （前端的下拉框挡不住手写的 IPC 调用）。
        cookies_from_browser: s
            .cookies_from_browser
            .as_deref()
            .and_then(easydownvideo::settings::sanitize_browser),
        impersonate: s.impersonate,
    }
}

/// 下载事件转 JSON。桥泛化成"接受一个 `fn(E) -> Value` 的映射函数"之后，
/// 这个转换就得有人做 —— 放在这里与 `video_event_dto` 对称，两边都由 dto 拥有线格式。
pub fn download_event_json(ev: EngineEvent) -> serde_json::Value {
    match serde_json::to_value(event_dto(ev)) {
        Ok(v) => v,
        Err(e) => {
            // 理论上到不了这里（EventDto 里没有 non-string map key 或 NaN），
            // 但原实现有这条日志，泛化时不该顺手把它丢掉。
            tracing::warn!("事件序列化失败: {e}");
            serde_json::Value::Null
        }
    }
}

// ===== 视频操作 =====
//
// 与下载那边**刻意分开**：`EventDto` 是下载契约的一部分，往里加视频变体会让两边的
// 类型互相污染，也会逼前端在一个联合类型里同时处理两套东西。所以视频事件直接转
// JSON 交给桥（`video_event_dto`），字段风格仍与下载事件一致（`type` 内部标签 +
// camelCase 字段），前端的路由方式也就跟着一致。

use easydownvideo::core::videoops::{JobSnapshot, JobState, VideoOpEvent};

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum JobStateDto {
    Queued,
    Running,
    Done,
    Failed { message: String },
    Canceled,
}

impl From<&JobState> for JobStateDto {
    fn from(s: &JobState) -> Self {
        match s {
            JobState::Queued => Self::Queued,
            JobState::Running => Self::Running,
            JobState::Done => Self::Done,
            JobState::Failed(m) => Self::Failed { message: m.clone() },
            JobState::Canceled => Self::Canceled,
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JobDto {
    pub id: u64,
    pub label: String,
    pub input: String,
    pub output: String,
    pub state: JobStateDto,
    /// `None` = 时长未知，界面显示不确定态进度条。**必须是 null 而不是缺字段。**
    pub progress: Option<f64>,
}

pub fn job_dto(j: JobSnapshot) -> JobDto {
    JobDto {
        id: j.id,
        label: j.label,
        input: j.input.to_string_lossy().into_owned(),
        output: j.output.to_string_lossy().into_owned(),
        state: JobStateDto::from(&j.state),
        progress: j.progress,
    }
}

/// 读一个视频的基本信息（界面上的预览，不是任务）。
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfoDto {
    /// `None` = 读不到（损坏文件等）。界面显示"时长未知"，不显示 0。
    pub duration_secs: Option<f64>,
    pub has_video: bool,
    pub size_bytes: u64,
}

pub fn video_event_dto(ev: VideoOpEvent) -> serde_json::Value {
    match ev {
        VideoOpEvent::Added { id } => serde_json::json!({ "type": "added", "id": id }),
        VideoOpEvent::Removed { id } => serde_json::json!({ "type": "removed", "id": id }),
        VideoOpEvent::Progress { id, fraction } => {
            serde_json::json!({ "type": "progress", "id": id, "fraction": fraction })
        }
        VideoOpEvent::StateChanged { id, state } => serde_json::json!({
            "type": "stateChanged",
            "id": id,
            "state": serde_json::to_value(JobStateDto::from(&state))
                .unwrap_or(serde_json::Value::Null),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use easydownvideo::core::events::{ChoiceKind, EngineEvent, ProgressUnit, TaskState};
    use easydownvideo::core::hls::parse::VariantChoice;
    use easydownvideo::core::ytdlp::YtdlpFormat;

    fn snap(state: TaskState) -> easydownvideo::core::engine::TaskSnapshot {
        easydownvideo::core::engine::TaskSnapshot {
            id: 7,
            url: "https://example.com/a.mp4".into(),
            filename: "a.mp4".into(),
            state,
            progress: easydownvideo::core::events::ProgressHandle::new(),
        }
    }

    #[test]
    fn failed_state_carries_its_message_as_a_tagged_object() {
        let d = task_dto(snap(TaskState::Failed("ffmpeg 非零退出".into())));
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(j["state"]["kind"], "failed");
        assert_eq!(j["state"]["message"], "ffmpeg 非零退出");
        assert_eq!(j["id"], 7);
        assert_eq!(j["filename"], "a.mp4");
    }

    #[test]
    fn plain_states_serialize_as_a_bare_kind() {
        let j = serde_json::to_value(task_dto(snap(TaskState::AwaitingChoice))).unwrap();
        assert_eq!(j["state"]["kind"], "awaitingChoice");
    }

    #[test]
    fn progress_event_pins_every_key() {
        // 走**整个对象相等**：id/total 与 type/unit/downloaded 同属这一份契约，
        // 任何一个键改名或漏掉都要在这里变红，而不是静默地只在 TS 镜像里错位。
        let d = event_dto(EngineEvent::Progress {
            id: 3,
            downloaded: 100,
            total: 400,
            unit: ProgressUnit::Segments,
            bytes_downloaded: 2_000_000,
        });
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({
                "type": "progress",
                "id": 3,
                "downloaded": 100,
                "total": 400,
                "unit": "segments",
                "bytesDownloaded": 2_000_000
            })
        );
    }

    #[test]
    fn settings_round_trip() {
        let s = easydownvideo::settings::Settings {
            download_dir: std::path::PathBuf::from("/tmp/x"),
            segment_concurrency: 6,
            max_active_tasks: 2,
            ytdlp_enabled: true,
            ffmpeg_path_override: None,
            // 刻意都用**非默认值**：用默认值的话，往返里把这两个字段整个漏掉也照样绿。
            cookies_from_browser: Some("firefox".into()),
            impersonate: true,
        };
        let d = settings_dto(s.clone());
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(j["downloadDir"], "/tmp/x");
        assert_eq!(j["segmentConcurrency"], 6);
        assert_eq!(j["cookiesFromBrowser"], "firefox");
        assert_eq!(j["impersonate"], true);
        assert_eq!(settings_from(d), s, "往返之后必须与原来完全一致");
    }

    /// 入站方向的注入防护。`cookies_from_browser` 会被拼进 yt-dlp 的命令行，
    /// 而它来自前端的设置页 —— 前端的下拉框挡不住手写的 IPC 调用，所以 core 这边
    /// 必须自己再挡一次。
    #[test]
    fn settings_from_rejects_a_non_whitelisted_browser() {
        let d = SettingsDto {
            download_dir: "/tmp/x".into(),
            segment_concurrency: 6,
            max_active_tasks: 2,
            ytdlp_enabled: true,
            ffmpeg_path_override: None,
            cookies_from_browser: Some("chrome --exec rm -rf /".into()),
            impersonate: false,
        };
        assert_eq!(
            settings_from(d).cookies_from_browser,
            None,
            "不在白名单里的浏览器名必须被丢掉，不能原样进命令行"
        );

        // 合法名照常保留（顺带覆盖大小写/空格规范化）
        let d = SettingsDto {
            cookies_from_browser: Some(" Chrome ".into()),
            ..serde_json::from_str::<SettingsDto>(
                r#"{"downloadDir":"/tmp/x","segmentConcurrency":6,"maxActiveTasks":2,
                    "ytdlpEnabled":true,"ffmpegPathOverride":null,
                    "cookiesFromBrowser":"Chrome","impersonate":false}"#,
            )
            .unwrap()
        };
        assert_eq!(settings_from(d).cookies_from_browser.as_deref(), Some("chrome"));
    }

    /// 守卫 `ChoicesOffered` 这条**事件**路径的 wire 形状——前端真正收到的东西。
    ///
    /// 为什么它必须单独存在：上面四条测试没有一条序列化 `ChoicesOffered`，而
    /// `ChoiceKindDto` 一度是**新类型变体**(`HlsVariants(Vec<..>)`)——那种写法**编译得过**，
    /// 只在序列化时炸：`cannot serialize tagged newtype variant ... containing a sequence`。
    /// 也就是说，有人把结构体变体"顺手简化"回新类型变体时，**没有任何一条别的测试会变红**，
    /// 只有真实运行时才会崩。想动这个枚举，先看这条测试为什么这么写。
    ///
    /// 这里刻意走 `event_dto(...)` 而不是直接构造 `ChoiceKindDto`：要钉的是事件路径。
    /// 断言用**整个对象的相等**而非逐个取字段——这样既能钉住内部标签与内层 camelCase 字段名
    /// (`hlsVariants` / `ytdlpFormats`，正是 `rename_all_fields` 买来的、TS 镜像要读的名字)，
    /// 也能钉住 payload 内部每一个键(uri/bandwidth/resolution、id/ext/note/filesize/url)，
    /// 且多出或漏掉任何一个键都会红。
    #[test]
    fn choices_offered_pins_the_internal_tag_and_inner_field_names() {
        let hls = event_dto(EngineEvent::ChoicesOffered {
            id: 3,
            kind: ChoiceKind::HlsVariants(vec![VariantChoice {
                uri: "https://example.com/1080.m3u8".into(),
                bandwidth: 5_000_000,
                resolution: Some((1920, 1080)),
            }]),
        });
        assert_eq!(
            serde_json::to_value(&hls).unwrap(),
            serde_json::json!({
                "type": "choicesOffered",
                "id": 3,
                "kind": {
                    "kind": "hlsVariants",
                    "hlsVariants": [{
                        "uri": "https://example.com/1080.m3u8",
                        "bandwidth": 5_000_000,
                        "resolution": [1920, 1080]
                    }]
                }
            })
        );

        // `resolution: None` 必须是 `null`，不能把字段省掉——TS 侧是 `[number, number] | null`。
        let hls_none = event_dto(EngineEvent::ChoicesOffered {
            id: 3,
            kind: ChoiceKind::HlsVariants(vec![VariantChoice {
                uri: "https://example.com/audio.m3u8".into(),
                bandwidth: 128_000,
                resolution: None,
            }]),
        });
        let j = serde_json::to_value(&hls_none).unwrap();
        assert_eq!(j["kind"]["hlsVariants"][0]["resolution"], serde_json::Value::Null);
        assert!(j["kind"]["hlsVariants"][0].get("resolution").is_some());

        let ytdlp = event_dto(EngineEvent::ChoicesOffered {
            id: 3,
            kind: ChoiceKind::YtdlpFormats(vec![YtdlpFormat {
                id: "137".into(),
                ext: "mp4".into(),
                note: "1080p video only".into(),
                filesize: Some(123_456_789),
                url: "https://example.com/v".into(),
            }]),
        });
        assert_eq!(
            serde_json::to_value(&ytdlp).unwrap(),
            serde_json::json!({
                "type": "choicesOffered",
                "id": 3,
                "kind": {
                    "kind": "ytdlpFormats",
                    "ytdlpFormats": [{
                        "id": "137",
                        "ext": "mp4",
                        "note": "1080p video only",
                        "filesize": 123_456_789,
                        "url": "https://example.com/v"
                    }]
                }
            })
        );
    }

    /// 前端必须在**全部九种** state 上 switch，所以九种 kind 字符串都得钉住。
    /// 用整对象相等：多一个键、少一个键、大小写变了都会红。
    #[test]
    fn every_state_kind_serializes_to_its_exact_wire_string() {
        let cases: [(TaskState, &str); 8] = [
            (TaskState::Queued, "queued"),
            (TaskState::Probing, "probing"),
            (TaskState::AwaitingChoice, "awaitingChoice"),
            (TaskState::Downloading, "downloading"),
            (TaskState::Merging, "merging"),
            (TaskState::Paused, "paused"),
            (TaskState::Done, "done"),
            (TaskState::Canceled, "canceled"),
        ];
        for (state, kind) in cases {
            assert_eq!(
                serde_json::to_value(StateDto::from(&state)).unwrap(),
                serde_json::json!({ "kind": kind }),
                "state {state:?} 应当序列化成 {{kind: {kind:?}}}"
            );
        }
        // 第九种：带 message 的那个。
        assert_eq!(
            serde_json::to_value(StateDto::from(&TaskState::Failed("boom".into()))).unwrap(),
            serde_json::json!({ "kind": "failed", "message": "boom" })
        );
    }

    /// `StateChanged` 是 reducer 必须 switch 的另一条事件。它的 `type` 标签是多词 camelCase
    /// (`stateChanged`)，最容易在改名时静默漂移，而它此前**完全没有测试**。
    #[test]
    fn state_changed_event_pins_its_tag_and_state() {
        let d = event_dto(EngineEvent::StateChanged {
            id: 9,
            state: TaskState::Paused,
        });
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({
                "type": "stateChanged",
                "id": 9,
                "state": { "kind": "paused" }
            })
        );

        // state 里带 message 的分支也要走通，别只在 TaskDto 那一层测过。
        let d = event_dto(EngineEvent::StateChanged {
            id: 9,
            state: TaskState::Failed("截图解码失败".into()),
        });
        assert_eq!(
            serde_json::to_value(&d).unwrap()["state"],
            serde_json::json!({ "kind": "failed", "message": "截图解码失败" })
        );
    }

    /// `TaskDto` 的**每一个**键都要钉住。`url` / `downloaded` / `total` / `unit` 此前只在
    /// `Progress` 事件里出现过——那是另一个对象，钉不住 `TaskDto` 自己的键名。
    /// 这里特意用 `Segments` 单位，顺带覆盖 `unit` 在 `TaskDto` 上的取值。
    #[test]
    fn task_dto_ships_exactly_the_contract_keys() {
        let progress = easydownvideo::core::events::ProgressHandle::new();
        progress.add(250);
        progress.add_bytes(1_000_000);
        progress.set_total(1000);
        progress.set_unit(ProgressUnit::Segments);
        let t = easydownvideo::core::engine::TaskSnapshot {
            id: 7,
            url: "https://example.com/a.mp4".into(),
            filename: "a.mp4".into(),
            state: TaskState::Downloading,
            progress,
        };
        assert_eq!(
            serde_json::to_value(task_dto(t)).unwrap(),
            serde_json::json!({
                "id": 7,
                "url": "https://example.com/a.mp4",
                "filename": "a.mp4",
                "state": { "kind": "downloading" },
                "downloaded": 250,
                "total": 1000,
                "unit": "segments",
                "bytesDownloaded": 1_000_000
            })
        );
    }

    /// `SettingsDto` 的每一个键名都不能漏。`settings_from(d) == s` 那条是**Rust 结构体**比较，
    /// 一个 JSON 键名都钉不住——所以必须单独有这条。
    #[test]
    fn settings_dto_ships_exactly_the_contract_keys() {
        let s = Settings {
            download_dir: std::path::PathBuf::from("/tmp/x"),
            segment_concurrency: 6,
            max_active_tasks: 2,
            ytdlp_enabled: true,
            ffmpeg_path_override: Some(std::path::PathBuf::from("/opt/ffmpeg")),
            cookies_from_browser: Some("edge".into()),
            impersonate: true,
        };
        assert_eq!(
            serde_json::to_value(settings_dto(s)).unwrap(),
            serde_json::json!({
                "downloadDir": "/tmp/x",
                "segmentConcurrency": 6,
                "maxActiveTasks": 2,
                "ytdlpEnabled": true,
                "ffmpegPathOverride": "/opt/ffmpeg",
                "cookiesFromBrowser": "edge",
                "impersonate": true
            })
        );

        // 没选浏览器时同样必须是 `null` 而不是缺字段：TS 写的是 `string | null`。
        let j = serde_json::to_value(settings_dto(Settings::default())).unwrap();
        assert_eq!(j["cookiesFromBrowser"], serde_json::Value::Null);
        assert!(j.get("cookiesFromBrowser").is_some());
        assert_eq!(j["impersonate"], false);

        // None 必须是 `null` 而不是缺字段：TS 写的是 `string | null`，不是可选属性。
        let s = Settings {
            ffmpeg_path_override: None,
            ..Settings::default()
        };
        let j = serde_json::to_value(settings_dto(s)).unwrap();
        assert_eq!(j["ffmpegPathOverride"], serde_json::Value::Null);
        assert!(j.get("ffmpegPathOverride").is_some());
    }

    /// 入站方向：前端**发**给我们的形状。此前 `ChoiceDto` 只 derive 了 `Deserialize`，
    /// 从来没有一条测试真的喂过它一个 JSON 值。
    #[test]
    fn choice_dto_deserializes_the_inbound_shape() {
        let d: ChoiceDto = serde_json::from_str(r#"{"kind":"variant","index":2}"#).unwrap();
        assert_eq!(d, ChoiceDto::Variant { index: 2 });
        match Choice::from(d) {
            Choice::Variant(2) => {}
            other => panic!("variant 应转成 Choice::Variant(2)，得到 {other:?}"),
        }

        let d: ChoiceDto = serde_json::from_str(r#"{"kind":"ytdlpFormat","index":5}"#).unwrap();
        assert_eq!(d, ChoiceDto::YtdlpFormat { index: 5 });
        match Choice::from(d) {
            Choice::YtdlpFormat(5) => {}
            other => panic!("ytdlpFormat 应转成 Choice::YtdlpFormat(5)，得到 {other:?}"),
        }

        // 标签名是精确契约：写错大小写或换名字必须解析失败，不能静默落到某个默认分支。
        assert!(serde_json::from_str::<ChoiceDto>(r#"{"kind":"Variant","index":2}"#).is_err());
        assert!(serde_json::from_str::<ChoiceDto>(r#"{"kind":"variant","idx":2}"#).is_err());
    }

    /// `taskRemoved` 是前端 reducer 必须 switch 的第三条事件。它的 `type` 标签是
    /// 多词 camelCase，最容易在改名时静默漂移，而漂移的后果是**行永远删不掉**
    /// （reducer 的 default 分支吞掉它）—— 正是这次要修的 bug 的翻版。
    #[test]
    fn task_removed_event_pins_its_tag_and_id() {
        let d = event_dto(EngineEvent::TaskRemoved { id: 42 });
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({ "type": "taskRemoved", "id": 42 })
        );
    }
    /// 视频任务事件的 wire 形状。与下载事件一样走**整个对象相等**，
    /// 漏键或改名都要在这里变红。
    #[test]
    fn video_job_events_pin_their_keys() {
        let j = video_event_dto(VideoOpEvent::StateChanged {
            id: 3,
            state: JobState::Failed("ffmpeg 失败".into()),
        });
        assert_eq!(
            j,
            serde_json::json!({
                "type": "stateChanged",
                "id": 3,
                "state": { "kind": "failed", "message": "ffmpeg 失败" }
            })
        );

        let j = video_event_dto(VideoOpEvent::Progress { id: 3, fraction: 0.25 });
        assert_eq!(j, serde_json::json!({ "type": "progress", "id": 3, "fraction": 0.25 }));

        let j = video_event_dto(VideoOpEvent::Added { id: 9 });
        assert_eq!(j, serde_json::json!({ "type": "added", "id": 9 }));
        let j = video_event_dto(VideoOpEvent::Removed { id: 9 });
        assert_eq!(j, serde_json::json!({ "type": "removed", "id": 9 }));
    }

    /// 任务快照的每一个键都要钉住。`progress` 为 `None` 时必须是 `null` 而不是缺字段
    /// —— 前端写的是 `number | null`。
    #[test]
    fn job_dto_ships_exactly_the_contract_keys() {
        let j = job_dto(JobSnapshot {
            id: 7,
            label: "压缩 · 均衡".into(),
            input: std::path::PathBuf::from("/in/a.mp4"),
            output: std::path::PathBuf::from("/out/a-压缩.mp4"),
            state: JobState::Running,
            progress: Some(0.5),
        });
        assert_eq!(
            serde_json::to_value(&j).unwrap(),
            serde_json::json!({
                "id": 7,
                "label": "压缩 · 均衡",
                "input": "/in/a.mp4",
                "output": "/out/a-压缩.mp4",
                "state": { "kind": "running" },
                "progress": 0.5
            })
        );

        let j = job_dto(JobSnapshot {
            id: 8,
            label: "压缩 · 小体积".into(),
            input: "/in/b.mp4".into(),
            output: "/out/b-压缩.mp4".into(),
            state: JobState::Queued,
            progress: None,
        });
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v["progress"], serde_json::Value::Null);
        assert!(v.get("progress").is_some(), "None 必须是 null 而不是缺字段");
    }
}
