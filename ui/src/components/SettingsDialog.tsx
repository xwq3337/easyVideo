import { useEffect, useState } from "react";
import type { SettingsDto } from "../lib/types";
import * as ipc from "../lib/ipc";
import { loadSettings, run, useSettings } from "../store/store";
import { applyTheme, readThemeMode, type ThemeMode } from "../lib/theme";

interface Props {
  open: boolean;
  onClose: () => void;
}

/**
 * 设置对话框。**只有点"保存"才写盘** —— 控件变化一律只落在本地 `draft` 上，
 * 不碰 core。（egui 版每帧落盘的教训。）"取消"（以及点遮罩）直接丢弃 draft。
 */
export default function SettingsDialog({ open, onClose }: Props) {
  const current = useSettings();
  const [draft, setDraft] = useState<SettingsDto | null>(null);
  const [theme, setTheme] = useState<ThemeMode>(readThemeMode());
  const [error, setError] = useState<string | null>(null);

  // 每次打开都从 store 里的当前设置重新起草。保存后 store 会被刷新（见 save()），
  // 所以再打开看到的是**真正存下去**的值，而不是打开前那一份。
  useEffect(() => {
    if (open && current) setDraft({ ...current });
  }, [open, current]);

  if (!open || !draft) return null;
  const d = draft;

  async function pick() {
    // 走 run()：命令失败变成一条 toast，而不是一个没人接的 rejected promise。
    const picked = await run(ipc.pickDownloadDir());
    if (picked) setDraft((prev) => (prev ? { ...prev, downloadDir: picked } : prev));
  }

  async function save() {
    try {
      await ipc.updateSettings(d);
    } catch (e) {
      setError(String(e));
      return;
    }
    setError(null);
    // applyTheme 只写 localStorage，是同步的；万一它抛（webview 禁用存储之类），
    // 设置其实已经落盘成功，不该反过来报"保存失败"，所以单独兜一下 ——
    // save() 是 `void save()` 调起来的，漏出去的异常没人接。
    try {
      applyTheme(theme);
    } catch {
      /* 主题没写进去不影响设置本身 */
    }
    // 写盘成功后把 store 的 settings 拉回来：core 是唯一真相源，
    // 而且 downloadDir 在 core 那边是 PathBuf（回传时要经过 to_string_lossy），
    // 界面照着 draft 自己拼一份未必与 core 存下来的完全一致。
    // 走 run()：拉取失败给一条 toast，不要让它在后台无声失败。
    await run(loadSettings());
    onClose();
  }

  return (
    <div
      className="fixed inset-0 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,.35)" }}
      onClick={onClose}
    >
      <div
        className="w-[520px] rounded-lg p-5"
        style={{ background: "var(--bg)", border: "1px solid var(--line-strong)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-[15px] font-semibold mb-4">设置</h2>

        <label className="block mb-4">
          <span style={{ color: "var(--fg-muted)" }}>保存位置</span>
          <div className="flex gap-2 mt-1">
            <input
              readOnly
              value={d.downloadDir}
              className="flex-1 min-w-0 rounded-md px-2.5 py-1.5 border"
              style={{ background: "var(--bg-inset)", borderColor: "var(--line-strong)" }}
            />
            <button
              onClick={() => void pick()}
              className="rounded-md px-3 py-1.5 border"
              style={{ borderColor: "var(--line-strong)" }}
            >
              更改
            </button>
          </div>
          <span className="text-[12px]" style={{ color: "var(--fg-faint)" }}>
            只影响之后新建的任务；正在下载的任务沿用创建时的目录。
          </span>
        </label>

        <label className="block mb-4">
          <span style={{ color: "var(--fg-muted)" }}>线程数（1–16）</span>
          <input
            type="number"
            min={1}
            max={16}
            value={d.segmentConcurrency}
            onChange={(e) =>
              setDraft((prev) =>
                prev ? { ...prev, segmentConcurrency: Number(e.target.value) || 1 } : prev
              )
            }
            className="w-24 rounded-md px-2.5 py-1.5 border mt-1"
            style={{ background: "var(--bg-inset)", borderColor: "var(--line-strong)" }}
          />
        </label>

        <label className="block mb-4">
          <span style={{ color: "var(--fg-muted)" }}>同时下载任务数</span>
          <input
            type="number"
            min={1}
            max={8}
            value={d.maxActiveTasks}
            onChange={(e) =>
              setDraft((prev) =>
                prev ? { ...prev, maxActiveTasks: Number(e.target.value) || 1 } : prev
              )
            }
            className="w-24 rounded-md px-2.5 py-1.5 border mt-1"
            style={{ background: "var(--bg-inset)", borderColor: "var(--line-strong)" }}
          />
        </label>

        <label className="block mb-4">
          <span style={{ color: "var(--fg-muted)" }}>外观</span>
          <select
            value={theme}
            onChange={(e) => setTheme(e.target.value as ThemeMode)}
            className="w-40 rounded-md px-2.5 py-1.5 border mt-1"
            style={{ background: "var(--bg-inset)", borderColor: "var(--line-strong)" }}
          >
            <option value="system">跟随系统</option>
            <option value="light">浅色</option>
            <option value="dark">深色</option>
          </select>
        </label>

        <label className="flex items-center gap-2 mb-2">
          <input
            type="checkbox"
            checked={d.ytdlpEnabled}
            onChange={(e) =>
              setDraft((prev) => (prev ? { ...prev, ytdlpEnabled: e.target.checked } : prev))
            }
          />
          <span>用 yt-dlp 解析网页视频（需自行安装 yt-dlp）</span>
        </label>

        <label className="block mb-4">
          <span style={{ color: "var(--fg-muted)" }}>ffmpeg 路径覆盖（留空则用内嵌的）</span>
          <input
            value={d.ffmpegPathOverride ?? ""}
            onChange={(e) =>
              setDraft((prev) =>
                prev ? { ...prev, ffmpegPathOverride: e.target.value || null } : prev
              )
            }
            className="w-full rounded-md px-2.5 py-1.5 border mt-1"
            style={{ background: "var(--bg-inset)", borderColor: "var(--line-strong)" }}
          />
        </label>

        {error && (
          <p className="mb-2" style={{ color: "var(--accent-err)" }}>
            保存失败：{error}
          </p>
        )}

        <div className="flex justify-end gap-2 mt-5">
          <button
            onClick={onClose}
            className="rounded-md px-3 py-1.5 border"
            style={{ borderColor: "var(--line-strong)" }}
          >
            取消
          </button>
          <button
            onClick={() => void save()}
            className="rounded-md px-3 py-1.5 font-medium"
            style={{ background: "var(--fill)", color: "var(--btn-fg)" }}
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
