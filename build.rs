//! 构建之前确保内嵌的 ffmpeg 在盘上。
//!
//! `src/core/ffmpeg.rs` 用 `include_bytes!` 把它编译进二进制，所以**文件必须在
//! rustc 读源码之前就位** —— build script 正好跑在那个时点。
//!
//! 为什么这两个二进制不在 git 里：`windows-x86_64/ffmpeg.exe` 是 100.54 MB，
//! 超过 GitHub 单文件 100 MB 的硬上限，只要它在历史里 push 就会被拒（GH001）；
//! macOS 那个 63 MB 不触发硬限，但两个加起来压缩后约 67 MiB，每次推送都要重传。
//! 细节见 `vendor/ffmpeg/README.md` 与 `scripts/fetch-ffmpeg.sh` 顶部。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 与 `src/core/ffmpeg.rs` 的 cfg 一一对应：只有这两个目标真的把 ffmpeg 嵌进去
/// （其余目标那边是 `b"placeholder"`），所以这里也什么都不做。
///
/// 返回 `(vendor 子目录, 文件名, 期望字节数)`。
fn wanted() -> Option<(&'static str, &'static str, u64)> {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    match (os.as_str(), arch.as_str()) {
        ("macos", "aarch64") => Some(("macos-aarch64", "ffmpeg", 66_332_880)),
        ("windows", "x86_64") => Some(("windows-x86_64", "ffmpeg.exe", 105_423_872)),
        _ => None,
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=scripts/fetch-ffmpeg.sh");
    println!("cargo:rerun-if-env-changed=EASYVIDEO_BASH");

    let Some((dir, file, want_bytes)) = wanted() else {
        return;
    };

    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let path = root.join("vendor/ffmpeg").join(dir).join(file);

    // **也盯住这个文件本身**：只盯 build.rs 的话，构建成功之后把二进制删掉不会
    // 触发重新跑脚本，于是 `include_bytes!` 直接报 "file not found"，
    // 而用户刚删掉它正是为了验证下载能不能工作。
    println!("cargo:rerun-if-changed={}", path.display());

    if ready(&path, want_bytes) {
        return;
    }

    eprintln!(
        "vendor/ffmpeg/{dir}/{file} 缺失或大小不对，正在取回（首次构建需要联网，约 30–100 MB）…"
    );
    let script = root.join("scripts/fetch-ffmpeg.sh");
    let bash = find_bash();
    let status = Command::new(&bash)
        .arg(&script)
        .arg(dir)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "跑不起来 {}：{e}\n这个脚本要用 bash —— Windows 上请用 Git Bash / MSYS2 构建。",
                bash.display()
            )
        });
    if !status.success() {
        panic!(
            "取回 vendor/ffmpeg/{dir}/{file} 失败（{status}）。这个文件是 `include_bytes!` \
             的编译期输入，没有它编不过。手动跑 `scripts/fetch-ffmpeg.sh {dir}` 看详细原因。"
        );
    }
    // 脚本自己会完整校验 SHA256；这里再确认一次大小，防止它半路被别的进程改掉
    if !ready(&path, want_bytes) {
        panic!("脚本跑完了，但 vendor/ffmpeg/{dir}/{file} 仍然不对（不存在或字节数不符）");
    }
}

/// 找一个**能真正执行脚本**的 bash。
///
/// 不能直接 `Command::new("bash")`：Windows 的 PATH 里通常先有
/// `C:\Windows\System32\bash.exe`（WSL 启动器）。没装 WSL 发行版时它不执行
/// 任何东西，只打印一行 UTF-16 错误并以 1 退出 —— build.rs 只能看到
/// "exit code: 1"，脚本自己的日志一条都没有，看起来像下载失败，其实是根本没跑。
/// 所以这里优先 Git for Windows / MSYS2 的 bash，并显式跳过 WSL 启动器。
///
/// 找不到时返回 `bash` 交给系统解析（非 Windows 上就是这样）。
fn find_bash() -> PathBuf {
    if let Some(p) = std::env::var_os("EASYVIDEO_BASH") {
        return PathBuf::from(p);
    }
    if !cfg!(windows) {
        return PathBuf::from("bash");
    }

    let mut candidates = Vec::new();

    // 1) PATH 上的 git.exe 反推安装根：<root>\cmd\git.exe 或 <root>\bin\git.exe；
    //    同时收下 PATH 里直接出现的 bash.exe（MSYS2 / Cygwin / 自定义 Git）。
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("git.exe").is_file() {
                if let Some(git_root) = dir.parent() {
                    candidates.push(git_root.join("bin").join("bash.exe"));
                    candidates.push(git_root.join("usr").join("bin").join("bash.exe"));
                }
            }
            candidates.push(dir.join("bash.exe"));
        }
    }

    // 2) 常见安装位置（PATH 上恰好没有 git 时）
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base);
            for sub in ["Git", "Programs/Git"] {
                let root = base.join(sub);
                candidates.push(root.join("bin").join("bash.exe"));
                candidates.push(root.join("usr").join("bin").join("bash.exe"));
            }
        }
    }

    // 3) MSYS2 默认安装位置
    if let Some(drive) = std::env::var_os("SystemDrive") {
        let root = PathBuf::from(format!("{}\\", drive.to_string_lossy()));
        for name in ["msys64", "msys32"] {
            candidates.push(root.join(name).join("usr").join("bin").join("bash.exe"));
        }
    }

    for c in &candidates {
        if c.is_file() && !is_wsl_stub(c) {
            return c.clone();
        }
    }

    let wsl_hint = candidates
        .iter()
        .find(|c| is_wsl_stub(c))
        .map(|p| format!("\n注意：{} 是 WSL 启动器，不是 Git Bash。", p.display()))
        .unwrap_or_default();
    panic!(
        "Windows 上找不到可用的 bash。取回内嵌 ffmpeg 的脚本要用它 —— 请安装 Git for Windows\
         （https://git-scm.com/download/win），或把 bash 的完整路径设到环境变量 EASYVIDEO_BASH。{wsl_hint}"
    );
}

/// `C:\Windows\System32\bash.exe` 与 WindowsApps 里的 `bash.exe` 都是 WSL 启动器。
fn is_wsl_stub(p: &Path) -> bool {
    let Some(dir) = p.parent() else {
        return false;
    };
    let dir = dir.to_string_lossy().to_ascii_lowercase();
    let win = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let sys32 = Path::new(&win)
        .join("System32")
        .to_string_lossy()
        .to_ascii_lowercase();
    dir == sys32 || dir.ends_with(r"\windowsapps")
}

fn ready(p: &Path, want_bytes: u64) -> bool {
    std::fs::metadata(p)
        .map(|m| m.len() == want_bytes)
        .unwrap_or(false)
}
