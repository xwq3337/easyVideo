import { X } from "lucide-react";
import type { JobView } from "../lib/types";
import { jobPct, jobStateLabel } from "../lib/jobDisplay";
import * as ipc from "../lib/ipc";
import { run } from "../store/store";

/** 与 TaskRow 同样的行内动作按钮样式（见那里对悬停字色的说明）。 */
const ACTION_BTN =
  "flex items-center gap-1 text-[12px] transition-colors hover:text-[var(--fg)]";

export default function JobRow({ job }: { job: JobView }) {
  const pct = jobPct(job.progress);
  const running = job.state.kind === "running" || job.state.kind === "queued";
  const done = job.state.kind === "done";
  const failed = job.state.kind === "failed";

  return (
    <div className="px-3.5 py-3" style={{ borderBottom: "1px solid var(--line)" }}>
      <div className="flex items-baseline justify-between gap-3">
        <span className="truncate font-medium">{job.label}</span>
        <span
          className="tnum shrink-0 text-[12px]"
          style={{
            color: failed ? "var(--accent-err)" : done ? "var(--accent-ok)" : "var(--fg-muted)",
          }}
        >
          {jobStateLabel(job.state)}
        </span>
      </div>

      {/* 源文件路径：文件名往往不透明，靠它认人（与 TaskRow 摆 URL 同理） */}
      <div className="truncate text-[12px]" style={{ color: "var(--fg-muted)" }} title={job.input}>
        {job.input}
      </div>

      <div
        className="h-1 rounded-full my-1.5 overflow-hidden"
        style={{ background: "var(--track)" }}
      >
        <div
          className="h-full rounded-full transition-[width] duration-200"
          style={{
            // 进度未知（时长读不出来）时不画填充、也不显示 0% —— 那看着像卡死。
            // 宽度 0 配合下面那句"处理中…"照实表达不确定。
            width: `${pct ?? (done ? 100 : 0)}%`,
            background: done ? "var(--accent-ok)" : "var(--fill)",
          }}
        />
      </div>

      <div
        className="flex items-center justify-between gap-3 text-[12px]"
        style={{ color: "var(--fg-muted)" }}
      >
        <span className="truncate" title={done ? job.output : undefined}>
          {done ? `已保存到 ${job.output}` : pct === null ? "处理中…" : `${pct}%`}
        </span>
        <span className="shrink-0 flex gap-1">
          {running ? (
            <button className={ACTION_BTN} onClick={() => void run(ipc.cancelVideoJob(job.id))}>
              <X size={14} strokeWidth={1.75} />
              取消
            </button>
          ) : (
            <button className={ACTION_BTN} onClick={() => void run(ipc.removeVideoJob(job.id))}>
              <X size={14} strokeWidth={1.75} />
              从列表移除
            </button>
          )}
        </span>
      </div>

      {failed && (
        <details className="mt-1">
          <summary className="cursor-pointer" style={{ color: "var(--accent-err)" }}>
            出错详情
          </summary>
          <pre className="whitespace-pre-wrap mt-1" style={{ color: "var(--fg-muted)" }}>
            {job.state.kind === "failed" ? job.state.message : ""}
          </pre>
        </details>
      )}
    </div>
  );
}
