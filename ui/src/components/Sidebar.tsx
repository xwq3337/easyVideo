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
      // **必须是 `"deep"`，不能是裸属性。** Tauri 2.12.4 注入的 `drag.js` 里：
      //   :64  `attr === 'deep'`            → true（子树内任意位置都算）
      //   :66  `attr === '' || 'true'`      → `el === composedPath[0]`（**仅自身**）
      // 而 nav 的每一个像素都被子元素盖住（spacer、导航项容器、flex-1、footer），
      // 它永远不可能是 mousedown 的目标 —— 裸属性写在这里纯粹是装饰，
      // 侧边栏的空白区和底部的 `StatusBar` 都拖不动窗口。
      //
      // 导航项不受影响：drag.js:58 规定"可点击元素且自身没有该属性"直接返回 false。
      data-tauri-drag-region="deep"
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
