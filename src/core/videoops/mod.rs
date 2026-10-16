pub mod compress;
pub mod ffmpeg_job;
pub mod media_info;
pub mod merge;
pub mod params;
pub mod transcode;
pub mod trim;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::core::direct::unique_path_avoiding;
use crate::core::ffmpeg::ensure_ffmpeg;
use ffmpeg_job::{run_job, JobOutcome};
use params::{JobContext, JobParams};

pub type JobId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed(String),
    Canceled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JobSnapshot {
    pub id: JobId,
    pub label: String,
    /// **一组**而不是一个：合并要把 N 个文件按这个顺序拼起来。
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub state: JobState,
    pub progress: Option<f64>,
}

#[derive(Clone, Debug)]
pub enum VideoOpEvent {
    Added { id: JobId },
    StateChanged { id: JobId, state: JobState },
    Progress { id: JobId, fraction: f64 },
    Removed { id: JobId },
}

fn is_terminal(s: &JobState) -> bool {
    matches!(s, JobState::Done | JobState::Failed(_) | JobState::Canceled)
}

struct JobEntry {
    label: String,
    inputs: Vec<PathBuf>,
    output: PathBuf,
    /// 已经拼好的 ffmpeg 参数。入队时算一次就够了 —— 参数只由用户的选择决定，
    /// 之后不会再变；放在 worker 里重算只会多一处可能与校验时不一致的地方。
    args: Vec<String>,
    /// 本次任务要写的临时文件：路径 + **内容**。内容在入队时就算好
    /// （`concat_list` 是纯函数），worker 只负责写。
    /// 排队中被取消的任务从没写过它，所以没有可清理的东西。
    scratch: Option<(PathBuf, String)>,
    /// 进度条的分母，**入队时就算好**（`params.duration_secs(ctx)`）。
    /// 不放进 worker 去现算：那时 entry 里已经没有逐段的媒体信息了，
    /// 而合并要的是**各段之和**。
    duration: Option<f64>,
    state: JobState,
    progress: Option<f64>,
    cancel: CancellationToken,
}

struct Inner {
    rt: Handle,
    data_dir: PathBuf,
    /// 解出来的 ffmpeg 路径。解一次就记住 —— `ensure_ffmpeg` 会校验哈希，
    /// 但那是每次几 MB 的读盘，不该每个任务都做一遍。
    ffmpeg: Mutex<Option<PathBuf>>,
    /// 设置页里的「ffmpeg 路径覆盖」。非空时**优先于**内嵌那份。
    ffmpeg_override: Mutex<Option<PathBuf>>,
    jobs: Mutex<HashMap<JobId, JobEntry>>,
    queue: Mutex<VecDeque<JobId>>,
    tx: broadcast::Sender<VideoOpEvent>,
    next_id: AtomicU64,
}

#[derive(Clone)]
pub struct VideoOps {
    inner: Arc<Inner>,
}

/// 并发上限**写死 1**：转码吃满 CPU，与下载抢资源只会让两边都变慢。
/// 要放开时再加设置项，现在加就是一个没人调过的旋钮。
const MAX_ACTIVE_JOBS: usize = 1;

