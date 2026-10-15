# Easy视频下载器 —— UI 重构设计（组件库 / 图标库 / Apple 风格 / 移除功能）

日期：2026-10-15
状态：待实施

## 1. 背景

当前的 UI 是 Tauri 迁移时手写的一套界面：React 19 + Vite 6 + Tailwind 4，8 个组件共约 730 行，没有组件库、没有图标库。设计 token 在 `ui/src/index.css`，是一套刻意的纯黑白灰（`--fg: #0a0a0a`、`--line: #ededed`），几乎不使用彩色。

用户提出三件事：

1. 引入组件库和图标库来构建 UI
2. 「从列表移除」按钮点了没有任何反应（bug）
3. 视觉风格向 Apple 靠拢

第 3 条的具体含义在澄清中确定为：**apple.com 的排版气质，但信息密度要高于官网**；同时左侧新增侧边栏，并为以后要加的「视频操作」功能留出位置。

## 2. 需求与澄清结论

| 议题 | 结论 |
|---|---|
| 视觉目标 | apple.com 的排版气质（大字号落差、留白、柔和灰阶），但密度高于官网 |
| 信息架构 | 侧边栏为**导航式**：下载 / 视频操作 / 设置 三个平级页面 |
| 设置 | 从模态对话框改为整页 |
| 「视频操作」 | 处理已下好的视频（转码/压缩/裁剪/提取音频/生成封面）。本期只做占位空状态，骨架按「选文件 → 选操作 → 队列」预留 |
| 图标 | 用图标库解决当前设置图标（`⚙` emoji，跟随 13px 文字）过小的问题 |
| 组件库 | shadcn/ui（Radix Primitives + Tailwind） |
| 图标库 | lucide-react |
| 强调色 | **引入官网蓝 `#0071e3`**（深色模式 `#2997ff`）——这是本次唯一推翻原有「零彩色」设计意图的地方 |
| 页面标题字号 | 22px |
| 「从列表移除」 | **真的移除，重启也不回来** |
| URL 输入框 | 只放在「下载」页；剪贴板提示本期不做 |
| 窗口 `minWidth` | 保持 720 |

## 3. Bug 根因：「从列表移除」无反应

**位置**：`ui/src/components/TaskRow.tsx:140`

**根因**：按钮标签承诺「移除」，`onClick` 实际调用 `ipc.cancel(task.id)`；而**整条链路上不存在「移除」这个操作**：

- `src/core/engine.rs` 公开 API 无 remove/forget
- `src-tauri/src/main.rs:68-79` 注册的 12 个 Tauri 命令无 remove
- 前端 reducer 无 `removeTask` 类 action

因此该按钮**在任何情况下都不可能把行移出列表**。实际效果：

| 行当前状态 | 点击后 |
|---|---|
| 已完成 / 失败 | 状态被改写成 `Canceled`，行仍在，标签从「已完成」变「已取消」 |
| 已经是「已取消」 | 状态无变化 → 字面意义上的「没有任何反应」 |

**顺带排除的怀疑**：最终文件落在 `download_dir/<文件名>`，`task_dir`（`download_dir/task-{id}`）只放分片与 manifest。所以 `cancel()` 中的 `remove_dir_all(task_dir)` **不会删除用户已下载的视频**。但 `TaskRow` 中「cancel 不删盘上的任何东西」这句注释比实际宽——它确实会删任务目录，只是对已结束的任务该目录本来就是空的。

**同类问题**：非终态那个按钮写着「删除」，调的同样是 `cancel()`，而 core **从不删除用户文件**。标签同样是假的，本期一并改为「取消」。

## 4. 架构：骨架与导航

### 现状

```
Toolbar(URL输入 + 下载 + ⚙ + 窗口键)   ← 通栏
TaskList
StatusBar(总速度 / 版本)               ← 通栏
+ SettingsDialog、ChoiceDialog 两个模态 + Toasts 浮层
```

### 新骨架

