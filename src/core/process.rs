//! 子进程启动的统一入口。
//!
//! 所有外部程序（ffmpeg / yt-dlp）都必须经由 [`hidden_command`] 启动，
//! 不要直接写 `tokio::process::Command::new`。

use std::ffi::OsStr;

use tokio::process::Command;

/// 建一个"不弹控制台"的 `Command`。
///
/// **为什么需要这个：** release 构建的主程序是 GUI 子系统
/// （`src-tauri/src/main.rs` 里的 `windows_subsystem = "windows"`），
/// 本身没有控制台。Windows 在这种父进程下启动控制台程序（`ffmpeg.exe`、
/// `yt-dlp.exe`）时，会**给子进程新分配一个空控制台窗口** —— 我们的
/// stdout/stderr 要么接了管道、要么指向 null，所以那个窗口里一个字都没有，
/// 用户看到的就是一个莫名其妙闪出来的黑框。`CREATE_NO_WINDOW` 禁止这个分配。
///
/// 之前只有主程序自己加了子系统标记，子进程一处都没加，于是合并/裁剪
/// （探测信息、抽预览帧、跑任务各 spawn 一次）会连着弹好几个空控制台。
///
/// debug 构建下主程序自己带控制台，子进程会复用它，所以开发时看不到这个问题 ——
/// 只在装出来的 release 版复现。
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        // https://learn.microsoft.com/windows/win32/procthread/process-creation-flags
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
