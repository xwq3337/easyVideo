import { useTaskList } from "../store/store";
import { formatSpeed } from "../lib/format";

/**
 * 总速度对所有任务求和。**不再按 unit 过滤**：分片任务的速度现在取自引擎报的字节数
 * （`bytesDownloaded`），和直链一样是字节/秒，可以安全相加。旧写法只累加 `unit === "bytes"`
 * 的行，于是只要列表里全是 m3u8，这里就永远是 0 B/s。
 */
export default function StatusBar({ version }: { version: string }) {
  const tasks = useTaskList();
  const active = tasks.filter((t) =>
    ["probing", "downloading", "merging"].includes(t.state.kind)
  ).length;
  const bytesPerSec = tasks.reduce((sum, t) => sum + (t.speedBps ?? 0), 0);

  // 从底部的通栏挪到侧边栏底部：总速度与版本号是全局信息，但底下那条横栏
  // 在有了侧边栏之后就是白占一行高度。
  return (
    <div className="px-4 py-2.5 tnum text-[11px]" style={{ color: "var(--fg-muted)" }}>
      <div>{formatSpeed(bytesPerSec)} · {active} 个下载中</div>
      <div style={{ color: "var(--fg-faint)" }}>{version}</div>
    </div>
  );
}
