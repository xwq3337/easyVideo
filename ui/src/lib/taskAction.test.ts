import { describe, expect, it } from "vitest";
import { taskAction } from "./taskAction";
import * as ipc from "./ipc";

describe("taskAction", () => {
  it.each(["done", "failed", "canceled"] as const)(
    "%s 是终态,按钮应当移除而不是取消",
    (kind) => {
      const a = taskAction(kind);
      // core 从不删除用户文件。对已经结束的行叫「删除」是假话,
      // 而且它真正能做、也应该做的只有"移出列表"。
      expect(a.label).toBe("从列表移除");
      // **必须连 invoke 一起钉住。** 这个纯函数存在的唯一理由,就是让标签与
      // 它真正调用的命令**出自同一个对象** —— 原来的 bug 正是这两者不一致
      // (标签写「从列表移除」、处理函数调 cancel)。只断言 label 的话,把
      // invoke 换成 ipc.cancel 会让原 bug 原样复活,而全部测试仍然通过。
      expect(a.invoke).toBe(ipc.removeTask);
    }
  );

  it.each(["queued", "probing", "awaitingChoice", "downloading", "merging", "paused"] as const)(
    "%s 不是终态,按钮应当叫取消",
    (kind) => {
      const a = taskAction(kind);
      // 它调的就是 cancel()。叫「删除」会让用户以为点了就丢文件。
      expect(a.label).toBe("取消");
      expect(a.invoke).toBe(ipc.cancel);
    }
  );
});