```
┌───────────┬──────────────────────────────────┐
│  ● ● ●    │  页面标题           [页面级操作]  │
│           ├──────────────────────────────────┤
│  ⬇  下载   │                                  │
│  ✂  视频操作│                                  │
│  ⚙  设置   │           页面内容                │
│           │                                  │
│  ─────────│                                  │
│  2.3 MB/s │                                  │
│  v0.1.0   │                                  │
└───────────┴──────────────────────────────────┘
```

**侧边栏**（宽 200px，`--bg-subtle` 底 + 右侧发丝线）

- 顶部留 52px 给红绿灯，整块作为 `data-tauri-drag-region`（macOS 使用 `titleBarStyle: Overlay`，红绿灯由系统绘制）
- 三个导航项：lucide 图标 + 文字，行高 32px、圆角 6px
- **选中态用浅灰底 `rgba(0,0,0,.06)` + 文字转深，不使用品牌色填充**——Apple 自家的侧边栏（系统设置、访达）即如此，彩色填充会立刻显得不像苹果
- 底部放 `StatusBar` 的**总速度 + 版本号**，与导航之间一条发丝线分隔。这两个是全局信息，放侧边栏底部可省掉一条通栏

**主区**：顶部 PageHeader（页面标题 + 右侧页面级操作），下方可滚动内容区。非 macOS 的窗口控件从 `Toolbar` 挪到 PageHeader 右端。

**三个页面**

- **下载**：页顶为 URL 输入 + 下载按钮（从 `Toolbar` 挪入），下方为任务列表
- **视频操作**：占位空状态
- **设置**：`SettingsDialog.tsx`（273 行）拆为整页

**不动的部分**

- `ChoiceDialog` 保持模态——它是必须回答的阻塞式选择，不该被侧边栏「切走」
- `Toasts` 保持右下浮层

**页面切换不引入 react-router**：3 个页面用 `useState` 即可，桌面 App 没有 URL 需求。

## 5. 视觉语言

### 5.1 灰阶（最要紧的一条）

当前 `--bg-subtle: #fcfcfc` 与 `--fg: #0a0a0a` 几乎纯白配纯黑，撑不起侧边栏与分层。改用 Apple 官网的灰阶：

| 用途 | 现在 | 改为 | 出处 |
|---|---|---|---|
| 主文字 | `#0a0a0a` | `#1d1d1f` | 官网正文色 |
| 次要文字 | `#8a8a8a` | `#6e6e73` | 官网副标题 |
| 弱化文字 | `#b0b0b0` | `#86868b` | 官网页脚 |
| 次级面（侧边栏底） | `#fcfcfc` | `#f5f5f7` | 官网经典灰 |
| 描边 | `#ededed` | `#d2d2d7` | 官网分隔线 |
| 强调色 | 无 | `#0071e3` | 官网按钮蓝 |

深色模式对应：底 `#000000`、次级面 `#1d1d1f`、主文字 `#f5f5f7`、描边 `#424245`、强调色 `#2997ff`。

`--accent-ok` / `--accent-warn` / `--accent-err` 保持不变。

### 5.2 字号阶梯

现在字号全部挤在 12–13px。apple.com 的标志是层级落差大：

| 用途 | 字号 / 字重 / 行高 | 字距 |
|---|---|---|
| 页面标题 | 22 / 600 / 1.2 | `-0.01em` |
| 区块标题 | 15 / 600 / 1.3 | 0 |
| 列表主行 | 13 / 400 / 1.4 | 0 |
| 次要文字 | 12 / 400 / 1.35 | 0 |

正文保持 13px 是刻意的：官网的 17px 正文在 900px 窗口里过于奢侈。**落差靠标题撑，密度靠正文守。**

### 5.3 圆角与形状

- 主操作按钮（下载）：**胶囊** `border-radius: 980px`——官网最 signature 的形状
- 次级控件：6px
- 卡片：12px
- 输入框：8px

### 5.4 少用线，多用面

