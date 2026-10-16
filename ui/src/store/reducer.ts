import type {
  ChoiceKindDto,
  EventDto,
  JobView,
  SettingsDto,
  TaskView,
  VideoOpEventDto,
} from "../lib/types";

export interface UiState {
  tasks: TaskView[];
  /** 视频操作任务。与 tasks 同住一个 store，因为 RESYNC 是两条通道共用的：
   *  任一条落后都整体重取，分成两个 store 反而要维护两条重同步路径。 */
  jobs: JobView[];
  settings: SettingsDto | null;
  /** 收到过无法对应到已知任务的 id。为 true 时由 store 去拉一次全量。 */
  needsResync: boolean;
  /** 上一次重同步丢了多少条（用于状态栏提示；0 表示没丢过）。 */
  lastDropped: number;
  /** 被「稍后再说」压下去、暂时不再弹选择框的任务 id。任务一旦离开 awaitingChoice 就会被放掉。 */
  dismissedChoices: number[];
  /** 命令级失败的提示，逐条可关。 */
  toasts: string[];
}

export const initialState: UiState = {
  tasks: [],
  jobs: [],
  settings: null,
  needsResync: false,
  lastDropped: 0,
  dismissedChoices: [],
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
  | { kind: "hydrateJobs"; jobs: JobView[] }
  | { kind: "videoEvent"; event: VideoOpEventDto }
  | { kind: "event"; event: EventDto; now?: number } // now 便于测试，默认 Date.now()
  | { kind: "choices"; id: number; choices: ChoiceKindDto }
  | { kind: "settings"; settings: SettingsDto }
  | { kind: "resyncRequested"; dropped: number }
  | { kind: "editFilename"; id: number; filename: string }
  | { kind: "setEditing"; id: number; editing: boolean }
  | { kind: "dismissChoice"; id: number }
  | { kind: "openChoice"; id: number }
  | { kind: "pushToast"; text: string }
  | { kind: "dismissToast"; index: number };

/**
 * 只留下"仍然停在等待选择"的 id。任务一旦离开 awaitingChoice（或从列表里彻底消失），
 * 就把它从抑制名单里放掉 —— 否则一个任务以后重新进入该状态时，会被"稍后再说"的旧记录
 * 无声地永久压住，用户再也看不到它的选择框。
 *
 * 在 `event`（含 stateChanged）与 `hydrate` 两条路径上都会调用：
 * - 事件路径：任务状态一变就即时放掉，见 applyEvent。
 * - hydrate 路径：重同步整体替换任务列表，此时按 core 报来的新状态重新判定 ——
 *   仍然 awaitingChoice 的保住记录（重同步不该让"稍后再说"失效、把模态框又弹回来），
 *   已经离开该状态的（哪怕中途的事件被丢过）也一并放掉。
 */
function pruneDismissed(dismissed: number[], tasks: TaskView[]): number[] {
  if (dismissed.length === 0) return dismissed;
  const waiting = new Set<number>();
  for (const t of tasks) if (t.state.kind === "awaitingChoice") waiting.add(t.id);
  const kept = dismissed.filter((id) => waiting.has(id));
  return kept.length === dismissed.length ? dismissed : kept;
}

/**
 * 当前该弹哪个任务的选择框：取 id 最小、且**没有被「稍后再说」压住**的那个。
 * id 由 core 单调分配，等价于"最早排队的先答"，与数组顺序、对象身份都无关。
 * 全被压住（或根本没有等待中的任务）时返回 null —— 此时 ChoiceDialog 不渲染任何东西。
 */
export function selectPendingChoice(tasks: TaskView[], dismissed: number[]): TaskView | null {
  let best: TaskView | null = null;
  for (const t of tasks) {
    if (t.state.kind !== "awaitingChoice" || dismissed.includes(t.id)) continue;
    if (best === null || t.id < best.id) best = t;
  }
  return best;
}

export function reduce(state: UiState, action: Action): UiState {
  switch (action.kind) {
    case "hydrate": {
      // 整体替换，不做合并：core 是唯一真相源。
      // 本地那些只属于界面的字段（choices / editing）尽量保留，
      // 否则重同步会把用户正在输入的框清空。
      const tasks = action.tasks.map((fresh) => {
        const old = state.tasks.find((t) => t.id === fresh.id);
        return {
          ...fresh,
          choices: old?.choices ?? null,
          editing: old?.editing ?? false,
          filename: old?.editing ? old.filename : fresh.filename,
        };
      });
      return {
        ...state,
        tasks,
        dismissedChoices: pruneDismissed(state.dismissedChoices, tasks),
        needsResync: false,
      };
    }

    case "hydrateJobs": {
      // 整体替换，不做合并：core 是唯一真相源。
      // 进度也在里面 —— 视频任务的进度本来就是全局量（core 按 ffmpeg 自己的节奏报），
      // 不像下载那边那样是"两次事件之间的差"，所以不需要在重同步时归零。
      return { ...state, jobs: action.jobs, needsResync: false };
    }
    case "videoEvent": {
      const { event } = action;
      if (event.type === "added") {
        // 事件里只有 id，没有可显示的内容。不在这里编一行出来 —— 编出来的一行
        // 在随后的 hydrate 里还要被替换掉，等于凭空多一个中间态。
        return { ...state, needsResync: true };
      }
      if (event.type === "removed") {
        const exists = state.jobs.some((j) => j.id === event.id);
        if (!exists) return { ...state, needsResync: true };
        return { ...state, jobs: state.jobs.filter((j) => j.id !== event.id), needsResync: false };
      }
      const known = state.jobs.some((j) => j.id === event.id);
      if (!known) {
        // 与下载事件同义：可能事件先于水合到达，记一笔去拉全量。
        return { ...state, needsResync: true };
      }
      const jobs = state.jobs.map((j): JobView => {
        if (j.id !== event.id) return j;
        if (event.type === "progress") return { ...j, progress: event.fraction };
        return { ...j, state: event.state };
      });
      return { ...state, jobs, needsResync: false };
    }
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

    case "dismissChoice":
      // 只记一笔"先不弹"，**不碰任务本身** —— 「稍后再说」绝不能取消下载。
      return state.dismissedChoices.includes(action.id)
        ? state
        : { ...state, dismissedChoices: [...state.dismissedChoices, action.id] };

    case "openChoice":
      return {
        ...state,
        dismissedChoices: state.dismissedChoices.filter((id) => id !== action.id),
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
  // 移除单独处理，**不是**因为它必须抢在 `known` 检查之前 —— 真放到后面去，
  // 未知 id 也只会走那条 `!known` 早返回，置出完全相同的 `needsResync: true`。
  // 真正的理由是结构：移除不是一次"更新"，套不进下面那台"按 id 找到那一行、
  // 就地改字段"的机器 —— 它改的是列表本身的长度。
  if (event.type === "taskRemoved") {
    const exists = state.tasks.some((t) => t.id === event.id);
    if (!exists) {
      // 没这个 id：可能事件先于 hydrate 到达。与其它事件同义 —— 拉一次全量对齐。
      return { ...state, needsResync: true };
    }
    return {
      ...state,
      tasks: state.tasks.filter((t) => t.id !== event.id),
      // 顺手把"稍后再说"的抑制记录里那条也放掉。**这条是防御性的**：id 在一次会话内
      // 不会被复用（core 的 `next_id` 单调递增，`resume()` 用 `fetch_max`），而
      // dismissedChoices 本来就只活在内存里。留着它，只是让这份名单始终只装活着的 id。
      dismissedChoices: state.dismissedChoices.filter((id) => id !== event.id),
      needsResync: false,
    };
  }
  const known = state.tasks.some((t) => t.id === event.id);
  if (!known) {
    // 见 reducer.test.ts 里那条测试：不丢帧、不报错，记一笔去拉全量。
    return { ...state, needsResync: true };
  }
  const tasks: TaskView[] = state.tasks.map((t): TaskView => {
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
  });
  return {
    ...state,
    // 能落到已知行上，说明我们又收到了可应用的一帧：把"待重同步"清掉。
    // （未知 id 那条走上面的早返回，仍会把 needsResync 置 true —— 见 reducer.test.ts。）
    // 注意：清它不会取消已经在途的那次 hydrate —— store 的 dispatch 里
    // scheduleResyncIfNeeded 与置位是**同一个同步 tick**，置位后 hydrate 已经开始。
    needsResync: false,
    tasks,
    // 状态一变就顺手放掉"离开等待选择"的抑制记录（stateChanged 落到这里）。
    dismissedChoices: pruneDismissed(state.dismissedChoices, tasks),
  };
}

function progressFields(
  t: TaskView,
  event: Extract<EventDto, { type: "progress" }>,
  now: number
): Pick<
  TaskView,
  "downloaded" | "total" | "unit" | "bytesDownloaded" | "speedBps" | "etaText" | "lastTickAt"
> {
  const dt = (now - t.lastTickAt) / 1000;
  // 差分走**字节数**，不走 `downloaded`：后者对分片任务是"片"，除出来的单位是片/秒。
  // 引擎为两种单位都报 bytesDownloaded，所以这里不再需要按 unit 分支
  // （旧写法 `unit === "bytes" && ...` 正是"m3u8 恒显示 0 B/s"的成因）。
  const dd = event.bytesDownloaded - t.bytesDownloaded;
  // dt 太小（同一批事件）时沿用上一次的值，避免除出天文数字。
  const speedBps = t.lastTickAt > 0 && dt > 0.2 && dd >= 0 ? dd / dt : t.speedBps;
  // 剩余时间仍只对字节单位有效：分片任务的 `total` 是片数，除以字节/秒毫无意义。
  const remaining = Math.max(0, event.total - event.downloaded);
  const etaText =
    t.unit === "bytes" && speedBps && speedBps > 0 && remaining > 0
      ? `剩 ${humanizeSeconds(remaining / speedBps)}`
      : null;
  return {
    downloaded: event.downloaded,
    total: event.total,
    unit: event.unit,
    bytesDownloaded: event.bytesDownloaded,
    speedBps,
    etaText,
    lastTickAt: now,
  };
}
