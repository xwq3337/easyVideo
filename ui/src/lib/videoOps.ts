/**
 * 五个视频操作与它们的完成状态。
 *
 * `ready` 不是装饰：界面据此把未实现的标灰并加 `aria-disabled`。
 * 仓库里那条"明确不做胜过一个假装能用、点了没反应的界面"就落在这个字段上 ——
 * 有一个测试钉住"本轮只有压缩是 ready 的"。
 */
export type VideoOp = "compress" | "transcode" | "merge" | "trim" | "crop";

export const VIDEO_OPS: ReadonlyArray<{ op: VideoOp; label: string; ready: boolean }> = [
  { op: "compress", label: "压缩", ready: true },
  { op: "transcode", label: "转码", ready: true },
  { op: "merge", label: "合并", ready: false },
  { op: "trim", label: "裁剪视频", ready: true },
  { op: "crop", label: "画面裁剪", ready: false },
];

export function opLabel(op: VideoOp): string {
  return VIDEO_OPS.find((o) => o.op === op)?.label ?? "视频操作";
}
