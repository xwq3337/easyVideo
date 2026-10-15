# Apple 风格 UI 重构与 remove_task 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修好同名不副实的「从列表移除」按钮（core 新增真正的移除能力），并把界面重构为侧边栏导航 + apple.com 视觉语言。

**Architecture:** 分三段推进。① 在 `Engine` 上新增 `remove_task` 与 `TaskRemoved` 事件，打通 Rust → IPC → reducer → 组件的完整链路，独立可发布；② 引入 shadcn/ui 与 lucide，搭出侧边栏 + 三页导航的骨架；③ 用 apple.com 的灰阶、字号阶梯与胶囊形状替换现有 token。

**Tech Stack:** Rust（Tauri 2 / tokio / broadcast）、React 19、Vite 6、Tailwind 4、shadcn/ui、lucide-react、vitest、wiremock。

**Spec:** `docs/superpowers/specs/2026-10-15-apple-ui-redesign-design.md`

## Global Constraints

- `Engine` 是任务的唯一真相源；前端不做乐观更新，一律等 core 事件或重新 `get_tasks`。
- `remove_task` **只接受终态**（`Done` / `Failed(_)` / `Canceled`），其余返回 `false`。
- **任务目录清理是不可省略的动作，不是可选清理。** `Failed` 的任务在收尾时刻意保留 manifest（为了下次续传，见 `engine.rs:572-575`），只删内存条目会让 `scan_resume` 在重启后把它复活。
- `release_reserved_path` 必须在**移除条目之前**调用（`engine.rs:848` 读的是条目上的字段）。参见下方 Review Focus 第 3 条。
- 命令层薄封装，业务逻辑一律在 core；`src-tauri/src/commands.rs` 无单测，由 `cargo check --workspace` 与手动验收覆盖。
- 前端纯逻辑进测试（`lib/`、`store/reducer.ts`），组件层不进测试。**不引入 `@testing-library/react`。**
- 视觉 token 的权威定义在 `ui/src/index.css`。shadcn 自带变量必须映射到这套 token，不允许仓库内并存两套颜色变量。
- 主文字 `#1d1d1f`、次要 `#6e6e73`、弱化 `#86868b`、次级面 `#f5f5f7`、描边 `#d2d2d7`、强调 `#0071e3`；深色对应 `#f5f5f7` / `#86868b` / `#424245` / `#1d1d1f` / `#000000` / `#2997ff`。
- 字号阶梯：页面标题 22/600/1.2/`-0.01em`，区块标题 15/600/1.3，列表主行 13/400/1.4，次要 12/400/1.35。
- 图标一律 lucide，`stroke-width: 1.75`；侧边栏与行内 16px，页面级 20px。
- 每个任务结束时 `cargo test`、`npm --prefix ui test`、`npm run build` 三者必须全绿。

## Review Focus

以下五类输入/时序，spec 隐含要求但没有现成测试覆盖，按最可能伤到用户的顺序排列。每条都在“归属任务”里配了测试。

1. **对一个正在下载的任务点「移除」**（按钮尚未变灰的竞态）——必须被拒绝并回一条可读提示，绝不能删掉正在写的分片。→ Task 1
2. **移除一个失败任务后重启应用**——`scan_resume` 不得把它捞回来。失败任务刻意留了 manifest，这是最容易被改回去的一条。→ Task 1
3. **在 worker 还没退出时移除一个刚被取消的任务**——`reserved_paths` 占位必须仍然被释放，否则之后同名文件会莫名变成 `xxx (1).mp4`。→ Task 1
4. **`taskRemoved` 事件到达一个前端不认识的 id**——必须触发重同步，不得静默忽略，否则列表与 core 永久不一致。→ Task 3
5. **对一条已经「已取消」的行点按钮**——必须真的从列表里消失。这是本次 bug 的原始症状，也是验收的底线。→ Task 4

---

## Task 1: core 新增 `remove_task` 与 `TaskRemoved` 事件

**Files:**
- Modify: `src/core/events.rs`（`EngineEvent` 枚举）
- Modify: `src/core/engine.rs`（新增 `is_terminal` 与 `remove_task`，紧挨 `release_reserved_path` 之前插入）
- Modify: `src-tauri/src/dto.rs`（`EventDto` 新增变体 + `event_dto` 匹配臂 + 测试）
- Test: `tests/engine_test.rs`（文件末尾追加 3 条）
- Test: `src-tauri/src/dto.rs` 的 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `Engine::actual_task_dir(id) -> PathBuf`（已有，`engine.rs:830`）、`Engine::inner.reserved_paths`（已有）、`Engine::inner.tx`（已有）
- Produces:
  - `Engine::remove_task(&self, id: TaskId) -> bool`
  - `EngineEvent::TaskRemoved { id: TaskId }`
  - `EventDto::TaskRemoved { id: u64 }`，线格式为 `{"type":"taskRemoved","id":N}`

- [ ] **Step 1: 先写失败测试——终态可移除、非终态被拒**

在 `tests/engine_test.rs` 末尾追加。注意用 `max_active_tasks = 0` 让任务停在 `Queued`，不联网：

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_terminal_task_but_reject_active() {
    // 终态才是"已经结束"。正在跑的任务语义含糊 —— 盘上的 .part 分片删不删?
    // 删了是数据丢失,不删是磁盘泄漏 —— 所以一律拒绝,让调用方先取消。
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s = settings_with(dir.path());
    s.max_active_tasks = 0; // 不启动任何 worker,任务停在 Queued
    let engine = Engine::new(rt, s, dir.path().join("data"));

    let id = engine.add_url("https://example.com/a.mp4".into());
    assert!(!engine.remove_task(id), "排队中的任务不该能被移除");
    assert_eq!(engine.list_tasks().len(), 1, "被拒的移除不该改动列表");

    engine.cancel(id); // 排队中被取消会直接落成 Canceled
    assert!(engine.remove_task(id), "已取消的任务应当能移除");
    assert_eq!(engine.list_tasks().len(), 0, "移除后列表里不该还有它");
    assert!(!engine.remove_task(id), "重复移除应当返回 false");
}
```

- [ ] **Step 2: 运行确认它失败**

Run: `cargo test --test engine_test remove_terminal_task_but_reject_active`
Expected: 编译失败，`no method named 'remove_task' found for struct 'Engine'`

- [ ] **Step 3: 加事件变体**

`src/core/events.rs`，在 `EngineEvent` 枚举的 `ChoicesOffered` 之后加：

```rust
    /// 任务被移出列表。前端据此把行删掉，而不是靠请求的返回值自己猜。
    TaskRemoved { id: TaskId },
```

- [ ] **Step 4: 补 dto 映射（否则无法编译——`event_dto` 的 match 不再穷尽）**

`src-tauri/src/dto.rs`，`EventDto` 枚举末尾加：

```rust
    TaskRemoved { id: u64 },
```

`event_dto` 的 match 末尾加：

```rust
        EngineEvent::TaskRemoved { id } => EventDto::TaskRemoved { id },
```

- [ ] **Step 5: 实现 `remove_task`**

`src/core/engine.rs`，插在 `release_reserved_path`（第 842 行那段文档注释）**之前**：

```rust
    /// 把一条**已结束**的任务从列表里彻底移除。
    ///
    /// 只接受终态。正在跑的任务语义含糊：盘上的 `.part` 分片删不删？删了是数据丢失，
    /// 不删是磁盘泄漏。所以一律拒绝，让调用方先把它取消掉。
    ///
    /// **必须连任务目录一起删。** `Failed` 的任务在收尾时**刻意保留** manifest
    /// （为了下次能续传，见 worker 的错误分支：那里只 `set_state(Failed)`，不碰盘）。
    /// 只删内存条目的话，重启时 `scan_resume` 会把它捞回来并 `resume()` 回列表 ——
    /// 用户明明"移除"过的任务又出现了。
    ///
    /// 返回 `false` 表示任务不存在，或不是终态。
    pub fn remove_task(&self, id: TaskId) -> bool {
        // 两个字段都读自条目本身，所以必须在条目消失之前取出来。
        // `actual_task_dir` 用的是任务**真正**写入的那个目录：用户可以在任务跑着的
        // 时候改保存位置，退回 `current_settings().download_dir` 会删错地方。
        let tdir = self.actual_task_dir(id);
        let (reserved, removed) = {
            let mut tasks = self.inner.tasks.lock().unwrap();
            // 判定与移除在**同一个锁**里完成：`retry()` 会把 `Failed` 挪回 `Queued`，
            // 先查后删之间留个窗口的话，用户刚重试的任务会被无声抹掉。
            if !tasks
                .get(&id)
                .map(|t| is_terminal(&t.state))
                .unwrap_or(false)
            {
                (None, false)
            } else {
                let reserved = tasks.get_mut(&id).and_then(|t| t.reserved_path.take());
                tasks.remove(&id);
                (reserved, true)
            }
        };
        if !removed {
            return false;
        }
        // 释放必须由这里做完。`WorkerGuard::drop` 也会释放，但它读的同样是条目上的
        // 字段 —— 条目已被我们删掉，那一次会静默落空。而 `cancel()` 是**立刻**把状态
        // 置成 `Canceled` 的：worker 可能还挂在网络请求上没退出，于是"移除一个刚取消
        // 的任务"正好落在这个窗口里，占位会**因为这个**泄漏，之后同名文件莫名变成
        // `v (1).mp4`。
        if let Some(p) = reserved {
            self.inner.reserved_paths.lock().unwrap().remove(&p);
        }
        std::fs::remove_dir_all(&tdir).ok();
        let _ = self.inner.tx.send(EngineEvent::TaskRemoved { id });
        true
    }
