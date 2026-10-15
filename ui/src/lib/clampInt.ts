/**
 * 把输入框的原始字符串夹进 [lo, hi]。
 *
 * 这个范围是**产品决定**，core 并不校验它 —— 那边只按 `usize` 收值。
 * `<input type="number" min max>` 也挡不住任何东西：程序化设值、粘贴、科学计数法
 * 都能绕过去，而 `save()` 是直接把 draft 发给 core 的。不夹的话有两种坏结果：
 *
 * - `-5` / `1e30` 这类值**收不下**（core 那边是 `usize`），`update_settings` 会返回
 *   serde 的原始英文报错，那段英文会原样显示在**保存按钮上方的红字**里（不是 toast ——
 *   这里直接调 `updateSettings`，没走 `run()` 那条路）；
 * - `99999` 这种**收得下但荒谬**的值会真的生效，并发分块数被拉到分块总数那么多
 *   （实际取 `min(该值, 分块总数)`，所以"十万"是上界而非实际值）。
 *
 * 非法/空值退回 fallback（等价于 egui 滑块拖不动的那一格）。
 *
 * 住在 `lib/` 是因为它是组件树里唯一一个有分量的纯函数，而本仓库的规矩是纯逻辑放这里、
 * 就地测（`taskAction.ts` 同因同果）；组件层不进测试（spec §9）。
 */
export function clampInt(raw: string, lo: number, hi: number, fallback: number): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(hi, Math.max(lo, Math.round(n)));
}
