import { describe, expect, it } from "vitest";
import { compressQuality, planKey, trimFormReduce, trimParamsOf } from "./opForms";
import { DEFAULT_COMPRESS } from "./opDefaults";

describe("trimParamsOf / trimFormReduce", () => {
  it("**中途输入非法不能把档位改掉**（这是复审抓到的 Critical）", () => {
    // 用户先点「精确」，然后在结束框里逐字输入。中间态（"0"、"0:"）都是非法的，
    // 每一次非法输入都会让外框把 params 置成 null —— 而表单如果在 params 为 null 时
    // 退回默认档位，下一按键就会带着 "fast" 提交。用户点的是精确，拿到的是 -c copy，
    // 而界面会跟着翻回"快速"，他看不出自己点的是另一个操作。
    let s = { startText: "0", endText: "", mode: "accurate" };
    s = trimFormReduce(s, { kind: "end", text: "0" }); // 非法：end === start
    expect(trimParamsOf(s)).toBeNull();
    s = trimFormReduce(s, { kind: "end", text: "0:" }); // 非法：秒那格解析不出来
    expect(trimParamsOf(s)).toBeNull();
    // 到这里为止，父级的 params 已经被置成 null 两次了 —— 档位必须挺过来
    s = trimFormReduce(s, { kind: "end", text: "0:30" });
    expect(trimParamsOf(s)).toEqual({
      op: "trim",
      startSecs: 0,
      endSecs: 30,
      mode: "accurate",
    });
  });

  it("合法区间照常解析，结束留空=到结尾", () => {
    expect(trimParamsOf({ startText: "1:30", endText: "", mode: "fast" })).toEqual({
      op: "trim",
      startSecs: 90,
      endSecs: null,
      mode: "fast",
    });
    expect(trimParamsOf({ startText: "0:12", endText: "0:20", mode: "fast" })).toEqual({
      op: "trim",
      startSecs: 12,
      endSecs: 20,
      mode: "fast",
    });
  });

  it("结束早于起始、起始解析不出来 —— 都是 null", () => {
    expect(trimParamsOf({ startText: "20", endText: "10", mode: "fast" })).toBeNull();
    expect(trimParamsOf({ startText: "abc", endText: "30", mode: "fast" })).toBeNull();
  });
});

describe("compressQuality", () => {
  it("画质按**当前**编码器解释，不是打字那一刻的", () => {
    // 复审抓到的回归：原来 crf/qv 是在打字时按当时的编码器算的，之后再切编码器
    // 就只更新 encoder —— 于是输入框里还写着 40，提交却用档位默认值。UI 在撒谎。
    expect(compressQuality("software", "40")).toEqual({ crf: 40, qv: null });
    expect(compressQuality("auto", "40")).toEqual({ crf: 40, qv: null });
    expect(compressQuality("hardware", "40")).toEqual({ crf: null, qv: 40 });
  });

  it("留空 = 跟随档位（两个字段都是 null）", () => {
    for (const e of ["auto", "software", "hardware"]) {
      expect(compressQuality(e, "")).toEqual({ crf: null, qv: null });
      expect(compressQuality(e, "  ")).toEqual({ crf: null, qv: null });
    }
  });

  it("非数字进不去（那是输入框的脏值，不该变成 NaN 传给 core）", () => {
    expect(compressQuality("software", "abc")).toEqual({ crf: null, qv: null });
  });
});

describe("planKey", () => {
  it("**保存目录变了就必须重算** —— 它是产物名的一部分", () => {
    const p = { ...DEFAULT_COMPRESS };
    expect(planKey("/a.mp4", "compress", p, "/out1")).not.toBe(
      planKey("/a.mp4", "compress", p, "/out2")
    );
    expect(planKey("/a.mp4", "compress", p, "/out1")).toBe(
      planKey("/a.mp4", "compress", p, "/out1")
    );
  });

  it("决定扩展名的那两项也要在里面（转码的 target、裁剪的 mode）", () => {
    const t1 = { op: "transcode" as const, target: "mp4h264", preset: "balanced" };
    const t2 = { op: "transcode" as const, target: "webmvp9", preset: "balanced" };
    expect(planKey("/a.mp4", "transcode", t1, "/o")).not.toBe(planKey("/a.mp4", "transcode", t2, "/o"));
    // 不影响名字的（时间、画质）不该在里面 —— 否则用户每敲一个数字就跑一次 ffmpeg -i
    const c1 = { op: "compress" as const, preset: "small", encoder: "auto", maxHeight: null, crf: null, qv: null, audioKbps: null };
    const c2 = { op: "compress" as const, preset: "high", encoder: "hardware", maxHeight: 720, crf: 18, qv: null, audioKbps: null };
    expect(planKey("/a.mp4", "compress", c1, "/o")).toBe(planKey("/a.mp4", "compress", c2, "/o"));
  });

  it("缺任一项时返回 null（还没选文件 / 还没拿到保存目录）", () => {
    expect(planKey(null, "compress", DEFAULT_COMPRESS, "/o")).toBeNull();
    expect(planKey("/a.mp4", "compress", DEFAULT_COMPRESS, null)).toBeNull();
    expect(planKey("/a.mp4", "compress", null, "/o")).toBeNull();
  });
});
