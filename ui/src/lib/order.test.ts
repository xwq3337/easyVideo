import { describe, expect, it } from "vitest";
import { newestFirst } from "./order";

function row(id: number) {
  return { id } as { id: number };
}

describe("newestFirst", () => {
  it("id 大的在前：core 给的是 id 升序，界面要倒过来显示", () => {
    // core 的 list_tasks() 按 id 升序返回（最老的在前），而界面上最新加的任务
    // 必须在最上面 —— 否则用户每加一个 URL 都要滚到底才能看到它。
    expect(newestFirst([row(1), row(2), row(3)]).map((t) => t.id)).toEqual([3, 2, 1]);
  });

  it("不改动传入的数组", () => {
    // 传进来的正是 store 那份 state.tasks。原地 reverse 会改掉快照本身：
    // useSyncExternalStore 靠引用相等判断"变没变"，就地改会让 React 读到
    // 一个已经变了、引用却没变的快照 —— 界面不更新，而内存里顺序已经乱了。
    const tasks = [row(1), row(2), row(3)];
    const before = tasks.map((t) => t.id);
    newestFirst(tasks);
    expect(tasks.map((t) => t.id)).toEqual(before);
  });

  it("空列表与单元素照常返回", () => {
    expect(newestFirst([])).toEqual([]);
    expect(newestFirst([row(7)]).map((t) => t.id)).toEqual([7]);
  });
});
