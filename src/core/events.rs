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
    /// 任务被移出列表。前端据此把行删掉，而不是靠请求的返回值自己猜。
    TaskRemoved { id: TaskId },
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
    /// 饱和回滚：失败的分块重试需要撤销该次尝试已计入的进度。
    /// 用 `fetch_update` + `saturating_sub`，因此回滚量大于已计入量时
    /// 只会停在 0，绝不会下溢回绕成一个天文数字。
    pub fn sub(&self, n: u64) {
        let _ = self
            .downloaded
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(n))
            });
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