```

在 `TaskEntry` 的 `impl` 附近（`fn is_active` 之后，`engine.rs:107` 左右）加自由函数：

```rust
/// 已经走到头、不会再被 worker 推进的状态。`remove_task` 只认这几个。
fn is_terminal(state: &TaskState) -> bool {
    matches!(
        state,
        TaskState::Done | TaskState::Failed(_) | TaskState::Canceled
    )
}
```

- [ ] **Step 6: 运行确认通过**

Run: `cargo test --test engine_test remove_terminal_task_but_reject_active`
Expected: PASS

- [ ] **Step 7: 写第二条失败测试——失败任务移除后不得被复活**

这是本任务最重要的一条。追加到 `tests/engine_test.rs` 末尾：

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removed_failed_task_is_not_resurrected_by_scan_resume() {
    // 失败的任务在收尾时**刻意保留** manifest（为了下次能续传）。所以"从列表移除"
    // 若只删内存条目，重启时 scan_resume 会把它捞回来 —— 用户移除过的任务又出现了。
    // 这条测试钉住的就是"移除必须连目录一起清"。
    //
    // 关键是让失败发生在 **manifest 落盘之后**：probe 请求（bytes=0-63）必须成功，
    // 随后的分块下载才失败。若让 probe 自己 404，manifest 根本不会写，这条测试就空跑了。
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-63"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/4096")
                .set_body_bytes(vec![0u8; 64]),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let engine = Engine::new(rt, settings_with(dir.path()), dir.path().join("data"));
    let mut rx = engine.events();

    let id = engine.add_url(format!("{}/v.mp4", server.uri()));
    wait_failed(&mut rx, id, 30).await;

    // 前提检查：失败任务确实留下了 manifest。不成立的话这条测试什么也没测到。
    assert!(
        easydownvideo::core::task::scan_resume(dir.path())
            .iter()
            .any(|m| m.id == id),
        "失败的任务本应留下可恢复的 manifest，前提不成立"
    );

    assert!(engine.remove_task(id), "失败的任务应当能移除");
    assert!(
        !easydownvideo::core::task::scan_resume(dir.path())
            .iter()
            .any(|m| m.id == id),
        "移除后盘上仍有它的 manifest，重启会把这个任务复活"
    );
}
```

- [ ] **Step 8: 运行它，应当直接通过**

Run: `cargo test --test engine_test removed_failed_task_is_not_resurrected_by_scan_resume`
Expected: PASS

`remove_task` 在 Step 5 已经实现，所以这条写完就是绿的 —— 这里没有"先看它红"的环节。

- [ ] **Step 9: 做一次负向验证，证明这条测试不是空跑**

一条写完即绿的测试有两种可能：它真的在测那行代码，或者它什么都没测。必须区分开。

临时把 Step 5 里这两行注释掉：

```rust
        // ACTUALLY 不要连目录一起删
        // std::fs::remove_dir_all(&tdir).ok();
```

再跑：

Run: `cargo test --test engine_test removed_failed_task_is_not_resurrected_by_scan_resume`
Expected: FAIL，且失败信息是 `移除后盘上仍有它的 manifest，重启会把这个任务复活`

看到这个失败，就证明了测试确实钉在"删目录"那个动作上。**验证完立刻把注释恢复。**

- [ ] **Step 10: 恢复后确认重新变绿**

Run: `cargo test --test engine_test removed_failed_task_is_not_resurrected_by_scan_resume`
Expected: PASS

- [ ] **Step 10: 写第三条失败测试——worker 还在退出时移除，占位仍要释放**

