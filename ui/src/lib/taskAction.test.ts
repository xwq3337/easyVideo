import { describe, expect, it } from "vitest";
import { taskAction } from "./taskAction";

describe("taskAction", () => {
  it.each(["done", "failed", "canceled"] as const)(
    "%s 是终态,按钮应当移除而不是取消",
    (kind) => {
      // core 从不删除用户文件。对已经结束的行叫「删除」是假话,
      // 而且它真正能做、也应该做的只有"移出列表"。
      expect(taskAction(kind).label).toBe("从列表移除");
    }
  );

  it.each(["queued", "probing", "awaitingChoice", "downloading", "merging", "paused"] as const)(
    "%s 不是终态,按钮应当叫取消",
    (kind) => {
      // 它调的就是 cancel()。叫「删除」会让用户以为点了就丢文件。
      expect(taskAction(kind).label).toBe("取消");
    }
  );
});
