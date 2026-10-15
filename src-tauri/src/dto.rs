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
    }
}

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EventDto {
    StateChanged { id: u64, state: StateDto },
    Progress {
        id: u64,
        downloaded: u64,
        total: u64,
        unit: UnitDto,
    },
    ChoicesOffered { id: u64, kind: ChoiceKindDto },
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
        } => EventDto::Progress {
            id,
            downloaded,
            total,
            unit: UnitDto::from(unit),
        },
        EngineEvent::ChoicesOffered { id, kind } => EventDto::ChoicesOffered {
            id,
            kind: ChoiceKindDto::from(kind),
        },
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
}

pub fn settings_dto(s: Settings) -> SettingsDto {
    SettingsDto {
        download_dir: s.download_dir.to_string_lossy().into_owned(),
        segment_concurrency: s.segment_concurrency,
        max_active_tasks: s.max_active_tasks,
        ytdlp_enabled: s.ytdlp_enabled,
        ffmpeg_path_override: s.ffmpeg_path_override.map(|p| p.to_string_lossy().into_owned()),
    }
}

pub fn settings_from(s: SettingsDto) -> Settings {
    Settings {
        download_dir: s.download_dir.into(),
        segment_concurrency: s.segment_concurrency,
        max_active_tasks: s.max_active_tasks,
        ytdlp_enabled: s.ytdlp_enabled,
        ffmpeg_path_override: s.ffmpeg_path_override.map(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use easydownvideo::core::events::{EngineEvent, ProgressUnit, TaskState};

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
    fn progress_event_maps_field_by_field() {
        let d = event_dto(EngineEvent::Progress {
            id: 3,
            downloaded: 100,
            total: 400,
            unit: ProgressUnit::Segments,
        });
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(j["type"], "progress");
        assert_eq!(j["unit"], "segments");
        assert_eq!(j["downloaded"], 100);
    }

    #[test]
    fn settings_round_trip() {
        let s = easydownvideo::settings::Settings {
            download_dir: std::path::PathBuf::from("/tmp/x"),
            segment_concurrency: 6,
            max_active_tasks: 2,
            ytdlp_enabled: true,
            ffmpeg_path_override: None,
        };
        let d = settings_dto(s.clone());
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(j["downloadDir"], "/tmp/x");
        assert_eq!(j["segmentConcurrency"], 6);
        assert_eq!(settings_from(d), s, "往返之后必须与原来完全一致");
    }
}
