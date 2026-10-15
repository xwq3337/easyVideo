import { Film } from "lucide-react";

/**
 * 占位。骨架按「选文件 → 选操作 → 队列」预留，本期不实现。
 * 明确不做胜过一个假装能用、点了没反应的界面 —— 这次的 bug 就是那么来的。
 */
export default function VideoOpsPage() {
  return (
    <div className="h-full flex flex-col items-center justify-center gap-2">
      <Film size={32} strokeWidth={1.5} style={{ color: "var(--fg-faint)" }} />
      <p style={{ color: "var(--fg-muted)" }}>视频操作还在做</p>
      <p className="text-[12px]" style={{ color: "var(--fg-faint)" }}>
        以后可以在这里转码、压缩、裁剪、提取音频
      </p>
    </div>
  );
}
