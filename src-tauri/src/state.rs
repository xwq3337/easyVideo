//! 应用级共享状态：引擎句柄 + 设置副本。
//!
//! 这是把 Task 1–5 的零件装起来的那一层：`main` 建好 runtime、装好 `AppState` 交给 Tauri
//! `manage`，命令层从 `State<'_, AppState>` 里取。

use std::path::PathBuf;
use std::sync::Mutex;

use easydownvideo::core::engine::Engine;
use easydownvideo::core::videoops::VideoOps;
use easydownvideo::settings::Settings;

use crate::extension_api::LocalApi;

pub struct AppState {
    pub engine: Engine,
    /// 视频操作子系统。**与下载引擎完全独立** —— 不共用任务表、队列、并发槽，
    /// 也不共用事件通道。转码吃满 CPU，不该把下载的并发槽占住。
    pub videoops: VideoOps,
    pub settings: Mutex<Settings>,
    /// 浏览器扩展用的本地 HTTP 接口。生命周期独立于设置：改端口/令牌时
    /// 只有它会重启监听，下载引擎不受影响。
    pub local_api: LocalApi,
}

impl AppState {
    /// 用户改了设置之后必须**同时**更新引擎内部那一份——并发上限、并发数、yt-dlp 开关
    /// 都是引擎在用的；只写文件不改引擎，界面上的改动要等下次重启才生效。
    pub fn apply_settings(&self, s: Settings) {
        *self.settings.lock().unwrap() = s.clone();
        // ffmpeg 路径覆盖是全局设置，两个子系统都要跟着换 —— 只推给下载引擎的话，
        // 用户改了设置会看到"下载用了新 ffmpeg、压缩还在用旧的"。
        self.videoops.set_ffmpeg_override(s.ffmpeg_path_override.clone());
        self.engine.update_settings(s);
    }
}

pub fn build_engine(rt: tokio::runtime::Handle, settings: Settings, data_dir: PathBuf) -> Engine {
    Engine::new(rt, settings, data_dir)
}

pub fn build_videoops(rt: tokio::runtime::Handle, data_dir: PathBuf) -> VideoOps {
    VideoOps::new(rt, data_dir)
}

/// 建好之后立刻把设置里的 ffmpeg 覆盖推给它 —— 否则重启后要等用户重新保存一次设置
/// 才会生效。
pub fn build_videoops_with_settings(
    rt: tokio::runtime::Handle,
    data_dir: PathBuf,
    settings: &Settings,
) -> VideoOps {
    let ops = VideoOps::new(rt, data_dir);
    ops.set_ffmpeg_override(settings.ffmpeg_path_override.clone());
    ops
}