当前满屏 `1px solid var(--line)`。Apple 几乎不用分隔线，靠 `#f5f5f7` 色块与留白分层。**列表行之间的分隔线保留**（列表需要），但区块之间改用留白与背景色差。

### 5.5 图标

lucide，`stroke-width: 1.75`（默认 2 偏粗像描边图标，1.5 偏细发虚）。侧边栏与行内 16px，页面级 20px。

当前设置图标「太小」的根因是它由 `⚙` emoji 跟随 13px 文字渲染；换成 lucide `Settings` 给 16px 即可解决。

### 5.6 字体

沿用现有的 `-apple-system, "SF Pro Text", "PingFang SC", system-ui, sans-serif`。官网大标题用 SF Pro Display，`-apple-system` 在 macOS 上会自动按字号选择 Display/Text 光学尺寸。

## 6. 组件层

### 6.1 引入的 shadcn 组件

拷入 `ui/src/components/ui/`。**只拷真正需要的**——每一份拷进来的组件都是此后要自己维护的代码：

| 组件 | 用在哪 |
|---|---|
| `button` | 胶囊主按钮 + 次级按钮 |
| `input` | URL 框、设置页输入 |
| `tabs` | 设置页「外观」三选一，改样式为 Apple 分段控件 |

以下原本列在计划里，核对后**决定不拷**，理由逐条记录：

| 组件 | 不拷的理由 |
|---|---|
| `dialog` | 现有 `ChoiceDialog` 是手写的，而且带着"同一时刻只弹一个、Esc 只压下不取消"这类精心处理过的语义（注释里写明了不这么做的后果）。换 shadcn 的 Dialog 等于把这些逻辑重写一遍。 |
| `progress` | 现有进度条对 `merging` 状态有特殊表现（填满且转绿），shadcn 的 `Progress` 没有这个概念。 |
| `select` | 唯一需要下拉的是「外观」，而它改用 `tabs` 了；其余数值输入本来就是原生 `<input type="number">`。 |
| `tooltip` | 侧边栏与行内按钮的文字标签本来就可见，加提示是多余的。 |
| `separator` | 一行 `<div style={{borderTop}}>` 就够，不值一份组件。 |

`form`、`table`、`navigation-menu` 等桌面 App 用不上的，自然更不拷。

### 6.2 图标映射（lucide）

```
侧边栏   下载 Download │ 视频操作 Film │ 设置 Settings
任务行   暂停 Pause │ 继续 Play │ 重试 RotateCcw │ 移除 X │ 选择格式 ListChecks
其它     Toast 关闭 X │ 非 macOS 窗口键 Minus / Square / X
```

### 6.3 两个接线坑

1. shadcn 需要路径别名 `@/`，必须**同时**修改 `vite.config.ts` 的 `resolve.alias` 与 `tsconfig.json` 的 `paths`。漏一处即为构建期报错。
2. shadcn 组件自带一套 `--primary` / `--radius` 变量体系，**必须映射到第 5 节的 token**。不映射则仓库内同时存在两套颜色变量，之后改主题必然漏掉一边。

## 7. core 变更：`remove_task`

```rust
pub fn remove_task(&self, id: TaskId) -> bool
```

必须做满三件事：

1. **只接受「终态 且 没有 worker 在跑」**——即 `is_terminal(state) && workers == 0`。其余返回 `false`。缺少 `workers == 0` 这一半会打开两条危险路径，见 §7.1。
2. **删除任务目录** `actual_task_dir(id)`。**对 `Failed` 是必需的**——见 §7.2；对 `Done` / `Canceled` 该目录本已为空，删了无害。
3. 从 `inner.tasks` 移除，并广播新事件 `EngineEvent::TaskRemoved { id }`。

判定与移除必须在**同一个 `tasks` 锁**内完成：`retry()` 会把 `Failed` 挪回 `Queued`，先查后删之间留个窗口的话，用户刚重试的任务会被无声抹掉。

