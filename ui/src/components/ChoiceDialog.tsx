import { useEffect } from "react";
import * as ipc from "../lib/ipc";
import { formatSize } from "../lib/format";
import { dispatch, run, useDismissedChoices, useTaskList } from "../store/store";
import { selectPendingChoice } from "../store/reducer";

/**
 * "选清晰度"与"选格式"是同一个组件的两种数据。
 *
 * 一个任务处于 awaitingChoice 却**没有候选副本**时（事件被丢掉、或刚重同步过），
 * 这里不发请求也不显示"加载中"就完事 —— store.hydrate() 会去调 pending_choice 补齐。
 * 在补齐之前这一行渲染成"正在获取候选…"，而不是一个没有任何按钮的死界面。
 *
 * **一次只弹一个**。默认 max_active_tasks = 2，两个 master playlist 任务会同时
 * 停在等待选择；若像草图那样对每个等待任务各渲染一层 `fixed inset-0`，
 * 后面那层会盖住并吞掉点击，前面那个任务的按钮就永远点不到（egui 版"两个模态框、
 * 有一个是死的"就是这个毛病）。所以这里只取一个来显示，解决掉或取消掉它，
 * 下一个自然出现。
 *
 * **可以关掉而不取消**：这个 fixed inset-0 遮罩没有点击穿透，过去唯一的出口是选一个格式
 * 或者「取消下载」—— 于是某个任务一直在等选择时，整个界面就被它冻住了，别的 URL 粘不进来、
 * 设置也打不开（egui 版是可关闭的浮动窗 + 每行一个「选择格式」按钮，这是对齐它）。
 * 现在「稍后再说」/Esc 只把这一条压下去（dispatch dismissChoice，不碰下载），
 * 由 TaskRow 的「选择格式」再把它放回来（openChoice）。
 */
export default function ChoiceDialog() {
  const tasks = useTaskList();
  const dismissed = useDismissedChoices();
  // id 最小的、且没被「稍后再说」压住的那条；全被压住时返回 null。
  const task = selectPendingChoice(tasks, dismissed);
  const taskId = task?.id ?? null;

  // Esc 与「稍后再说」同义：压下去，绝不取消下载。
  useEffect(() => {
    if (taskId === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") dispatch({ kind: "dismissChoice", id: taskId });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [taskId]);

  if (task === null) return null;

  return (
    <div
      className="fixed inset-0 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,.35)" }}
    >
      <div
        className="w-[460px] max-h-[70vh] overflow-y-auto rounded-lg p-5"
        style={{ background: "var(--bg)", border: "1px solid var(--line-strong)" }}
      >
        <h2 className="text-[15px] font-semibold mb-1">选择下载格式</h2>
        <p className="mb-4 truncate" style={{ color: "var(--fg-muted)" }}>
          {task.filename}
        </p>

        {task.choices === null && (
          <p style={{ color: "var(--fg-faint)" }}>正在获取候选…</p>
        )}

        {task.choices?.kind === "hlsVariants" && (
          <div className="grid gap-1.5">
            {task.choices.hlsVariants.map((v, i) => (
              <button
                key={v.uri}
                onClick={() => void run(ipc.setChoice(task.id, { kind: "variant", index: i }))}
                className="flex justify-between rounded-md px-3 py-2 border text-left"
                style={{ borderColor: "var(--line-strong)", background: "var(--bg-inset)" }}
              >
                <span>
                  {v.resolution ? `${v.resolution[0]}×${v.resolution[1]}` : "未知分辨率"}
                </span>
                <span className="tnum" style={{ color: "var(--fg-muted)" }}>
                  {Math.round(v.bandwidth / 1000)} kbps
                </span>
              </button>
            ))}
          </div>
        )}

        {task.choices?.kind === "ytdlpFormats" && (
          <div className="grid gap-1.5">
            {task.choices.ytdlpFormats.map((f, i) => (
              <button
                key={f.id}
                onClick={() => void run(ipc.setChoice(task.id, { kind: "ytdlpFormat", index: i }))}
                className="flex justify-between rounded-md px-3 py-2 border text-left"
                style={{ borderColor: "var(--line-strong)", background: "var(--bg-inset)" }}
              >
                <span>
                  {f.note}（.{f.ext}）
                  {!f.hasAudio && (
                    <span className="ml-1.5" style={{ color: "var(--fg-faint)" }}>
                      仅视频
                    </span>
                  )}
                </span>
                <span className="tnum" style={{ color: "var(--fg-muted)" }}>
                  {f.filesize ? formatSize(f.filesize) : "大小未知"}
                </span>
              </button>
            ))}
          </div>
        )}

        <div className="flex justify-end gap-2 mt-4">
          <button
            onClick={() => dispatch({ kind: "dismissChoice", id: task.id })}
            className="rounded-md px-3 py-1.5 border"
            style={{ borderColor: "var(--line-strong)" }}
          >
            稍后再说
          </button>
          <button onClick={() => void run(ipc.cancel(task.id))}>取消下载</button>
        </div>
      </div>
    </div>
  );
}
