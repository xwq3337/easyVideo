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

    pub fn cancel(&self, id: TaskId) {
        let entry = { self.inner.tasks.lock().unwrap().get(&id).cloned() };
        if let Some(e) = entry {
            e.cancel.cancel();
            self.set_state(id, TaskState::Canceled);
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

    pub fn set_choice(&self, id: TaskId, choice: Choice) {
        let pending = {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let Some(t) = tasks.get_mut(&id) else { return };
            t.pending_choice.take()
        };
        let Some(kind) = pending else { return };
        let url = match (kind, choice) {
            (ChoiceKind::HlsVariants(v), Choice::Variant(i)) => v.get(i).map(|v| v.uri.clone()),
            (ChoiceKind::YtdlpFormats(f), Choice::YtdlpFormat(i)) => {
                f.get(i).map(|f| f.url.clone())
            }
            _ => None,
        };
        let Some(url) = url else { return };
        {
            let mut tasks = self.inner.tasks.lock().unwrap();
            if let Some(t) = tasks.get_mut(&id) {
                t.url = url;
                t.state = TaskState::Queued;
            }
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
                },
            );
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
            let active = {
                let tasks = self.inner.tasks.lock().unwrap();
                tasks.values().filter(|t| t.is_active()).count()
            };
            if active >= settings.max_active_tasks {
                return;
            }
            let next = { self.inner.queue.lock().unwrap().pop_front() };
            let Some(id) = next else { return };
            let (url, progress) = {
                let mut tasks = self.inner.tasks.lock().unwrap();
                let Some(t) = tasks.get_mut(&id) else { continue };
                // **只有仍处于 Queued 的才允许启动。** 取消/暂停发生在"排队期间"时，id 还留在
                // 队列里；没有这道闸，pump 会把它重新拉起来 —— 用户明明点了取消，任务却自己
                // 复活并开始下载。这正是 `cancel_cleans_resume_artifacts` 覆盖不到的那条路径
                // （那个测试取消的是已经在下载的任务）。
                if t.state != TaskState::Queued {
                    continue;
                }
                t.cancel = CancellationToken::new();
                (t.url.clone(), t.progress.clone())
            };
            self.set_state(id, TaskState::Probing);
            let engine = self.clone();
            let tx = self.inner.tx.clone();
            self.inner.rt.spawn(async move {
                // 进度转发：每 300ms 把原子计数器快照发为事件
                let mut ticker = tokio::time::interval(std::time::Duration::from_millis(300));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                // `run_task` 的 future **必须在 select! 之外创建并 pin 住**，循环里只借用它
                // （`&mut task`）。若把 `engine.run_task(...)` 直接写在 select! 分支里，每轮
                // tick 分支胜出时 `select!` 都会丢弃另一分支的 future —— 于是任务每 300ms 被
                // 从头重建一次：probe 反复重跑，永远走不到终止态。实测症状：一个会在 7 秒后
                // 失败的 404 任务会永久卡在 Downloading（20 秒都不出结果）。
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

    async fn run_task(
        &self,
        id: TaskId,
        url: String,
        progress: ProgressHandle,
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
                        self.set_state(id, TaskState::AwaitingChoice);
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
            ProbeOutcome::Hls { url } => self.run_hls(id, &url, &settings, &progress).await,
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
                let (final_path, tdir) = self.paths(id, &settings);
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
                self.set_state(id, TaskState::Downloading);
                let cancel = self
                    .inner
                    .tasks
                    .lock()
                    .unwrap()
                    .get(&id)
                    .expect("task entry present")
                    .cancel
                    .clone();
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
                self.set_state(id, TaskState::AwaitingChoice);
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
        let (final_path, tdir) = self.paths(id, settings);
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
        self.set_state(id, TaskState::Downloading);
        let cancel = self
            .inner
            .tasks
            .lock()
            .unwrap()
            .get(&id)
            .expect("task entry present")
            .cancel
            .clone();
        let plan = download_media(
            &self.inner.client,
            &media,
            &tdir,
            settings.segment_concurrency,
            progress,
            cancel,
        )
        .await?;
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
        let filename = self
            .inner
            .tasks
            .lock()
            .unwrap()
            .get(&id)
            .expect("task entry present")
            .filename
            .clone();
        let final_path = unique_path(root, &filename);
        (final_path, tdir)
    }
}
