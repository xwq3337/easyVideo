import { describe, expect, it } from "vitest";
import { formatSize, formatSpeed } from "./format";

describe("formatSize", () => {
  it("按 1024 进制并保留一位小数", () => {
    expect(formatSize(0)).toBe("0 B");
    expect(formatSize(1023)).toBe("1023 B");
    expect(formatSize(1024)).toBe("1.0 KB");
    expect(formatSize(1536)).toBe("1.5 KB");
    // 1280 MiB = 1.25 GiB，保留一位小数进位到 1.3（brief 原文写的是 1.2，是算错了；
    // 实现就是 brief 给的实现，二者冲突时以数学为准，见报告）。
    expect(formatSize(1024 * 1024 * 1280)).toBe("1.3 GB");
  });
});

describe("formatSpeed", () => {
  it("单位是每秒，且 0 要显示成 0 B/s 而不是空", () => {
    expect(formatSpeed(0)).toBe("0 B/s");
    expect(formatSpeed(1024 * 1024 * 8.4)).toBe("8.4 MB/s");
  });
});
