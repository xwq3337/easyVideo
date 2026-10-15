import { useEffect, useState } from "react";
import type { SettingsDto } from "../lib/types";
import * as ipc from "../lib/ipc";
import { loadSettings, run, useSettings } from "../store/store";
import { readThemeMode, setThemeMode, type ThemeMode } from "../lib/theme";
import { clampInt } from "../lib/clampInt";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

const SEGMENT_CONCURRENCY = { min: 1, max: 16 } as const;
const MAX_ACTIVE_TASKS = { min: 1, max: 8 } as const;

/**
 * 设置**页**（原先是模态对话框）。
 *
 * **只有点「保存」才写盘** —— 控件变化一律只落在本地 `draft` 上，不碰 core。
 * 写盘失败时保留 draft、把 core 回的原文显示出来，用户改一改还能再存一次。
 * （egui 版每帧落盘的教训。）
 *
 * 对话框时代的「取消」/ Esc / 点遮罩丢弃 draft 一并消失：页面没有"取消"这个动作，
 * 改错了再改回来就是。
 */
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
    // 提交前再夹一次：onChange 已夹过，但"onChange 一定跑过"这件事不该由 save 来假设
    // （draft 若被别的路径设成越界值，这里就是唯一还站着的闸）。
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
    // setThemeMode 做两件事：上色 + 把"跟随系统"的订阅重新绑到新模式上。
    // 它是同步的（内部只写 localStorage、加/退一个 matchMedia 监听）；
    // 万一它抛（webview 禁用存储之类），设置其实已经落盘成功，不该反过来报"保存失败"，
    // 所以单独兜一下 —— save() 是 `void save()` 调起来的，漏出去的异常没人接。
    try {
      setThemeMode(theme);
    } catch {
      /* 主题没写进去不影响设置本身 */
    }
    // 写盘成功后把 store 的 settings 拉回来：core 是唯一真相源，
    // 而且 downloadDir 在 core 那边是 PathBuf（回传时要经过 to_string_lossy），
    // 界面照着 draft 自己拼一份未必与 core 存下来的完全一致。
    // 走 run()：拉取失败给一条 toast，不要让它在后台无声失败。
    await run(loadSettings());
  }

  if (!draft) return null;

  // **不要再给下面的根元素加 px-6。** App 的滚动容器已经提供了左右内边距
  // （见 App.tsx 里"左右内边距放在滚动容器上"那段），这里再加一层，设置页的
  // 内容就会比其它页多缩进 24px —— 切过去时整块右移，还跟自己的页面标题对不齐。
  //
  // 这条注释写在 `return (` **之上**、用 `//`，不能写成 `{/* … */}` 塞在
  // `return (` 与根元素之间 —— 那是表达式位置，`{…}` 会被当成空对象字面量，
  // 后面再跟一个元素就是语法错误（实测 `tsc` 报 TS1005/TS2657/TS1128/TS1109）。
  return (
    <div className="max-w-[520px] pb-8">
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
