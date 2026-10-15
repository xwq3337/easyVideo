use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::core::direct::{download_direct, unique_path_avoiding, MIN_BLOCK_SIZE};
use crate::core::events::{Choice, ChoiceKind, EngineEvent, ProgressHandle, TaskId, TaskState};
use crate::core::ffmpeg::{ensure_ffmpeg, remux_to_mp4, RemuxOutcome};
use crate::core::hls::{download_media, parse::parse_playlist, parse::ParsedPlaylist};
use crate::core::probe::{filename_from_url, probe, ProbeOutcome};
use crate::core::task::{task_dir, write_manifest, ManifestKind, TaskManifest};
use crate::core::ytdlp::{fetch_formats, find_ytdlp};
use crate::settings::Settings;

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

/// GUI 启动时用于水合任务列表（尤其是 `resume` 恢复出来的任务）。
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
    /// 已被某个在途任务占用、但尚未落盘的最终路径。见 `unique_path_avoiding`：
    /// 只问文件系统是不够的，两个任务会在同一个时刻都得到"这个文件不存在"的答案。
    reserved_paths: Mutex<HashSet<PathBuf>>,
}

#[derive(Clone)]
struct TaskEntry {
    url: String,
    filename: String,
    state: TaskState,
    cancel: CancellationToken,
    pending_choice: Option<ChoiceKind>,
    progress: ProgressHandle,
    /// 该任务当前还活着的 worker 数。**同一任务永远不允许两个 worker 并发**：
    /// 「暂停 → 立刻继续」时旧 worker 可能还挂在网络请求上没退出，而 `unpause` 已经把它
    /// 重新排进队列；此时若直接起新 worker，两者会同时写同一批 `f.part-N`（都是
    /// `File::create` 截断写），产出一个静默损坏的文件 —— Direct 下载没有校验和，
    /// 没有任何东西会把这个错误暴露出来。
    workers: Arc<AtomicUsize>,
    /// 本次 `paths()` 占下来的最终路径，worker 结束时要把它释放掉。
    reserved_path: Option<PathBuf>,
    /// 本次 `paths()` 定下的任务目录。**清理用它，不要用 `current_settings()`** ——
    /// 用户可以在任务跑着的时候改保存位置，见 `actual_task_dir`。
    dir: Option<PathBuf>,
}

/// worker 生命周期守卫：退出时把该任务的 worker 计数减回去、释放最终路径的占用，并再
/// `pump` 一次。
///
/// 用 `Drop` 而不是在收尾处手写一次减法：收尾路径里有早退、有 `?`、也有 panic 的可能，
/// 而计数一旦泄漏（没有减回去），这个任务就**永远不会再被启动** —— 比它要修的缺陷更糟。
struct WorkerGuard {
    id: TaskId,
    workers: Arc<AtomicUsize>,
    engine: Engine,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.workers.fetch_sub(1, Ordering::SeqCst);
        // 先释放占用再 pump：被挡回队列的那个任务重新 `paths()` 时要能看到干净的状态。
        self.engine.release_reserved_path(self.id);
        // 因为"旧 worker 还没退干净"被挡回队列的那个任务，现在可以起来了。
        self.engine.pump();
    }
}

