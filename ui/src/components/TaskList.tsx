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
    <div>
      {tasks.map((t) => (
        <TaskRow key={t.id} task={t} />
      ))}
    </div>
  );
}
