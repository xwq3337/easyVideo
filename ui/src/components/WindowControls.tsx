import { getCurrentWindow } from "@tauri-apps/api/window";

/** 仅 Windows/Linux 渲染：macOS 用 titleBarStyle: Overlay，红绿灯由系统画。 */
export default function WindowControls() {
  const isMac = navigator.userAgent.includes("Mac");
  if (isMac) return null;

  const w = getCurrentWindow();
  return (
    <div className="flex items-center">
      <button
        aria-label="最小化"
        className="w-11 h-8 hover:bg-[var(--bg-inset)] transition-colors"
        onClick={() => void w.minimize()}
      >
        ─
      </button>
      <button
        aria-label="最大化"
        className="w-11 h-8 hover:bg-[var(--bg-inset)] transition-colors"
        onClick={() => void w.toggleMaximize()}
      >
        ☐
      </button>
      <button
        aria-label="关闭"
        className="w-11 h-8 text-[var(--accent-err)] hover:bg-[var(--accent-err)] hover:text-white transition-colors"
        onClick={() => void w.close()}
      >
        ✕
      </button>
    </div>
  );
}