impl TaskEntry {
    /// 这个任务是否正**占着一个并发槽** —— 也就是是否有 worker 在它身上跑。
    ///
    /// **`AwaitingChoice` 不算。** 该状态下 `run_task` 已经返回 Ok、没有任何 worker 存在，
    /// 把它算作活跃会让它白占一个槽：默认 `max_active_tasks = 2` 时，连续两个 master
    /// playlist 任务都停在"等待选择"就足以把并发数占满，队列里其他任务**永远不会开始** ——
    /// 用户再加一个普通直链也没反应。真实世界的 m3u8 大多是 master playlist，很容易走到。
    ///
    /// 这与 `cancel` 里"有没有 worker 在跑"的判据保持一致（那里同样把 `AwaitingChoice`
    /// 排除在外，理由是同一个：这个状态没有 worker 会执行收尾逻辑）。
    fn is_active(&self) -> bool {
        matches!(
            self.state,
            TaskState::Probing | TaskState::Downloading | TaskState::Merging
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
                reserved_paths: Mutex::new(HashSet::new()),
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
        // 先克隆出 id/url/filename，再把完整的 manifest 交给 insert_entry——直接
        // `insert_entry(m.id, m.url, m.filename, Some(m))` 会 partial move：字段被挪走后
        // 整个 `m` 无法再传值（E0382）。manifest 需要保持完整以便写盘。
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

    /// 暂停：取消 token 并把状态落成 `Paused`。
    ///
    /// 接受的状态必须覆盖**界面会给出暂停按钮的那些**（`src/app.rs:290-297`），否则就是一个
    /// 点了没反应的死按钮。
    ///
    /// - `Queued`：还没轮到它，界面当前没给按钮，但对着队列里的任务点暂停是正常操作；
    /// - `Probing` / `Downloading`：worker 在跑。token 一取消，worker 会停在下一个检查点
    ///   （`advance_state` 的闸门 + `download_direct`/`download_media` 里的取消检查）；
    /// - `Merging`：ffmpeg 不看我们的 token，靠 `remux_to_mp4` 里的 `select!` 把等待它的
    ///   future 连同子进程一起收掉，并删掉可能写了一半的产物。暂停时任务目录**保留**
    ///   （分片还在），继续时只重跑合并这一步，不必重下。
    ///
    /// 不接受的：`AwaitingChoice`（没有 worker，pause 无从生效）、以及各终态。
    pub fn pause(&self, id: TaskId) {
        let entry = { self.inner.tasks.lock().unwrap().get(&id).cloned() };
        if let Some(e) = entry {
            // 标记为 Paused 后，pump 的分支闸会因为状态不是 Queued 而跳过它。
            if matches!(
                e.state,
                TaskState::Queued
                    | TaskState::Probing
                    | TaskState::Downloading
                    | TaskState::Merging
            ) {
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
            std::fs::remove_dir_all(self.actual_task_dir(id)).ok();
        }
    }

    /// 重试：把失败的任务重新排队。
    ///
    /// 与 `unpause` 的区别是它接受 `Failed` 状态 —— GUI 的"重试"按钮直接调这个方法。
    /// （原先 GUI 调的是 `unpause`，而 `unpause` 只认 `Paused`，于是失败任务的
    /// "重试"按钮点了毫无反应：不报错、不变化，纯粹是个死按钮。）
    ///
    /// 清掉 `pending_choice`：失败可能发生在等待选择的阶段，重试应当重新探测而不是
    /// 沿用上一次那份已经过期的候选列表。
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

    fn state_of(&self, id: TaskId) -> Option<TaskState> {
        self.inner
            .tasks
            .lock()
            .unwrap()
            .get(&id)
            .map(|t| t.state.clone())
    }

    /// 应用用户的格式/码率选择，把任务重新入队。
    ///
    /// **只有这次选择有效时才把候选列表取走。** 原先是先 `take()` 再校验：索引越界、或
    /// 类型与候选不符时提前返回，而候选列表已经被丢弃 —— 任务从此永久卡在 `AwaitingChoice`：
    /// 界面上「选择格式」还能点，点了弹出同样的对话框，再点仍然毫无反应，任务再也下不完。
    ///
    /// 引擎是公开 API，越界索引不是只有手滑才造得出来；一个坏调用不应该把任务变成不可恢复。
    /// 选择无效时任务**保持原样**，候选列表还在，可以用正确的选择再调一次。
    pub fn set_choice(&self, id: TaskId, choice: Choice) {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let Some(t) = tasks.get_mut(&id) else { return };
            let url = {
                let Some(kind) = t.pending_choice.as_ref() else {
                    return;
                };
                match (kind, choice) {
                    (ChoiceKind::HlsVariants(v), Choice::Variant(i)) => {
                        v.get(i).map(|c| c.uri.clone())
                    }
                    (ChoiceKind::YtdlpFormats(f), Choice::YtdlpFormat(i)) => {
                        f.get(i).map(|c| c.url.clone())
                    }
                    _ => None,
                }
            };
            let Some(url) = url else {
                return; // 无效选择：什么都不动，候选列表留给下一次正确的调用
            };
            t.pending_choice = None;
            t.url = url;
            t.state = TaskState::Queued;
        }
        self.inner.queue.lock().unwrap().push_back(id);
        self.pump();
    }

    // ===== 内部 =====

    fn insert_entry(
        &self,
        id: TaskId,
        url: String,
        filename: String,
        manifest: Option<TaskManifest>,
    ) {
        let (state, cancel) = (TaskState::Queued, CancellationToken::new());
        let progress = ProgressHandle::new();
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            tasks.insert(
                id,
                TaskEntry {
                    url: url.clone(),
                    filename,
                    state,
                    cancel,
                    pending_choice: None,
                    progress,
                    workers: Arc::new(AtomicUsize::new(0)),
                    reserved_path: None,
                    dir: None,
                },
            );
        }
        self.inner.queue.lock().unwrap().push_back(id);
        // 启动时若带 manifest 立即写盘，保证恢复信息存在
        if let Some(m) = manifest {
            let root = self.inner.settings.lock().unwrap().download_dir.clone();
            let tdir = task_dir(&root, id);
            let _ = write_manifest(&tdir, &m);
            // 连同**写入位置**一起记下来。清理必须按实际写入的目录来，而不是执行清理那一刻的
            // 当前设置 —— 用户在"恢复任务入队"和"取消它"之间改保存位置，是这个任务的常规走法。
            if let Some(t) = self.inner.tasks.lock().unwrap().get_mut(&id) {
                t.dir = Some(tdir);
            }
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

    /// worker 的**进度**状态推进（Probing → Downloading → Merging 之类），与 `set_state`
    /// 的唯一区别、也是它存在的唯一理由：**当前状态已经是 `Canceled` 或 `Paused` 时拒绝生效**
    /// —— 这两种都表示"用户已经把这个任务从我手里拿走了"。返回 `false` 表示这次推进被拒绝，
    /// 调用方必须立刻收手（`bail!("canceled")`）。
    ///
    /// `Paused` 必须在闸门里：`pause()` 能作用在 `Probing` 上（界面在探测期间就给出暂停按钮），
    /// 而 `probe()` 不接受 token、不会中途返回 —— worker 只能等它跑完，再靠这道闸停住。
    /// 没有它的话，暂停会被 worker 随后的 `Downloading` 覆盖掉，而 token 已被取消，
    /// 收尾分支既看不到 `Canceled` 也看不到 `Paused`，任务就永远停在 `Downloading` 且没有 worker。
    ///
    /// 为什么必须有这道闸：`run_task` 要等 `probe().await` 这个网络往返结束之后才会写
    /// manifest 并置 `Downloading`，而用户完全可以在探测期间点删除 —— `cancel()` 那时已经
    /// 把终态落成 `Canceled`（它看到 `Probing` 就认为有 worker 在跑，因此**不会**自己删目录，
    /// 这是对的，worker 可能正在写盘）。若无条件覆盖，`Canceled` 就被抹成 `Downloading`：
    /// worker 的收尾分支随后看到的不是 `Canceled`，于是走 `else` —— 标记 `Failed` **并且
    /// 跳过清理**，任务目录与 manifest 留在盘上，下次启动 `scan_resume` 把它复活重下。
    /// 这正是"取消是不要了"那条全局约束明令禁止的事，且是实测复现过的（见 progress.md
    /// 的 `cancel-during-probe` 场景与 `cancel_during_probe_ends_canceled_and_leaves_nothing`）。
    ///
    /// 判定与写入在**同一个锁**里完成：拆成"先查后写"两步，中间仍然留着同一个窗口。
    fn advance_state(&self, id: TaskId, state: TaskState) -> bool {
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            match tasks.get_mut(&id) {
                Some(t)
                    if !matches!(t.state, TaskState::Canceled | TaskState::Paused) =>
                {
                    t.state = state.clone()
                }
                _ => return false,
            }
        }
        let _ = self.inner.tx.send(EngineEvent::StateChanged { id, state });
        self.pump();
        true
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
                    // **只有仍处于 Queued、且没有 worker 在跑时才允许启动。**
                    //
                    // 第一道闸（取消/暂停发生在"排队期间"）：id 还留在队列里，没有它 pump 会把
                    // 任务重新拉起来 —— 用户明明点了取消，任务却自己复活。
                    //
                    // 第二道闸（`workers == 0`）：暂停后**立刻**继续时，旧 worker 可能还挂在
                    // 网络请求上没退出，而 `unpause` 已经把它重新排进队列。此时起新 worker，
                    // 两者会并发写同一批 `f.part-N`（都是 `File::create` 截断写），互相截断，
                    // 而 Direct 下载没有校验和，损坏不会以任何形式暴露出来。
                    Some(t) if t.state == TaskState::Queued && t.workers.load(Ordering::SeqCst) == 0 => {
                        t.cancel = CancellationToken::new();
                        t.state = TaskState::Probing; // 与上面的判定同锁，关掉 (b) 的窗口
                        t.workers.fetch_add(1, Ordering::SeqCst);
                        Some((
                            id,
                            t.url.clone(),
                            t.progress.clone(),
                            t.cancel.clone(),
                            t.workers.clone(),
                        ))
                    }
                    // 仍是 Queued、但旧 worker 还没退干净：放回**队尾**，什么都不启动。
                    // 旧 worker 的 `WorkerGuard::drop` 会再 pump 一次，那时才开始下载。
                    Some(t) if t.state == TaskState::Queued => {
                        self.inner.queue.lock().unwrap().push_back(id);
                        return;
                    }
                    _ => None,
                }
            };
            let Some((id, url, progress, cancel, workers)) = started else { continue };
            // 状态事件在锁外发。这里**不调 set_state**：它会再次进入 pump，把这条路径变成递归。
            let _ = self
                .inner
                .tx
                .send(EngineEvent::StateChanged { id, state: TaskState::Probing });
            let engine = self.clone();
            let tx = self.inner.tx.clone();
            self.inner.rt.spawn(async move {
                // 计数在 pump 里、与"Queued → Probing"同一个锁内已 +1；这里交给守卫在退出时 -1，
                // 并顺带把被它挡回队列的任务重新 pump 起来。
                let _worker = WorkerGuard {
                    id,
                    workers,
                    engine: engine.clone(),
                };
                // 进度转发：每 300ms 把原子计数器快照发为事件
                let mut ticker = tokio::time::interval(std::time::Duration::from_millis(300));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                // `run_task` 的 future **必须在 select! 之外创建并 pin 住**，循环里只借用它
                // （`&mut task`）。若把 `engine.run_task(...)` 直接写在 select! 分支里，每轮
                // tick 分支胜出时 `select!` 都会丢弃另一分支的 future —— 于是任务每 300ms 被
                // 从头重建一次：probe 反复重跑，永远走不到终止态。实测症状：一个会在 7 秒后
                // 失败的 404 任务会永久卡在 Downloading（20 秒都不出结果）。
                //
                // token 在 pump 里就取好了并随 worker 一起走：**worker 必须拿"自己那一个" token**。
                // 若像原先那样在 run_task 内部再去 entry 上读，暂停→继续换过 token 之后，旧 worker
                // 会读到新 token（没被取消），于是它既不会停下、收尾时也无法判断自己被顶替了。
                let task = engine.run_task(id, url.clone(), progress.clone(), cancel.clone());
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
                let _ = tx.send(EngineEvent::Progress {
                    id,
                    downloaded: d,
                    total: t,
                    unit: progress.unit(),
                });
                // 收尾：只有仍处于执行态才标记 Done；AwaitingChoice/Paused/Canceled 保持
                match result {
                    Ok(()) => {
                        if let Some(s) = engine.state_of(id) {
                            if matches!(
                                s,
                                TaskState::Probing | TaskState::Downloading | TaskState::Merging
                            ) {
                                engine.set_state(id, TaskState::Done);
                            }
                        }
                    }
                    Err(e) => {
                        // 判据是**这个 worker 自己的 token 是否被取消**，而不是"当前状态是什么"。
                        //
                        // token 被取消只有两种可能：用户点了取消，或者用户暂停后又继续
                        // （`unpause`/`retry` 会换一个**新的** token）。两种都不允许在这里标记
                        // `Failed`：暂停→立刻继续时任务已经重新排队、新 worker 正在跑，旧 worker
                        // 按"当前状态"分派只会看到 Probing/Downloading —— 既不是 Canceled 也不是
                        // Paused —— 于是把任务标成失败，用户双击"继续"就得到一个失败的任务。
                        if cancel.is_cancelled() {
                            // 只有"用户取消"才清盘；暂停、或被新 worker 接手的任务要留下断点。
                            if matches!(engine.state_of(id), Some(TaskState::Canceled)) {
                                std::fs::remove_dir_all(engine.actual_task_dir(id)).ok();
                            }
                        } else {
                            tracing::warn!("task {id} failed: {e:#}");
                            engine.set_state(id, TaskState::Failed(format!("{e:#}")));
                        }
                    }
                }
            });
        }
    }

    async fn run_task(
        &self,
        id: TaskId,
        url: String,
        progress: ProgressHandle,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
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
                        // 用户可能在 probe 期间已经取消：先确认状态还能推进，再动 pending_choice。
                        if !self.advance_state(id, TaskState::AwaitingChoice) {
                            anyhow::bail!("canceled");
                        }
                        let kind = {
                            let mut tasks = self.inner.tasks.lock().unwrap();
                            let t = tasks.get_mut(&id).expect("task entry present");
                            let kind = ChoiceKind::YtdlpFormats(formats);
                            t.pending_choice = Some(kind.clone());
                            kind
                        };
                        let _ = self.inner.tx.send(EngineEvent::ChoicesOffered { id, kind });
                        return Ok(()); // 等待 set_choice 重新入队
                    }
                }
                anyhow::bail!("无法识别的网页地址；可安装 yt-dlp 或在设置中启用");
            }
            ProbeOutcome::Hls { url } => self.run_hls(id, &url, &settings, &progress, &cancel).await,
            ProbeOutcome::Direct {
                url,
                size,
                accepts_ranges,
                filename,
            } => {
                // 若任务还未被用户重命名过（默认名），用 probe 得到的更准文件名
                {
                    let mut tasks = self.inner.tasks.lock().unwrap();
                    if let Some(t) = tasks.get_mut(&id) {
                        if t.filename == "video.mp4" && filename != "video.mp4" {
                            t.filename = filename;
                        }
                    }
                }
                let (final_path, tdir) = self.paths(id, &settings, None);
                // 任务一开始就写 manifest：中途退出应用可据扫描恢复。
                //
                // **必须在状态切到 Downloading 之前落盘。** Downloading 对外承诺
                // "断点信息已可恢复"（`cancel_cleans_resume_artifacts` 正是据此在看到
                // Downloading 后立刻取消，再轮询 `scan_resume` 判断清理是否完成）。若先发状态
                // 再写盘，就存在"目录已建、manifest.json 尚未写入"的窗口：此时 `scan_resume`
                // 返回空，而目录仍在 —— 测试会误判清理已完成并在随后断言目录不存在时失败。
                let manifest = TaskManifest {
                    id,
                    url: url.clone(),
                    filename: final_path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    download_dir: settings.download_dir.clone(),
                    kind: ManifestKind::Direct { size },
                };
                let _ = write_manifest(&tdir, &manifest);
                // 用户在探测期间可能已经点了删除。`advance_state` 会拒绝覆盖 `Canceled`
                // 并返回 false —— 此时立刻收手：收尾分支看到 Canceled，会把刚建出来的
                // 任务目录连同 manifest 一起清掉。
                if !self.advance_state(id, TaskState::Downloading) {
                    anyhow::bail!("canceled");
                }
                download_direct(
                    &self.inner.client,
                    &url,
                    &final_path,
                    &tdir,
                    size,
                    accepts_ranges,
                    settings.segment_concurrency,
                    MIN_BLOCK_SIZE,
                    &progress,
                    cancel,
                )
                .await?;
                // 完成后整个任务目录一起清掉（含 manifest），避免重启时误恢复已完成任务
                std::fs::remove_dir_all(&tdir).ok();
                Ok(())
            }
        }
    }

    async fn run_hls(
        &self,
        id: TaskId,
        playlist_url: &str,
        settings: &Settings,
        progress: &ProgressHandle,
        cancel: &CancellationToken,
    ) -> anyhow::Result<()> {
        let body = {
            let resp = self
                .inner
                .client
                .get(playlist_url)
                .send()
                .await?
                .error_for_status()?;
            resp.bytes().await? // 播放列表文件很小（KB 级），全量读取可接受
        };
        let parsed = parse_playlist(playlist_url, &body)?;
        let media = match parsed {
            ParsedPlaylist::Master { variants } => {
                // 同理：拉播放列表期间用户可能已经取消，先确认状态还能推进。
                if !self.advance_state(id, TaskState::AwaitingChoice) {
                    anyhow::bail!("canceled");
                }
                let kind = {
                    let mut tasks = self.inner.tasks.lock().unwrap();
                    let t = tasks.get_mut(&id).expect("task entry present");
                    let kind = ChoiceKind::HlsVariants(variants);
                    t.pending_choice = Some(kind.clone());
                    kind
                };
                let _ = self.inner.tx.send(EngineEvent::ChoicesOffered { id, kind });
                return Ok(());
            }
            ParsedPlaylist::Media(m) => m,
        };
        // 产物是 mp4，所以唯一性也必须按 mp4 判（见 `paths` 的说明）
        let (final_path, tdir) = self.paths(id, settings, Some("mp4"));
        // HLS 任务同样要落 manifest：否则中途退出后无法续传，违反 spec §6。
        // 记的是 media playlist 的 URL（master 已在上面转成具体码率的 media）。
        // 与 Direct 分支同理：manifest 先落盘，再切 Downloading 状态。
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
                kind: ManifestKind::Hls {
                    playlist_url: playlist_url.to_string(),
                },
            },
        );
        if !self.advance_state(id, TaskState::Downloading) {
            anyhow::bail!("canceled");
        }
        let plan = download_media(
            &self.inner.client,
            &media,
            &tdir,
            settings.segment_concurrency,
            progress,
            cancel.clone(),
        )
        .await?;
        // 合并前再确认一次：下载期间用户可能取消了。过了这一关就**不要**再跑 ffmpeg ——
        // 否则会为一个已取消的任务产出 mp4，而收尾分支随后会把整个任务目录删掉。
        if !self.advance_state(id, TaskState::Merging) {
            anyhow::bail!("canceled");
        }
        let ffmpeg = match &settings.ffmpeg_path_override {
            Some(p) => p.clone(),
            None => ensure_ffmpeg(&self.inner.data_dir)?,
        };
        // 合并可能很久（-c copy 要把整个文件搬一遍），必须能被暂停/取消打断。
        if remux_to_mp4(&ffmpeg, &plan, &final_path, cancel).await? == RemuxOutcome::Interrupted {
            // 用户暂停：保留任务目录（分片还在，继续时只重跑合并这一步）。
            // 用户取消：什么都不留 —— `cancel` 见到 Merging 就把清理交给了 worker，
            // 这个地方必须自己做完，否则任务目录会孤零零留在盘上被 scan_resume 复活。
            if !matches!(self.state_of(id), Some(TaskState::Paused)) {
                std::fs::remove_dir_all(&tdir).ok();
            }
            return Ok(());
        }
        // 合并成功后整个任务目录（含 manifest、分片、concat 列表）一起清掉，
        // 避免重启时 scan_resume 把已完成任务复活
        std::fs::remove_dir_all(&tdir).ok();
        Ok(())
    }

    /// 定下任务实际使用的任务目录与最终文件路径。
    ///
    /// `output_ext` 是**最终产物**的扩展名：Direct 传 `None`（直接用 entry 上当前的文件名），
    /// HLS 必须传 `Some("mp4")`。唯一性检查必须针对**最终产物**的名字 —— HLS 的入口是
    /// `master.m3u8` 而产物是 `master.mp4`：拿 `.m3u8` 去查当然查不到冲突（那个文件根本
    /// 不会被创建），`unique_path` 于是放行，接着 ffmpeg 带着 `-y` 把用户已存在的
    /// `master.mp4` 直接覆盖掉，**用户的文件永久丢失且全程没有任何提示**。
    fn paths(
        &self,
        id: TaskId,
        settings: &Settings,
        output_ext: Option<&str>,
    ) -> (PathBuf, PathBuf) {
        let root = &settings.download_dir;
        let tdir = task_dir(root, id);
        let current = self
            .inner
            .tasks
            .lock()
            .unwrap()
            .get(&id)
            .expect("task entry present")
            .filename
            .clone();
        let filename = match output_ext {
            Some(ext) => Path::new(&current)
                .with_extension(ext)
                .to_string_lossy()
                .into_owned(),
            None => current,
        };
        // **查询与登记必须在同一个 `reserved_paths` 锁内完成。** 只问文件系统的话，
        // 两个任务会在同一时刻都得到"这个文件不存在"，于是算出同一个最终路径。
        let final_path = {
            let mut reserved = self.inner.reserved_paths.lock().unwrap();
            let p = unique_path_avoiding(root, &filename, |c| reserved.contains(c));
            reserved.insert(p.clone());
            p
        };
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let t = tasks.get_mut(&id).expect("task entry present");
            t.reserved_path = Some(final_path.clone());
            // 记下**真正**用的目录。清理一律以它为准：用户可以在任务跑着的时候改保存
            // 位置，那时 `current_settings()` 已经指向别处了。
            t.dir = Some(tdir.clone());
        }
        (final_path, tdir)
    }

    /// 任务**实际**写入的那个任务目录。
    ///
    /// 清理一处都不能用 `current_settings().download_dir`：用户可以在任务跑着的时候改保存
    /// 位置（设置窗口两下就能改），那时 `current_settings()` 指向的已经不是这个任务真正用的
    /// 目录 —— 按它删会去删新目录里同号的 `task-<id>`（**另一个任务的数据**），而真正的
    /// `manifest.json` 留在原地，被下次启动的 `scan_resume` 复活重下。
    ///
    /// 没有记录时回退到"当前设置 + id"：那是 `paths()` 还没跑过的任务，它也确实只在
    /// `insert_entry` 的 eager 写入里用过那个目录。
    fn actual_task_dir(&self, id: TaskId) -> PathBuf {
        let fallback_root = self.current_settings().download_dir;
        let recorded = self
            .inner
            .tasks
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|t| t.dir.clone());
        recorded.unwrap_or_else(|| task_dir(&fallback_root, id))
    }

    /// worker 结束后释放该任务的最终路径占用。
    ///
    /// **总是**释放，不做条件判断：占用本身就是"已分配但还没落盘"的意思。成功后文件已经在
    /// 盘上，之后的唯一性判断由文件系统回答，结果一样；取消/失败/暂停时什么都没写出来，
    /// 不释放的话这个名字会被永久占着 —— 用户再下同一个视频只会得到
    /// `video (1).mp4`、`video (2).mp4`……越攒越多。
    fn release_reserved_path(&self, id: TaskId) {
        let path = {
            let mut tasks = self.inner.tasks.lock().unwrap();
            match tasks.get_mut(&id).and_then(|t| t.reserved_path.take()) {
                Some(p) => p,
                None => return,
            }
        };
        self.inner.reserved_paths.lock().unwrap().remove(&path);
    }
}
