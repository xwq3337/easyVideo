import { describe, expect, it } from "vitest";
import { opLabel, VIDEO_OPS } from "./videoOps";

describe("VIDEO_OPS", () => {
  it("五项全部 ready —— 侧边栏不该再有任何灰显项", () => {
    // 这条守卫"不做假按钮"：以前断言的是"只有这三项 ready、另两项不是"，
    // 现在五项都做完了，所以改成断言"一项灰显都没有"。
    // **不是删掉它** —— 将来有人加第六个操作，这条会提醒他要么实现要么标灰。
    expect(VIDEO_OPS.filter((o) => o.ready).map((o) => o.op)).toEqual([
      "compress", "transcode", "merge", "trim", "crop",
    ]);
    expect(VIDEO_OPS.filter((o) => !o.ready)).toEqual([]);
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
