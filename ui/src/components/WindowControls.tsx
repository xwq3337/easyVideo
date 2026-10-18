import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "lucide-react";

/** 仅 Windows/Linux 渲染：macOS 用 titleBarStyle: Overlay，红绿灯由系统画。 */
export default function WindowControls() {
  const isMac = navigator.userAgent.includes("Mac");
  if (isMac) return null;

  const w = getCurrentWindow();
  return (
    // 贴住窗口右上角：Windows 上这三个键就是标题栏的一部分，不该跟着内容区留白。
    // macOS 在函数开头就 return 了，这里不会影响红绿灯的布局。
    <div className="fixed top-0 right-0 z-50 flex items-center">
      {/* **必须显式居中**：Tailwind preflight 把 svg 设成 display:block，
          不写 flex 的话图标会贴在按钮左侧（垂直方向反而由原生 button 居中）。 */}
      <button
        aria-label="最小化"
        className="w-11 h-8 flex items-center justify-center hover:bg-[var(--bg-inset)] transition-colors"
        onClick={() => void w.minimize()}
      >
        <Minus size={14} strokeWidth={1.75} />
      </button>
      <button
        aria-label="最大化"
        className="w-11 h-8 flex items-center justify-center hover:bg-[var(--bg-inset)] transition-colors"
        onClick={() => void w.toggleMaximize()}
      >
        <Square size={12} strokeWidth={1.75} />
      </button>
      <button
        aria-label="关闭"
        className="w-11 h-8 flex items-center justify-center text-[var(--accent-err)] hover:bg-[var(--accent-err)] hover:text-white transition-colors"
        onClick={() => void w.close()}
      >
        <X size={14} strokeWidth={1.75} />
      </button>
    </div>
  );
}