追加到 `tests/engine_test.rs` 末尾：

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_releases_reserved_path_while_the_worker_is_still_exiting() {
    // 钉一个很窄但要命的窗口：`cancel()` 让状态**立刻**变成 Canceled，而 worker
    // 可能还挂在网络请求上没退出。此时移除会先删掉条目，随后 WorkerGuard 的释放
    // 在 `get_mut(&id)` 上落空 —— 于是那个"已分配但还没落盘"的占位**永久泄漏**，
    // 之后下同名文件会被算成冲突，用户莫名拿到 `v (1).mp4`。
    //
    // **不能写成"移除后再等 worker 退出"**：那样 WorkerGuard 已经把占位释放干净，
    // 测试会变成空跑。必须在 worker 仍被延迟响应卡住时就移除。
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .and(header("range", "bytes=0-63"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-63/4096")
                .set_body_bytes(vec![0u8; 64]),
        )
        .mount(&server)
        .await;
    // 全量请求延迟返回，把 worker 卡在这里
    Mock::given(method("GET"))
        .and(path("/v.mp4"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("content-type", "video/mp4")
                .insert_header("content-range", "bytes 0-4095/4096")
                .set_body_bytes(vec![0u8; 4096])
                .set_delay(std::time::Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Handle::current();
    let mut s = settings_with(dir.path());
    s.max_active_tasks = 2; // 第二个任务要在第一个 worker 还卡着的时候就能跑
    let engine = Engine::new(rt, s, dir.path().join("data"));
    let mut rx = engine.events();

    let first = engine.add_url(format!("{}/v.mp4", server.uri()));
    // 进入 Downloading 说明 paths() 已经跑过、占位已经登记
    wait_state(&mut rx, first, TaskState::Downloading, 10).await;

    engine.cancel(first); // 状态立刻变 Canceled，worker 还卡在那个延迟响应上
    assert!(engine.remove_task(first), "已取消的任务应当能移除");

    let second = engine.add_url(format!("{}/v.mp4", server.uri()));
    // 等第二个任务走到写 manifest 那一步 —— 那时它已经算过自己的最终路径
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let assigned = loop {
        if let Some(m) = easydownvideo::core::task::scan_resume(dir.path())
            .into_iter()
            .find(|m| m.id == second)
        {
            break m.filename;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "第二个任务一直没落下 manifest"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(
        assigned, "v.mp4",
        "占位没释放：同名文件被算成了冲突，第二个任务拿到 {assigned}"
    );
}
```

- [ ] **Step 11: 运行确认它失败**

Run: `cargo test --test engine_test remove_releases_reserved_path_while_the_worker_is_still_exiting`
Expected: 先临时把 Step 5 里的 `if let Some(p) = reserved { ... }` 整段注释掉再跑，应该 FAIL 并显示 `第二个任务拿到 v (1).mp4`。这条负向验证很重要——它证明测试确实在测那行代码。验证完把注释恢复。

- [ ] **Step 12: 恢复实现并确认通过**

Run: `cargo test --test engine_test remove_releases_reserved_path_while_the_worker_is_still_exiting`
Expected: PASS

- [ ] **Step 13: 写事件线格式的 pin 测试**

`src-tauri/src/dto.rs` 的 `mod tests` 末尾追加。用整对象相等——多一个键、少一个键、大小写漂移都要红：

```rust
    /// `taskRemoved` 是前端 reducer 必须 switch 的第三条事件。它的 `type` 标签是
    /// 多词 camelCase，最容易在改名时静默漂移，而漂移的后果是**行永远删不掉**
    /// （reducer 的 default 分支吞掉它）—— 正是这次要修的 bug 的翻版。
    #[test]
    fn task_removed_event_pins_its_tag_and_id() {
        let d = event_dto(EngineEvent::TaskRemoved { id: 42 });
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({ "type": "taskRemoved", "id": 42 })
        );
    }
```

- [ ] **Step 14: 跑完整测试套件**

Run: `cargo test --workspace`
Expected: 全绿。若 `dto.rs` 那条红，检查 `event_dto` 的匹配臂是否漏了 `TaskRemoved`。

- [ ] **Step 15: 提交**

```bash
git add src/core/events.rs src/core/engine.rs src-tauri/src/dto.rs tests/engine_test.rs
git commit -m "feat(core): 新增 remove_task,把任务真正移出列表

core 此前没有"移除"这个概念 —— 界面上那个按钮调的是 cancel(),
对已取消的行连状态都不变,是纯零反应。

三件事必须一起做:只接受终态、删任务目录、释放 reserved_paths 占位。
其中删目录不是可选清理:失败的任务刻意保留 manifest 以便续传,只删
内存条目的话重启时 scan_resume 会把它捞回来。

占位释放的理由与直觉相反:WorkerGuard 本来就会释放,但 cancel() 是
立刻置 Canceled 的,而 worker 可能还挂在网络上;此时移除先删了条目,
guard 那次释放会落空。所以必须自己做完。"
```

---

## Task 2: 注册 Tauri 命令并打通前端 IPC 契约

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs:67-79`
- Modify: `ui/src/lib/ipc.ts`

**Interfaces:**
- Consumes: `Engine::remove_task(id) -> bool`（Task 1）
- Produces:
  - Tauri 命令 `remove_task(id: u64) -> Result<(), String>`
  - `ipc.removeTask(id: number): Promise<void>`

- [ ] **Step 1: 加命令**

`src-tauri/src/commands.rs`，在 `cancel` 之后插入：

```rust
/// 把一条**已结束**的任务移出列表。非终态返回 `Err`——界面到时已经按状态把按钮
/// 渲染成「取消」了，所以这条只会在竞态下（状态刚变、按钮还没重渲染）被碰到。
///
/// 返回 `Result` 而不是 `bool`：前端全走 `run()` 那一条错误路径，命令级失败自动落成
/// 一条 toast，不必为它新写分支。
#[tauri::command]
pub fn remove_task(state: State<'_, AppState>, id: u64) -> R<()> {
    if state.engine.remove_task(id) {
        Ok(())
    } else {
        Err("只能移除已结束的任务".to_string())
    }
}
```

- [ ] **Step 2: 注册命令**

`src-tauri/src/main.rs`，在 `commands::cancel,` 之后加一行：

```rust
            commands::remove_task,
```

- [ ] **Step 3: 确认 Rust 侧编译并全绿**

Run: `cargo test --workspace`
Expected: 全绿（`commands.rs` 无单测，这一条覆盖的是"编译得过去"）。

- [ ] **Step 4: 加前端 IPC**

`ui/src/lib/ipc.ts`，在 `cancel` 那一行之后加：

```ts
export const removeTask = (id: number) => invoke<void>("remove_task", { id });
```

- [ ] **Step 5: 确认前端构建仍是绿的**

Run: `npm --prefix ui test && npm --prefix ui run build`
Expected: 全绿

**注意：本任务不动 `ui/src/lib/types.ts`。** `EventDto` 的 `taskRemoved` 成员挪到 Task 3 与 reducer 一起加 —— 在这里单独加会让 `applyEvent` 的 `switch` 不再穷尽、`tsc` 直接报错，于是这个任务以红收场，违反 Global Constraints 里"每个任务结束时三者全绿"。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs ui/src/lib/ipc.ts
git commit -m "feat(ipc): 注册 remove_task 命令并加前端 ipc.removeTask

只做命令与 IPC 的接线,事件类型留给下一个任务和 reducer 一起加 ——
单独加会让 applyEvent 的 switch 不再穷尽、tsc 报错,
这个任务就会以红收场。"
```

---

## Task 3: reducer 处理 `taskRemoved`

**Files:**
- Modify: `ui/src/lib/types.ts`（`EventDto` 加成员）
- Modify: `ui/src/store/reducer.ts`
- Test: `ui/src/store/reducer.test.ts`

**Interfaces:**
- Consumes: `EventDto`（Task 2 未改动它，本任务自己加）
- Produces: 无新导出

- [ ] **Step 1: 给 `EventDto` 加事件成员**

`ui/src/lib/types.ts`，`EventDto` 联合类型末尾加一个成员：

```ts
export type EventDto =
  | { type: "stateChanged"; id: number; state: StateDto }
  | { type: "progress"; id: number; downloaded: number; total: number; unit: UnitDto }
  | { type: "choicesOffered"; id: number; kind: ChoiceKindDto }
  | { type: "taskRemoved"; id: number };
```

改完先跑一次 `npm --prefix ui run build`，此时**应当报错**：`reducer.ts` 的 `applyEvent` 里那个 `switch (event.type)` 不再穷尽，`tsc` 会在函数末尾报「并非所有代码路径都有返回值」。

这是**想要的**——类型系统正把这个新事件推到 reducer 面前，而不是让它在 `default` 分支里被静默吞掉。下一步就修它。

- [ ] **Step 2: 写失败测试**

`ui/src/store/reducer.test.ts` 末尾追加。先看该文件顶部已有的辅助函数（`reduce`、`initialState`、构造 `TaskView` 的写法），沿用它的风格。若没有现成的行构造器，就照下面这样内联：

```ts
function view(id: number, kind: StateDto["kind"] = "done"): TaskView {
  return {
    id,
    url: `https://example.com/${id}.mp4`,
    filename: `${id}.mp4`,
    state: { kind } as StateDto,
    downloaded: 0,
    total: 0,
    unit: "bytes",
    choices: null,
    editing: false,
    speedBps: null,
    etaText: null,
    lastTickAt: 0,
  };
}

describe("taskRemoved", () => {
  it("把已知 id 从列表里删掉", () => {
    const s = { ...initialState, tasks: [view(1), view(2)] };
    const next = reduce(s, { kind: "event", event: { type: "taskRemoved", id: 1 } });
    expect(next.tasks.map((t) => t.id)).toEqual([2]);
  });

  it("未知 id 触发重同步,而不是静默忽略", () => {
    // 静默忽略会让列表与 core 永久不一致:core 里已经没有这条了,
    // 界面上却还挂着一个再也点不动的行。
    const s = { ...initialState, tasks: [view(1)] };
    const next = reduce(s, { kind: "event", event: { type: "taskRemoved", id: 99 } });
    expect(next.tasks.map((t) => t.id)).toEqual([1]);
    expect(next.needsResync).toBe(true);
  });
});
```

- [ ] **Step 3: 运行确认失败**

Run: `npm --prefix ui test`
Expected: FAIL —— `taskRemoved` 事件落进 `applyEvent` 的 switch 后没有匹配的分支。

- [ ] **Step 4: 实现**

`ui/src/store/reducer.ts`，`Action` 联合类型加一个自有用处不大的成员是不必要的——`taskRemoved` 只从事件流来。直接在 `applyEvent` 里处理：

在 `applyEvent` 开头、`const known = ...` **之前**插入：

```ts
  // 移除是唯一一条"任务会从列表里消失"的事件,它必须在 `known` 检查之前处理:
  // 已移除的 id 本来就不在 tasks 里,走下面那条路会被当成"未知 id"而触发一次
  // 毫无必要的重同步。
  if (event.type === "taskRemoved") {
    const exists = state.tasks.some((t) => t.id === event.id);
    if (!exists) {
      // 没这个 id:可能事件先于 hydrate 到达。与其它事件同义 —— 拉一次全量对齐。
      return { ...state, needsResync: true };
    }
    return {
      ...state,
      tasks: state.tasks.filter((t) => t.id !== event.id),
      // 顺手把"稍后再说"的抑制记录里那条也放掉,否则 id 若被复用会残留。
      dismissedChoices: state.dismissedChoices.filter((id) => id !== event.id),
      needsResync: false,
    };
  }
```

- [ ] **Step 5: 运行确认通过**

Run: `npm --prefix ui test`
Expected: 全绿

- [ ] **Step 6: 确认构建恢复**

Run: `npm --prefix ui run build`
Expected: 绿（Step 1 引进的那个 `switch` 不穷尽报错在这里消掉）

- [ ] **Step 7: 提交**

```bash
git add ui/src/lib/types.ts ui/src/store/reducer.ts ui/src/store/reducer.test.ts
git commit -m "feat(ui): reducer 处理 taskRemoved 事件

事件类型和 reducer 一起加:单独加类型会让 applyEvent 的 switch
不再穷尽、tsc 直接报错,中间那一版就是红的。

处理放在 known 检查之前:已移除的 id 本来就不在 tasks 里,
走未知 id 那条路会白拉一次全量。但真的是未知 id 时仍然要重同步 ——
静默忽略会让列表和 core 永久不一致。"
```

---

## Task 4: 修正 TaskRow 的按钮语义（bug 收尾）

**Files:**
- Create: `ui/src/lib/taskAction.ts`
- Create: `ui/src/lib/taskAction.test.ts`
- Modify: `ui/src/components/TaskRow.tsx`（底部按钮那一块，约 130-145 行）

**Interfaces:**
- Consumes: `ipc.removeTask` / `ipc.cancel`（Task 2、Task 1）
- Produces: `taskAction(kind: StateDto["kind"]) -> { label: string; invoke: (id: number) => Promise<void> }`

- [ ] **Step 1: 写失败测试**

`ui/src/lib/taskAction.test.ts`：

```ts
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
```

- [ ] **Step 2: 运行确认失败**

Run: `npm --prefix ui test`
Expected: FAIL —— 找不到模块 `./taskAction`

- [ ] **Step 3: 实现纯函数**

`ui/src/lib/taskAction.ts`：

```ts
import type { StateDto } from "./types";
import * as ipc from "./ipc";

/** 已经走到头、不会再产出下载的状态。对这些行,按钮只把它移出列表,不删任何文件。 */
const TERMINAL: ReadonlyArray<StateDto["kind"]> = ["done", "failed", "canceled"];

/**
 * 一行右下角那个按钮该叫什么、该调什么。
 *
 * 抽成纯函数是为了能单测:组件层不进测试(见 spec §9),但"终态归移除、其余归取消"
 * 这条判断是**界面契约的一部分**,之前正是因为写错才出现了"点了没反应"的假按钮。
 */
export function taskAction(kind: StateDto["kind"]): {
  label: string;
  invoke: (id: number) => Promise<void>;
} {
  if (TERMINAL.includes(kind)) {
    return { label: "从列表移除", invoke: ipc.removeTask };
  }
  // 非终态叫「取消」而不是「删除」:它调的就是 cancel(),而 core 从不删用户文件。
  return { label: "取消", invoke: ipc.cancel };
}
```

- [ ] **Step 4: 运行确认通过**

Run: `npm --prefix ui test`
Expected: 全绿

- [ ] **Step 5: 接进组件**

`ui/src/components/TaskRow.tsx`：

顶部 import 区加：

```ts
import { taskAction } from "../lib/taskAction";
```

删掉 `TERMINAL` 常量（已移入 `taskAction.ts`）以及 `const isTerminal = ...` 那一行。

底部按钮那一块，把原来的：

```tsx
          {/* 标签按状态说实话：对已结束的行，cancel 不删盘上的任何东西、只是把它移出列表
              （core 永远是唯一真相源，不做删除），此时叫「删除」是假话，而且紧接着的
              「已取消」还会跟「已完成」自相矛盾。取消中的行才是真的在取消。 */}
          <button onClick={() => void run(ipc.cancel(task.id))}>
            {isTerminal ? "从列表移除" : "删除"}
          </button>
```

换成：

```tsx
          {/* 按钮的叫法与调用的命令必须一致 —— 之前这里写着「从列表移除」却调 cancel()，
              对已取消的行连状态都不变，是个纯零反应的假按钮。见 lib/taskAction.ts。 */}
          <button onClick={() => void run(action.invoke(task.id))}>{action.label}</button>
```

并在组件顶部（`const labelColor = ...` 附近）加：

```ts
  const action = taskAction(task.state.kind);
```

- [ ] **Step 6: 确认构建与测试**

Run: `npm --prefix ui test && npm --prefix ui run build`
Expected: 两者全绿（`tsc` 开了 `noUnusedLocals`，`ipc` 若不再被直接使用会报错——`TaskRow` 里 `ipc` 仍被 `setFilename` 等用到，应当没问题）

- [ ] **Step 7: 手动验收——这是本次 bug 的原始症状**

Run: `npm run tauri dev`

逐条走：

1. 下一个文件，等它**完成** → 那一行按钮显示「从列表移除」→ 点击 → **行立刻消失**
2. 制造一个**失败**的任务（填一个不存在的 URL）→ 按钮显示「从列表移除」→ 点击 → 行消失
3. 取消一个任务 → 行显示「已取消」，按钮显示「从列表移除」→ 点击 → **行消失**（这是原始症状：以前这里毫无反应）
4. **关掉应用重新打开** → 上面三个任务都不应再出现
5. 一个正在下载的任务 → 按钮显示「取消」（不是「从列表移除」）→ 点击 → 变成「已取消」

- [ ] **Step 8: 提交**

```bash
git add ui/src/lib/taskAction.ts ui/src/lib/taskAction.test.ts ui/src/components/TaskRow.tsx
git commit -m "fix(ui): 「从列表移除」现在真的移除,并要求 core 重启后也不回来

原按钮标签写着「从列表移除」,调的是 cancel(),而链路上根本不存在
移除操作 —— 对已取消的行连状态都不变,就是字面意义上的零反应。

顺带把非终态那个按钮从「删除」改成「取消」:它调的同样是 cancel(),
而 core 从不删用户文件,叫「删除」是假话。

判断抽成 lib/taskAction.ts 的纯函数以便单测 —— 组件层不进测试,
但这条判断是界面契约的一部分,正是写错才出的这个 bug。"
```

---

## Task 5: 接入 shadcn/ui 与 lucide

**Files:**
- Modify: `ui/package.json`（新增依赖）
- Modify: `ui/vite.config.ts`（路径别名）
- Modify: `ui/tsconfig.json`（`paths`）
- Create: `ui/components.json`
- Create: `ui/src/lib/utils.ts`
- Create: `ui/src/components/ui/button.tsx`、`input.tsx`、`tabs.tsx`
- Modify: `ui/src/index.css`（shadcn 变量映射到现有 token）

**Interfaces:**
- Produces: `@/` 别名可用；`@/components/ui/button`、`@/components/ui/input`、`@/components/ui/tabs`；`@/lib/utils` 的 `cn()`

- [ ] **Step 1: 装依赖**

```bash
npm --prefix ui install class-variance-authority clsx tailwind-merge lucide-react @radix-ui/react-tabs
npm --prefix ui install -D @types/node
```

（只装 `@radix-ui/react-tabs` 这一个 Radix 包——它是唯一一个行为复杂到值得用现成实现的控件。其余控件见 spec §6.1「不拷」那张表。）

（`@types/node` 是给 `vite.config.ts` 里的 `path` 用的；不装的话别名那行会报找不到 `node:path`。）

- [ ] **Step 2: 配路径别名**

`ui/vite.config.ts` 改成：

```ts
import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  resolve: {
    // 必须与 tsconfig.json 的 paths 同时存在。只配一处的话，一处能跑、
    // 另一处报模块找不到 —— 而且报错信息指向的是别的文件，很难顺藤摸瓜。
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: { port: 5173, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true },
});
```

`ui/tsconfig.json` 的 `compilerOptions` 里加：

```json
    "baseUrl": ".",
    "paths": { "@/*": ["./src/*"] },
```

- [ ] **Step 3: 加 `cn()` 工具**

`ui/src/lib/utils.ts`：

```ts
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** shadcn 的类名合并约定：clsx 负责条件拼接，twMerge 负责消掉互斥的 Tailwind 类。 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
```

- [ ] **Step 4: 确认别名真的通了**

先临时在 `ui/src/main.tsx` 顶部加一行 `import { cn } from "@/lib/utils";`（下一步会用到它所在的那批文件，这里只是探路）。

Run: `npm --prefix ui run build`
Expected: 绿。若报找不到模块，说明 `vite.config.ts` 与 `tsconfig.json` 只改了一处。

然后把这行临时 import 删掉。

- [ ] **Step 5: 映射 shadcn 的变量到我们的 token**

`ui/src/index.css`，在现有 `:root { ... }` 块**之后**追加：

```css
/* shadcn 组件自带的变量体系。**必须**映射到上面那套 token ——
   不映射的话仓库里会同时存在两套颜色变量，之后改主题必然漏掉一边。
   shadcn 的变量写成裸 HSL 分量（不带 hsl()），所以这里用 hsl(var(--x))。 */
:root {
  --background: 0 0% 100%;
  --foreground: 240 6% 12%;          /* #1d1d1f */
  --muted-foreground: 240 2% 44%;    /* #6e6e73 */
  --border: 240 4% 83%;              /* #d2d2d7 */
  --input: 240 4% 83%;
  --primary: 211 100% 44%;           /* #0071e3 */
  --primary-foreground: 0 0% 100%;
  --radius: 0.375rem;

  /* 导航项的选中/悬停底色。**必须做成变量**：写死 rgba(0,0,0,.06) 的话，
     深色模式下就是黑底叠黑，选中项直接看不见。深色要用白色透明叠。 */
  --nav-sel: rgba(0, 0, 0, .06);
  --nav-hover: rgba(0, 0, 0, .03);
}
:root[data-theme="dark"] {
  --background: 0 0% 0%;             /* #000000 */
  --foreground: 240 6% 96%;          /* #f5f5f7 */
  --muted-foreground: 240 2% 53%;    /* #86868b */
  --border: 240 3% 26%;              /* #424245 */
  --input: 240 3% 26%;
  --primary: 209 100% 58%;           /* #2997ff */
  --primary-foreground: 0 0% 0%;

  --nav-sel: rgba(255, 255, 255, .08);
  --nav-hover: rgba(255, 255, 255, .04);
}
```

同时把 `:root` 里的 `--bg-subtle` 从 `#fcfcfc` 改成 `#f5f5f7`，深色版从 `#0e0e0e` 改成 `#1d1d1f`；`--fg` 改成 `#1d1d1f` / `#f5f5f7`；`--fg-muted` 改成 `#6e6e73` / `#86868b`；`--line` 改成 `#d2d2d7` / `#424245`。

（完整替换清单见 Task 8。这里先改侧边栏会立刻用到的那几个，避免 Task 6 的骨架建在一个撑不起来的底色上。）

- [ ] **Step 6: 加 button 与 input**

`ui/src/components/ui/button.tsx`：

```tsx
import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

// 主按钮用**胶囊**——这是 apple.com 最 signature 的形状。次级按钮用 6px 圆角，
// 两者的差距本身就是层级信号，不必再加颜色区分。
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-colors disabled:pointer-events-none disabled:opacity-40 outline-none",
  {
    variants: {
      variant: {
        primary:
          "bg-[#0071e3] text-white hover:bg-[#0077ed] active:bg-[#006edb] rounded-full",
        secondary:
          "border border-[var(--line-strong)] text-[var(--fg)] hover:bg-[var(--bg-subtle)] rounded-md",
        ghost:
          "text-[var(--fg-muted)] hover:bg-[var(--bg-subtle)] rounded-md",
        danger: "text-[var(--accent-err)] hover:bg-[var(--bg-subtle)] rounded-md",
      },
      size: {
        sm: "h-7 px-3 text-[12px]",
        md: "h-8 px-4 text-[13px]",
        icon: "size-7",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  }
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

export function Button({ className, variant, size, ...props }: ButtonProps) {
  return <button className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}

export { buttonVariants };
```

`ui/src/components/ui/input.tsx`：

```tsx
import * as React from "react";
import { cn } from "@/lib/utils";

export function Input({ className, ...props }: React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cn(
        "w-full min-w-0 rounded-lg border px-3 py-1.5 text-[13px] outline-none",
        "bg-[var(--bg-inset)] border-[var(--line-strong)] text-[var(--fg)]",
        "placeholder:text-[var(--fg-faint)]",
        "focus:border-[#0071e3] focus:ring-2 focus:ring-[#0071e3]/20",
        className
      )}
      {...props}
    />
  );
}
```

`ui/src/components/ui/tabs.tsx`——用 Radix 的 `Tabs` 做数据源，但视觉上**改造成 Apple 的分段控件**（一整块浅灰底 + 选中项浮起），而不是它默认的下划线样式：

```tsx
import * as React from "react";
import * as TabsPrimitive from "@radix-ui/react-tabs";
import { cn } from "@/lib/utils";

export const Tabs = TabsPrimitive.Root;

/** 分段控件的外壳：整块浅灰底，靠内层选中项"浮起"来表达当前值。 */
export function TabsList({ className, ...props }: TabsPrimitive.TabsListProps) {
  return (
    <TabsPrimitive.List
      className={cn("inline-flex items-center gap-0.5 rounded-lg p-0.5", className)}
      style={{ background: "var(--bg-inset)" }}
      {...props}
    />
  );
}

export function TabsTrigger({ className, ...props }: TabsPrimitive.TabsTriggerProps) {
  return (
    <TabsPrimitive.Trigger
      className={cn(
        "rounded-[6px] px-3 py-1 text-[12px] transition-colors outline-none",
        // 未选中是灰字；选中项白底 + 深字 + 极淡投影 —— 这正是 macOS 分段控件的样子，
        // 也是 Apple 表达"三选一"最标准的控件。
        "text-[var(--fg-muted)] hover:text-[var(--fg)]",
        "data-[state=active]:bg-[var(--bg)] data-[state=active]:text-[var(--fg)]",
        "data-[state=active]:shadow-[0_1px_2px_rgba(0,0,0,.08)] data-[state=active]:font-medium",
        className
      )}
      {...props}
    />
  );
}
```

- [ ] **Step 7: 确认构建**

Run: `npm --prefix ui run build`
Expected: 绿

- [ ] **Step 8: 提交**

```bash
git add ui/package.json ui/package-lock.json ui/vite.config.ts ui/tsconfig.json ui/src/lib/utils.ts ui/src/components/ui/ ui/src/index.css
git commit -m "build(ui): 接入 shadcn/ui 与 lucide,并把变量映射到现有 token

只做管道,不改观感。别名 @/ 在 vite 与 tsconfig 两处都配了 ——
漏一处就是构建期报错且报错信息指向别处。

shadcn 自带的 --primary/--border 等变量全部映射到 index.css 里
已有的 token。不映射的话仓库里会有两套颜色变量,之后改主题必然漏一边。"
```

---

## Task 6: 侧边栏骨架与三页导航

**Files:**
- Create: `ui/src/components/Sidebar.tsx`
- Create: `ui/src/pages/DownloadPage.tsx`
- Create: `ui/src/pages/VideoOpsPage.tsx`
- Create: `ui/src/pages/SettingsPage.tsx`（本任务先放占位，Task 7 填内容）
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/components/StatusBar.tsx`
- Modify: `ui/src/components/Toolbar.tsx`（内容迁入 DownloadPage，本文件删除）

**Interfaces:**
- Produces: `type Page = "download" | "videoOps" | "settings"`；`Sidebar` 的 props `{ page: Page; onNavigate: (p: Page) => void }`

- [ ] **Step 1: 写侧边栏**

`ui/src/components/Sidebar.tsx`：

```tsx
import type { ReactNode } from "react";
import { Download, Film, Settings } from "lucide-react";
import { cn } from "@/lib/utils";

export type Page = "download" | "videoOps" | "settings";

const ITEMS: ReadonlyArray<{ page: Page; label: string; Icon: typeof Download }> = [
  { page: "download", label: "下载", Icon: Download },
  { page: "videoOps", label: "视频操作", Icon: Film },
  { page: "settings", label: "设置", Icon: Settings },
];

export default function Sidebar({
  page,
  onNavigate,
  children,
}: {
  page: Page;
  onNavigate: (p: Page) => void;
  /** 底部槽位：总速度与版本号 */
  children?: ReactNode;
}) {
  return (
    <nav
      // 拖拽区挂在整个侧边栏上：顶部那 52px 是留给红绿灯的空白，
      // 用户在窗口左上角按下拖动时，抓到的就是这里。
      data-tauri-drag-region
      className="w-[200px] shrink-0 flex flex-col select-none"
      style={{
        background: "var(--bg-subtle)",
        borderRight: "1px solid var(--line)",
      }}
    >
      {/* 52px 给 macOS 的红绿灯（titleBarStyle: Overlay 时它们浮在内容上） */}
      <div className="h-[52px] shrink-0" data-tauri-drag-region />

      <div className="flex flex-col gap-0.5 px-2">
        {ITEMS.map(({ page: p, label, Icon }) => (
          <button
            key={p}
            onClick={() => onNavigate(p)}
            className={cn(
              "flex items-center gap-2 h-8 px-2 rounded-md text-[13px] transition-colors",
              // 选中态用浅灰底 + 文字转深，**不用品牌色填充** ——
              // Apple 自家的侧边栏（系统设置、访达）就是这样，彩色填充会立刻不像苹果。
              // 底色走 --nav-sel 变量：写死黑色透明在深色模式下会完全看不见。
              p === page
                ? "bg-[var(--nav-sel)] text-[var(--fg)] font-medium"
                : "text-[var(--fg-muted)] hover:bg-[var(--nav-hover)]"
            )}
          >
            <Icon size={16} strokeWidth={1.75} />
            {label}
          </button>
        ))}
      </div>

      <div className="flex-1" />
      <div style={{ borderTop: "1px solid var(--line)" }}>{children}</div>
    </nav>
  );
}
```

- [ ] **Step 2: 把 StatusBar 改成侧边栏底部的槽**

`ui/src/components/StatusBar.tsx` 整个替换成：

```tsx
import { useTaskList } from "../store/store";
import { formatSpeed } from "../lib/format";

/** HLS 任务的"速度"单位是分片/秒，累加进总速度会得到"3.0 B/s"这种假数字 —— 只统计字节单位的。 */
export default function StatusBar({ version }: { version: string }) {
  const tasks = useTaskList();
  const active = tasks.filter((t) =>
    ["probing", "downloading", "merging"].includes(t.state.kind)
  ).length;
  const bytesPerSec = tasks
    .filter((t) => t.unit === "bytes")
    .reduce((sum, t) => sum + (t.speedBps ?? 0), 0);

  // 从底部的通栏挪到侧边栏底部：总速度与版本号是全局信息，但底下那条横栏
  // 在有了侧边栏之后就是白占一行高度。
  return (
    <div className="px-4 py-2.5 tnum text-[11px]" style={{ color: "var(--fg-muted)" }}>
      <div>{formatSpeed(bytesPerSec)} · {active} 个下载中</div>
      <div style={{ color: "var(--fg-faint)" }}>{version}</div>
    </div>
  );
}
```

- [ ] **Step 3: 建下载页**

`ui/src/pages/DownloadPage.tsx`——把 `Toolbar.tsx` 的 URL 输入搬进来，去掉窗口控件与 header 外壳：

```tsx
import { useState } from "react";
import TaskList from "../components/TaskList";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { normalizeUrl } from "../lib/url";
import { dispatch } from "../store/store";

export default function DownloadPage({ onAdd }: { onAdd: (url: string) => Promise<boolean> }) {
  const [url, setUrl] = useState("");
  const [adding, setAdding] = useState(false);

  async function submit() {
    if (adding) return; // 防连点：一次只提交一个，否则双击会拿同一条 URL 建两个任务
    const normalized = normalizeUrl(url);
    if (normalized === null) {
      if (url.trim() !== "") dispatch({ kind: "pushToast", text: "请输入 http(s) 链接" });
      return;
    }
    setAdding(true);
    try {
      // 失败时把 URL 留在框里：用户要重试/改的就是它，清掉等于逼他重新粘贴一次。
      if (await onAdd(normalized)) setUrl("");
    } finally {
      setAdding(false);
    }
  }

  return (
    <>
      <div className="flex items-center gap-2 px-6 py-4">
        <Input
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void submit();
          }}
          placeholder="粘贴视频链接…"
          spellCheck={false}
        />
        <Button
          variant="primary"
          onClick={() => void submit()}
          disabled={adding || url.trim() === ""}
        >
          下载
        </Button>
      </div>
      <div className="px-6">
        <TaskList />
      </div>
    </>
  );
}
```

- [ ] **Step 4: 建视频操作占位页**

`ui/src/pages/VideoOpsPage.tsx`：

```tsx
import { Film } from "lucide-react";

/**
 * 占位。骨架按「选文件 → 选操作 → 队列」预留，本期不实现。
 * 明确不做胜过一个假装能用、点了没反应的界面 —— 这次的 bug 就是那么来的。
 */
export default function VideoOpsPage() {
  return (
    <div className="h-full flex flex-col items-center justify-center gap-2">
      <Film size={32} strokeWidth={1.5} style={{ color: "var(--fg-faint)" }} />
      <p style={{ color: "var(--fg-muted)" }}>视频操作还在做</p>
      <p className="text-[12px]" style={{ color: "var(--fg-faint)" }}>
        以后可以在这里转码、压缩、裁剪、提取音频
      </p>
    </div>
  );
}
```

- [ ] **Step 5: 设置页先放占位**

`ui/src/pages/SettingsPage.tsx`：

```tsx
export default function SettingsPage() {
  return <div style={{ color: "var(--fg-muted)" }}>设置（下一任务填充）</div>;
}
```

- [ ] **Step 6: 改 App.tsx 组装**

`ui/src/App.tsx` 整个替换：

```tsx
import { useEffect, useState } from "react";
import Sidebar, { type Page } from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import DownloadPage from "./pages/DownloadPage";
import VideoOpsPage from "./pages/VideoOpsPage";
import SettingsPage from "./pages/SettingsPage";
import ChoiceDialog from "./components/ChoiceDialog";
import Toasts from "./components/Toasts";
import WindowControls from "./components/WindowControls";
import * as ipc from "./lib/ipc";
import { startBridge, hydrate, loadSettings, run } from "./store/store";
import { readThemeMode, setThemeMode } from "./lib/theme";

const TITLES: Record<Page, string> = {
  download: "下载",
  videoOps: "视频操作",
  settings: "设置",
};

export default function App() {
  // 三个页面用 useState 切就够了，不引 react-router —— 桌面 App 没有 URL 的需求。
  const [page, setPage] = useState<Page>("download");

  useEffect(() => {
    setThemeMode(readThemeMode());
    void startBridge();
    void loadSettings();
  }, []);

  /** 返回是否真的建了任务，DownloadPage 据此决定清不清输入框。 */
  async function addUrl(url: string): Promise<boolean> {
    if ((await run(ipc.addUrl(url))) === undefined) return false;
    await hydrate();
    return true;
  }

  return (
    <div className="flex h-full">
      <Sidebar page={page} onNavigate={setPage}>
        <StatusBar version="v0.1.0" />
      </Sidebar>

      <main className="flex-1 min-w-0 flex flex-col">
        {/* 非 macOS 的自绘窗口键挂在这里；macOS 上 WindowControls 按设计不渲染 */}
        <header
          data-tauri-drag-region
          className="flex items-center justify-between px-6 pt-5 pb-3 select-none"
        >
          <h1 className="text-[22px] font-semibold" style={{ letterSpacing: "-0.01em" }}>
            {TITLES[page]}
          </h1>
          <WindowControls />
        </header>

        <div className="flex-1 overflow-y-auto">
          {page === "download" && <DownloadPage onAdd={addUrl} />}
          {page === "videoOps" && <VideoOpsPage />}
          {page === "settings" && <SettingsPage />}
        </div>
      </main>

      <ChoiceDialog />
      <Toasts />
    </div>
  );
}
```

- [ ] **Step 7: 删掉 Toolbar.tsx**

```bash
git rm ui/src/components/Toolbar.tsx
```

（它的内容已经拆进 `DownloadPage.tsx` 与 `App.tsx` 的 header。留着会变成两处真相源。）

- [ ] **Step 8: 确认构建与测试**

Run: `npm --prefix ui test && npm --prefix ui run build`
Expected: 全绿。`noUnusedLocals` 可能报 `Toolbar.tsx` 里残留的未用导入——已删文件不受影响。

- [ ] **Step 9: 手动验收**

Run: `npm run tauri dev`

1. 窗口左侧出现 200px 侧边栏，三个导航项
2. 点「视频操作」→ 主区换成占位页，标题变「视频操作」
3. 点「设置」→ 占位页
4. 点「下载」→ 回到列表与输入框
5. 侧边栏底部显示总速度与 `v0.1.0`
6. **macOS 上红绿灯仍可见可点**，且左上角空白处能拖动窗口

- [ ] **Step 10: 提交**

```bash
git add -A ui/src
git commit -m "feat(ui): 侧边栏导航与三个一级页面

设置从模态对话框变成整页,三个页面用 useState 切,不引 react-router。

总速度与版本号从底部通栏挪到侧边栏底部 —— 这两个是全局信息,
而底下那条横栏在有了侧边栏之后就是白占一行高度。

Toolbar.tsx 删除:内容已拆进 DownloadPage 与 App 的 header,
留着会变成两处真相源。「视频操作」只做占位空状态,不假装能用。"
```

---

## Task 7: 设置改整页

**Files:**
- Modify: `ui/src/pages/SettingsPage.tsx`
- Delete: `ui/src/components/SettingsDialog.tsx`

**Interfaces:**
- Consumes: 现有 `useSettings` / `loadSettings` / `run` / `ipc.updateSettings` / `ipc.pickDownloadDir` / `setThemeMode`
- Produces: 无新导出

- [ ] **Step 1: 把对话框的内容搬成页面**

`ui/src/pages/SettingsPage.tsx`：

```tsx
import { useEffect, useState } from "react";
import type { SettingsDto } from "../lib/types";
import * as ipc from "../lib/ipc";
import { loadSettings, run, useSettings } from "../store/store";
import { readThemeMode, setThemeMode, type ThemeMode } from "../lib/theme";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

const SEGMENT_CONCURRENCY = { min: 1, max: 16 } as const;
const MAX_ACTIVE_TASKS = { min: 1, max: 8 } as const;

/** 与 core 的接受范围一致。见原 SettingsDialog 的注释：`<input type="number">` 挡不住任何东西。 */
function clampInt(raw: string, lo: number, hi: number, fallback: number): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(hi, Math.max(lo, Math.round(n)));
}

export default function SettingsPage() {
  const current = useSettings();
  const [draft, setDraft] = useState<SettingsDto | null>(null);
  const [theme, setTheme] = useState<ThemeMode>(readThemeMode());
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (current) setDraft({ ...current });
  }, [current]);

  async function pick() {
    const picked = await run(ipc.pickDownloadDir());
    if (picked) setDraft((prev) => (prev ? { ...prev, downloadDir: picked } : prev));
  }

  async function save() {
    if (!draft) return;
    const toSave = {
      ...draft,
      segmentConcurrency: clampInt(
        String(draft.segmentConcurrency),
        SEGMENT_CONCURRENCY.min,
        SEGMENT_CONCURRENCY.max,
        SEGMENT_CONCURRENCY.min
      ),
      maxActiveTasks: clampInt(
        String(draft.maxActiveTasks),
        MAX_ACTIVE_TASKS.min,
        MAX_ACTIVE_TASKS.max,
        MAX_ACTIVE_TASKS.min
      ),
    };
    try {
      await ipc.updateSettings(toSave);
    } catch (e) {
      setError(String(e));
      return;
    }
    setError(null);
    try {
      setThemeMode(theme);
    } catch {
      /* 主题没写进去不影响设置本身 */
    }
    await run(loadSettings());
  }

  if (!draft) return null;

  return (
    <div className="max-w-[520px] px-6 pb-8">
      <label className="block mb-5">
        <span style={{ color: "var(--fg-muted)" }}>保存位置</span>
        <div className="flex gap-2 mt-1.5">
          <Input readOnly value={draft.downloadDir} />
          <Button onClick={() => void pick()}>更改</Button>
        </div>
        <span className="text-[12px]" style={{ color: "var(--fg-faint)" }}>
          只影响之后新建的任务；正在下载的任务沿用创建时的目录。
        </span>
      </label>

      <label className="block mb-5">
        <span style={{ color: "var(--fg-muted)" }}>线程数（1–16）</span>
        <Input
          type="number"
          className="w-24 mt-1.5"
          value={draft.segmentConcurrency}
          onChange={(e) =>
            setDraft((p) =>
              p
                ? {
                    ...p,
                    segmentConcurrency: clampInt(
                      e.target.value,
                      SEGMENT_CONCURRENCY.min,
                      SEGMENT_CONCURRENCY.max,
                      SEGMENT_CONCURRENCY.min
                    ),
                  }
                : p
            )
          }
        />
      </label>

      <label className="block mb-5">
        <span style={{ color: "var(--fg-muted)" }}>同时下载任务数</span>
        <Input
          type="number"
          className="w-24 mt-1.5"
          value={draft.maxActiveTasks}
          onChange={(e) =>
            setDraft((p) =>
              p
                ? {
                    ...p,
                    maxActiveTasks: clampInt(
                      e.target.value,
                      MAX_ACTIVE_TASKS.min,
                      MAX_ACTIVE_TASKS.max,
                      MAX_ACTIVE_TASKS.min
                    ),
                  }
                : p
            )
          }
        />
      </label>

      <div className="mb-5">
        <div style={{ color: "var(--fg-muted)" }}>外观</div>
        {/* 用分段控件而不是下拉框：三项互斥、且都是短词，分段控件一眼能看全，
            不必先展开再选。这是 macOS 表达"三选一"的标准控件。 */}
        <Tabs value={theme} onValueChange={(v) => setTheme(v as ThemeMode)} className="mt-1.5">
          <TabsList>
            <TabsTrigger value="system">跟随系统</TabsTrigger>
            <TabsTrigger value="light">浅色</TabsTrigger>
            <TabsTrigger value="dark">深色</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>

      <label className="flex items-center gap-2 mb-5 text-[13px]">
        <input
          type="checkbox"
          checked={draft.ytdlpEnabled}
          onChange={(e) =>
            setDraft((p) => (p ? { ...p, ytdlpEnabled: e.target.checked } : p))
          }
        />
        <span>用 yt-dlp 解析网页视频（需自行安装 yt-dlp）</span>
      </label>

      <label className="block mb-5">
        <span style={{ color: "var(--fg-muted)" }}>ffmpeg 路径覆盖（留空则用内嵌的）</span>
        <Input
          className="mt-1.5"
          value={draft.ffmpegPathOverride ?? ""}
          onChange={(e) =>
            setDraft((p) => (p ? { ...p, ffmpegPathOverride: e.target.value || null } : p))
          }
        />
      </label>

      {error && (
        // 不再自己加「保存失败：」前缀 —— core 回的已经是「保存设置失败：…」，
        // 再套一层会读成「保存失败：保存设置失败：…」。
        <p className="mb-3" style={{ color: "var(--accent-err)" }}>
          {error}
        </p>
      )}

      <Button variant="primary" onClick={() => void save()}>
        保存
      </Button>
    </div>
  );
}
```

- [ ] **Step 2: 删掉对话框**

```bash
git rm ui/src/components/SettingsDialog.tsx
```

- [ ] **Step 3: 确认构建与测试**

Run: `npm --prefix ui test && npm --prefix ui run build`
Expected: 全绿

- [ ] **Step 4: 手动验收**

Run: `npm run tauri dev`

1. 侧边栏点「设置」→ 一整页设置，不再是弹窗
2. 改「同时下载任务数」为 3 → 保存 → 切到「下载」再切回来 → 显示 3
3. 改「外观」为深色 → 保存 → 整个应用立刻变深，**侧边栏也跟着变深**
4. 切到「下载」页，任务正常
5. `Esc` **不再**关闭设置页（页面没有模态语义了，这是预期的）

- [ ] **Step 5: 提交**

```bash
git add -A ui/src
git commit -m "feat(ui): 设置改为整页,删掉 SettingsDialog

