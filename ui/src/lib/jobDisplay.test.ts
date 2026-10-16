import { describe, expect, it } from "vitest";
import { jobPct, jobStateLabel } from "./jobDisplay";

describe("jobStateLabel", () => {
  it("每种状态都有人话", () => {
    expect(jobStateLabel({ kind: "queued" })).toBe("排队中");
    expect(jobStateLabel({ kind: "running" })).toBe("处理中");
    expect(jobStateLabel({ kind: "done" })).toBe("已完成");
    expect(jobStateLabel({ kind: "canceled" })).toBe("已取消");
  });

  it("失败状态直接把原因带出来", () => {
    expect(jobStateLabel({ kind: "failed", message: "ffmpeg 失败: ..." }))
      .toBe("失败：ffmpeg 失败: ...");
  });
});

describe("jobPct", () => {
  it("把 0–1 换成整数百分比", () => {
    expect(jobPct(0.42)).toBe(42);
    expect(jobPct(1)).toBe(100);
  });

  it("进度未知时返回 null —— 界面据此显示不确定态，绝不编一个分数", () => {
    // 时长读不出来时 core 报的就是 null。显示 0% 会让用户以为卡住了。
    expect(jobPct(null)).toBeNull();
  });

  it("夹到 0–100，防止封装报出越界的 out_time", () => {
    expect(jobPct(-0.5)).toBe(0);
    expect(jobPct(1.5)).toBe(100);
  });
});
