import { describe, expect, it } from "vitest";
import { opLabel, VIDEO_OPS } from "./videoOps";

describe("VIDEO_OPS", () => {
  it("本轮 ready 的是压缩、转码、裁剪视频", () => {
    // 这条守卫"不做假按钮"：未实现的必须是 ready: false，界面据此灰显。
    // 全标成 true 的话，用户点上去不会有任何反应 —— 那个界面比没有入口更糟。
    expect(VIDEO_OPS.filter((o) => o.ready).map((o) => o.op)).toEqual([
      "compress",
      "transcode",
      "trim",
    ]);
    // 合并与画面裁剪仍留着 —— 各自带一个独立的设计问题（多文件输入、抽帧预览界面）
    expect(VIDEO_OPS.filter((o) => !o.ready).map((o) => o.op)).toEqual(["merge", "crop"]);
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
