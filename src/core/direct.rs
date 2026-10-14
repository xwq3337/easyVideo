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
    let candidate = dir.join(filename);
    if !candidate.exists() {
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
        if !p.exists() {
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
    size: Option<u64>,
    accepts_ranges: bool,
    concurrency: usize,
    min_block: u64,
    progress: &ProgressHandle,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(task_dir).context("create task dir")?;
    let blocks = match (size, accepts_ranges) {
        (Some(total), true) if total > 0 => plan_blocks(total, concurrency, min_block),
        _ => vec![(0, u64::MAX)], // 单流
    };
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
            let req = client.get(&url);
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
        self.progress.add(n);
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
        }
    }
}
