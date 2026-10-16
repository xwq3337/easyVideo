/**
 * 解析用户填的时间。
 *
 * **与 core 的 `trim::parse_time` 是同一套规则** —— 两边都实现，是因为前端要在
 * 输入时就把开始按钮禁掉（不能让用户提交一个明显越界的值、再等 core 报错）。
 * **改规则要两边一起改**，两边也都有测试。
 *
 * 纯数字一律按秒解释（`90` 是 90 秒，不是 90 分）。秒那一格必须落在 0–60，
 * 所以 `9:99` 是 null —— 那是打错，不是"99 秒"。
 */
export function parseTimeInput(s: string): number | null {
  const t = s.trim();
  if (t === "") return null;

  const secs = (x: string): number | null => {
    const v = Number(x.trim());
    if (x.trim() === "" || Number.isNaN(v) || v < 0 || v >= 60) return null;
    return v;
  };

  /**
   * 小时/分钟那两格：**必须是空串以外的东西**。
   *
   * 少了 `raw === ""` 这一条，`":30"` 会被 `Number("") === 0` 放过去、算出 30 秒 ——
   * 而 core 的 `parse_time` 那边 `"".parse::<u64>()` 是失败的、返回 None。
   * 两边就会在同一个输入上给出不同结果，而这条规则的意义正是"前端先把明显错的挡住"。
   * （这个分歧是被 `rejects_garbage_times` 那条测试抓出来的。）
   */
  const intOf = (x: string): number | null => {
    const raw = x.trim();
    if (raw === "") return null;
    const v = Number(raw);
    return Number.isInteger(v) && v >= 0 ? v : null;
  };

  const parts = t.split(":");
  if (parts.length === 1) {
    const v = Number(t);
    return Number.isNaN(v) || v < 0 ? null : v;
  }
  if (parts.length === 2) {
    const m = intOf(parts[0]);
    const s = secs(parts[1]);
    if (m === null || s === null) return null;
    return m * 60 + s;
  }
  if (parts.length === 3) {
    const h = intOf(parts[0]);
    const m = intOf(parts[1]);
    const s = secs(parts[2]);
    if (h === null || m === null || s === null) return null;
    return h * 3600 + m * 60 + s;
  }
  return null;
}

/** 秒 → `m:ss` / `h:mm:ss`。给人看的，不是给 ffmpeg 吃的（那个用固定小数位）。 */
export function formatTimecode(secs: number): string {
  const total = Math.max(0, Math.floor(secs));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
    : `${m}:${String(s).padStart(2, "0")}`;
}
