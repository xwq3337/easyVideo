//! core → 前端的单向事件桥。
//!
//! core 用 tokio 的 broadcast 通道往外发事件：容量 256，**跟不上的时候丢的是最旧的消息**。
//! 所以这里不是"把事件转发一下就完了"——`Lagged` 必须被当成一件正事处理：
//! 明确告诉前端"你落后了 n 条，请重新拉一次全量"。
//!
//! 反面教材就在这个仓库的历史里：egui 版把 `Lagged` 与"队列空"一视同仁
//! （`while let Ok(ev) = rx.try_recv()`），于是只要 UI 线程被阻塞一次（当时是文件选择对话框），
//! 丢掉一个 `Done` 事件，那一行就**永久**停在"下载中"，只有重启才能恢复。

use std::sync::Arc;

use easydownvideo::core::events::EngineEvent;
use tokio::sync::broadcast;

use crate::dto::{event_dto, EventDto};

pub const EVENT: &str = "engine://event";
pub const RESYNC: &str = "engine://resync";

/// 桥的出口。抽成 trait 是为了让 `Lagged` 分支可测——直接依赖 `AppHandle` 的话，
/// 恰恰是最该被钉死的那条分支测不了。
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

impl EventSink for tauri::AppHandle {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter;
        // 必须写全 `Emitter::emit`：`AppHandle` 同时实现了本 trait 和 tauri 的 `Emitter`，
        // 两个都有 `emit` 方法，裸写 `self.emit(...)` 是 E0034（multiple applicable items）。
        //
        // 发不出去（前端还没加载完）不是错误：前端水合时会用 get_tasks 补全，
        // 而任何一次状态变化之后也都会重新对账。
        let _ = Emitter::emit(self, event, payload);
    }
}

/// 把通道里**这次 `Lagged` 之前**已经发出的残余消息排空，返回一并丢掉的条数。
///
/// 为什么不能"顺手把剩下的转发出去就算完"（那是本任务草图里的写法，跑起来是**红的**）：
/// 这些消息的产生时间都早于 resync 信号。前端收到 resync 会立刻 `get_tasks()` 整体重取，
/// 而 invoke 的响应和事件走的是两条不同的 IPC 通道——排在 resync 之后的旧消息完全可能
/// **晚于**那份全量快照落地，用旧数据把刚拉到的正确状态盖回去；一旦被盖回去的是
/// `StateChanged { Done }` 这类"之后不会再有事件来纠正"的消息，那一行就永久停在错误状态。
/// 也就是说，跟在前端后面补发旧事件，正好重演了这个桥存在的理由所要防的那一类缺陷。
///
/// 丢掉的这些消息**信息不会丢**，这正是可以放心丢的理由：
/// 状态与进度由 `get_tasks()` 补齐，候选列表由 `pending_choice(id)` 补齐（见 plan 的 T7/T11）。
fn drain_stale(rx: &mut broadcast::Receiver<EngineEvent>) -> u64 {
    let mut dropped = 0;
    loop {
        match rx.try_recv() {
            Ok(_) => dropped += 1,
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(broadcast::error::TryRecvError::Closed) => break,
            // 排空的过程中可能又落后一次：那些同样是被覆盖掉的消息，一并计入。
            Err(broadcast::error::TryRecvError::Lagged(n)) => dropped += n,
        }
    }
    dropped
}

pub async fn run<S: EventSink>(sink: Arc<S>, mut rx: broadcast::Receiver<EngineEvent>) {
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let dto: EventDto = event_dto(ev);
                match serde_json::to_value(dto) {
                    Ok(v) => sink.emit(EVENT, v),
                    Err(e) => tracing::warn!("事件序列化失败: {e}"),
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // 先排空、后发 resync：`dropped` 报的是前端**永远看不到**的总条数
                // （被覆盖掉的 n 条 + 排在 resync 之前、被我们主动丢弃的残余）。
                let suppressed = drain_stale(&mut rx);
                sink.emit(RESYNC, serde_json::json!({ "dropped": n + suppressed }));
            }
            // 发送端没了就结束，否则这个循环会永远空转。
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use easydownvideo::core::events::{EngineEvent, ProgressUnit, TaskState};
    use std::sync::Mutex;
    use tokio::sync::broadcast;

    #[derive(Default)]
    struct FakeSink(Mutex<Vec<(String, serde_json::Value)>>);

    impl EventSink for FakeSink {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    fn fake() -> Arc<FakeSink> {
        Arc::new(FakeSink::default())
    }

    #[tokio::test]
    async fn forwards_state_changes_as_events() {
        let (tx, rx) = broadcast::channel(16);
        let sink = fake();
        let task_sink = sink.clone();
        let handle = tokio::spawn(run(task_sink, rx));

        tx.send(EngineEvent::StateChanged {
            id: 1,
            state: TaskState::Done,
        })
        .unwrap();
        drop(tx);
        handle.await.unwrap();

        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, EVENT);
        assert_eq!(got[0].1["type"], "stateChanged");
        assert_eq!(got[0].1["state"]["kind"], "done");
    }

    // 显式钉住 current_thread：这条测试的确定性依赖"单线程运行时"——桥的循环与
    // `tx.send(..)` 在同一线程上交错推进，`resyncs.len() == 1` 才稳定。
    // 默认 flavour 恰好是 current_thread，但那是个**未言明的**依赖：有人给 crate
    // 配了 multi_thread、或把这条测试搬到别的 runtime 上，它就会开始随机变红。
    #[tokio::test(flavor = "current_thread")]
    async fn lagged_turns_into_a_resync_signal_instead_of_silence() {
        // 这是整个桥存在的理由：广播通道容量有限，跟不上的时候**丢掉的是最旧的消息**。
        // 把 Lagged 当成"队列空"（egui 版就是这么写的），界面会永久停在一个错误状态：
        // 已经完成的任务仍显示"下载中"，或者卡在"等待选择"而没有任何按钮。
        // 正确的做法是明确告诉前端"你落后了，重新拉一次全量"。
        let (tx, rx) = broadcast::channel(2);
        let sink = fake();
        let task_sink = sink.clone();
        let handle = tokio::spawn(run(task_sink, rx));

        for i in 0..8u64 {
            let _ = tx.send(EngineEvent::Progress {
                id: i,
                downloaded: i,
                total: 100,
                unit: ProgressUnit::Bytes,
                bytes_downloaded: i,
            });
        }
        drop(tx);
        handle.await.unwrap();

        let got = sink.0.lock().unwrap();
        let resyncs: Vec<_> = got.iter().filter(|(name, _)| name == RESYNC).collect();
        assert_eq!(resyncs.len(), 1, "落后时必须发恰好一次 resync");
        assert!(resyncs[0].1["dropped"].as_u64().unwrap() > 0);

        // 而且 resync 必须**在最后**——前端收到它才会整体重取，之后不该再有旧事件
        assert_eq!(got.last().unwrap().0, RESYNC);
    }
}
