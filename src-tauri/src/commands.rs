//! 前端能调用的全部命令（**不写条数**——加一条命令就要回来改一次注释,
//! 漏掉的注释比没有注释更坏）。
//!
//! **全部是薄封装**——业务逻辑一律在 core 里，这里只做「取 `State`、调引擎、转 DTO」。
//! 所以本层没有单测（`cargo check --workspace` 加 Task 12 的手动验收覆盖它），
//! 与规格 §7 的测试策略一致。

use std::path::PathBuf;

use easydownvideo::core::videoops::params::JobParams;
use tauri::State;

use crate::dto::{
    job_dto, settings_dto, settings_from, task_dto, ChoiceDto, ChoiceKindDto, JobDto, JobParamsDto,
    SettingsDto, TaskDto, VideoInfoDto,
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

// ===== 视频操作 =====
//
// 与上面那些一样是薄封装：业务逻辑全在 core 的 videoops 里，这里只做
// 「取 State、调它、转 DTO」。

#[tauri::command]
pub fn get_video_jobs(state: State<'_, AppState>) -> Vec<JobDto> {
    state.videoops.list_jobs().into_iter().map(job_dto).collect()
}

/// 入队一次视频操作。**一条命令收一个带 tag 的枚举** —— 见 `JobParamsDto` 的说明。
#[tauri::command]
pub async fn add_video_job(
    state: State<'_, AppState>,
    input: String,
    output_dir: String,
    params: JobParamsDto,
) -> R<u64> {
    let params = JobParams::try_from(params)?; // 错误已经是给人看的一句话
    // 先把 VideoOps 克隆出来再 await：State 是个借用，跨 await 持有它会把借用
    // 连带进 future，而 Tauri 要求 async 命令返回 Send future。内部是 Arc，克隆很便宜。
    let ops = state.videoops.clone();
    ops.add_job(vec![PathBuf::from(input)], PathBuf::from(output_dir), params)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// 只算产物名给界面预览。**与 `add_video_job` 走 core 里同一个取名函数** ——
/// 两处各写一遍的话，预览与实际落盘会在某个边角上分叉。
#[tauri::command]
pub async fn plan_output(
    state: State<'_, AppState>,
    input: String,
    output_dir: String,
    params: JobParamsDto,
) -> R<String> {
    let params = JobParams::try_from(params)?; // 错误已经是给人看的一句话
    let ops = state.videoops.clone();
    ops.plan_output(vec![PathBuf::from(input)], PathBuf::from(output_dir), params)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn cancel_video_job(state: State<'_, AppState>, id: u64) {
    state.videoops.cancel(id);
}

/// 与 `remove_task` 同样返回 `Result` 而不是 `bool`：前端全走 `run()` 那一条错误路径。
#[tauri::command]
pub fn remove_video_job(state: State<'_, AppState>, id: u64) -> R<()> {
    if state.videoops.remove(id) {
        Ok(())
    } else {
        Err("无法移除：它可能仍在运行，也可能已不在列表里".to_string())
    }
}

/// 选一个视频文件。与 `pick_download_dir` 同理走回调式 API，不阻塞工作线程。
#[tauri::command]
pub async fn pick_video_file(app: tauri::AppHandle) -> R<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("视频", &["mp4", "mov", "mkv", "webm", "avi", "m4v", "ts"])
        .pick_file(move |picked| {
            let _ = tx.send(picked);
        });
    let picked = rx.await.map_err(|e| format!("文件选择被中断：{e}"))?;
    Ok(picked.map(|p| p.to_string()))
}

#[tauri::command]
pub async fn probe_video(state: State<'_, AppState>, path: String) -> R<VideoInfoDto> {
    let p = PathBuf::from(path);
    // 文件大小在这里取：`probe` 只负责"ffmpeg 读得懂的那部分"，而大小是纯文件系统的事。
    // 也顺手把"文件根本读不到"变成一条清楚的错误，而不是一个 0 字节的结果。
    let size_bytes = std::fs::metadata(&p)
        .map_err(|e| format!("读不到文件 {}：{e}", p.display()))?
        .len();
    let ops = state.videoops.clone();
    let info = ops.probe(p).await.map_err(|e| format!("{e:#}"))?;
    Ok(VideoInfoDto {
        duration_secs: info.duration_secs,
        has_video: info.has_video,
        video_codec: info.video_codec,
        container: info.container,
        size_bytes,
    })
}
