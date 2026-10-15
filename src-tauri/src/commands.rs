//! 前端能调用的 12 个命令。
//!
//! **全部是薄封装**——业务逻辑一律在 core 里，这里只做「取 `State`、调引擎、转 DTO」。
//! 所以本层没有单测（`cargo check --workspace` 加 Task 12 的手动验收覆盖它），
//! 与规格 §7 的测试策略一致。

use tauri::State;

use crate::dto::{
    settings_dto, settings_from, task_dto, ChoiceDto, ChoiceKindDto, SettingsDto, TaskDto,
};
use crate::state::AppState;

type R<T> = Result<T, String>;

#[tauri::command]
pub fn get_tasks(state: State<'_, AppState>) -> Vec<TaskDto> {
    state.engine.list_tasks().into_iter().map(task_dto).collect()
}

#[tauri::command]
pub fn add_url(state: State<'_, AppState>, url: String) -> u64 {
    state.engine.add_url(url)
}

#[tauri::command]
pub fn pause(state: State<'_, AppState>, id: u64) {
    state.engine.pause(id);
}

#[tauri::command]
pub fn unpause(state: State<'_, AppState>, id: u64) {
    state.engine.unpause(id);
}

#[tauri::command]
pub fn retry(state: State<'_, AppState>, id: u64) {
    state.engine.retry(id);
}

#[tauri::command]
pub fn cancel(state: State<'_, AppState>, id: u64) {
    state.engine.cancel(id);
}

/// 把一条**已经彻底结束**的任务移出列表。被拒时返回 `Err`。
///
/// **文案必须盖住 `false` 的全部三种来源**，这一点容易漏：
/// 1. **任务不存在**（双击时第二次点击、或行已经过期）；
/// 2. 状态不是终态（界面已经按状态把按钮渲染成「取消」了，这条只在竞态下触发）；
/// 3. 是终态、但 worker 还在收尾（`cancel()` 是立刻置状态、worker 可能还挂在网络上）。
///
/// 只写第 3 种的话，第 1 种情况下文案就变成假话：任务已经没了，"稍后再试"却
/// 暗示重试能成功 —— 永远不会。第 1 种恰恰是三种里最容易碰到的（双击）。
///
/// 返回 `Result` 而不是 `bool`：前端全走 `run()` 那一条错误路径，命令级失败自动落成
/// 一条 toast，不必为它新写分支。
#[tauri::command]
pub fn remove_task(state: State<'_, AppState>, id: u64) -> R<()> {
    if state.engine.remove_task(id) {
        Ok(())
    } else {
        Err("无法移除：它可能仍在运行或收尾，也可能已不在列表里".to_string())
    }
}

#[tauri::command]
pub fn set_filename(state: State<'_, AppState>, id: u64, name: String) {
    state.engine.set_filename(id, name);
}

/// 界面在「重同步后看到一个处于 `AwaitingChoice`、但本地没有候选副本」的任务时调用它。
///
/// 读的是候选字段本身、**不看状态**（见 `Engine::pending_choice` 的文档）：调用方必须
/// 自行按状态过滤，别假定非空就意味着任务还在等选择。
#[tauri::command]
pub fn pending_choice(state: State<'_, AppState>, id: u64) -> Option<ChoiceKindDto> {
    state.engine.pending_choice(id).map(ChoiceKindDto::from)
}

#[tauri::command]
pub fn set_choice(state: State<'_, AppState>, id: u64, choice: ChoiceDto) {
    state.engine.set_choice(id, choice.into());
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsDto {
    settings_dto(state.settings.lock().unwrap().clone())
}

#[tauri::command]
pub fn update_settings(state: State<'_, AppState>, settings: SettingsDto) -> R<()> {
    let s = settings_from(settings);
    s.save().map_err(|e| format!("保存设置失败：{e:#}"))?;
    state.apply_settings(s);
    Ok(())
}

/// 目录选择走系统对话框。
///
/// **用的是回调式 `pick_folder`，不是 `blocking_pick_folder`。** 本仓库依赖的
/// `tauri-plugin-dialog` 2.8.3 同时提供两者：`blocking_pick_folder` 会阻塞调用线程
/// （在 `#[tauri::command] async fn` 里也就是阻塞一个 tokio 工作线程），而 `pick_folder`
/// 内部经 `run_on_main_thread` 把对话框派发到主线程、并从另一个线程回调，**不阻塞**任何
/// 工作线程。所以这里用回调式 + `oneshot` 把它接成 `await`。
///
/// （无论哪种，都不会像 egui 版那样在对话框打开期间阻塞事件循环、丢掉引擎事件。）
#[tauri::command]
pub async fn pick_download_dir(app: tauri::AppHandle) -> R<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |picked| {
        // 接收端可能已经因为别的原因不在了；发不出去不是错误。
        let _ = tx.send(picked);
    });
    let picked = rx
        .await
        .map_err(|e| format!("目录选择被中断：{e}"))?;
    Ok(picked.map(|p| p.to_string()))
}
