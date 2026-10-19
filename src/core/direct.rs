use anyhow::{anyhow, Context};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::core::events::ProgressHandle;

pub const MIN_BLOCK_SIZE: u64 = 4 * 1024 * 1024;

pub fn plan_blocks(total: u64, concurrency: usize, min_block: u64) -> Vec<(u64, u64)> {
    if total == 0 {
        return vec![];
    }
    let n = concurrency.max(1) as u64;
    let chunk = ((total + n - 1) / n).max(min_block);
    let mut out = Vec::new();
    let mut start = 0u64;
    while start < total {
        let end = (start + chunk).min(total);
        out.push((start, end));
        start = end;
    }
    out
}

pub fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    unique_path_avoiding(dir, filename, |_| false)
}

/// `unique_path` 的并发安全版本：除文件系统外，还避开调用方已经占用、但**尚未落盘**的名字。
///
/// 为什么需要它：`unique_path` 是"先查后用"—— 两个任务在同一时刻都看到"最终文件不存在"，
/// 于是算出**同一个**最终路径，各自下载到自己的任务目录后都往那一个路径写；后完成的把先
/// 完成的覆盖掉，而两个任务都报成功。占用登记必须与查询在**同一个锁内**完成，否则只是把
/// 窗口挪了个位置。
pub fn unique_path_avoiding(dir: &Path, filename: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let free = |p: &Path| !p.exists() && !taken(p);
    let candidate = dir.join(filename);
    if free(&candidate) {
        return candidate;
    }
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .to_string();
    let ext = Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    for i in 1.. {
        let p = dir.join(format!("{stem} ({i}){ext}"));
        if free(&p) {
            return p;
        }
    }
    unreachable!()
}

pub async fn download_direct(
    client: &reqwest::Client,
    url: &str,
    final_path: &Path,
    task_dir: &Path,
    headers: &[(String, String)],
    size: Option<u64>,
    accepts_ranges: bool,
    concurrency: usize,
    min_block: u64,
    progress: &ProgressHandle,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(task_dir).context("create task dir")?;
    // 每条分块请求都要带浏览器扩展抓到的头（Referer / Cookie / UA）。
    // HeaderMap 内部是 Arc，克隆很便宜；在循环外建一次，循环里只 clone。
    let header_map = crate::core::request::header_map(headers);
    let blocks = match (size, accepts_ranges) {
        (Some(total), true) if total > 0 => plan_blocks(total, concurrency, min_block),
        _ => vec![(0, u64::MAX)], // 单流
    };
    // ProgressHandle 挂在 TaskEntry 上，**跨单次运行存活**：失败后的 retry、暂停后的
    // unpause、重启后的 resume 都会再次进入本函数，而上一轮已计入的字节仍留在 handle 里。
    // 本轮从零开始，否则重跑会把新一轮的计数叠加上去，downloaded 超过 total
    // （用户看到 1.4 GB / 2.0 GB）。
    progress.reset();
    if let Some(total) = size {
        progress.set_total(total);
    }

    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut handles = Vec::new();
    for (i, (start, end)) in blocks.iter().enumerate() {
        let client = client.clone();
        let url = url.to_string();
        let part_path = task_dir.join(format!("f.part-{i}"));
        let expected = *end - *start;
        let (start, end) = (*start, *end);
        let progress = progress.clone();
        let cancel = cancel.clone();
        let header_map = header_map.clone();
        let permit = sem.clone().acquire_owned().await?;
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            // 断点续传：已完成则跳过
            if let Ok(meta) = std::fs::metadata(&part_path) {
                if meta.len() == expected {
                    progress.add(expected);
                    return Ok(());
                }
            }
            let req = client.get(&url).headers(header_map);
            let req = if end != u64::MAX {
                req.header("Range", format!("bytes={start}-{}", end - 1))
            } else {
                req
            };
            // 分块级重试：指数退避 3 次（1s/2s/4s）
            let mut last_err = None;
            for attempt in 0..3 {
                match download_one_block(&req, &part_path, &progress, &cancel).await {
                    Ok(()) => {
                        last_err = None;
                        break;
                    }
                    Err(e) => {
                        last_err = Some(e);
                        if cancel.is_cancelled() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                    }
                }
            }
            match last_err {
                Some(e) => Err(e),
                None => Ok::<_, anyhow::Error>(()),
            }
        }));
    }
    for h in handles {
        h.await??;
    }

    // 流式拼接：逐块 64KB 拷贝，内存不随文件大小增长
    let mut out = tokio::fs::File::create(final_path).await?;
    for i in 0..blocks.len() {
        let part_path = task_dir.join(format!("f.part-{i}"));
        let mut input = tokio::fs::File::open(&part_path).await?;
        tokio::io::copy(&mut input, &mut out).await?;
        std::fs::remove_file(&part_path).ok();
    }
    out.flush().await?;
    Ok(())
}

