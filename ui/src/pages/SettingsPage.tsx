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
