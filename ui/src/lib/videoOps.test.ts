import { describe, expect, it } from "vitest";
import { opLabel, VIDEO_OPS } from "./videoOps";

describe("VIDEO_OPS", () => {
  it("本轮只有压缩是 ready 的", () => {
    // 这条是"不做假按钮"那条规矩的守卫：未实现的操作必须是 ready: false，
    // 界面据此灰显。全都标成 ready 的话，用户点上去不会有任何反应 ——
    // 那个界面比没有这个入口更糟。
    expect(VIDEO_OPS.filter((o) => o.ready).map((o) => o.op)).toEqual(["compress"]);
  });

  it("五个操作按固定顺序列出，且都有中文名", () => {
    expect(VIDEO_OPS.map((o) => o.op)).toEqual([
      "compress", "transcode", "merge", "trim", "crop",
    ]);
    expect(VIDEO_OPS.map((o) => o.label)).toEqual([
      "压缩", "转码", "合并", "裁剪视频", "画面裁剪",
    ]);
  });

  it("opLabel 对每个操作都返回非空", () => {
    for (const { op, label } of VIDEO_OPS) expect(opLabel(op)).toBe(label);
  });
});
