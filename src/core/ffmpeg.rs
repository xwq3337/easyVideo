use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Context};
use sha2::Digest;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

// ===== 内嵌二进制（Task 10 用真实文件替换占位内容）=====
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const FFMPEG_BYTES: &[u8] = include_bytes!("../../vendor/ffmpeg/macos-aarch64/ffmpeg");
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const FFMPEG_BYTES: &[u8] = include_bytes!("../../vendor/ffmpeg/windows-x86_64/ffmpeg.exe");
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const FFMPEG_BYTES: &[u8] = b"placeholder";

fn vendored_sha256() -> String {
    let mut h = sha2::Sha256::new();
    h.update(FFMPEG_BYTES);
    format!("{:x}", h.finalize())
}

pub fn ensure_ffmpeg(data_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(data_dir).context("create data dir")?;
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let target = data_dir.join(exe);
    let sha = vendored_sha256();
    if target.exists() {
        let mut h = sha2::Sha256::new();
        h.update(&std::fs::read(&target)?);
        if format!("{:x}", h.finalize()) == sha {
            return Ok(target);
        }
    }
    std::fs::write(&target, FFMPEG_BYTES)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&target)?.permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&target, perm)?;
    }
    Ok(target)
}

// 注意：**没有** `write_concat_list`。concat 列表由 Task 8 的 `download_media` 生成
// （分片顺序是在那里决定的），转义也在那里完成。本模块只负责把已有的 `plan.list_file`
// 交给 ffmpeg。曾计划在此提供一个 `write_concat_list`，但它从头到尾没有调用方——
// 属于死代码，且会让"转义"这件事出现两个可能不同步的实现位置。

pub fn build_remux(
    ffmpeg: &Path,
    list: &Path,
    output: &Path,
) -> tokio::process::Command {
    let mut cmd = crate::core::process::hidden_command(ffmpeg);
    cmd.arg("-y")
        .arg("-f").arg("concat")
        .arg("-safe").arg("0")
        .arg("-i").arg(list)
        .arg("-c").arg("copy");
    // 不要显式加 `-bsf:a aac_adtstoasc`。实测（ffmpeg 7.1.1）：
    //   AAC 的 TS 不加也能正确输出 h264+aac —— ffmpeg 会按需自动插入该滤镜；
    //   而 AC-3 的 TS 加了会直接失败："Codec 'ac3' is not supported by the bitstream
    //   filter 'aac_adtstoasc'"，整个合并就此中断。
    // 也就是说显式加它会让所有非 AAC 音频（AC-3/E-AC-3/MP3/Opus）的 HLS 流下载后合并失败，
    // 而不加则两种都正常。让 ffmpeg 自己决定。
    cmd.arg(output);
    // **必须开 `kill_on_drop`**：否则 ffmpeg 子进程会变成孤儿继续跑，而引擎在取消路径上会清掉
    // 任务目录（"取消是不要了"），于是孤儿进程还在往一个已被删除的目录里写。
    //
    // 现在 `remux_to_mp4` 会显式 `start_kill` + `wait`（暂停/取消路径，见那里的说明），这个标志
    // 是**兜底**：future 因为别的原因被 drop（例如整个 runtime 收尾）时，Child 一并 drop，
    // 进程同样会被收掉。设置在 `build_remux` 而不是调用点 —— 调用方拿不到 Command。
    //
    // 故意不写测试：`tokio::process::Command` 没有读取该标志的 getter，无法断言。
    // 这是有据可查的"未测试"而非"忘了测"。
    cmd.kill_on_drop(true);
    cmd
}

/// 合并的结果。把"被用户中断"与"真的失败"分开 —— 前者要保留分片目录供继续，后者不是。
#[derive(Debug, PartialEq, Eq)]
pub enum RemuxOutcome {
    Done,
    /// token 被取消（用户暂停或取消）。ffmpeg 子进程已被**杀掉并收尸**，
    /// 可能写了一半的产物也已删除。
    Interrupted,
}

