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
