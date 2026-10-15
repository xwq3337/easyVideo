import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ChoiceDto, ChoiceKindDto, EventDto, SettingsDto, TaskDto } from "./types";

// 命令名与 src-tauri/src/main.rs 的 invoke_handler![...] 逐字对应；
// 参数名与 commands.rs 里各命令的形参逐字对应。两边都靠人盯着（没有测试钉得住），
// 写错只会表现为运行时的 "command not found" / 参数缺失。
export const getTasks = () => invoke<TaskDto[]>("get_tasks");
export const addUrl = (url: string) => invoke<number>("add_url", { url });
export const pause = (id: number) => invoke<void>("pause", { id });
export const unpause = (id: number) => invoke<void>("unpause", { id });
export const retry = (id: number) => invoke<void>("retry", { id });
export const cancel = (id: number) => invoke<void>("cancel", { id });
export const setFilename = (id: number, name: string) => invoke<void>("set_filename", { id, name });
export const pendingChoice = (id: number) => invoke<ChoiceKindDto | null>("pending_choice", { id });
export const setChoice = (id: number, choice: ChoiceDto) => invoke<void>("set_choice", { id, choice });
export const getSettings = () => invoke<SettingsDto>("get_settings");
export const updateSettings = (settings: SettingsDto) => invoke<void>("update_settings", { settings });
export const pickDownloadDir = () => invoke<string | null>("pick_download_dir");

export const onEngineEvent = (cb: (e: EventDto) => void) =>
  listen<EventDto>("engine://event", (ev) => cb(ev.payload));

/** Rust 侧发的是 `{"dropped": n}` 这个**对象**（见 bridge.rs 的 `json!({ "dropped": n })`），
 *  不是裸数字 —— 类型必须与那边逐字对应。 */
export const onResync = (cb: (dropped: number) => void) =>
  listen<{ dropped: number }>("engine://resync", (ev) => cb(ev.payload.dropped));
