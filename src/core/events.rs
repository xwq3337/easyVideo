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
    /// `bytes_downloaded` 与 `downloaded` 是**两个独立的量**，不是同一件事的两种写法。
    /// `downloaded` 的单位由 `unit` 决定（分片任务是"片"），而 `bytes_downloaded` 永远是字节。
    /// 界面只用后者算速度：拿"片"做差得到的是片/秒，混进总速度会变成 "3.0 B/s" 这种假数字。
    Progress {
        id: TaskId,
        downloaded: u64,
        total: u64,
        unit: ProgressUnit,
        bytes_downloaded: u64,
    },
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
    /// **本轮运行**真正从网络上取回的字节数。与 `downloaded` 刻意分开：
    /// 分片任务里 `downloaded` 数的是"片"，拿它当字节算速度会得到片/秒。
    /// 续传跳过、重试回滚的部分都不计入这里（那些字节这一轮没有走网络）。
    bytes: Arc<AtomicU64>,
}

impl ProgressHandle {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&self, n: u64) {
        self.downloaded.fetch_add(n, Ordering::Relaxed);
    }
    /// 记下本轮从网络取回的字节数。**不要**用它代替 `add`：直链下载两者数值相同
    /// （`downloaded` 本来就是字节），分片下载则完全不同。
    pub fn add_bytes(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
    }
    /// 与 `add_bytes` 对称的饱和回滚，见 `sub` 的说明。
    pub fn sub_bytes(&self, n: u64) {
        let _ = self.bytes.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |v| Some(v.saturating_sub(n)),
        );
    }
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
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
    /// 一次新的运行开始。**两个计数器都要归零**：漏掉 `bytes` 的话，重跑会把上一轮
    /// 的字节数叠加上来，界面上就是一次凭空的巨大速度尖峰。
    pub fn reset(&self) {
        self.downloaded.store(0, Ordering::Relaxed);
        self.bytes.store(0, Ordering::Relaxed);
    }
}
