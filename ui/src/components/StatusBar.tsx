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
  // speedBps 由 reducer 在每次进度事件里算好（Task 7）。分片单位的任务它为 null，
  // 所以这里天然不会被"片/秒"污染。

  return (
    <footer
      className="flex items-center justify-between px-3 py-1.5 border-t tnum"
      style={{ borderColor: "var(--line)", color: "var(--fg-muted)", background: "var(--bg-subtle)" }}
    >
      <span>总速度 {formatSpeed(bytesPerSec)} · {active} 个下载中</span>
      <span>{version}</span>
    </footer>
  );
}
