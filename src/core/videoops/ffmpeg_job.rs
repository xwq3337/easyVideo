use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, Context};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_util::sync::CancellationToken;

#[derive(Debug, PartialEq, Eq)]
pub enum JobOutcome {
    Done,
    /// token 被取消。子进程已被杀掉并收尸，半截产物已删除。
    Interrupted,
}

/// 解析 `-progress pipe:1` 的一行，换算成 0.0–1.0。
///
/// **`out_time_ms` 也是微秒。** ffmpeg 这个字段名是错的：它和 `out_time_us` 一样
/// 输出微秒（实测 5 秒的片子两者都是 4933333）。照字面当毫秒用，进度会永远停在
/// 0.5%，而且看起来像卡死而不像报错。
///
/// 不相关的行（`frame=`、`progress=continue` …）与解析不出来的值一律 `None`。
pub fn parse_progress_fraction(line: &str, duration_secs: f64) -> Option<f64> {
    if duration_secs <= 0.0 {
        // 没有分母就不编一个分数出来 —— 调用方据此显示不确定态
        return None;
    }
    let raw = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    let micros: f64 = raw.trim().parse().ok()?;
    let frac = micros / (duration_secs * 1_000_000.0);
    Some(frac.clamp(0.0, 1.0))
}

/// 跑一次 ffmpeg，全程可被 `cancel` 打断。**成功时产物就位在 `output`。**
///
/// 临时名（`<output>.part`）由本函数自己推导 —— 与 `compress_args` 用的是同一个
/// `part_path_for`，所以"ffmpeg 写到哪"与"我们清理/改名哪一个"不可能对不上。
/// rename 也在这里做：**"原子地产出"是一个动作**，拆给调用方做的话，
/// 每个调用方都要自己记得改名，而且失败时留下半截 `.part` 的责任也跟着散开。
///
/// 收尾顺序**固定**：先 kill、再等它真的退出、最后才删产物。顺序反了的话，
/// kill 只是发信号，进程可能还有一次写在做 —— 不等它就删，它随后会把半截文件
/// 重新写出来，用户会在目录里看到一个打不开的 mp4。
pub async fn run_job(
    ffmpeg: &Path,
    args: &[String],
    output: &Path,
    duration_secs: Option<f64>,
    on_progress: Arc<dyn Fn(f64) + Send + Sync>,
    cancel: &CancellationToken,
) -> anyhow::Result<JobOutcome> {
    let part_path = crate::core::videoops::presets::part_path_for(output);
    let mut cmd = tokio::process::Command::new(ffmpeg);
    cmd.args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    // 兜底：future 因为别的原因被 drop（例如整个 runtime 收尾）时，Child 一并 drop，
    // 进程同样会被收掉。与 build_remux 同样的理由。
    cmd.kill_on_drop(true);
    let mut child = cmd.spawn().context("spawn ffmpeg")?;

    // stdout 是进度（`-progress pipe:1`），stderr 是错误。两条管道都必须有人抽干 ——
    // 没人读的话 ffmpeg 写满 64KB 管道缓冲就会卡在 write 上永不退出。
    let stdout = child.stdout.take();
    let progress_task = tokio::spawn(async move {
        let Some(out) = stdout else { return };
        let mut lines = BufReader::new(out).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(frac) = parse_progress_fraction(&line, duration_secs.unwrap_or(0.0)) {
                on_progress(frac);
            }
        }
    });
    let mut err_pipe = child.stderr.take();
    let err_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(e) = err_pipe.as_mut() {
            let _ = e.read_to_end(&mut buf).await;
        }
        buf
    });

    tokio::select! {
        status = child.wait() => {
            let status = status.context("wait ffmpeg")?;
            let _ = progress_task.await; // 进程已退出，stdout 会 EOF，读循环自然收尾
            let stderr = err_task.await.unwrap_or_default();
            if !status.success() {
                std::fs::remove_file(&part_path).ok();
                return Err(anyhow!("ffmpeg 失败: {}", summarize_stderr(&stderr)));
            }
            // 成功：`.part` → 目标名。同一文件系统内 rename 原子，所以"改过名"等价于"完整"。
            if let Err(e) = std::fs::rename(&part_path, output) {
                // **改名失败也要删掉临时文件** —— 这是唯一一条曾经会留下半截产物的路径。
                // spec §7 写死了"失败或取消绝不留下半截文件"：留着它，用户会在目录里
                // 看到一个打不开的 `<名字>-压缩.mp4.part`，而列表里那一行写着"失败"。
                std::fs::remove_file(&part_path).ok();
                return Err(anyhow!(
                    "产物改名失败: {} → {}: {e}",
                    part_path.display(),
                    output.display()
                ));
            }
            Ok(JobOutcome::Done)
        }
        _ = cancel.cancelled() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = progress_task.await;
            let _ = err_task.await;
            std::fs::remove_file(&part_path).ok();
            Ok(JobOutcome::Interrupted)
        }
    }
}

/// 错误摘要：取最后 20 行、且总长不超过 2 KB。
///
/// ffmpeg 的错误总在尾部，而它跑一个长任务能吐出几千行 —— 全量塞进界面没有意义。
pub fn summarize_stderr(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = if lines.len() > 20 { &lines[lines.len() - 20..] } else { &lines[..] };
    let mut joined = tail.join("\n");
    while joined.len() > 2048 {
        // 从头丢，保住尾部（错误在那头）
        match joined.find('\n') {
            Some(i) => joined = joined[i + 1..].to_string(),
            None => break,
        }
    }
    joined
}