`actual_task_dir` 要在条目消失**之前**取——它读的是条目上的 `dir` 记录，用户可能中途改过保存位置。

### 7.1 准入条件里为什么必须有 `workers == 0`

原本只写「终态」是不够的。这是一个**审查阶段才发现**的缺口，写清楚以免被后来的人当作冗余"简化"掉。

`cancel()` 是**立刻**把状态置成 `Canceled` 的，而 worker 可能还活着——`probe()` 不接受取消令牌，worker 只能等那个网络往返自己跑完。于是「取消 → 立刻移除」会删掉一个仍然活着的 worker 脚下的条目：

- **panic**：worker 回来走到 `paths()`，那里是 `.expect("task entry present")`（四处：`engine.rs` 619 / 718 / 806 / 826）——条目没了就 panic。这四处 `expect` 本来就是按「有 worker ⟹ 条目还在」写的，新 API 把这个前提破坏了。
- **复活**：若移除恰好落在 `paths()` 与 `write_manifest` 之间，`write_manifest` 会把目录重新建出来；而 worker 随后的错误收尾判据是 `state_of(id) == Some(Canceled)`——条目已没了，判据为假，**没有任何人再删这个目录**。结果：被「移除」的任务留下 manifest，重启时 `scan_resume` 把它复活。

第二条正是本设计要消灭的那件事，被新 API 从另一条路重新打开了。

`workers == 0` 这道闸把两条路一起关掉：只要还有 worker 活着，条目就不可能被删，四处 `expect` 因此重新变得安全。判定与移除同锁，且终态任务不会被 `pump()` 启动（`pump` 只启动 `Queued`），所以过了这道闸就真的不会有新 worker 冒出来。

**代价**：取消一个正卡在网络上的任务后立刻点「移除」，会得到一次失败提示，需要等 worker 退出后重试。这个窗口通常毫秒级，探测请求慢时可能到秒级。这个取舍是划算的——**一个会说"稍后再试"的按钮，好过一个会 panic 或悄悄复活任务的按钮。**

### 7.2 「重启也不回来」的约束来源

`src/core/engine.rs:572-575`：任务失败时**只置 `Failed`，不清任务目录**（这是刻意的，为了下次可续传）。因此 `Failed` 任务的 `manifest.json` 仍留在盘上。一个只删内存条目的 `remove_task`，重启后会被 `scan_resume` 捞回来并 `resume()` 回列表——**「重启也不回来」对失败任务不成立**。所以删除目录是必需动作，不是可选清理。

### 7.3 `WorkerGuard::drop` 的顺序要调整

```rust
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        // 先释放占用，**再**减计数。
        self.engine.release_reserved_path(self.id);
        self.workers.fetch_sub(1, Ordering::SeqCst);
        self.engine.pump();
    }
}
```

原顺序是「减计数 → 释放占用 → `pump`」。把释放提到减计数之前，是为了建立一条不变量：

> **`workers == 0` ⟹ 这个 worker 的 `reserved_path` 占位已经释放干净。**

有了它，`remove_task` 就不必自己管占位——它读到 `workers == 0` 时释放必然已完成。若保持原顺序，会留下一个"减了计数但还没释放"的窗口，`remove_task` 就必须在自己的临界区里额外处理它，而那个窗口窄到写不出确定性测试。

影响面是闭合的：`release_reserved_path` 全文只有 `WorkerGuard::drop` 这一个调用点，`workers` 也只有 `drop` 一处减、`pump()` 一处增。原注释要求的「释放必须在 `pump()` 之前」仍然满足。

### 7.4 配套改动

- `src-tauri/src/main.rs` 注册命令 `remove_task`
- `ui/src/lib/ipc.ts` 增加 `removeTask`
- reducer 增加 `taskRemoved` 分支（`tasks.filter` 掉）

## 8. 错误处理