impl VideoOps {
    pub fn new(rt: Handle, data_dir: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Inner {
                rt,
                data_dir,
                ffmpeg: Mutex::new(None),
                ffmpeg_override: Mutex::new(None),
                jobs: Mutex::new(HashMap::new()),
                queue: Mutex::new(VecDeque::new()),
                tx,
                next_id: AtomicU64::new(1),
            }),
        }
    }

    pub fn events(&self) -> broadcast::Receiver<VideoOpEvent> {
        self.inner.tx.subscribe()
    }

    /// 用户在设置页指定的 ffmpeg。非空时它**压过**内嵌那份 —— 设置页的文案是
    /// "ffmpeg 路径覆盖（留空则用内嵌的）"，那是个全局承诺：用户设它往往正是因为
    /// 内嵌那份在他机器上跑不起来。只有下载那一半认它的话，他的下载正常、
    /// 每一次压缩都失败，而错误看着毫不相干。
    pub fn set_ffmpeg_override(&self, p: Option<PathBuf>) {
        *self.inner.ffmpeg_override.lock().unwrap() = p;
    }

    /// 解出要用的 ffmpeg：覆盖优先，其次内嵌（`ensure_ffmpeg` 会校验哈希、复用已解出的那份）。
    fn ffmpeg(&self) -> anyhow::Result<PathBuf> {
        // 覆盖**先**看：可能刚设上，而缓存里还留着内嵌那份
        if let Some(p) = self.inner.ffmpeg_override.lock().unwrap().as_ref() {
            return Ok(p.clone());
        }
        if let Some(p) = self.inner.ffmpeg.lock().unwrap().as_ref() {
            return Ok(p.clone());
        }
        let p = ensure_ffmpeg(&self.inner.data_dir)?;
        *self.inner.ffmpeg.lock().unwrap() = Some(p.clone());
        Ok(p)
    }

    pub fn list_jobs(&self) -> Vec<JobSnapshot> {
        let jobs = self.inner.jobs.lock().unwrap();
        let mut out: Vec<JobSnapshot> = jobs
            .iter()
            .map(|(id, j)| JobSnapshot {
                id: *id,
                label: j.label.clone(),
                inputs: j.inputs.clone(),
                output: j.output.clone(),
                state: j.state.clone(),
                progress: j.progress,
            })
            .collect();
        out.sort_by_key(|s| s.id);
        out
    }

    /// 读一个视频的基本信息。**不产生"任务"** —— 它只是界面上的预览，
    /// 失败也不该在任务列表里留下一行。
    pub async fn probe(&self, input: PathBuf) -> anyhow::Result<media_info::MediaInfo> {
        let meta = std::fs::metadata(&input)
            .map_err(|e| anyhow::anyhow!("读不到文件 {}：{e}", input.display()))?;
        if !meta.is_file() {
            anyhow::bail!("不是一个文件：{}", input.display());
        }
        let ffmpeg = self.ffmpeg()?;
        media_info::inspect(&ffmpeg, &input).await
    }

    /// 入队一次压缩。**校验一律在这里做完**：输入不存在、输出目录不可写、文件里没有
    /// 视频流，都在入队那一刻就报错，不要排进队列跑一半才失败 —— 那时用户已经等了半天。
    ///
    /// 是 async 只因为"有没有视频流"要跑一次 `ffmpeg -i`。这条校验不能省：
    /// 纯音频输入**不会**让 ffmpeg 报错，它会老老实实编出一个只有音轨的 mp4 并退出 0，
    /// 用户点的是"压缩视频"却拿到一个没有画面的文件 —— 那种"看起来成功了"的结果
    /// 比直接报错坏得多。
    pub async fn add_job(
        &self,
        inputs: Vec<PathBuf>,
        output_dir: PathBuf,
        params: JobParams,
    ) -> anyhow::Result<JobId> {
        if inputs.is_empty() {
            anyhow::bail!("没有选文件");
        }
        for p in &inputs {
            if !p.is_file() {
                anyhow::bail!("找不到输入文件：{}", p.display());
            }
        }
        if !output_dir.is_dir() {
            anyhow::bail!("输出目录不存在：{}", output_dir.display());
        }
        let ffmpeg = self.ffmpeg()?;
        // 只探**首段**：单输入的操作看的就是它，而"按首段定名字与容器"对合并也成立。
        // 逐段的媒体信息留给合并自己的兼容性校验（`check_merge` 会全探一遍）。
        let src = media_info::inspect(&ffmpeg, &inputs[0]).await?;
        if !src.has_video {
            anyhow::bail!(
                "这个文件里读不到视频流（可能是纯音频，或者文件已损坏）：{}",
                inputs[0].display()
            );
        }
        // 真写一个探针文件，而不是查权限位 —— 权限位在 ACL、只读卷、网络盘上都不作数
        let probe = output_dir.join(".easydownvideo-write-probe");
        std::fs::write(&probe, b"")
            .map_err(|e| anyhow::anyhow!("输出目录不可写（{}）：{e}", output_dir.display()))?;
        std::fs::remove_file(&probe).ok();

        let stem = inputs[0]
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video")
            .to_string();

        // **取名 + 登记必须在同一个锁里** —— 见 `plan_output_path` 的说明。
        let id = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let infos = vec![src.clone()];
            // `extension` 只看首段，所以不必先有 ctx —— 先算 output（它靠 extension），
            // 再拿 output 的引用构造 ctx。**顺序不能反**：ctx 里要放 output 的引用，
            // 而 output 要靠 extension 算。
            let output = Self::plan_output_path(
                &jobs,
                &output_dir,
                &stem,
                params.suffix(),
                &params.extension(&src),
            );
            let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
            // 需要临时文件的操作（目前只有合并）在这里把**路径与内容**都备好：
            // 内容是纯函数算的（`concat_list`），worker 只负责写。
            // 排队中被取消的任务从没写过它，所以没有可清理的东西。
            let scratch = matches!(params, JobParams::Merge).then(|| {
                (
                    merge::scratch_path(&self.inner.data_dir, id),
                    merge::concat_list(&inputs),
                )
            });
            // `args` 需要列表文件的**路径**，所以从 scratch 借一个引用给它
            let scratch_path = scratch.as_ref().map(|(p, _)| p.as_path());
            let ctx = JobContext {
                inputs: &inputs,
                infos: &infos,
                output: &output,
                scratch: scratch_path,
            };
            // **操作自己的语义校验**（裁剪的时间范围就靠这一步挡住）。
            // 放在这里而不是命令层：入队校验全在 core，命令层只做薄封装，这条规矩别破。
            params.validate(&ctx)?;
            let duration = params.duration_secs(&ctx);
            let args = params.args(&ctx); // ctx 的最后一处使用 —— 之后才能把 scratch 移进 entry
            jobs.insert(
                id,
                JobEntry {
                    label: params.label(),
                    inputs,
                    output,
                    args,
                    scratch,
                    duration,
                    state: JobState::Queued,
                    progress: None,
                    cancel: CancellationToken::new(),
                },
            );
            id
        };
        self.inner.queue.lock().unwrap().push_back(id);
        let _ = self.inner.tx.send(VideoOpEvent::Added { id });
        self.pump();
        Ok(id)
    }

    /// 只算产物名，**不入队**。界面用它做产物名预览。
    ///
    /// 与 `add_job` 走同一个取名函数 —— 两处各写一遍的话，预览与实际落盘会在某个
    /// 边角上分叉（比如"在途任务占位"只加了一边），而那正是预览存在的意义。
    pub async fn plan_output(
        &self,
        inputs: Vec<PathBuf>,
        output_dir: PathBuf,
        params: JobParams,
    ) -> anyhow::Result<String> {
        if inputs.is_empty() {
            anyhow::bail!("没有选文件");
        }
        if !inputs[0].is_file() {
            anyhow::bail!("找不到输入文件：{}", inputs[0].display());
        }
        let ffmpeg = self.ffmpeg()?;
        // 只探首段 —— 产物名与容器都按它定（与 `add_job` 同一套规则）
        let src = media_info::inspect(&ffmpeg, &inputs[0]).await?;
        let stem = inputs[0]
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video")
            .to_string();
        let jobs = self.inner.jobs.lock().unwrap();
        Ok(Self::plan_output_path(
            &jobs,
            &output_dir,
            &stem,
            params.suffix(),
            &params.extension(&src),
        )
        .to_string_lossy()
        .into_owned())
    }

    /// 算产物的完整路径。**`plan_output` 与 `add_job` 都走这里** —— 两处各写一遍的话，
    /// 预览与实际落盘会在某个边角上分叉，而那正是产物名预览存在的意义。
    ///
    /// `taken` 问的是"有没有别的**在途任务**已经占了它"。只问文件系统是不够的：
    /// 产物要等任务真跑完才存在，于是两个都还没跑完的任务会拿到同一个"这个文件不存在"
    /// 的答案，随后后完成的那次 rename 会静默覆盖先完成的那份 —— 界面上两行都显示
    /// "已完成"、都指着同一个路径，而用户真正想要的那个变体已经没了。
    /// 下载那边有同样的教训（`engine.rs` 的 `reserved_paths` 就是为它存在的）。
    ///
    /// 重名让路成 " (1)"、" (2)"，与下载那边同一个 helper、同一个约定。
    /// **绝不覆盖源文件**：源文件若正好叫 `<stem>-压缩.mp4`，这里也会让到下一位。
    fn plan_output_path(
        jobs: &HashMap<JobId, JobEntry>,
        output_dir: &Path,
        stem: &str,
        suffix: &str,
        extension: &str,
    ) -> PathBuf {
        unique_path_avoiding(output_dir, &format!("{stem}{suffix}.{extension}"), |c| {
            jobs.values().any(|j| j.output == c)
        })
    }

    pub fn cancel(&self, id: JobId) {
        let token = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let Some(j) = jobs.get_mut(&id) else { return };
            if is_terminal(&j.state) {
                return;
            }
            // 排队中的任务没有 worker 会来收尾，所以状态要在这里直接落定；
            // 运行中的交给 worker（它会把状态推到 Canceled 并清掉半截产物）。
            if j.state == JobState::Queued {
                j.state = JobState::Canceled;
            }
            j.cancel.clone()
        };
        token.cancel();
        // 排队中的任务没有 worker 会再发事件，这里补一条，否则界面停在"排队中"
        let state = self.inner.jobs.lock().unwrap().get(&id).map(|j| j.state.clone());
        if let Some(s) = state {
            let _ = self.inner.tx.send(VideoOpEvent::StateChanged { id, state: s });
        }
        self.pump(); // 空出一个槽，后面排队的可以动了
    }

    /// 只对终态任务生效。返回 `false` 表示拒绝（还在跑，或本来就不在列表里）。
    pub fn remove(&self, id: JobId) -> bool {
        let removed = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            match jobs.get(&id) {
                Some(j) if is_terminal(&j.state) => {
                    jobs.remove(&id);
                    true
                }
                _ => false,
            }
        };
        if removed {
            let _ = self.inner.tx.send(VideoOpEvent::Removed { id });
            self.pump();
        }
        removed
    }

    /// 启动队列里下一个能跑的。与下载引擎同构：**"读活跃数 → 判上限 → 置状态"
    /// 必须在同一个锁里**，拆开的话两个线程会同时读到"还有空位"并各自启动一个，超出上限。
    fn pump(&self) {
        loop {
            let started = {
                let mut jobs = self.inner.jobs.lock().unwrap();
                let active = jobs.values().filter(|j| j.state == JobState::Running).count();
                if active >= MAX_ACTIVE_JOBS {
                    return;
                }
                let next = { self.inner.queue.lock().unwrap().pop_front() };
                let Some(id) = next else { return };
                match jobs.get_mut(&id) {
                    // 只有仍处于 Queued 才启动：排队期间被取消的任务 id 还在队列里，
                    // 没有这道闸 pump 会把它拉起来 —— 用户明明点了取消，它却自己复活。
                    Some(j) if j.state == JobState::Queued => {
                        j.state = JobState::Running;
                        // 换一个新 token，与下载引擎同理：上一轮的取消不该影响这一轮
                        j.cancel = CancellationToken::new();
                        Some((id, j.args.clone(), j.output.clone(), j.cancel.clone()))
                    }
                    _ => None,
                }
            };
            let Some((id, args, output, cancel)) = started else { continue };

            let _ = self.inner.tx.send(VideoOpEvent::StateChanged {
                id,
                state: JobState::Running,
            });

            let engine = self.clone();
            let tx = self.inner.tx.clone();
            self.inner.rt.spawn(async move {
                if !engine.job_exists(id) {
                    return; // 起 worker 之前就被移除了，没有收尾可言
                }
                let outcome = async {
                    let ffmpeg = engine.ffmpeg()?;
                    // 临时文件由 worker 写：排队中被取消的任务从没写过它。
                    // 删由 `run_job` 负责（成功、失败、取消三条路都删）。
                    let scratch = engine.job_scratch(id);
                    if let Some((path, content)) = &scratch {
                        std::fs::write(path, content)?;
                    }
                    // 分母**入队时就算好了** —— 那会儿 entry 里还有逐段的媒体信息，
                    // 而合并要的是各段之和。这里现算只能拿到首段。
                    let duration = engine.job_duration(id);
                    let tx_progress = tx.clone();
                    // 进度要**同时**落进内存状态与发事件：只发事件的话，重同步之后
                    // 列表里读到的进度会退回上一次事件的值（水合读的是 JobEntry）。
                    let engine_progress = engine.clone();
                    let on_progress: Arc<dyn Fn(f64) + Send + Sync> = Arc::new(move |f| {
                        engine_progress.note_progress(id, f);
                        let _ = tx_progress.send(VideoOpEvent::Progress { id, fraction: f });
                    });
                    run_job(
                        &ffmpeg,
                        &args,
                        &output,
                        duration,
                        scratch.as_ref().map(|(p, _)| p.as_path()),
                        on_progress,
                        &cancel,
                    )
                    .await
                }
                .await;

                match outcome {
                    // rename 由 run_job 自己做完 —— 它返回 Done 就意味着产物已在 output 上就位。
                    Ok(JobOutcome::Done) => engine.finish(id, JobState::Done),
                    Ok(JobOutcome::Interrupted) => engine.finish(id, JobState::Canceled),
                    Err(e) => engine.finish(id, JobState::Failed(format!("{e:#}"))),
                }
                engine.pump(); // 空出一个槽
            });
        }
    }

    /// worker 起跑前确认任务还在 —— 排队期间被移除的没有收尾可言。
    fn job_exists(&self, id: JobId) -> bool {
        self.inner.jobs.lock().unwrap().contains_key(&id)
    }

    fn job_scratch(&self, id: JobId) -> Option<(PathBuf, String)> {
        self.inner.jobs.lock().unwrap().get(&id).and_then(|j| j.scratch.clone())
    }

    fn job_duration(&self, id: JobId) -> Option<f64> {
        self.inner.jobs.lock().unwrap().get(&id).and_then(|j| j.duration)
    }

    /// 进度落进内存状态。列表水合时读的就是它。
    fn note_progress(&self, id: JobId, fraction: f64) {
        if let Some(j) = self.inner.jobs.lock().unwrap().get_mut(&id) {
            j.progress = Some(fraction);
        }
    }

    /// 落终态并发事件。**状态以这里为唯一出口** —— 所有分支都走它，
    /// 保证界面看到的终态与内部状态永远一致。
    fn finish(&self, id: JobId, state: JobState) {
        let changed = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            match jobs.get_mut(&id) {
                Some(j) => {
                    // 运行中被取消的任务会走到这里（worker 看到 Interrupted），
                    // 但用户已经点过取消了 —— 以用户的取消为准，不覆盖成别的。
                    if j.state == JobState::Canceled {
                        false
                    } else {
                        j.state = state.clone();
                        if state == JobState::Done {
                            j.progress = Some(1.0);
                        }
                        true
                    }
                }
                None => false,
            }
        };
        if changed {
            let _ = self.inner.tx.send(VideoOpEvent::StateChanged { id, state });
        }
    }
}
