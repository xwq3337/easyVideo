import { useTaskList } from "../store/store";
import { formatSpeed } from "../lib/format";

/** HLS 任务的"速度"单位是分片/秒，累加进总速度会得到"3.0 B/s"这种假数字 —— 只统计字节单位的。 */
export default function StatusBar({ version }: { version: string }) {
  const tasks = useTaskList();
  const active = tasks.filter((t) =>
    ["probing", "downloading", "merging"].includes(t.state.kind)
  ).length;
  const bytesPerSec = tasks
    .filter((t) => t.unit === "bytes")
    .reduce((sum, t) => sum + (t.speedBps ?? 0), 0);

  // 从底部的通栏挪到侧边栏底部：总速度与版本号是全局信息，但底下那条横栏
  // 在有了侧边栏之后就是白占一行高度。
  return (
    <div className="px-4 py-2.5 tnum text-[11px]" style={{ color: "var(--fg-muted)" }}>
      <div>{formatSpeed(bytesPerSec)} · {active} 个下载中</div>
      <div style={{ color: "var(--fg-faint)" }}>{version}</div>
    </div>
  );
}
