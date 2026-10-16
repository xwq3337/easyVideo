import { describe, expect, it } from "vitest";
import {
  clampCrop,
  compressQuality,
  evenDown,
  moveDown,
  moveUp,
  planKey,
  realignPicks,
  trimFormReduce,
  trimParamsOf,
} from "./opForms";
import { DEFAULT_COMPRESS } from "./opDefaults";

describe("evenDown / clampCrop", () => {
  it("奇数向下取偶 —— ffmpeg 会静默取偶，我们让这件事**看得见**", () => {
    // 实测：请求 crop=641 得到的是 640，而且 stderr 里没有提示。
    // 界面上让数字当场变成 640，用户就知道实际会裁多少。
    expect(evenDown(641)).toBe(640);
    expect(evenDown(640)).toBe(640);
    expect(evenDown(3)).toBe(2);
    expect(evenDown(1)).toBe(0);
  });

  it("越界时把左/上**可见地**钳到上限", () => {
    // 源 1920x1080，要裁 1280x720：左最多 640、上最多 360
    const c = clampCrop({ width: 1280, height: 720, x: 1200, y: 800 }, 1920, 1080);
    expect(c).toEqual({ width: 1280, height: 720, x: 640, y: 360 });
  });

  it("宽高超过源尺寸时也钳（并且仍然是偶数）", () => {
    expect(clampCrop({ width: 4000, height: 3000, x: 0, y: 0 }, 1920, 1080))
      .toEqual({ width: 1920, height: 1080, x: 0, y: 0 });
    // 源是奇数时（罕见但合法）取偶
    expect(clampCrop({ width: 999, height: 999, x: 0, y: 0 }, 641, 361))
      .toEqual({ width: 640, height: 360, x: 0, y: 0 });
  });

  it("源尺寸未知时返回 null —— 界面据此禁用开始按钮并说明原因", () => {
    expect(clampCrop({ width: 640, height: 360, x: 0, y: 0 }, null, null)).toBeNull();
  });

  it("钳完小于 2 像素时也返回 null —— 那不是个合法的裁剪框", () => {
    // 表单刚拿到源尺寸、还没把草稿填成整个画面时，草稿是 {0,0,0,0}。若不判这一条，
    // 中间那一帧会把宽高为 0 的参数交上去，开始按钮亮着，而 core 一定拒它。
    // （与 core 的 `validate_crop` 第一条规则一致：宽高至少 2 像素。）
    expect(clampCrop({ width: 0, height: 0, x: 0, y: 0 }, 1920, 1080)).toBeNull();
    expect(clampCrop({ width: 1, height: 100, x: 0, y: 0 }, 1920, 1080)).toBeNull();
    expect(clampCrop({ width: 2, height: 2, x: 0, y: 0 }, 1920, 1080))
      .toEqual({ width: 2, height: 2, x: 0, y: 0 });
  });
});

describe("moveUp / moveDown", () => {
  const xs = ["a", "b", "c"];
  it("上下移动一格", () => {
    expect(moveUp(xs, 1)).toEqual(["b", "a", "c"]);
    expect(moveDown(xs, 1)).toEqual(["a", "c", "b"]);
  });
  it("到头不动，且**不返回原数组**（React 靠引用判断变没变）", () => {
    expect(moveUp(xs, 0)).toEqual(["a", "b", "c"]);
    expect(moveDown(xs, 2)).toEqual(["a", "b", "c"]);
    expect(moveUp(xs, 0)).not.toBe(xs);
  });
});

describe("realignPicks", () => {
  const prev = [
    { path: "/a.mp4", info: "信息A" },
    { path: "/b.mp4", info: "信息B" },
    { path: "/c.mp4", info: "信息C" },
  ];

  it("按**路径**找回信息 —— 调顺序、删一项都不会错位", () => {
    // 按下标对齐的话，删掉第 2 个之后剩下的两行会分别顶着「信息A」和「信息B」，
    // 也就是第 3 个文件顶着第 2 个文件的编码与尺寸。而这个页面唯一的任务就是
    // 让用户按看到的信息定顺序 —— 看到的信息是错的，比不显示更坏。
    expect(realignPicks(prev, ["/a.mp4", "/c.mp4"])).toEqual([
      { path: "/a.mp4", info: "信息A" },
      { path: "/c.mp4", info: "信息C" },
    ]);
    expect(realignPicks(prev, ["/c.mp4", "/b.mp4", "/a.mp4"])).toEqual([
      { path: "/c.mp4", info: "信息C" },
      { path: "/b.mp4", info: "信息B" },
      { path: "/a.mp4", info: "信息A" },
    ]);
  });

  it("追加没见过的路径 → 信息为 null（界面显示「读取中…」）", () => {
    expect(realignPicks(prev, ["/a.mp4", "/新.mp4"])).toEqual([
      { path: "/a.mp4", info: "信息A" },
      { path: "/新.mp4", info: null },
    ]);
  });

  it("同一个文件选两次：两条都能拿到信息（「合」的意思就是把它重复一遍）", () => {
    expect(realignPicks(prev, ["/a.mp4", "/a.mp4"])).toEqual([
      { path: "/a.mp4", info: "信息A" },
      { path: "/a.mp4", info: "信息A" },
    ]);
  });

  it("探测失败留下的 null **原地保留** —— 不能把它过滤掉，否则后面的整体前移", () => {
    const withHole = [
      { path: "/a.mp4", info: "信息A" },
      { path: "/b.mp4", info: null },
      { path: "/c.mp4", info: "信息C" },
    ];
    expect(realignPicks(withHole, ["/a.mp4", "/b.mp4", "/c.mp4"])).toEqual(withHole);
  });
});

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
