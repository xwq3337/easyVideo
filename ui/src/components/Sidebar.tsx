import type { ReactNode } from "react";
import { ChevronDown, Download, Film, Settings } from "lucide-react";
import { cn } from "@/lib/utils";
import { VIDEO_OPS, opLabel, type VideoOp } from "../lib/videoOps";

export type Page =
  | { kind: "download" }
  | { kind: "settings" }
  | { kind: "videoOp"; op: VideoOp };

/** 页签标题。**按页算**而不是查表 —— `Page` 现在带参数，表查不了。 */
export function pageTitle(page: Page): string {
  if (page.kind === "download") return "下载";
  if (page.kind === "settings") return "设置";
  return opLabel(page.op);
}

/** 两个页签是否同一页。**不能再用 `===`** —— 那是按引用比，而 setPage 每次都建新对象，
 *  永远不相等，选中态会一直不亮。 */
function samePage(a: Page, b: Page): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "videoOp" && b.kind === "videoOp") return a.op === b.op;
  return true;
}

type Item = { page: Page; label: string; Icon: typeof Download };

const TOP: ReadonlyArray<Item> = [{ page: { kind: "download" }, label: "下载", Icon: Download }];
const BOTTOM: ReadonlyArray<Item> = [{ page: { kind: "settings" }, label: "设置", Icon: Settings }];

function NavButton({
  item,
  page,
  onNavigate,
}: {
  item: Item;
  page: Page;
  onNavigate: (p: Page) => void;
}) {
  const active = samePage(item.page, page);
  return (
    <button
      onClick={() => onNavigate(item.page)}
      // 选中态不能只靠颜色说：浅灰底对读屏软件不存在，得给语义。
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex items-center gap-2 h-8 px-2 rounded-md text-[13px] transition-colors",
        // 选中态用浅灰底 + 文字转深，**不用品牌色填充** ——
        // Apple 自家的侧边栏（系统设置、访达）就是这样，彩色填充会立刻不像苹果。
        // 底色走 --nav-sel 变量：写死黑色透明在深色模式下会完全看不见。
        active
          ? "bg-[var(--nav-sel)] text-[var(--fg)] font-medium"
          : "text-[var(--fg-muted)] hover:bg-[var(--nav-hover)]"
      )}
    >
      <item.Icon size={16} strokeWidth={1.75} />
      {item.label}
    </button>
  );
}

export default function Sidebar({
  page,
  onNavigate,
  subOpen,
  onToggleSub,
  children,
}: {
  page: Page;
  onNavigate: (p: Page) => void;
  /** 「视频操作」这一组展开没有。**由 App 持有** —— 页面是条件渲染的，
   *  放这里的话切一次页就重置了（与 url 同理）。 */
  subOpen: boolean;
  onToggleSub: () => void;
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
      // 屏幕阅读器要靠它把这块与页面里别的 nav/列表区分开（页面只有这一处导航）。
      aria-label="主导航"
      className="w-[200px] shrink-0 flex flex-col select-none"
      style={{
        background: "var(--bg-subtle)",
        borderRight: "1px solid var(--line)",
      }}
    >
      {/* 52px 给 macOS 的红绿灯（titleBarStyle: Overlay 时它们浮在内容上） */}
      <div className="h-[52px] shrink-0" data-tauri-drag-region />

      <div className="flex flex-col gap-0.5 px-2">
        {TOP.map((it) => (
          <NavButton key={it.label} item={it} page={page} onNavigate={onNavigate} />
        ))}

        {/* 「视频操作」是一个**可展开的组**：点它只展开/收起，不导航。
            那一下的反馈就是子菜单展开 —— 不把"当前在哪一页"与"展开没展开"绑在一起。 */}
        <button
          onClick={onToggleSub}
          aria-expanded={subOpen}
          className={cn(
            "flex items-center gap-2 h-8 px-2 rounded-md text-[13px] transition-colors",
            page.kind === "videoOp"
              ? "text-[var(--fg)] font-medium"
              : "text-[var(--fg-muted)] hover:bg-[var(--nav-hover)]"
          )}
        >
          <Film size={16} strokeWidth={1.75} />
          视频操作
          <ChevronDown
            size={14}
            strokeWidth={2}
            className={cn("ml-auto transition-transform", subOpen && "rotate-180")}
          />
        </button>

        {subOpen &&
          VIDEO_OPS.map(({ op, label, ready }) => (
            <button
              key={op}
              // 未实现的**不导航**，而且**不能用 `disabled`** —— 那会让它连 `title`
              // 悬停提示都出不来，而我们要靠那句提示告诉用户"还没做"。
              // 灰显靠 `cursor-default` 这个工具类：它落在 utilities 层，能压过
              // index.css 里那条全局的 `button:not(:disabled) { cursor: pointer }`。
              onClick={() => ready && onNavigate({ kind: "videoOp", op })}
              aria-disabled={!ready}
              title={ready ? undefined : "还没做"}
              className={cn(
                // 缩进 + 左侧一条细线表达层级（macOS 侧边栏的做法），不用底色块
                "flex items-center gap-2 h-7 pl-7 pr-2 text-[12px] transition-colors",
                "border-l border-[var(--line)] ml-3 rounded-r-md text-[var(--fg-muted)]",
                !ready && "opacity-40 cursor-default",
                ready && "hover:bg-[var(--nav-hover)]",
                samePage(page, { kind: "videoOp", op }) &&
                  "bg-[var(--nav-sel)] text-[var(--fg)] font-medium"
              )}
            >
              {label}
            </button>
          ))}

        {BOTTOM.map((it) => (
          <NavButton key={it.label} item={it} page={page} onNavigate={onNavigate} />
        ))}
      </div>

      <div className="flex-1" />
      <div style={{ borderTop: "1px solid var(--line)" }}>{children}</div>
    </nav>
  );
}
