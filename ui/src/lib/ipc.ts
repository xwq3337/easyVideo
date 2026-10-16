import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ChoiceDto,
  ChoiceKindDto,
  EventDto,
  JobDto,
  JobParamsDto,
  MergeCheckDto,
  SettingsDto,
  TaskDto,
  VideoInfoDto,
  VideoOpEventDto,
} from "./types";

// 命令名与 src-tauri/src/main.rs 的 invoke_handler![...] 逐字对应；
// 参数名与 commands.rs 里各命令的形参对应。两边都靠人盯着（没有测试钉得住），
// 写错只会表现为运行时的 "command not found" / 参数缺失。
//
// **多词形参要写成 camelCase，不是逐字照抄。** `#[tauri::command]` 默认
// `argument_case: ArgumentCase::Camel`（tauri-macros 的 wrapper.rs:51），所以 Rust 的
// `output_dir` / `audio_kbps` 在 JS 侧是 `outputDir` / `audioKbps`。
// 照抄成 snake_case 会在运行时得到 "missing required key"。
export const getTasks = () => invoke<TaskDto[]>("get_tasks");
export const addUrl = (url: string) => invoke<number>("add_url", { url });
export const pause = (id: number) => invoke<void>("pause", { id });
export const unpause = (id: number) => invoke<void>("unpause", { id });
export const retry = (id: number) => invoke<void>("retry", { id });
export const cancel = (id: number) => invoke<void>("cancel", { id });
export const removeTask = (id: number) => invoke<void>("remove_task", { id });
export const setFilename = (id: number, name: string) => invoke<void>("set_filename", { id, name });
export const pendingChoice = (id: number) => invoke<ChoiceKindDto | null>("pending_choice", { id });
export const setChoice = (id: number, choice: ChoiceDto) => invoke<void>("set_choice", { id, choice });
export const getSettings = () => invoke<SettingsDto>("get_settings");
export const updateSettings = (settings: SettingsDto) => invoke<void>("update_settings", { settings });
export const pickDownloadDir = () => invoke<string | null>("pick_download_dir");

// ===== 视频操作 =====

export const getVideoJobs = () => invoke<JobDto[]>("get_video_jobs");

export const addVideoJob = (inputs: string[], outputDir: string, params: JobParamsDto) =>
  invoke<number>("add_video_job", { inputs, outputDir, params });

/** 只算产物名给界面预览。**与 addVideoJob 走 core 里同一个取名函数** —— 见 core 的说明。 */
export const planOutput = (inputs: string[], outputDir: string, params: JobParamsDto) =>
  invoke<string>("plan_output", { inputs, outputDir, params });

export const cancelVideoJob = (id: number) => invoke<void>("cancel_video_job", { id });
export const removeVideoJob = (id: number) => invoke<void>("remove_video_job", { id });
export const pickVideoFile = () => invoke<string | null>("pick_video_file");
/** 多选（合并用）。选中的会**追加**到现有列表 —— 用户常分几次挑。 */
export const pickVideoFiles = () => invoke<string[]>("pick_video_files");
export const probeVideo = (path: string) => invoke<VideoInfoDto>("probe_video", { path });

/** 合并的兼容性检查。规则只在 core 一处，前端只负责把结果画出来。 */
export const checkMerge = (inputs: string[]) =>
  invoke<MergeCheckDto>("check_merge", { inputs });

/** 抽一帧做裁剪预览，返回可直接塞进 `<img src>` 的 data URL。 */
export const previewFrame = (path: string, atSecs: number) =>
  invoke<string>("preview_frame", { path, atSecs });

/** 视频任务的事件走**自己的通道名**，所以前端按事件名路由，不必嗅探联合类型的 tag。 */
export const onVideoOpEvent = (cb: (e: VideoOpEventDto) => void) =>
  listen<VideoOpEventDto>("videoops://event", (ev) => cb(ev.payload));

export const onEngineEvent = (cb: (e: EventDto) => void) =>
  listen<EventDto>("engine://event", (ev) => cb(ev.payload));

/** Rust 侧发的是 `{"dropped": n}` 这个**对象**（见 bridge.rs 的 `json!({ "dropped": n })`），
 *  不是裸数字 —— 类型必须与那边逐字对应。 */
export const onResync = (cb: (dropped: number) => void) =>
  listen<{ dropped: number }>("engine://resync", (ev) => cb(ev.payload.dropped));
