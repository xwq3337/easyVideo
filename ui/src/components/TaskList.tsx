import { useTaskList } from "../store/store";
import TaskRow from "./TaskRow";

export default function TaskList() {
  const tasks = useTaskList();
  if (tasks.length === 0) {
    return (
      <div
        className="h-full flex items-center justify-center"
        style={{ color: "var(--fg-faint)" }}
      >
        粘贴一个视频链接，回车开始
      </div>
    );
  }
  return (
    // 区块之间改用背景色差而不是分隔线。列表行之间仍保留发丝线 ——
    // 列表需要它来对齐视线，区块不需要。
    <div className="rounded-xl overflow-hidden" style={{ background: "var(--bg-subtle)" }}>
      {tasks.map((t) => (
        <TaskRow key={t.id} task={t} />
      ))}
    </div>
  );
}
