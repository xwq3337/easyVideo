import { describe, expect, it } from "vitest";
import {
  applyDrag,
  clampCrop,
  compressQuality,
  evenDown,
  moveDown,
  moveUp,
  planKey,
  realignPicks,
  sourceDelta,
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

  it("x/y **不**取偶 —— core 的 validate_crop 只要求宽高是偶数", () => {
    // 早期这里把位置也取偶了。拖动时那会让框一格一格地黏（左边缘在偶数像素之间跳），
    // 而 core 根本不要求它 —— 白白多一条限制。
    expect(clampCrop({ width: 640, height: 360, x: 5, y: 7 }, 1920, 1080))
      .toEqual({ width: 640, height: 360, x: 5, y: 7 });
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

describe("applyDrag", () => {
  const start = { width: 640, height: 360, x: 100, y: 100 }; // 右边缘 740、下边缘 460

  it("移动：尺寸一点不变，位置被钳在画面里", () => {
    expect(applyDrag(start, { kind: "move" }, 50, 20, 1920, 1080))
      .toEqual({ width: 640, height: 360, x: 150, y: 120 });
    // 顶到右下角
    expect(applyDrag(start, { kind: "move" }, 99999, 99999, 1920, 1080))
      .toEqual({ width: 640, height: 360, x: 1280, y: 720 });
    // 顶到左上角
    expect(applyDrag(start, { kind: "move" }, -99999, -99999, 1920, 1080))
      .toEqual({ width: 640, height: 360, x: 0, y: 0 });
  });

  it("缩放：**对角固定**，只有被拖的那两条边动", () => {
    // 拖右下角：左上不动
    expect(applyDrag(start, { kind: "resize", corner: "se" }, 60, 40, 1920, 1080))
      .toEqual({ width: 700, height: 400, x: 100, y: 100 });
    // 拖左上角：右下不动（740/460 是那两个不动的边）
    const nw = applyDrag(start, { kind: "resize", corner: "nw" }, 60, 40, 1920, 1080);
    expect(nw).toEqual({ width: 580, height: 320, x: 160, y: 140 });
    expect(nw.x + nw.width).toBe(740);
    expect(nw.y + nw.height).toBe(460);
    // 拖右上角：左边与下边不动
    const ne = applyDrag(start, { kind: "resize", corner: "ne" }, 60, 40, 1920, 1080);
    expect(ne).toEqual({ width: 700, height: 320, x: 100, y: 140 });
    expect(ne.y + ne.height).toBe(460);
    // 拖左下角：右边与上边不动
    const sw = applyDrag(start, { kind: "resize", corner: "sw" }, 60, 40, 1920, 1080);
    expect(sw).toEqual({ width: 580, height: 400, x: 160, y: 100 });
    expect(sw.x + sw.width).toBe(740);
  });

  it("缩放：越界顶住，**不翻转**成反向的框", () => {
    // 往右下角外面拖：宽度顶到 1920-100 = 1820
    const pulled = applyDrag(start, { kind: "resize", corner: "se" }, 99999, 99999, 1920, 1080);
    expect(pulled).toEqual({ width: 1820, height: 980, x: 100, y: 100 });
    // 把右下角往回拖过头：宽高停在最小值 2，位置不越过另一条边
    const crushed = applyDrag(start, { kind: "resize", corner: "se" }, -99999, -99999, 1920, 1080);
    expect(crushed).toEqual({ width: 2, height: 2, x: 100, y: 100 });
    // 左上角往回拖过头：右下那条边固定，所以 x 顶到 738（= 740 - 2）
    const crushedNw = applyDrag(start, { kind: "resize", corner: "nw" }, 99999, 99999, 1920, 1080);
    expect(crushedNw).toEqual({ width: 2, height: 2, x: 738, y: 458 });
    // 左上角越过画面左上：x 停在 0，宽就是 740
    const pinnedNw = applyDrag(start, { kind: "resize", corner: "nw" }, -99999, -99999, 1920, 1080);
    expect(pinnedNw).toEqual({ width: 740, height: 460, x: 0, y: 0 });
  });

  it("缩放结果里的宽高**永远是偶数** —— ffmpeg 会静默取偶，我们当场取", () => {
    // 拖 1 像素：宽高从 640/360 变成 641/361，取偶回到 640/360（宽高不变，位置也不动）
    expect(applyDrag(start, { kind: "resize", corner: "se" }, 1, 1, 1920, 1080)).toEqual(start);
    expect(applyDrag(start, { kind: "resize", corner: "se" }, 2, 2, 1920, 1080))
      .toEqual({ width: 642, height: 362, x: 100, y: 100 });
  });

  it("小数增量先取整 —— 界面上的数字永远是整数", () => {
    expect(applyDrag(start, { kind: "move" }, 10.4, 10.6, 1920, 1080))
      .toEqual({ width: 640, height: 360, x: 110, y: 111 });
  });
});

describe("sourceDelta", () => {
  it("client 像素 → **源像素**：分母是画面那一块渲染出来的尺寸", () => {
    // 1920 宽的源在 640 宽的框里显示 → 1 个 client 像素 = 3 个源像素
    expect(sourceDelta(10, 10, { width: 640, height: 360 }, 1920, 1080))
      .toEqual({ dx: 30, dy: 30 });
    // 放大到 1280 → 1 个 client 像素是 1.5 个源像素
    expect(sourceDelta(10, 10, { width: 1280, height: 720 }, 1920, 1080))
      .toEqual({ dx: 15, dy: 15 });
  });

  it("渲染尺寸为 0 时给 0 而不是 Infinity/NaN", () => {
    // getBoundingClientRect 在元素还没布局时会全 0。除以 0 得到 Infinity，
    // 下一帧就成了 NaN 坐标 —— 框会直接消失
    expect(sourceDelta(10, 10, { width: 0, height: 0 }, 1920, 1080)).toEqual({ dx: 0, dy: 0 });
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
