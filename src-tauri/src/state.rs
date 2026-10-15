//! 应用级共享状态：引擎句柄 + 设置副本。
//!
//! 这是把 Task 1–5 的零件装起来的那一层：`main` 建好 runtime、装好 `AppState` 交给 Tauri
//! `manage`，12 个命令从 `State<'_, AppState>` 里取。

use std::path::PathBuf;
use std::sync::Mutex;

use easydownvideo::core::engine::Engine;
use easydownvideo::settings::Settings;

pub struct AppState {
    pub engine: Engine,
    pub settings: Mutex<Settings>,
}

impl AppState {
    /// 用户改了设置之后必须**同时**更新引擎内部那一份——并发上限、并发数、yt-dlp 开关
    /// 都是引擎在用的；只写文件不改引擎，界面上的改动要等下次重启才生效。
    pub fn apply_settings(&self, s: Settings) {
        *self.settings.lock().unwrap() = s.clone();
        self.engine.update_settings(s);
    }
}

pub fn build_engine(rt: tokio::runtime::Handle, settings: Settings, data_dir: PathBuf) -> Engine {
    Engine::new(rt, settings, data_dir)
}
