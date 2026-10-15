import { describe, expect, it } from "vitest";
import { clampInt } from "./clampInt";

// 与设置页的两处调用保持一致：范围是产品决定（线程数 1–16、同时下载数 1–8），
// fallback 取各自的下界 —— 也就是"拖不动的那一格"。
const LO = 1;
const HI = 16;
const FALLBACK = LO;

describe("clampInt", () => {
  it("空串与解析不出来的字符串都退回 fallback", () => {
    // 空串走的是另一条路：`Number("")` 是 0（不是 NaN），于是被**下界**夹回来。
    // 因为 fallback 就是下界，结果一样；断言的是结果，不是路径。
    expect(clampInt("", LO, HI, FALLBACK)).toBe(FALLBACK);
    expect(clampInt("abc", LO, HI, FALLBACK)).toBe(FALLBACK);
  });

  it("1e30 这类科学计数法被夹到上界", () => {
    // `<input type="number">` 的 min/max 拦不住它 —— 粘贴进去照样是合法值。
    expect(clampInt("1e30", LO, HI, FALLBACK)).toBe(HI);
  });

  it("99999 是收得下但荒谬的值，同样夹到上界", () => {
    // core 那边是 usize，收 99999 一点问题没有；是这里把它挡在 hi=16 上。
    expect(clampInt("99999", LO, HI, FALLBACK)).toBe(16);
  });

  it("范围内的值原样通过", () => {
    expect(clampInt("8", LO, HI, FALLBACK)).toBe(8);
    expect(clampInt("1", LO, HI, FALLBACK)).toBe(LO);
    expect(clampInt("16", LO, HI, FALLBACK)).toBe(HI);
  });

  it("非整数四舍五入", () => {
    expect(clampInt("8.6", LO, HI, FALLBACK)).toBe(9);
    expect(clampInt("8.4", LO, HI, FALLBACK)).toBe(8);
  });
});