内容原样搬过来,只换了外壳与控件的实现。对话框自己的 Esc 关闭与
焦点陷阱一并去掉 —— 设置现在是一级页面,没有模态语义。

「取消」按钮也去掉了:页面没有"取消"这个动作可言,改了什么再改回来
就是,保存按钮依旧只写一次盘。"
```

---

## Task 8: 用 apple.com 的灰阶、字号与形状替换视觉 token

**Files:**
- Modify: `ui/src/index.css`
- Modify: `ui/src/components/TaskRow.tsx`
- Modify: `ui/src/components/TaskList.tsx`
- Modify: `ui/src/components/Toasts.tsx`
- Modify: `ui/src/components/ChoiceDialog.tsx`

**Interfaces:**
- Consumes: `--bg-subtle` 已是 `#f5f5f7`（Task 5 改的）
- Produces: 无新导出

- [ ] **Step 1: 换掉整套灰阶与字号**

⚠️ **只改配色变量，不要整块覆盖。** Task 5 已经往 `:root` 里加了 `--nav-sel` / `--nav-hover`，并**另起了一个** `:root` 块放 shadcn 的变量。把 `:root` 整块替换掉会连 `--nav-sel` 一起删掉，而 `Sidebar.tsx` 正依赖它——后果是侧边栏选中态在浅色下变成纯黑底、深色下完全看不见。

