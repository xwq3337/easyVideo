import { useJobList } from "../store/store";
import { newestFirst } from "../lib/order";
import JobRow from "./JobRow";

export default function JobList() {
  const jobs = useJobList();
  if (jobs.length === 0) return null;
  return (
    // 与 TaskList 同样的收口：末行那条 borderBottom 会被圆角切掉两端。
    // `!` 是必须的 —— JobRow 的边框写在行内 style 上，行内样式压得过普通 class。
    <div
      className="rounded-xl overflow-hidden [&>div:last-child]:border-b-0!"
      style={{ background: "var(--bg-subtle)" }}
    >
      {/* 最新的在最上面，与下载列表一致 */}
      {newestFirst(jobs).map((j) => (
        <JobRow key={j.id} job={j} />
      ))}
    </div>
  );
}
