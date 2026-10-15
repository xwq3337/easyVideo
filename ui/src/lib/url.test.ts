import { describe, expect, it } from "vitest";
import { normalizeUrl } from "./url";

describe("normalizeUrl", () => {
  it("http(s) 原样通过（两端空白去掉）", () => {
    expect(normalizeUrl("  https://example.com/v.mp4  ")).toBe("https://example.com/v.mp4");
    expect(normalizeUrl("HTTP://Example.com/a")).toBe("HTTP://Example.com/a");
  });

  it("缺 scheme 的裸主机名补上 https://", () => {
    expect(normalizeUrl("example.com/v.mp4")).toBe("https://example.com/v.mp4");
    expect(normalizeUrl("www.bilibili.com")).toBe("https://www.bilibili.com");
    expect(normalizeUrl("example.com:8080/v?a=1")).toBe("https://example.com:8080/v?a=1");
    expect(normalizeUrl("localhost:3000/x")).toBe("https://localhost:3000/x");
  });

  it("明确不是 http(s) 的一律拒绝", () => {
    expect(normalizeUrl("ftp://example.com/a")).toBeNull();
    expect(normalizeUrl("file:///etc/passwd")).toBeNull();
    expect(normalizeUrl("javascript:alert(1)")).toBeNull();
    expect(normalizeUrl("hello world")).toBeNull();
    expect(normalizeUrl("notaurl")).toBeNull();
  });

  it("空串返回 null（调用方当「没输入」，不报错）", () => {
    expect(normalizeUrl("")).toBeNull();
    expect(normalizeUrl("   ")).toBeNull();
  });
});
