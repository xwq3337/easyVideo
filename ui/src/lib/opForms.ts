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