`Engine::remove_task` 返回 `bool`；Tauri 命令层包成 `Result<(), String>`，被拒时返回 `Err("无法移除：它可能仍在运行或收尾，也可能已不在列表里")`。前端直接走现成的 `run()` → 右下角 toast，**不为它新写错误路径**。

**文案必须盖住 `false` 的全部三种来源**——这一点容易漏，初稿就只写了后两种：

1. **任务不存在**：双击时第二次点击、或行已经过期。
2. 状态不是终态。
3. 是终态、但 worker 还在收尾。

漏掉第 1 种的后果是文案变成假话：任务已经没了，"稍后再试"却暗示重试能成功——永远不会。

UI 层仍会把非终态按钮渲染成「取消」而非「移除」，所以第 2 种只在竞态下触发。第 1 种则在双击时就会命中（两次点击都落在同一行消失之前），是三种里最容易碰到的。

## 9. 测试策略

### Rust（`cargo test`）—— 新增 3 条

1. `remove_terminal_task_but_reject_active`——终态可移除，`Queued` / `Downloading` 被拒。
2. **`removed_failed_task_is_not_resurrected_by_scan_resume`**——最有价值的一条。造一个失败任务（有 manifest 落盘）→ 移除 → 断言 `scan_resume` 扫不到它。这是防「重启又回来」的回归测试，**没有它 §7.2 那个坑随时会被改回去**。
3. **`remove_is_rejected_while_a_worker_is_still_alive`**——钉住 §7.1 的准入闸。让一个任务卡在延迟响应上（`Downloading`），`cancel()` 之后状态立刻是 `Canceled` 而 worker 还活着：此时移除应返回 `false`；等 worker 退出后再移除应返回 `true`，且盘上不留 manifest。**这条直接对着「取消后立刻点移除」那个手势**——它是 §7.1 里两条危险交错（`paths()` 的 panic、`write_manifest` 的复活）的唯一入口。

### 前端（`npm --prefix ui test`）—— 新增 2 条

4. reducer `taskRemoved`：已知 id 从 `tasks` 消失；未知 id 置 `needsResync`（与现有未知 id 行为一致）。
5. 把「终态→移除 / 非终态→取消」的判断抽成 `lib/` 下的纯函数并单测。**必须同时钉住 `label` 与 `invoke`**：只断言 `label` 的话，把 `invoke` 换成另一个命令会让原 bug 原样复活而全部测试仍然通过——测试要守的正是这两者的**对应关系**，不是其中一半。

**刻意贴着既有风格**：`reducer.ts` / `format.ts` / `url.ts` 都是纯逻辑进测试、组件层不进测试。当前没有 `@testing-library/react`，**不为此引入组件测试框架**。

### 视觉

没有自动化手段能验证「像不像 Apple」，只能人工核对。实施后提供一份对照清单（灰阶值、字号阶梯、胶囊按钮、选中态）逐条走一遍。

## 10. 实施顺序

```
① core: remove_task + 3 条 Rust 测试        ← 独立、可先跑通、不碰 UI
② IPC 打通: 命令 + ipc.ts + reducer + 2 条前端测试
③ 骨架: 侧边栏 + 3 页面切换 + 设置改整页（先不动视觉）
④ 视觉: 灰阶/字号/圆角 token 替换 + shadcn 组件逐个换装
```

①② 完全独立于视觉改造。先落地的话，即使后面视觉做四不像，坏掉的按钮也已经修好了。

## 11. 验收标准

- `cargo test` 全绿
- `npm --prefix ui test` 全绿
- `npm run build` 全绿（含 `tsc --noEmit`）
- 视觉核对清单逐条通过
- 手动验证：「从列表移除」在已完成 / 失败 / 已取消三种行上都生效，且**重启后确实不再出现**

## 12. 本期明确不做（YAGNI）

- 「视频操作」的实际功能（本期只做占位页与骨架）
- 剪贴板链接检测与自动跳转
- 非终态任务的「直接移除」
- `@testing-library/react` 与组件测试
- react-router
- 窗口 `minWidth` 调整（保持 720）
