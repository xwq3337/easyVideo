/**
 * 把任务列表翻成"最新的在最上面"。
 *
 * core 的 `list_tasks()` 按 id 升序返回（`sort_by_key(|s| s.id)`），而 id 是单调递增的，
 * 所以翻转即可 —— 不在这里重新排序，是为了让"顺序由谁定"这件事只有一处答案：
 * core 定序，界面只决定朝哪头显示。
 *
 * **必须复制再翻**：传进来的是 store 里那份 `state.tasks`，原地 `reverse()` 会改掉
 * 快照本身。`useSyncExternalStore` 靠引用相等判断有没有变化，就地改会让 React 读到
 * "引用没变、内容却变了"的快照 —— 界面不重绘，而内存里的顺序已经乱了。
 */
export function newestFirst<T>(tasks: readonly T[]): T[] {
  return [...tasks].reverse();
}