所以：**逐个改下面列出的变量，其余原样保留**（包括 `--nav-sel`、`--nav-hover`，以及那个独立的 shadcn 映射块）。

浅色（`ui/src/index.css` 第一个 `:root` 块里改这些值）：

```css
  --bg-subtle: #f5f5f7;   /* 官网经典灰；原来的 #fcfcfc 太接近白色，撑不起侧边栏 */
  --fg: #1d1d1f;          /* 官网正文色，比纯黑柔和 */
  --fg-muted: #6e6e73;    /* 官网副标题 */
  --fg-faint: #86868b;    /* 官网页脚 */
  --line: #d2d2d7;        /* 官网分隔线 */
  --line-strong: #d2d2d7;
  --fill: #1d1d1f;
```

再在同一个块里**新增**一行：

```css
  --accent-blue: #0071e3;
```

深色（`[data-theme="dark"]` 那块）同理，逐个改：

```css
  --bg: #000000;
  --bg-subtle: #1d1d1f;
  --fg: #f5f5f7;
  --fg-muted: #86868b;
  --fg-faint: #6e6e73;
  --line: #424245;
  --line-strong: #424245;
  --fill: #f5f5f7;
```

并新增：

```css
  --accent-blue: #2997ff;
```

`--bg`（浅色 `#ffffff`）、`--bg-inset`、`--track`、`--accent-ok`、`--accent-warn`、`--accent-err`、`--btn-fg` **不动**。

