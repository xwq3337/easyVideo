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
  const width = Math.min(evenDown(d.width), evenDown(srcW));
  const height = Math.min(evenDown(d.height), evenDown(srcH));
  // 钳完小于 2 像素也不是个合法的框（与 core 的 `validate_crop` 第一条规则一致）。
  // 少了这条，表单刚拿到源尺寸而草稿还是 {0,0,0,0} 的那一帧会交出一个 width=0 的
  // 参数 —— 开始按钮亮着，而 core 一定拒它。
  if (width < 2 || height < 2) return null;
  const x = Math.min(Math.max(0, evenDown(d.x)), srcW - width);
  const y = Math.min(Math.max(0, evenDown(d.y)), srcH - height);
  return { width, height, x, y };
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
