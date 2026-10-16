import { parseTimeInput } from "./timecode";
import type { JobParamsDto } from "./types";

/**
 * 三个操作表单里**能被判断对错的那部分**。
 *
 * 它们单独成模块、单独有测试，是因为组件本身在这个仓库里不做 DOM 测试 ——
 * 而这三条恰好都是"状态机搞错了"型的缺陷，纯靠肉眼看代码很难发现：
 * 中途输入非法时把档位改掉、画质按打字那一刻的编码器解释、预览漏了保存目录。
 */

// ===== 裁剪表单 =====

export type TrimFormState = { startText: string; endText: string; mode: string };
export type TrimAction =
  | { kind: "start"; text: string }
  | { kind: "end"; text: string }
  | { kind: "mode"; mode: string };

/**
 * **档位住在表单自己这里**，不从父级的 `params` 推。
 *
 * 这是复审抓到的那条 Critical 的修法：父级的 `params` 在输入中途是 `null`
 * （非法中间态），若档位从它推，就会在那一刻退回默认的 "fast" ——
 * 用户点了「精确」、逐字输入结束时间，最后提交的却是 `-c copy`，
 * 而界面会跟着翻回"快速"，他看不出自己点的是另一个操作。
 */
export function trimFormReduce(s: TrimFormState, a: TrimAction): TrimFormState {
  switch (a.kind) {
    case "start":
      return { ...s, startText: a.text };
    case "end":
      return { ...s, endText: a.text };
    case "mode":
      return { ...s, mode: a.mode };
  }
}

/** 表单状态 → 入参。任一项非法时 `null`（外框据此禁掉开始按钮）。 */
export function trimParamsOf(s: TrimFormState): JobParamsDto | null {
  const start = parseTimeInput(s.startText);
  if (start === null) return null;
  const hasEnd = s.endText.trim() !== "";
  const end = hasEnd ? parseTimeInput(s.endText) : null;
  if (hasEnd && end === null) return null;
  if (end !== null && end <= start) return null;
  return { op: "trim", startSecs: start, endSecs: end, mode: s.mode };
}

// ===== 压缩表单 =====

/**
 * 画质输入框的文本 → `crf` / `qv`，**按当前编码器解释**。
 *
 * 两种刻度的方向相反（CRF 越小越清晰，`-q:v` 越大越清晰），所以同一个数字要送进
 * 不同的字段 —— 而"当前"必须是**调用这一刻**的编码器。早先是在打字时算的，
 * 之后切换编码器只更新了 encoder：输入框里还写着 40，提交却用档位默认值。
 */
export function compressQuality(
  encoder: string,
  qualityText: string
): { crf: number | null; qv: number | null } {
  const raw = qualityText.trim();
  const v = raw === "" ? null : Number(raw);
  // 非数字（打字中间态）不进 params —— 别把 NaN 递给 core
  const q = v !== null && Number.isFinite(v) ? v : null;
  return encoder === "hardware" ? { crf: null, qv: q } : { crf: q, qv: null };
}

// ===== 产物名预览 =====

/**
 * 产物名预览的"何时重算"键。**`null` 表示现在还不用算**（缺文件或保存目录）。
 *
 * 键里只放**影响产物名**的东西：
 * - 源文件（决定 `<原名>`）
 * - **保存目录**（产物名的一部分，而且重名让路就是在它里面找空位）
 * - 操作种类（决定后缀）
 * - 决定扩展名的那两项（转码的 target、裁剪的 mode）
 *
 * 画质与时间**不放**：它们不影响名字，放进去会让用户每敲一个数字就跑一次
 * `ffmpeg -i`（`plan_output` 要探一次源文件）。
 */
export function planKey(
  input: string | null,
  op: string,
  params: JobParamsDto | null,
  outputDir: string | null
): string | null {
  if (!input || !outputDir || !params) return null;
  return JSON.stringify([
    input,
    outputDir,
    op,
    params.op === "transcode" ? params.target : null,
    params.op === "trim" ? params.mode : null,
  ]);
}

// ===== 画面裁剪的钳制 =====

/**
 * 奇数向下取偶。
 *
 * **ffmpeg 会静默取偶**（实测：请求 crop=641 得到 640，stderr 里一条提示都没有），
 * 所以界面上要让这件事**看得见** —— 输入框里的数字当场变成 640，用户就知道
 * 实际会裁多少。
 */
export function evenDown(n: number): number {
  return Math.max(0, Math.floor(n / 2) * 2);
}

/** 裁剪框草稿。四个值都是用户刚敲进去的、可能还没合法化。 */
export type CropDraft = { width: number; height: number; x: number; y: number };

/**
 * 把草稿钳成一个**合法**的裁剪框：取偶、不超过源、左/上不越界。
 *
 * 与 ffmpeg 的静默钳制有本质区别：这里钳完的结果**会写回输入框**，
 * 用户看得见数字变了；ffmpeg 钳完什么都没说。
 *
 * 源尺寸未知时返回 `null` —— 界面据此禁用开始按钮并说明原因，
 * 而不是拿 0 去算一个必然越界的框。
 */
export function clampCrop(
  d: CropDraft,
  srcW: number | null,
  srcH: number | null
): CropDraft | null {
  if (srcW == null || srcH == null || srcW < 2 || srcH < 2) return null;
  // **只把宽高取偶** —— core 的 `validate_crop` 也只要求这两项是偶数。
  // 曾经连 x/y 一起取偶，那会在拖动时让框一格一格地黏（左边缘在偶数像素之间跳），
  // 而多出来这条限制没有任何依据。
  const width = Math.min(evenDown(d.width), evenDown(srcW));
  const height = Math.min(evenDown(d.height), evenDown(srcH));
  // 钳完小于 2 像素也不是个合法的框（与 core 的 `validate_crop` 第一条规则一致）。
  // 少了这条，表单刚拿到源尺寸而草稿还是 {0,0,0,0} 的那一帧会交出一个 width=0 的
  // 参数 —— 开始按钮亮着，而 core 一定拒它。
  if (width < 2 || height < 2) return null;
  const x = Math.min(Math.max(0, d.x), srcW - width);
  const y = Math.min(Math.max(0, d.y), srcH - height);
  return { width, height, x, y };
}

