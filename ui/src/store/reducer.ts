import type { ChoiceKindDto, EventDto, SettingsDto, TaskView } from "../lib/types";

export interface UiState {
  tasks: TaskView[];
  settings: SettingsDto | null;
  /** 收到过无法对应到已知任务的 id。为 true 时由 store 去拉一次全量。 */
  needsResync: boolean;
  /** 上一次重同步丢了多少条（用于状态栏提示；0 表示没丢过）。 */
  lastDropped: number;
  /** 命令级失败的提示，逐条可关。 */
  toasts: string[];
}

export const initialState: UiState = {
  tasks: [],
  settings: null,
  needsResync: false,
  lastDropped: 0,
  toasts: [],
};

/** 把秒数说成人话。速度算不出来时调用方应当直接不显示，而不是显示 "剩 0 秒"。 */
export function humanizeSeconds(s: number): string {
  const total = Math.max(0, Math.round(s));
  if (total < 60) return `${total} 秒`;
  if (total < 3600) {
    const m = Math.floor(total / 60);
    const sec = total % 60;
    return sec === 0 ? `${m} 分` : `${m} 分 ${sec} 秒`;
  }
  let h = Math.floor(total / 3600);
  let m = Math.round((total % 3600) / 60);
  // 分钟四舍五入后可能正好是 60（例如 7175 秒 = 1 小时 59.5 分），要进位到小时，
  // 否则会打印出 "1 小时 60 分" —— 人话里没有这种说法。
  if (m === 60) {
    m = 0;
    h += 1;
  }
  return m === 0 ? `${h} 小时` : `${h} 小时 ${m} 分`;
}

export type Action =
  | { kind: "hydrate"; tasks: TaskView[] }
  | { kind: "event"; event: EventDto; now?: number } // now 便于测试，默认 Date.now()
  | { kind: "choices"; id: number; choices: ChoiceKindDto }
  | { kind: "settings"; settings: SettingsDto }
  | { kind: "resyncRequested"; dropped: number }
  | { kind: "editFilename"; id: number; filename: string }
  | { kind: "setEditing"; id: number; editing: boolean }
  | { kind: "pushToast"; text: string }
  | { kind: "dismissToast"; index: number };

export function reduce(state: UiState, action: Action): UiState {
  switch (action.kind) {
    case "hydrate":
      // 整体替换，不做合并：core 是唯一真相源。
      // 本地那些只属于界面的字段（choices / editing）尽量保留，
      // 否则重同步会把用户正在输入的框清空。
      return {
        ...state,
        tasks: action.tasks.map((fresh) => {
          const old = state.tasks.find((t) => t.id === fresh.id);
          return {
            ...fresh,
            choices: old?.choices ?? null,
            editing: old?.editing ?? false,
            filename: old?.editing ? old.filename : fresh.filename,
          };
        }),
        needsResync: false,
      };

    case "resyncRequested":
      return { ...state, needsResync: true, lastDropped: action.dropped };

    case "settings":
      return { ...state, settings: action.settings };

    case "choices":
      return {
        ...state,
        tasks: state.tasks.map((t) => (t.id === action.id ? { ...t, choices: action.choices } : t)),
      };

    case "editFilename":
      return {
        ...state,
        tasks: state.tasks.map((t) => (t.id === action.id ? { ...t, filename: action.filename } : t)),
      };

    case "setEditing":
      return {
        ...state,
        tasks: state.tasks.map((t) => (t.id === action.id ? { ...t, editing: action.editing } : t)),
      };

    case "pushToast":
      return { ...state, toasts: [...state.toasts, action.text] };

    case "dismissToast":
      return { ...state, toasts: state.toasts.filter((_, i) => i !== action.index) };

    case "event":
      return applyEvent(state, action.event, action.now ?? Date.now());
  }
}

function applyEvent(state: UiState, event: EventDto, now: number): UiState {
  const known = state.tasks.some((t) => t.id === event.id);
  if (!known) {
    // 见 reducer.test.ts 里那条测试：不丢帧、不报错，记一笔去拉全量。
    return { ...state, needsResync: true };
  }
  return {
    ...state,
    // 能落到已知行上，说明我们又收到了可应用的一帧：把"待重同步"清掉。
    // （未知 id 那条走上面的早返回，仍会把 needsResync 置 true —— 见 reducer.test.ts。）
    // 注意：清它不会取消已经在途的那次 hydrate —— store 的 dispatch 里
    // scheduleResyncIfNeeded 与置位是**同一个同步 tick**，置位后 hydrate 已经开始。
    needsResync: false,
    tasks: state.tasks.map((t) => {
      if (t.id !== event.id) return t;
      switch (event.type) {
        case "progress":
          return { ...t, ...progressFields(t, event, now) };
        case "stateChanged":
          // 离开下载态就把速度清掉，否则"已完成"那一行还挂着一个最后算出来的速度
          return event.state.kind === "downloading"
            ? { ...t, state: event.state }
            : { ...t, state: event.state, speedBps: null, etaText: null };
        case "choicesOffered":
          return { ...t, state: { kind: "awaitingChoice" }, choices: event.kind };
      }
    }),
  };
}

function progressFields(
  t: TaskView,
  event: Extract<EventDto, { type: "progress" }>,
  now: number
): Pick<TaskView, "downloaded" | "total" | "unit" | "speedBps" | "etaText" | "lastTickAt"> {
  const dt = (now - t.lastTickAt) / 1000;
  const dd = event.downloaded - t.downloaded;
  // 分片单位不报速度：那是"片/秒"，混进总速度会得到 "3.0 B/s" 这种假数字。
  // dt 太小（同一批事件）时沿用上一次的值，避免除出天文数字。
  const speedBps =
    t.unit === "bytes" && t.lastTickAt > 0 && dt > 0.2 && dd >= 0 ? dd / dt : t.speedBps;
  const remaining = Math.max(0, event.total - event.downloaded);
  const etaText =
    t.unit === "bytes" && speedBps && speedBps > 0 && remaining > 0
      ? `剩 ${humanizeSeconds(remaining / speedBps)}`
      : null;
  return {
    downloaded: event.downloaded,
    total: event.total,
    unit: event.unit,
    speedBps,
    etaText,
    lastTickAt: now,
  };
}
