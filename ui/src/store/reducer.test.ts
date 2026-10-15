import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./reducer";
import type { TaskView } from "../lib/types";

function task(id: number, over: Partial<TaskView> = {}): TaskView {
  return {
    id,
    url: `https://example.com/${id}.mp4`,
    filename: `${id}.mp4`,
    state: { kind: "queued" },
    downloaded: 0,
    total: 0,
    unit: "bytes",
    choices: null,
    editing: false,
    speedBps: null,
    etaText: null,
    lastTickAt: 0,
    ...over,
  };
}

describe("reduce", () => {
  it("hydrate 整体替换，不做合并", () => {
    const before = { ...initialState, tasks: [task(1), task(2)] };
    const after = reduce(before, { kind: "hydrate", tasks: [task(3)] });
    expect(after.tasks.map((t) => t.id)).toEqual([3]);
  });

  it("进度事件只改那一行", () => {
    const before = { ...initialState, tasks: [task(1), task(2)] };
    const after = reduce(before, {
      kind: "event",
      event: { type: "progress", id: 2, downloaded: 50, total: 100, unit: "bytes" },
    });
    expect(after.tasks[0].downloaded).toBe(0);
    expect(after.tasks[1].downloaded).toBe(50);
  });

  it("未知 id 不丢帧，而是标记需要重同步", () => {
    // add_url 返回 id 之前，core 已经在发这个任务的事件了。丢掉它，那一行就要等
    // 下一次状态变化才出现；报错更糟。正确做法是"记一笔，去拉一次全量"。
    const before = { ...initialState, tasks: [task(1)] };
    const after = reduce(before, {
      kind: "event",
      event: { type: "progress", id: 404, downloaded: 1, total: 2, unit: "bytes" },
    });
    expect(after.tasks).toHaveLength(1);
    expect(after.needsResync).toBe(true);
  });

  it("状态变化落到对应行，并清掉 needsResync", () => {
    const before = { ...initialState, tasks: [task(1)], needsResync: true };
    const after = reduce(before, {
      kind: "event",
      event: { type: "stateChanged", id: 1, state: { kind: "done" } },
    });
    expect(after.tasks[0].state).toEqual({ kind: "done" });
    expect(after.needsResync).toBe(false);
  });

  it("正在编辑的行不被进度事件覆盖文件名", () => {
    const before = { ...initialState, tasks: [task(1, { editing: true, filename: "我改的名字" })] };
    const after = reduce(before, {
      kind: "event",
      event: { type: "progress", id: 1, downloaded: 5, total: 9, unit: "bytes" },
    });
    expect(after.tasks[0].filename).toBe("我改的名字");
  });

  it("choicesOffered 存下候选", () => {
    const before = { ...initialState, tasks: [task(1)] };
    const after = reduce(before, {
      kind: "event",
      event: {
        type: "choicesOffered",
        id: 1,
        kind: { kind: "hlsVariants", hlsVariants: [{ uri: "a.m3u8", bandwidth: 1, resolution: null }] },
      },
    });
    expect(after.tasks[0].state).toEqual({ kind: "awaitingChoice" });
    expect(after.tasks[0].choices).not.toBeNull();
  });

  it("两次进度之间的差算出速度，并给出剩余时间", () => {
    // 传 now 是为了不依赖真实时钟 —— 测试里等一秒不许写 sleep。
    const t0 = 1_000_000;
    const a = reduce({ ...initialState, tasks: [task(1, { total: 1000 })] }, {
      kind: "event",
      event: { type: "progress", id: 1, downloaded: 0, total: 1000, unit: "bytes" },
      now: t0,
    });
    const b = reduce(a, {
      kind: "event",
      event: { type: "progress", id: 1, downloaded: 500, total: 1000, unit: "bytes" },
      now: t0 + 1000,
    });
    expect(b.tasks[0].speedBps).toBeCloseTo(500, 0);
    expect(b.tasks[0].etaText).toBe("剩 1 秒");
  });

  it("分片单位不报速度：那是片/秒，不是字节/秒", () => {
    // 混进"总速度"会得到 "3.0 B/s" 这种假数字（egui 版就是这么错的）。
    const t0 = 1_000_000;
    const a = reduce({ ...initialState, tasks: [task(1, { unit: "segments" })] }, {
      kind: "event",
      event: { type: "progress", id: 1, downloaded: 0, total: 10, unit: "segments" },
      now: t0,
    });
    const b = reduce(a, {
      kind: "event",
      event: { type: "progress", id: 1, downloaded: 5, total: 10, unit: "segments" },
      now: t0 + 1000,
    });
    expect(b.tasks[0].speedBps).toBeNull();
    expect(b.tasks[0].etaText).toBeNull();
  });

  it("hydrate 之后速度归零，不会把上一轮的数字留在界面上", () => {
    const before = { ...initialState, tasks: [task(1, { speedBps: 999, etaText: "剩 1 秒" })] };
    const after = reduce(before, { kind: "hydrate", tasks: [task(1)] });
    expect(after.tasks[0].speedBps).toBeNull();
    expect(after.tasks[0].etaText).toBeNull();
  });

  it("命令失败只加一条提示，不动任务列表", () => {
    // 命令级失败不打断操作：列表照旧，只在角落多一条可关掉的提示。
    const before = { ...initialState, tasks: [task(1)] };
    const after = reduce(before, { kind: "pushToast", text: "保存设置失败：权限不足" });
    expect(after.tasks).toEqual(before.tasks);
    expect(after.toasts).toEqual(["保存设置失败：权限不足"]);
  });

  it("提示可以逐条关掉", () => {
    const before = { ...initialState, toasts: ["a", "b"] };
    expect(reduce(before, { kind: "dismissToast", index: 0 }).toasts).toEqual(["b"]);
  });
});
