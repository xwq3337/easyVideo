import { describe, expect, it } from "vitest";
import { formatTimecode, parseTimeInput } from "./timecode";

describe("parseTimeInput", () => {
  it("接受 mm:ss / hh:mm:ss / 纯秒数三种写法", () => {
    expect(parseTimeInput("12")).toBe(12);
    expect(parseTimeInput("1:30")).toBe(90);
    expect(parseTimeInput("0:00:12")).toBe(12);
    expect(parseTimeInput("1:02:03")).toBe(3723);
    expect(parseTimeInput("12.5")).toBe(12.5);
  });

  it("容忍前后空白（用户常从别处复制粘贴）", () => {
    expect(parseTimeInput("  1:30  ")).toBe(90);
  });

  it("空与非法一律 null —— 界面据此把开始按钮禁掉，而不是拿 0 去裁出个空文件", () => {
    for (const bad of ["", "  ", "abc", "1:2:3:4", "-5", "1:", ":30", "1:xx", "9:99"]) {
      expect(parseTimeInput(bad)).toBeNull();
    }
  });

  it("**秒那一格必须落在 0–60**：9:99 是打错，不是 99 秒", () => {
    expect(parseTimeInput("1:60")).toBeNull();
    expect(parseTimeInput("1:59")).toBe(59 + 60);
  });
});

describe("formatTimecode", () => {
  it("小给 m:ss，大给 h:mm:ss", () => {
    expect(formatTimecode(8)).toBe("0:08");
    expect(formatTimecode(90)).toBe("1:30");
    expect(formatTimecode(3723)).toBe("1:02:03");
  });
});
