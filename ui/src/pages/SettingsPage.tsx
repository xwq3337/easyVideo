import { useEffect, useRef, useState } from "react";
import type { ExtensionConfigDto, SettingsDto } from "../lib/types";
import * as ipc from "../lib/ipc";
import { loadSettings, run, useSettings } from "../store/store";
import { readThemeMode, setThemeMode, type ThemeMode } from "../lib/theme";
import { clampInt } from "../lib/clampInt";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

const SEGMENT_CONCURRENCY = { min: 1, max: 16 } as const;
const MAX_ACTIVE_TASKS = { min: 1, max: 8 } as const;
/** 与 extension_api.rs 的 `validated` 保持一致；这里只是提前挡一下。 */
const EXTENSION_PORT = { min: 1024, max: 65535 } as const;

/**
 * 与 core 的 `settings::BROWSERS` 必须一致 —— 这份名单只是为了把选项画出来，
 * 真正的闸在 core（`sanitize_browser`）。前端漏掉一个只是选项少一个，
 * 前端多出一个才会被 core 静默丢掉。
 */
/** 「不读取」的哨兵值。radix 不允许 SelectItem 的 value 为空串。 */
const NO_BROWSER = "__none__";

const BROWSERS = [
  { value: "chrome", label: "Chrome" },
  { value: "firefox", label: "Firefox" },
  { value: "safari", label: "Safari" },
  { value: "edge", label: "Edge" },
  { value: "brave", label: "Brave" },
  { value: "chromium", label: "Chromium" },
  { value: "vivaldi", label: "Vivaldi" },
  { value: "opera", label: "Opera" },
  { value: "whale", label: "Whale" },
] as const;

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

  // 扩展配置是**另一份**落盘文件，与 draft 分开：主设置页的「保存」不碰它。
  const [extension, setExtension] = useState<ExtensionConfigDto | null>(null);
  const [extensionError, setExtensionError] = useState<string | null>(null);
  const [extensionStatus, setExtensionStatus] = useState<string | null>(null);
  const [extensionBusy, setExtensionBusy] = useState(false);
  const tokenRow = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (current) setDraft({ ...current });
  }, [current]);

  useEffect(() => {
    let alive = true;
    ipc
      .getExtensionConfig()
      .then((cfg) => {
        if (alive) setExtension(cfg);
      })
      .catch((e) => {
        if (alive) setExtensionError(String(e));
      });
    return () => {
      alive = false;
    };
  }, []);

  async function copyToken() {
    if (!extension) return;
    try {
      await navigator.clipboard.writeText(extension.token);
      setExtensionStatus("访问令牌已复制");
      return;
    } catch {
      // 某些 webview 里 Clipboard API 不可用：退回选中 + execCommand。
    }
    const input = tokenRow.current?.querySelector("input");
    input?.focus();
    input?.select();
    try {
      document.execCommand("copy");
      setExtensionStatus("访问令牌已复制");
    } catch {
      setExtensionStatus("复制失败，请手动选中令牌复制");
    }
  }

  async function saveExtension() {
    if (!extension) return;
    setExtensionBusy(true);
    setExtensionError(null);
    setExtensionStatus(null);
    try {
      const saved = await ipc.updateExtensionConfig(extension);
      setExtension(saved);
      setExtensionStatus(`已保存，正在监听 127.0.0.1:${saved.port}`);
    } catch (e) {
      setExtensionError(String(e));
    } finally {
      setExtensionBusy(false);
    }
  }

  async function regenerateToken() {
    if (!extension) return;
    if (!window.confirm("重新生成后，浏览器扩展里必须填入新令牌才能继续下载。继续吗？")) {
      return;
    }
    setExtensionBusy(true);
    setExtensionError(null);
    setExtensionStatus(null);
    try {
      const next = await ipc.regenerateExtensionToken();
      setExtension(next);
      setExtensionStatus("已生成新令牌。请把它粘贴到浏览器扩展的设置页。");
    } catch (e) {
      setExtensionError(String(e));
    } finally {
      setExtensionBusy(false);
    }
  }

  async function testExtension() {
    if (!extension) return;
    setExtensionBusy(true);
    setExtensionError(null);
    setExtensionStatus(null);
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort(), 3000);
    try {
      const resp = await fetch(`http://127.0.0.1:${extension.port}/api/v1/ping`, {
        signal: controller.signal,
      });
      const data = (await resp.json()) as { ok?: boolean; version?: string };
      if (resp.ok && data.ok) {
        setExtensionStatus(`连接成功：easyVideo ${data.version ?? ""}`.trim());
      } else {
        setExtensionError(`端口 ${extension.port} 有响应，但不是 easyVideo 的接口`);
      }
    } catch {
      setExtensionError(
        `连不上 127.0.0.1:${extension.port}。请确认端口已保存，且 easyVideo 正在运行。`
      );
    } finally {
      window.clearTimeout(timer);
      setExtensionBusy(false);
    }
  }

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

      {/* ↓ 下面三块是"站点不让我下"那一类问题的总出口。Cloudflare 人机验证
          挡的是"这次请求看着不像浏览器"，能改的只有两件事：带上你的登录 cookie，
          以及让请求看着像浏览器。摆在 yt-dlp 开关旁边是因为三者是一件事。 */}
      <label className="block mb-5">
        <span style={{ color: "var(--fg-muted)" }}>读取 cookie 的浏览器</span>
        {/* radix 的 SelectItem **不接受空字符串** value（会直接抛错），
            所以"不读取"用一个哨兵值，落到 draft 里再转回 null。 */}
        <Select
          value={draft.cookiesFromBrowser ?? NO_BROWSER}
          onValueChange={(v) =>
            setDraft((p) => (p ? { ...p, cookiesFromBrowser: v === NO_BROWSER ? null : v } : p))
          }
        >
          <SelectTrigger className="w-48 mt-1.5">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NO_BROWSER}>不读取</SelectItem>
            {BROWSERS.map((b) => (
              <SelectItem key={b.value} value={b.value}>
                {b.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
          遇到"登录才能看""请先验证"的站点时选它。首次读取系统会要求授权。
        </span>
      </label>

      <label className="flex items-center gap-2 mb-1.5 text-[13px]">
        <input
          type="checkbox"
          checked={draft.impersonate}
          onChange={(e) => setDraft((p) => (p ? { ...p, impersonate: e.target.checked } : p))}
        />
        <span>伪装浏览器指纹（对付 Cloudflare 等验证）</span>
      </label>
      <span className="block text-[12px] mb-5" style={{ color: "var(--fg-faint)" }}>
        需要 yt-dlp 装有 curl_cffi，否则会提示"伪装不可用"。
      </span>

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

      {/* ===== 浏览器扩展 ===== */}
      {/*
        这一块**不受上面「保存」管辖**：它有自己的落盘文件与命令。视觉上必须与
        下载设置隔开，否则用户改完端口点上面的保存，会以为已经生效。
      */}
      {extension && (
        <div
          className="mt-8 pt-6 border-t"
          style={{ borderColor: "var(--line)" }}
        >
          <div className="flex items-start justify-between gap-3">
            <div>
              <div style={{ color: "var(--fg-muted)" }}>浏览器扩展</div>
              <span className="text-[12px]" style={{ color: "var(--fg-faint)" }}>
                在浏览器里捕获视频链接，点击下载时发送到本应用。
              </span>
            </div>
            <Button
              className="shrink-0"
              onClick={() => void testExtension()}
              disabled={extensionBusy}
            >
              测试连接
            </Button>
          </div>

          <label className="block mt-4">
            <span className="text-[13px]" style={{ color: "var(--fg-muted)" }}>
              监听端口（1024–65535）
            </span>
            <Input
              type="number"
              className="w-32 mt-1.5"
              value={extension.port}
              onChange={(e) =>
                setExtension((p) =>
                  p
                    ? {
                        ...p,
                        port: clampInt(
                          e.target.value,
                          EXTENSION_PORT.min,
                          EXTENSION_PORT.max,
                          EXTENSION_PORT.min
                        ),
                      }
                    : p
                )
              }
            />
            <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
              默认 27853。改端口后，扩展选项里也要填同一个值。
            </span>
          </label>

          <div className="mt-4" ref={tokenRow}>
            <span className="text-[13px]" style={{ color: "var(--fg-muted)" }}>
              访问令牌
            </span>
            <div className="flex gap-2 mt-1.5">
              <Input
                readOnly
                className="font-mono text-[12px]"
                value={extension.token}
                onFocus={(e) => e.currentTarget.select()}
              />
              <Button className="shrink-0" onClick={() => void copyToken()}>
                复制
              </Button>
              <Button
                className="shrink-0"
                onClick={() => void regenerateToken()}
                disabled={extensionBusy}
              >
                重新生成
              </Button>
            </div>
            <span className="block text-[12px]" style={{ color: "var(--fg-faint)" }}>
              把它粘贴到扩展的设置页。令牌只保存在本机。
            </span>
          </div>

          <div className="flex items-center gap-3 mt-4">
            <Button
              variant="primary"
              onClick={() => void saveExtension()}
              disabled={extensionBusy}
            >
              保存扩展设置
            </Button>
            {extensionStatus && (
              <span className="text-[12px]" style={{ color: "var(--accent-ok)" }}>
                {extensionStatus}
              </span>
            )}
            {extensionError && (
              <span className="text-[12px]" style={{ color: "var(--accent-err)" }}>
                {extensionError}
              </span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
