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
