import type { StateDto } from "./types";
import * as ipc from "./ipc";

/** 一行右下角那个按钮该叫什么、该调什么。 */
type TaskAction = { label: string; invoke: (id: number) => Promise<void> };

/**
 * **逐状态列全**，写成 `Record<StateDto["kind"], …>` 而不是"终态名单 + 兜底 return"。
 *
 * 兜底版本的问题：core 日后加第十种状态时，`TERMINAL.includes(kind)` 为假，它会**无声地**
 * 落进「取消」分支 —— 恰好是这次要根除的那类假按钮（叫法与实际调用不一致），而编译器
 * 什么都不会说。换成 Record 之后，漏掉一种状态就是编译错误，逼着人来显式分派。
 *
 * 终态（core 不会再推进）归「从列表移除」，只把行移出列表、不删任何文件；
 * 其余归「取消」，它调的就是 `cancel()`，core 从不删用户文件。
 */
const ACTIONS: Record<StateDto["kind"], TaskAction> = {
  queued: { label: "取消", invoke: ipc.cancel },
  probing: { label: "取消", invoke: ipc.cancel },
  awaitingChoice: { label: "取消", invoke: ipc.cancel },
  downloading: { label: "取消", invoke: ipc.cancel },
  merging: { label: "取消", invoke: ipc.cancel },
  paused: { label: "取消", invoke: ipc.cancel },
  done: { label: "从列表移除", invoke: ipc.removeTask },
  failed: { label: "从列表移除", invoke: ipc.removeTask },
  canceled: { label: "从列表移除", invoke: ipc.removeTask },
};

/**
 * 一行右下角那个按钮该叫什么、该调什么。
 *
 * 抽成纯函数是为了能单测:组件层不进测试(见 spec §9),但"终态归移除、其余归取消"
 * 这条判断是**界面契约的一部分**,之前正是因为写错才出现了"点了没反应"的假按钮。
 */
export function taskAction(kind: StateDto["kind"]): TaskAction {
  return ACTIONS[kind];
}