/**
 * client 像素增量 → **源像素**增量。
 *
 * 分母是**画面那一块**渲染出来的尺寸（`getBoundingClientRect`），不是窗口、也不是
 * 外层容器 —— 预览图被 `max-w` 缩过，用错分母的话拖动会跟着窗口宽度变快变慢。
 *
 * 渲染尺寸为 0 时给 0 而不是除以它：`getBoundingClientRect` 在元素还没布局时全 0，
 * 除出来的 Infinity 下一帧就是 NaN 坐标，框会直接消失。
 */
export function sourceDelta(
  dx: number,
  dy: number,
  frame: { width: number; height: number },
  srcW: number,
  srcH: number
): { dx: number; dy: number } {
  if (frame.width <= 0 || frame.height <= 0) return { dx: 0, dy: 0 };
  return { dx: (dx * srcW) / frame.width, dy: (dy * srcH) / frame.height };
}

/** 拖动的手势：整框移动，或者抓某个角缩放。 */
export type CropGesture =
  | { kind: "move" }
  | { kind: "resize"; corner: "nw" | "ne" | "sw" | "se" };

const clampTo = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);

/** 裁剪框的最小边长。2 是 `validate_crop` 的技术下限，也是 h264/yuv420p 的下限。 */
const MIN_CROP = 2;

/**
 * 拖动之后的新裁剪框。**纯函数** —— 组件只负责把 client 像素换算成源像素再喂进来。
 *
 * 这是这次改动的全部逻辑，也是唯一能被测试钉住的部分（这个仓库不做 DOM 测试）：
 * 对角固定、越界顶住而不翻转、宽高当场取偶。
 *
 * **对角固定**：拖右下角时左上那个角一动不动，拖左上角时右下不动。少了它，
 * 缩放会连着位置一起漂，用户的手感是"框在滑走"而不是"我在拉这个角"。
 *
 * **越界顶住而不翻转**：把右下角一路拖过左上角时，宽高停在 2 像素，不变成负的、
 * 也不翻到反方向去。翻转会让「继续往回拖」变成「框从另一边长出来」。
 */
export function applyDrag(
  start: CropDraft,
  g: CropGesture,
  dx: number,
  dy: number,
  srcW: number,
  srcH: number
): CropDraft {
  // 源像素的增量在缩放后的图上通常带小数，先取整：界面上的数字永远是整数
  const mx = Math.round(dx);
  const my = Math.round(dy);

  if (g.kind === "move") {
    return {
      ...start,
      x: clampTo(start.x + mx, 0, Math.max(0, srcW - start.width)),
      y: clampTo(start.y + my, 0, Math.max(0, srcH - start.height)),
    };
  }

  const west = g.corner === "nw" || g.corner === "sw";
  const north = g.corner === "nw" || g.corner === "ne";
  // 不动的那个角就是它的位置基准
  const right = start.x + start.width;
  const bottom = start.y + start.height;

  const width = clampTo(
    evenDown(west ? start.width - mx : start.width + mx),
    MIN_CROP,
    Math.max(MIN_CROP, evenDown(west ? right : srcW - start.x))
  );
  const height = clampTo(
    evenDown(north ? start.height - my : start.height + my),
    MIN_CROP,
    Math.max(MIN_CROP, evenDown(north ? bottom : srcH - start.y))
  );

  return {
    width,
    height,
    x: west ? right - width : start.x,
    y: north ? bottom - height : start.y,
  };
}

// ===== 合并列表的排序 =====

/**
 * 列表里第 i 项与上一项交换。到顶时返回**新数组**（内容不变）。
 *
 * 到边界也返回新数组而不是原数组：返回原数组的话 React 的 `setState` 会因为
 * 引用相同而跳过重渲染（与 `newestFirst` 那条是同一个道理）。
 */
export function moveUp<T>(list: readonly T[], i: number): T[] {
  const out = [...list];
  if (i > 0 && i < out.length) [out[i - 1], out[i]] = [out[i], out[i - 1]];
  return out;
}

export function moveDown<T>(list: readonly T[], i: number): T[] {
  const out = [...list];
  if (i >= 0 && i < out.length - 1) [out[i], out[i + 1]] = [out[i + 1], out[i]];
  return out;
}

/**
 * 把「已选文件 + 它的媒体信息」重新对齐到一组**新**的路径。
 *
 * 按**路径**找回信息，而不是按下标。上下移、删一项、同一个文件选两次都能对上 ——
 * 而按下标对齐时，删掉第 2 个之后剩下的两行会分别顶着原第 1、2 个的编码与尺寸，
 * 也就是**第 3 个文件顶着第 2 个文件的信息**。这个页面唯一的任务就是让用户按看到
 * 的信息定拼接顺序，而看到的信息是错的。
 *
 * 探测失败的那一项**保留 `null`** 而不是被过滤掉 —— `got.filter(...)` 那种写法会让
 * 后面所有行整体前移，是同一个错位的第二个入口。
 */
export function realignPicks<T>(
  prev: ReadonlyArray<{ path: string; info: T | null }>,
  next: readonly string[]
): Array<{ path: string; info: T | null }> {
  return next.map((path) => {
    const hit = prev.find((p) => p.path === path);
    return { path, info: hit ? hit.info : null };
  });
}
