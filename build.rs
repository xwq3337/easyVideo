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
    let status = Command::new("bash")
        .arg(&script)
        .arg(dir)
        .status()
        .unwrap_or_else(|e| {
            panic!(
                "跑不起来 {}：{e}\n这个脚本要用 bash —— Windows 上请用 Git Bash / MSYS2 构建。",
                script.display()
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

fn ready(p: &Path, want_bytes: u64) -> bool {
    std::fs::metadata(p)
        .map(|m| m.len() == want_bytes)
        .unwrap_or(false)
}