`body` 块里 `font-size: 13px;` 之后加一行：

```css
  letter-spacing: -0.005em;   /* 官网正文的紧凑感；纯 0 在 macOS 上会显得松 */
```

- [ ] **Step 2: 少用线，多用面**

`ui/src/components/TaskList.tsx`，给列表外面套一层底色块，让行与页面背景分开：

```tsx
  return (
    // 区块之间改用背景色差而不是分隔线。列表行之间仍保留发丝线 ——
    // 列表需要它来对齐视线，区块不需要。
    <div className="rounded-xl overflow-hidden" style={{ background: "var(--bg-subtle)" }}>
      {tasks.map((t) => (
        <TaskRow key={t.id} task={t} />
      ))}
    </div>
  );
```

`ui/src/components/TaskRow.tsx`，把最外层 div 的：

```tsx
    <div className="px-3 py-2.5" style={{ borderBottom: "1px solid var(--line)" }}>
```

改成：

```tsx
    <div className="px-3.5 py-3" style={{ borderBottom: "1px solid var(--line)" }}>
```

并把它内部的 `style={{ background: "var(--track)" }}` 进度条背景保持不变（`--track` 未改）。

- [ ] **Step 3: Toast 换成官网的中性面色 + 错误色描边**

`ui/src/components/Toasts.tsx` 的按钮样式改为：

