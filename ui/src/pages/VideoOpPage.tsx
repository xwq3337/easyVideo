import { Film } from "lucide-react";
import { VIDEO_OPS, opLabel, type VideoOp } from "../lib/videoOps";

/**
 * 视频操作页。
 *
 * 本轮只有「压缩」真能用（`VIDEO_OPS` 的 `ready`）。其余四个走不到这里
 * —— 侧边栏不给它们导航 —— 但类型上 `op` 是五个的联合，所以这里也照实说，
 * 不假装能用。
 */
export default function VideoOpPage({ op }: { op: VideoOp }) {
  const ready = VIDEO_OPS.find((o) => o.op === op)?.ready ?? false;
  return (
    <div className="h-full flex flex-col items-center justify-center gap-2">
      <Film size={32} strokeWidth={1.5} style={{ color: "var(--fg-faint)" }} />
      <p style={{ color: "var(--fg-muted)" }}>{ready ? `${opLabel(op)}的界面还在做` : `${opLabel(op)}还没做`}</p>
    </div>
  );
}
