import { useEffect, useState } from "react";
import type { SettingsDto } from "../lib/types";
import * as ipc from "../lib/ipc";
import { loadSettings, run, useSettings } from "../store/store";
import { readThemeMode, setThemeMode, type ThemeMode } from "../lib/theme";

interface Props {
  open: boolean;
  onClose: () => void;
}

/** 线程数/并发任务数的上界，与 core 的接受范围一致（egui 版是这两个取值的滑块）。 */
const SEGMENT_CONCURRENCY = { min: 1, max: 16 } as const;
const MAX_ACTIVE_TASKS = { min: 1, max: 8 } as const;

/**
 * 把输入框的原始字符串夹进 [lo, hi]。`<input type="number" min max>` 挡不住任何东西 ——
 * 程序化设值、粘贴、科学计数法都能绕过去，而 save() 是直接把 draft 发给 core 的。
 * 不夹的话 `99999` 会落在 serde 的原始英文报错上、`-5`/`1e30` 同理，那些英文会原样变成
 * toast 文案。非法/空值退回 fallback（等价于 egui 滑块拖不动的那一格）。
 */
function clampInt(raw: string, lo: number, hi: number, fallback: number): number {
  const n = Number(raw);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(hi, Math.max(lo, Math.round(n)));
}

/**
 * 设置对话框。**只有点"保存"才写盘** —— 控件变化一律只落在本地 `draft` 上，
 * 不碰 core。（egui 版每帧落盘的教训。）"取消"、Esc、点遮罩都直接丢弃 draft。
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

  // 外观与错误横幅**不在 draft 里**（外观住在 theme.ts 的 localStorage，不在 SettingsDto），
  // 所以上面那条 effect 管不到它们。不一起重置的话：改外观 → 取消（正确地没生效）→ 重开，
  // 下拉框仍显示那个被取消的值，再随便改点别的按保存，它就被**真的**应用并落下去了 ——
  // 「取消丢弃草稿」对外观这一格就是假的，取消被推迟了而已。重开时一律以"当前真正生效的"为准。
  // error 同理：一次失败的保存留下的红字，不该在取消重开之后还挂在那儿。
  useEffect(() => {
    if (!open) return;
    setTheme(readThemeMode());
    setError(null);
  }, [open]);

  // Esc 关闭。挂 window 而不是挂在遮罩上：遮罩要能收键盘事件得先有焦点，
  // 而这里没有任何元素被自动聚焦过。（Esc 与点遮罩同义：不写盘，丢弃 draft。）
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open || !draft) return null;
  const d = draft;

  async function pick() {
    // 走 run()：命令失败变成一条 toast，而不是一个没人接的 rejected promise。
    const picked = await run(ipc.pickDownloadDir());
    if (picked) setDraft((prev) => (prev ? { ...prev, downloadDir: picked } : prev));
  }

  async function save() {
    // 提交前再夹一次：onChange 已夹过，但"onChange 一定跑过"这件事不该由 save 来假设
    // （draft 若被别的路径设成越界值，这里就是唯一还站着的闸）。
    const toSave = {
      ...d,
      segmentConcurrency: clampInt(
        String(d.segmentConcurrency),
        SEGMENT_CONCURRENCY.min,
        SEGMENT_CONCURRENCY.max,
        SEGMENT_CONCURRENCY.min
      ),
      maxActiveTasks: clampInt(
        String(d.maxActiveTasks),
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
                prev
                  ? {
                      ...prev,
                      segmentConcurrency: clampInt(
                        e.target.value,
                        SEGMENT_CONCURRENCY.min,
                        SEGMENT_CONCURRENCY.max,
                        SEGMENT_CONCURRENCY.min
                      ),
                    }
                  : prev
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
                prev
                  ? {
                      ...prev,
                      maxActiveTasks: clampInt(
                        e.target.value,
                        MAX_ACTIVE_TASKS.min,
                        MAX_ACTIVE_TASKS.max,
                        MAX_ACTIVE_TASKS.min
                      ),
                    }
                  : prev
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
          // 不再自己加「保存失败：」前缀 —— core 的 update_settings 失败时回的已经是
          // 「保存设置失败：…」（commands.rs:73），再套一层会读成「保存失败：保存设置失败：…」。
          // 直接照抄 core 的话，它本来就说清了这是一次保存的失败。
          <p className="mb-2" style={{ color: "var(--accent-err)" }}>
            {error}
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
