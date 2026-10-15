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
function scheduleResyncIfNeeded() {
  if (!state.needsResync || resyncInFlight) return;
  resyncInFlight = true;
  // 不在这里清 needsResync：hydrate 会清。这样连续来几个未知 id 也只拉一次。
  void hydrate().finally(() => {
    resyncInFlight = false;
  });
}

export async function hydrate() {
  const dtos = await ipc.getTasks();
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
      const c = await ipc.pendingChoice(t.id);
      if (c) dispatch({ kind: "choices", id: t.id, choices: c });
    }
  }
}

export async function loadSettings() {
  dispatch({ kind: "settings", settings: await ipc.getSettings() });
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
  await ipc.onEngineEvent((event) => dispatch({ kind: "event", event }));
  await ipc.onResync((dropped) => dispatch({ kind: "resyncRequested", dropped }));
  await hydrate();
}

export function useTaskList(): TaskView[] {
  return useSyncExternalStore(store.subscribe, store.getSnapshot).tasks;
}

export function useSettings() {
  return useSyncExternalStore(store.subscribe, store.getSnapshot).settings;
}