/// 把 DASH 的视频轨与音频轨无损合成一个 mp4。
///
/// B 站（以及所有 DASH 站点）的高清格式是 video-only 的 `.m4s` + 独立的 `.m4a`，
/// 只下视频轨会得到无声文件，所以 `-map` 明确各取一条流，`-c copy` 不做任何重编码，
/// `+faststart` 让 moov 前置、浏览器/播放器可以边下边播。
pub fn build_mux(
    ffmpeg: &Path,
    video: &Path,
    audio: &Path,
    output: &Path,
) -> tokio::process::Command {
    let mut cmd = crate::core::process::hidden_command(ffmpeg);
    cmd.arg("-y")
        .arg("-i")
        .arg(video)
        .arg("-i")
        .arg(audio)
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("1:a:0")
        .arg("-c")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output);
    // 与 build_remux 同一个理由：调用点拿不到 Command，兜底的 kill_on_drop 只能设在这里。
    cmd.kill_on_drop(true);
    cmd
}

pub async fn mux_video_audio(
    ffmpeg: &Path,
    video: &Path,
    audio: &Path,
    output: &Path,
    cancel: &CancellationToken,
) -> anyhow::Result<RemuxOutcome> {
    let cmd = build_mux(ffmpeg, video, audio, output);
    run_ffmpeg(cmd, output, cancel).await
}

/// 合并成 mp4，全程可被 `cancel` 打断。
///
/// 合并可能很久（`-c copy` 要把整个文件搬一遍），而界面在 `Merging` 上同样给出暂停按钮，
/// 所以这里必须真的能停下 —— 而 ffmpeg 本身**不看我们的 token**，只能把等待它的 future
/// 连同子进程一起处理掉。
pub async fn remux_to_mp4(
    ffmpeg: &Path,
    plan: &crate::core::hls::ConcatPlan,
    output: &Path,
    cancel: &CancellationToken,
) -> anyhow::Result<RemuxOutcome> {
    let cmd = build_remux(ffmpeg, &plan.list_file, output);
    run_ffmpeg(cmd, output, cancel).await
}

async fn run_ffmpeg(
    mut cmd: tokio::process::Command,
    output: &Path,
    cancel: &CancellationToken,
) -> anyhow::Result<RemuxOutcome> {
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("spawn ffmpeg")?;

    // stderr 必须有人抽干：`wait()` 期间没人读管道的话，ffmpeg 写满 64KB 就会卡在 write 上
    // 永不退出。（原先用 `Command::output()`，它替我们做了这件事；改成 spawn 就得自己做。）
    let mut err_pipe = child.stderr.take();
    let drain = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(e) = err_pipe.as_mut() {
            let _ = e.read_to_end(&mut buf).await;
        }
        buf
    });

    tokio::select! {
        status = child.wait() => {
            let status = status.context("wait ffmpeg")?;
            let stderr = drain.await.unwrap_or_default();
            if !status.success() {
                // 失败时 ffmpeg 往往已经写了一半产物。把它留在最终路径上，
                // 用户会看到一个打不开的 mp4；重试还会因唯一性检查改名成
                // `name (1).mp4`，损坏文件反而一直占着原名。失败就删掉。
                std::fs::remove_file(output).ok();
                return Err(anyhow!(
                    "ffmpeg failed: {}",
                    String::from_utf8_lossy(&stderr)
                ));
            }
            Ok(RemuxOutcome::Done)
        }
        _ = cancel.cancelled() => {
            // **先杀、再等它真的退出，最后才删产物。** 顺序不能反：kill 只是发信号，进程
            // 可能还有一次写在做；不等它就删，它随后会把半截文件重新写出来 —— 用户会在
            // 下载目录里看到一个打不开的 mp4，而任务显示"已暂停"。
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = drain.await;
            std::fs::remove_file(output).ok();
            Ok(RemuxOutcome::Interrupted)
        }
    }
}
