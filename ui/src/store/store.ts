import { useSyncExternalStore } from "react";
import type { TaskView } from "../lib/types";
import * as ipc from "../lib/ipc";
import { initialState, reduce, type Action, type UiState } from "./reducer";

let state: UiState = initialState;
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

export const store = {
  subscribe(l: () => void) {
    listeners.add(l);
    return () => listeners.delete(l);
  },
  getSnapshot() {
    return state;
  },
};

export function dispatch(action: Action) {
  const next = reduce(state, action);
  if (next !== state) {
    state = next;
    emit();
    scheduleResyncIfNeeded();
  }
}

let resyncInFlight = false;

/**
 * 拉全量（get_tasks / get_settings）失败后是否已经报过一次。
 *
 * 为什么要这个闩：未知 id 的事件是**每个进度 tick 都来一条**的。若每次失败都各报一条
 * toast、各重试一次，用户会被同一件坏消息刷屏，而且每个 tick 都白打一次必然失败的 IPC。
 * 于是规定：一次失败只报一次、只重试一次；直到下一次拉取**成功**才解锁。
 */
let reloadFailureReported = false;

function reportReloadFailure(e: unknown) {
  if (reloadFailureReported) return;
  reloadFailureReported = true;
  dispatch({ kind: "pushToast", text: typeof e === "string" ? e : String(e) });
}

function scheduleResyncIfNeeded() {
  // 上次就失败了：别再每个 tick 都重试 + 报错。needsResync 留着，等下次成功后再恢复调度。
  if (!state.needsResync || resyncInFlight || reloadFailureReported) return;
  resyncInFlight = true;
  // 不在这里清 needsResync：hydrate 会清。这样连续来几个未知 id 也只拉一次。
  // hydrate 自带 catch，永远不会 reject，所以这里没有未处理的 rejection。
  void hydrate().finally(() => {
    resyncInFlight = false;
  });
}

/**
 * 拉一次全量任务。**自带显式 catch**：失败时只报一次 toast 就收手，绝不向外抛 ——
 * 每个调用点（每个 tick 的重同步、startBridge、addUrl）都不必再接 rejection。
 * 见 item 2 的说明与 final-fix-wave 报告。
 */
export async function hydrate() {
  let dtos: Awaited<ReturnType<typeof ipc.getTasks>>;
  try {
    dtos = await ipc.getTasks();
  } catch (e) {
    reportReloadFailure(e);
    return;
  }
  reloadFailureReported = false;
  dispatch({
    kind: "hydrate",
    // 速度/剩余时间是"两次事件之间"的量，全量刷新时无从谈起 —— 一律归零，
    // 否则界面上会挂着上一轮算出来的数字。
    tasks: dtos.map((d) => ({
      ...d,
      choices: null,
      editing: false,
      speedBps: null,
      etaText: null,
      lastTickAt: 0,
    })),
  });
  // 重同步之后，处于"等待选择"却没有候选副本的行，逐个去问 core 要。
  // 这一步就是"丢掉 ChoicesOffered 之后模态框还能弹出来"的唯一保证。
  for (const t of state.tasks) {
    if (t.state.kind === "awaitingChoice" && t.choices === null) {
      // 单条补齐失败只落一条 toast（run()），不带崩整次重同步。
      const c = await run(ipc.pendingChoice(t.id));
      if (c) dispatch({ kind: "choices", id: t.id, choices: c });
    }
  }
}

/** 拉一次设置。与 hydrate 同理：自带 catch，失败只报一次 toast，不向外抛。 */
export async function loadSettings() {
  try {
    const settings = await ipc.getSettings();
    reloadFailureReported = false;
    dispatch({ kind: "settings", settings });
  } catch (e) {
    reportReloadFailure(e);
  }
}

/**
 * 所有命令调用的统一入口。**命令级失败不打断操作**：列一张 toast 就好，
 * 不要把界面卡住或者弹一个必须先关掉的对话框。
 * 用法：`void run(ipc.pause(id))`。
 */
export async function run<T>(p: Promise<T>): Promise<T | undefined> {
  try {
    return await p;
  } catch (e) {
    dispatch({ kind: "pushToast", text: typeof e === "string" ? e : String(e) });
    return undefined;
  }
}

export function dismissToast(index: number) {
  dispatch({ kind: "dismissToast", index });
}

export function useToasts(): string[] {
  return useSyncExternalStore(store.subscribe, store.getSnapshot).toasts;
}

/** 由 <App> 在挂载时调用一次。 */
export async function startBridge() {
  // startBridge 是 `void startBridge()` 调起来的，任何漏出的 rejection 都没人接。
  // 监听注册失败（webview 权限之类）也只报一条 toast；hydrate 自带 catch。
  await run(ipc.onEngineEvent((event) => dispatch({ kind: "event", event })));
  await run(ipc.onResync((dropped) => dispatch({ kind: "resyncRequested", dropped })));
  await hydrate();
}

export function useTaskList(): TaskView[] {
  return useSyncExternalStore(store.subscribe, store.getSnapshot).tasks;
}

/** 被「稍后再说」压住、暂不弹选择框的任务 id。 */
export function useDismissedChoices(): number[] {
  return useSyncExternalStore(store.subscribe, store.getSnapshot).dismissedChoices;
}

export function useSettings() {
  return useSyncExternalStore(store.subscribe, store.getSnapshot).settings;
}