```tsx
          className="text-left rounded-xl px-3.5 py-2.5 text-[12px]"
          style={{
            background: "var(--bg)",
            border: "1px solid var(--accent-err)",
            color: "var(--accent-err)",
            boxShadow: "0 4px 16px rgba(0,0,0,.12)",
          }}
```

- [ ] **Step 4: 确认构建与测试**

Run: `npm --prefix ui test && npm --prefix ui run build`
Expected: 全绿

- [ ] **Step 5: 逐条走视觉核对清单**

Run: `npm run tauri dev`，在浅色与深色两种模式下各走一遍（设置 → 外观）：

- [ ] 主文字不是纯黑，是 `#1d1d1f`（深色下 `#f5f5f7`）
- [ ] 侧边栏底是 `#f5f5f7`，与主区的白**能看出差别**（这是这次改动最直观的一条）
- [ ] 页面标题是 22px semibold，明显大于正文
- [ ] 下载按钮是胶囊形（完全圆头），次级按钮是 6px 圆角
- [ ] 主按钮是官网蓝 `#0071e3`（深色下 `#2997ff`）
- [ ] 任务列表整块有浅灰底，与页面背景分层
- [ ] 侧边栏选中项是浅灰底 + 文字变深，**不是**蓝色填充
- [ ] 深色模式下没有出现"白底深字"或"黑底黑字"的漏网元素

