use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use sha2::Digest;

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
    let mut cmd = tokio::process::Command::new(ffmpeg);
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
    // 用户取消任务时，`remux_to_mp4` 的 future 会被 drop，而 `cmd.output()` 内部持有的 Child
    // 一并被 drop。**必须开 `kill_on_drop`**：否则 ffmpeg 子进程会变成孤儿继续跑，而引擎在取消路径上
    // 会清掉任务目录（"取消是不要了"），于是孤儿进程还在往一个已被删除的目录里写。
    // 设置在 `build_remux` 而不是调用点：Task 13 调的是高层 `remux_to_mp4`，拿不到 Command，
    // 没有别的地方能设置它 —— 这是唯一正确的归属点。
    //
    // 故意不写测试：`tokio::process::Command` 没有读取该标志的 getter，无法断言。
    // 这是有据可查的"未测试"而非"忘了测"。
    cmd.kill_on_drop(true);
    cmd
}

pub async fn remux_to_mp4(
    ffmpeg: &Path,
    plan: &crate::core::hls::ConcatPlan,
    output: &Path,
) -> anyhow::Result<()> {
    let out = output.to_path_buf();
    let mut cmd = build_remux(ffmpeg, &plan.list_file, &out);
    let output_status = cmd.output().await.context("spawn ffmpeg")?;
    if !output_status.status.success() {
        return Err(anyhow!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output_status.stderr)
        ));
    }
    Ok(())
}
