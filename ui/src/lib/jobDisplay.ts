import type { JobStateDto } from "./types";

/**
 * 状态的中文。**穷尽所有分支且不留 `default`** —— 以后 core 加了新状态而这里漏了，
 * 编译就红，而不是渲染出一个空白的状态格（与 TaskRow 的 `LABELS` 同一个理由）。
 */
export function jobStateLabel(state: JobStateDto): string {
  switch (state.kind) {
    case "queued":
      return "排队中";
    case "running":
      return "处理中";
    case "done":
      return "已完成";
    case "canceled":
      return "已取消";
    // 失败原因直接带出来：这是用户唯一能看到的线索，藏进折叠区等于没有
    case "failed":
      return `失败：${state.message}`;
  }
}

/** `null` = 时长未知。界面显示不确定态，**绝不编一个 0%** —— 那看着像卡死。 */
export function jobPct(progress: number | null): number | null {
  if (progress === null) return null;
  return Math.round(Math.min(1, Math.max(0, progress)) * 100);
}
