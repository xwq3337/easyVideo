import type { StateDto } from "./types";
import * as ipc from "./ipc";

/** 已经走到头、不会再产出下载的状态。对这些行,按钮只把它移出列表,不删任何文件。 */
const TERMINAL: ReadonlyArray<StateDto["kind"]> = ["done", "failed", "canceled"];

/**
 * 一行右下角那个按钮该叫什么、该调什么。
 *
 * 抽成纯函数是为了能单测:组件层不进测试(见 spec §9),但"终态归移除、其余归取消"
 * 这条判断是**界面契约的一部分**,之前正是因为写错才出现了"点了没反应"的假按钮。
 */
export function taskAction(kind: StateDto["kind"]): {
  label: string;
  invoke: (id: number) => Promise<void>;
} {
  if (TERMINAL.includes(kind)) {
    return { label: "从列表移除", invoke: ipc.removeTask };
  }
  // 非终态叫「取消」而不是「删除」:它调的就是 cancel(),而 core 从不删用户文件。
  return { label: "取消", invoke: ipc.cancel };
}