- [ ] **Step 6: 提交**

```bash
git add ui/src/index.css ui/src/components/
git commit -m "style(ui): 换成 apple.com 的灰阶、字号与形状

灰阶是这次最要紧的一条:原来的 --bg-subtle #fcfcfc 与 --fg #0a0a0a
几乎纯白配纯黑,撑不起侧边栏与分层。换成官网那套更柔和的灰。

区块之间改用背景色差而不是分隔线,列表行之间的发丝线保留 ——
列表需要它对齐视线,区块不需要。

正文保持 13px 是刻意的:官网的 17px 正文在 900px 窗口里太奢侈。
落差靠 22px 的标题撑,密度靠正文守。"
```

---

## Task 9: 图标全量替换

**Files:**
- Modify: `ui/src/components/TaskRow.tsx`、`WindowControls.tsx`

**Interfaces:**
- Consumes: `lucide-react`（Task 5 已装）
- Produces: 无新导出

- [ ] **Step 1: 替换 TaskRow 的行内按钮**

`ui/src/components/TaskRow.tsx`：import 区加

```ts
import { ListChecks, Pause, Play, RotateCcw, X } from "lucide-react";
```

按钮区改成图标 + 文字（图标 `size={14} strokeWidth={1.75}`）。**这里继续用裸 `<button>`，不换成 `Button` 组件**：这几个是行内的轻量文字按钮，`Button` 的 `h-7/h-8` 固定高度和 `px-3` 会破坏行高节奏，而它们的样式本来就只有字号和颜色两件事。

```tsx
        <span className="shrink-0 flex gap-1">
          {task.state.kind === "awaitingChoice" && (
            <button
              className="flex items-center gap-1 text-[12px]"
              onClick={() => dispatch({ kind: "openChoice", id: task.id })}
            >
              <ListChecks size={14} strokeWidth={1.75} />
              选择格式
            </button>
          )}
          {task.state.kind === "paused" && (
            <button
              className="flex items-center gap-1 text-[12px]"
              onClick={() => void run(ipc.unpause(task.id))}
            >
              <Play size={14} strokeWidth={1.75} />
              继续
            </button>
          )}
          {task.state.kind === "failed" && (
            <button
              className="flex items-center gap-1 text-[12px]"
              onClick={() => void run(ipc.retry(task.id))}
            >
              <RotateCcw size={14} strokeWidth={1.75} />
              重试
            </button>
          )}
          {["queued", "probing", "downloading", "merging"].includes(task.state.kind) && (
            <button
              className="flex items-center gap-1 text-[12px]"
              onClick={() => void run(ipc.pause(task.id))}
            >
              <Pause size={14} strokeWidth={1.75} />
              暂停
            </button>
          )}
          <button
            className="flex items-center gap-1 text-[12px]"
            onClick={() => void run(action.invoke(task.id))}
          >
            <X size={14} strokeWidth={1.75} />
            {action.label}
          </button>
        </span>
```

- [ ] **Step 2: 替换 WindowControls 的图标**

全仓库只有这一处还在拿文字符号当图标。`ui/src/components/WindowControls.tsx` 的三个按钮，把 `─` `☐` `✕` 换成

```ts
import { Minus, Square, X } from "lucide-react";
```

分别渲染 `<Minus size={14} strokeWidth={1.75} />`、`<Square size={12} strokeWidth={1.75} />`、`<X size={14} strokeWidth={1.75} />`，并保留各自的 `aria-label`。

（`ChoiceDialog.tsx` 的按钮全是纯文字，没有图标，不用动。）

- [ ] **Step 3: 确认全仓库再没有 emoji 当图标用**

Run: `grep -rn -e '⚙' -e '✕' -e '─' -e '☐' ui/src`
Expected: 无输出。若有，逐个换成对应的 lucide 图标。

（这条检查就是本次那个「设置图标太小」的根因：`⚙` 是 emoji，跟着 13px 的文字渲染，永远大不了。）

- [ ] **Step 4: 确认构建与测试**

Run: `npm --prefix ui test && npm --prefix ui run build`
Expected: 全绿

- [ ] **Step 5: 手动验收**

Run: `npm run tauri dev`

1. 侧边栏三个图标（`Download` / `Film` / `Settings`）大小一致、描边一致
2. 行内按钮的图标与文字基线对齐，没有一大一小
3. 浅色与深色下图标都清晰（lucide 用 `currentColor`，天然跟随 `--fg`）

- [ ] **Step 6: 提交**

```bash
git add ui/src/components/
git commit -m "style(ui): 图标全量换成 lucide

原来的图标是 ⚙ ✕ ─ ☐ 这类 emoji/文字符号,跟着 13px 的文字渲染,
所以永远大不了 —— 用户报的"设置图标很小"就是这个。

统一 stroke-width 1.75:默认的 2 偏粗像描边图标,1.5 在浅色底上发虚。"
```

---

## 收尾验收

全部任务完成后，走一遍完整的手动验收：

- [ ] `cargo test --workspace` 全绿
- [ ] `npm --prefix ui test` 全绿
- [ ] `npm run build` 全绿（含 `tsc --noEmit`）
- [ ] `npm run tauri dev`：三个页面都能切、设置能存、下载流程正常
- [ ] **「从列表移除」在已完成 / 失败 / 已取消三种行上都生效，且重启后不再出现**（这是本次的原始诉求）
- [ ] 视觉核对清单（Task 8 Step 5）逐条通过