/// `path` 是否已经是一个**完整的**轨道文件。
///
/// `expected` 为 `None` 时一律返回 false：不知道大小就无法断言完整，宁可重下。
/// 暂停在"视频下完、音频下到一半"时，恢复的 worker 靠它跳过整段视频。
pub fn file_matches_size(path: &Path, expected: Option<u64>) -> bool {
    let Some(expected) = expected.filter(|s| *s > 0) else {
        return false;
    };
    std::fs::metadata(path)
        .map(|m| m.len() == expected)
        .unwrap_or(false)
}

/// 下载一条独立轨道（合并下载时的音频轨）。
///
/// 与 `download_direct` 的区别：不规划分块、不续传，直接整条流式写入
/// `dest` 旁边的 `.part`，完成后原子改名。DASH 音频通常只有几 MB，分块收益
/// 抵不上复杂度。失败重试 3 次，进度按本模块统一的回滚规则处理。
pub async fn download_track(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    dest: &Path,
    expected_size: Option<u64>,
    progress: &ProgressHandle,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).context("create track dir")?;
    }
    let part_path = dest.with_extension("part");
    let header_map = crate::core::request::header_map(headers);
    let mut last_err = None;
    for attempt in 0..3 {
        if cancel.is_cancelled() {
            anyhow::bail!("canceled");
        }
        match download_track_once(
            client,
            url,
            &header_map,
            &part_path,
            expected_size,
            progress,
            cancel,
        )
        .await
        {
            Ok(()) => {
                tokio::fs::rename(&part_path, dest).await?;
                return Ok(());
            }
            Err(e) => {
                last_err = Some(e);
                if cancel.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("track download failed")))
}

async fn download_track_once(
    client: &reqwest::Client,
    url: &str,
    header_map: &reqwest::header::HeaderMap,
    part_path: &Path,
    expected_size: Option<u64>,
    progress: &ProgressHandle,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    // 与分块下载同一套回滚：重试时不能把上一次尝试已计入的字节留在进度里。
    let mut rollback = ProgressRollback::new(progress);
    let resp = client
        .get(url)
        .headers(header_map.clone())
        .send()
        .await?
        .error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(part_path).await?;
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(anyhow!("canceled")); // 守卫在此 drop，自动回滚
        }
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        rollback.add(chunk.len() as u64);
    }
    file.flush().await?;
    // 服务端提前断流时 reqwest 不一定报错。已知总大小时必须核对，
    // 否则会把一个截断的音频轨送进 ffmpeg，得到音画不同步的成品。
    if let Some(expected) = expected_size.filter(|s| *s > 0) {
        let actual = std::fs::metadata(part_path)?.len();
        if actual != expected {
            anyhow::bail!("track incomplete: {actual}/{expected} bytes");
        }
    }
    rollback.disarm();
    Ok(())
}

async fn download_one_block(
    req: &reqwest::RequestBuilder,
    part_path: &Path,
    progress: &ProgressHandle,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    // 进度回滚守卫：本次尝试计入的字节先记在守卫里，若这次尝试失败（守卫未被解除），
    // 退出时把已计入的字节减回去。不做这件事的话，重试会把同一段字节重复累加——
    // 三次重试后同一块被计三倍，进度超过总数，速度与 ETA 同时失真。
    let mut rollback = ProgressRollback::new(progress);
    let resp = req
        .try_clone()
        .context("request not cloneable")?
        .send()
        .await?
        .error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(part_path).await?;
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            return Err(anyhow!("canceled")); // 守卫在此 drop，自动回滚
        }
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        rollback.add(chunk.len() as u64);
    }
    file.flush().await?;
    rollback.disarm();
    Ok(())
}

/// 见 `download_one_block` 中的说明：为一次下载尝试的进度累加提供失败回滚。
struct ProgressRollback<'a> {
    progress: &'a ProgressHandle,
    counted: u64,
    armed: bool,
}

impl<'a> ProgressRollback<'a> {
    fn new(progress: &'a ProgressHandle) -> Self {
        Self {
            progress,
            counted: 0,
            armed: true,
        }
    }
    fn add(&mut self, n: u64) {
        // 直链的 downloaded 本来就是字节数，所以两个计数器一起加、也必须一起退。
        // 只退其中一个的话，重试过的块会把进度留在 downloaded 上（界面显示超过总数），
        // 而速度那边却少了这段。
        self.progress.add(n);
        self.progress.add_bytes(n);
        self.counted += n;
    }
    /// 尝试成功：保留已计入的字节，撤销回滚。
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProgressRollback<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.progress.sub(self.counted);
            self.progress.sub_bytes(self.counted);
        }
    }
}
