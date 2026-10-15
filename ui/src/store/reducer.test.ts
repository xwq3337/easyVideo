import { describe, expect, it } from "vitest";
import { humanizeSeconds, initialState, reduce, selectPendingChoice } from "./reducer";
import type { ChoiceKindDto, SettingsDto, TaskView } from "../lib/types";

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

  // 下面两条是 `hydrate` 里仅有的两处「保留本地状态」逻辑，也是整个 store 存在的理由：
  // 一次重同步不能把用户正在输入的框擦掉，也不能把已经弹出的选择模态框清空。
  // 其余 hydrate 测试的旧行都是 editing:false / choices:null —— 那两处 `?:` 分支
  // 在别处永远不会被走到，删掉它们不会有任何测试变红。

  it("hydrate 保留正在编辑的文件名：重同步不擦掉用户正在输入的字", () => {
    const before = {
      ...initialState,
      tasks: [task(1, { editing: true, filename: "我正在输入.mp4" })],
    };
    const after = reduce(before, {
      kind: "hydrate",
      tasks: [task(1, { filename: "core给的名字.mp4" })],
    });
    expect(after.tasks[0].filename).toBe("我正在输入.mp4");
    expect(after.tasks[0].editing).toBe(true);
  });

  it("hydrate 对没在编辑的行采用 core 的新文件名", () => {
    // 上一条的另一半：保留本地文件名**只**发生在 editing 为 true 时，
    // 否则重同步会让 core 改名永远落不到界面上。
    const before = { ...initialState, tasks: [task(1, { filename: "旧名字.mp4" })] };
    const after = reduce(before, {
      kind: "hydrate",
      tasks: [task(1, { filename: "新名字.mp4" })],
    });
    expect(after.tasks[0].filename).toBe("新名字.mp4");
  });

  it("hydrate 保留本地候选副本：重同步不把已弹出的选择模态框清空", () => {
    const cached: ChoiceKindDto = {
      kind: "hlsVariants",
      hlsVariants: [{ uri: "a.m3u8", bandwidth: 1, resolution: null }],
    };
    const before = {
      ...initialState,
      tasks: [task(1, { state: { kind: "awaitingChoice" }, choices: cached })],
    };
    const after = reduce(before, {
      kind: "hydrate",
      tasks: [task(1, { state: { kind: "awaitingChoice" } })],
    });
    expect(after.tasks[0].choices).toEqual(cached);
  });

  it("resyncRequested 置 needsResync 并记下丢了多少条", () => {
    const after = reduce(initialState, { kind: "resyncRequested", dropped: 7 });
    expect(after.needsResync).toBe(true);
    expect(after.lastDropped).toBe(7);
    expect(after.tasks).toEqual([]);
  });

  it("settings 存下设置", () => {
    const s: SettingsDto = {
      downloadDir: "/tmp/x",
      segmentConcurrency: 6,
      maxActiveTasks: 2,
      ytdlpEnabled: true,
      ffmpegPathOverride: "/opt/ffmpeg",
    };
    expect(reduce(initialState, { kind: "settings", settings: s }).settings).toEqual(s);
  });

  it("editFilename 只改目标行", () => {
    const before = { ...initialState, tasks: [task(1), task(2)] };
    const after = reduce(before, { kind: "editFilename", id: 2, filename: "改过.mp4" });
    expect(after.tasks[0].filename).toBe("1.mp4");
    expect(after.tasks[1].filename).toBe("改过.mp4");
  });

  it("setEditing 只改目标行的编辑态", () => {
    const before = { ...initialState, tasks: [task(1), task(2)] };
    const after = reduce(before, { kind: "setEditing", id: 1, editing: true });
    expect(after.tasks[0].editing).toBe(true);
    expect(after.tasks[1].editing).toBe(false);
  });

  it("choices 只把候选挂到目标行", () => {
    const cached: ChoiceKindDto = {
      kind: "ytdlpFormats",
      ytdlpFormats: [
        { id: "137", ext: "mp4", note: "1080p", filesize: 123, url: "https://example.com/v" },
      ],
    };
    const before = { ...initialState, tasks: [task(1), task(2)] };
    const after = reduce(before, { kind: "choices", id: 1, choices: cached });
    expect(after.tasks[0].choices).toEqual(cached);
    expect(after.tasks[1].choices).toBeNull();
  });

  // 「稍后再说」：任务停在 awaitingChoice 时可以暂时压下选择框而不取消下载。
  // 下面是这套抑制逻辑的四条不变量，也是 ChoiceDialog 取谁的关键。
  const waiting = (id: number) => task(id, { state: { kind: "awaitingChoice" } });

  it("dismissChoice 压住它，openChoice 又放出来", () => {
    const before = { ...initialState, tasks: [waiting(1), waiting(2)] };
    const dismissed = reduce(before, { kind: "dismissChoice", id: 1 });
    expect(dismissed.dismissedChoices).toEqual([1]);
    // 压住 id 1 之后，该弹的就是 id 2。
    expect(selectPendingChoice(dismissed.tasks, dismissed.dismissedChoices)?.id).toBe(2);

    const reopened = reduce(dismissed, { kind: "openChoice", id: 1 });
    expect(reopened.dismissedChoices).toEqual([]);
    expect(selectPendingChoice(reopened.tasks, reopened.dismissedChoices)?.id).toBe(1);
  });

  it("dismissChoice 不动任务本身：只是「先不弹」，不是取消", () => {
    const before = { ...initialState, tasks: [waiting(1)] };
    const after = reduce(before, { kind: "dismissChoice", id: 1 });
    expect(after.tasks).toEqual(before.tasks);
    expect(after.tasks[0].state.kind).toBe("awaitingChoice");
  });

  it("两条等待任务、只压住 id 大的那条：仍然弹 id 小的那条", () => {
    // 最低 id 优先 —— 与数组顺序无关，压住 7 不影响"该弹 3"这条规则。
    const before = { ...initialState, tasks: [waiting(3), waiting(7)] };
    const after = reduce(before, { kind: "dismissChoice", id: 7 });
    expect(selectPendingChoice(after.tasks, after.dismissedChoices)?.id).toBe(3);

    // 两条都被压住 → 不弹（ChoiceDialog 此时渲染 null）。
    const both = reduce(after, { kind: "dismissChoice", id: 3 });
    expect(selectPendingChoice(both.tasks, both.dismissedChoices)).toBeNull();
  });

  it("任务离开 awaitingChoice 就放掉抑制记录：重新进入时会再弹", () => {
    const before = { ...initialState, tasks: [waiting(1)], dismissedChoices: [1] };
    const left = reduce(before, {
      kind: "event",
      event: { type: "stateChanged", id: 1, state: { kind: "downloading" } },
    });
    expect(left.dismissedChoices).toEqual([]);

    // 以后若再次进入等待选择（例如又一次 choicesOffered），不再被旧记录压住。
    const back = reduce(left, {
      kind: "event",
      event: {
        type: "choicesOffered",
        id: 1,
        kind: { kind: "hlsVariants", hlsVariants: [] },
      },
    });
    expect(selectPendingChoice(back.tasks, back.dismissedChoices)?.id).toBe(1);
  });

  it("hydrate 保住仍等待选择者的抑制记录，放掉已离开者的", () => {
    // 重同步整体替换任务列表。仍然 awaitingChoice 的 id 要保住（否则"稍后再说"被重同步
    // 抹掉、模态框立刻又弹回来）；已经被 core 报成别的状态的 id 要放掉（哪怕中途的事件丢过）。
    const before = {
      ...initialState,
      tasks: [waiting(1), waiting(2)],
      dismissedChoices: [1, 2],
    };
    const after = reduce(before, {
      kind: "hydrate",
      tasks: [waiting(1), task(2, { state: { kind: "downloading" } })],
    });
    expect(after.dismissedChoices).toEqual([1]);
  });
});

describe("humanizeSeconds", () => {
  it("不足一分钟直接报秒", () => {
    expect(humanizeSeconds(0)).toBe("0 秒");
    expect(humanizeSeconds(45)).toBe("45 秒");
  });

  it("分与秒；整分不拖一个 '0 秒' 的尾巴", () => {
    expect(humanizeSeconds(90)).toBe("1 分 30 秒");
    expect(humanizeSeconds(120)).toBe("2 分");
  });

  it("小时与分；整小时不拖一个 '0 分' 的尾巴", () => {
    expect(humanizeSeconds(3600)).toBe("1 小时");
    expect(humanizeSeconds(3900)).toBe("1 小时 5 分");
  });

  it("分钟进位到小时，不打印人话里不存在的「1 小时 60 分」", () => {
    // 7175 秒 = 1h59m35s。分钟四舍五入到 60，必须进位成 2 小时，
    // 而不是把 60 直接塞进 "… 分" 那个位置。
    expect(humanizeSeconds(7175)).toBe("2 小时");
  });
});
