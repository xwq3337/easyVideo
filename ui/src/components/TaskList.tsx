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
    // 列表这块靠背景色差（--bg-subtle）与页面底色区分，不画外框；行与行之间仍保留发丝线 ——
    // 列表需要它来对齐视线。**这只说列表**：侧边栏的上下两块之间仍然是 borderRight /
    // borderTop 的实线，不受这句话约束。
    <div
      // 末行自己那条 borderBottom 正好压在容器的底边上，会被 rounded-xl + overflow-hidden
      // 的两个圆角切掉两端，看着像断了一截。去掉它，让列表以圆角收口。
      //
      // 末尾那个 `!` 不是多余：TaskRow 的边框写在**行内 style** 上，而行内样式
      // 压得过任何普通 class —— 没有 `!` 的话这条规则会被静默吃掉，看着就像没写。
      className="rounded-xl overflow-hidden [&>div:last-child]:border-b-0!"
      style={{ background: "var(--bg-subtle)" }}
    >
      {tasks.map((t) => (
        <TaskRow key={t.id} task={t} />
      ))}
    </div>
  );
}
