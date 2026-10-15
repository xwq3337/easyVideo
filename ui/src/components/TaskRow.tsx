import { useState } from "react";
import type { TaskView } from "../lib/types";
import { formatSize, formatSpeed } from "../lib/format";
import * as ipc from "../lib/ipc";
import { dispatch, run } from "../store/store";

const LABELS: Record<string, string> = {
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
  const done = task.state.kind === "done" || task.state.kind === "canceled";

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
          {task.state.kind === "failed" ? (
            <span style={{ color: "var(--accent-err)" }}>{LABELS[task.state.kind]}</span>
          ) : task.state.kind === "merging" ? (
            <span style={{ color: "var(--accent-ok)" }}>{LABELS[task.state.kind]}</span>
          ) : task.state.kind === "paused" ? (
            <span style={{ color: "var(--accent-warn)" }}>{LABELS[task.state.kind]}</span>
          ) : done ? (
            // 终态给名字，不给「100%」—— brief 的 Step 4 明确要求完成时"显示已完成";
            // 作者已定义 done/已取消 两条文案，并把 done 算出来却只写了个 `{done && null}` 的桩。
            <span
              style={{
                color: task.state.kind === "done" ? "var(--accent-ok)" : "var(--fg-muted)",
              }}
            >
              {LABELS[task.state.kind]}
            </span>
          ) : task.unit === "bytes" ? (
            `${pct}%`
          ) : (
            `${task.downloaded} / ${task.total} 片`
          )}
        </span>
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
          <button onClick={() => void run(ipc.cancel(task.id))}>删除</button>
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
