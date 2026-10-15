import { useState } from "react";
import type { TaskView } from "../lib/types";
import { formatSize, formatSpeed } from "../lib/format";
import * as ipc from "../lib/ipc";
import { taskAction } from "../lib/taskAction";
import { dispatch, run } from "../store/store";

// 类型钉在 StateDto 的联合上：以后 core 加了新状态而这里漏了，**编译就红**，
// 而不是渲染出一个空白的状态格（Record<string, string> 是绝不会报错的）。
const LABELS: Record<TaskView["state"]["kind"], string> = {
  queued: "排队中",
  probing: "解析中",
  awaitingChoice: "等待选择",
  downloading: "下载中",
  merging: "正在合并…",
  paused: "已暂停",
  done: "已完成",
  failed: "失败",
  canceled: "已取消",
};

export default function TaskRow({ task }: { task: TaskView }) {
  const [draft, setDraft] = useState(task.filename);
  const pct =
    task.total > 0 ? Math.min(100, Math.round((task.downloaded / task.total) * 100)) : 0;
  // 只用来取色；中文取词统一走 LABELS。纯展示，不进 reducer。
  const labelColor =
    task.state.kind === "failed"
      ? "var(--accent-err)"
      : task.state.kind === "merging" || task.state.kind === "done"
        ? "var(--accent-ok)"
        : task.state.kind === "paused"
          ? "var(--accent-warn)"
          : "var(--fg-muted)";
  const action = taskAction(task.state.kind);

  async function commitName() {
    dispatch({ kind: "setEditing", id: task.id, editing: false });
    const name = draft.trim();
    if (!name || name === task.filename) {
      setDraft(task.filename);
      return;
    }
    // 走 run()：命令级失败只落一条 toast，而不是未处理的 rejection。
    // `set_filename` 返回 void，run 的返回值本身区分不出成败，所以把"成功"显式映射成 true；
    // 失败时不写本地行 —— core 才是唯一真相源，界面不该挂一个它并不认的名字。
    const ok = await run(ipc.setFilename(task.id, name).then(() => true));
    if (ok) {
      dispatch({ kind: "editFilename", id: task.id, filename: name });
    } else {
      setDraft(task.filename);
    }
  }

  return (
    <div className="px-3 py-2.5" style={{ borderBottom: "1px solid var(--line)" }}>
      <div className="flex items-baseline justify-between gap-3">
        <input
          value={task.editing ? draft : task.filename}
          readOnly={!task.editing}
          onChange={(e) => setDraft(e.target.value)}
          onFocus={() => {
            setDraft(task.filename);
            dispatch({ kind: "setEditing", id: task.id, editing: true });
          }}
          onBlur={() => void commitName()}
          onKeyDown={(e) => {
            if (e.key === "Enter") void commitName();
            if (e.key === "Escape") {
              setDraft(task.filename);
              dispatch({ kind: "setEditing", id: task.id, editing: false });
            }
          }}
          className="flex-1 min-w-0 bg-transparent outline-none font-medium"
          title="点一下可改文件名"
        />
        <span className="tnum shrink-0" style={{ color: "var(--fg-muted)" }}>
          {/* 只有下载中才谈得上「进度」；其余状态显示状态名 —— 否则「等待选择」会显示成 0%，
              看着像什么都没发生。百分比也只对字节单位有意义。 */}
          {task.state.kind === "downloading" ? (
            task.unit === "bytes" ? (
              `${pct}%`
            ) : (
              `${task.downloaded} / ${task.total} 片`
            )
          ) : (
            <span style={{ color: labelColor }}>{LABELS[task.state.kind]}</span>
          )}
        </span>
      </div>

      {/* 源地址：文件名不透明时（m3u8 / yt-dlp 起的名）靠它认人。egui 版也是把它摆在名字底下。 */}
      <div className="truncate text-[12px]" style={{ color: "var(--fg-muted)" }} title={task.url}>
        {task.url}
      </div>

      <div
        className="h-1 rounded-full my-1.5 overflow-hidden"
        style={{ background: "var(--track)" }}
      >
        <div
          className="h-full rounded-full transition-[width] duration-200"
          style={{
            width: `${task.state.kind === "merging" ? 100 : pct}%`,
            background:
              task.state.kind === "merging" ? "var(--accent-ok)" : "var(--fill)",
          }}
        />
      </div>

      <div className="flex items-center justify-between gap-3 tnum" style={{ color: "var(--fg-muted)" }}>
        <span className="truncate">
          {task.unit === "bytes"
            ? `${formatSize(task.downloaded)} / ${formatSize(task.total)}`
            : `分片 ${task.downloaded} / ${task.total}`}
          {task.speedBps ? ` · ${formatSpeed(task.speedBps)}` : ""}
          {task.etaText ? ` · ${task.etaText}` : ""}
        </span>
        <span className="shrink-0 flex gap-3">
          {task.state.kind === "awaitingChoice" && (
            // 等待选择的行可以把选择框重新叫回来（「稍后再说」把它压下去之后的出口）。
            <button onClick={() => dispatch({ kind: "openChoice", id: task.id })}>选择格式</button>
          )}
          {task.state.kind === "paused" && (
            <button onClick={() => void run(ipc.unpause(task.id))}>继续</button>
          )}
          {task.state.kind === "failed" && (
            <button onClick={() => void run(ipc.retry(task.id))}>重试</button>
          )}
          {/* 暂停按钮覆盖的正是 core 里 pause 接受的那四个状态 —— 否则就是死按钮 */}
          {["queued", "probing", "downloading", "merging"].includes(task.state.kind) && (
            <button onClick={() => void run(ipc.pause(task.id))}>暂停</button>
          )}
          {/* 按钮的叫法与调用的命令必须一致 —— 之前这里写着「从列表移除」却调 cancel()，
              对已取消的行连状态都不变，是个纯零反应的假按钮。见 lib/taskAction.ts。 */}
          <button onClick={() => void run(action.invoke(task.id))}>{action.label}</button>
        </span>
      </div>

      {task.state.kind === "failed" && (
        <details className="mt-1">
          <summary className="cursor-pointer" style={{ color: "var(--accent-err)" }}>
            出错详情
          </summary>
          <pre className="whitespace-pre-wrap mt-1" style={{ color: "var(--fg-muted)" }}>
            {task.state.message}
          </pre>
        </details>
      )}
    </div>
  );
}
