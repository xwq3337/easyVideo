import * as SelectPrimitive from "@radix-ui/react-select";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * 下拉选择。radix 原语 + **本仓库自己的 token**。
 *
 * **为什么不直接用 `shadcn add select` 的产物**：那套源码引用的是 shadcn 默认的
 * 类名（`bg-popover` / `bg-accent` / `text-muted-foreground` / `border-input` /
 * `ring-ring` …），而它们要靠 `@theme` 把 `--color-*` 映射进来才存在 ——
 * `index.css` 里没有 `@theme` 块，实测这些类名在产物 CSS 里一个都没有。
 * 直接放进来得到的是一个没有底色、没有边框的浮层。
 * 同目录的 `tabs.tsx` 也是这么改写的（用 `var(--track)` / `var(--fg-muted)`），
 * 这里沿用同一条路，两个组件才不会各说各的配色。
 */
export const Select = SelectPrimitive.Root;

export function SelectTrigger({
  className,
  children,
  ...props
}: SelectPrimitive.SelectTriggerProps) {
  return (
    <SelectPrimitive.Trigger
      className={cn(
        // 高度/圆角/边框/内陷底色**与 `Input` 逐项对齐**：两者常并排出现
        // （比如"保存到"是 Input、"编码器"是 Select），差一档就一眼看得出来。
        "inline-flex items-center justify-between gap-2 h-8 rounded-lg border px-3 text-[13px] outline-none",
        "bg-[var(--bg-inset)] border-[var(--line-strong)] text-[var(--fg)]",
        "data-[placeholder]:text-[var(--fg-faint)]",
        "focus:border-[var(--accent-blue)] focus:ring-2 focus:ring-[var(--accent-blue-ring)]",
        // 展开时禁用指针事件（radix 的约定），把光标从手型改回默认，
        // 否则浮层打开期间触发器仍显示"可点"。
        "data-[state=open]:border-[var(--accent-blue)] disabled:opacity-40 disabled:pointer-events-none",
        className
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDown size={14} strokeWidth={2} style={{ color: "var(--fg-muted)" }} />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

export function SelectValue(props: SelectPrimitive.SelectValueProps) {
  return <SelectPrimitive.Value {...props} />;
}

export function SelectContent({
  className,
  children,
  ...props
}: SelectPrimitive.SelectContentProps) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        // popper 而不是 item-aligned：后者的定位依赖触发器尺寸，而这个 Select
        // 的宽度是定死的（w-48 之类），popper 让浮层与触发器左对齐更整齐。
        position="popper"
        sideOffset={4}
        className={cn(
          // 底色用 --bg 而不是 --bg-subtle：浮层压在页面内容之上，必须比它更亮/更"前"，
          // 而这套灰阶里 --bg 就是最前的那一支。
          "z-50 min-w-[var(--radix-select-trigger-width)] overflow-hidden rounded-lg border",
          "bg-[var(--bg)] border-[var(--line-strong)] text-[var(--fg)]",
          "shadow-[0_4px_16px_rgba(0,0,0,.12)]",
          className
        )}
        {...props}
      >
        <SelectPrimitive.Viewport className="p-1">{children}</SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

export function SelectItem({ className, children, ...props }: SelectPrimitive.SelectItemProps) {
  return (
    <SelectPrimitive.Item
      className={cn(
        "relative flex items-center h-7 rounded-[6px] pl-7 pr-6 text-[13px] outline-none cursor-pointer select-none",
        // 高亮/选中都走侧边栏那两支已存在的悬停变量，不新开 token
        "data-[highlighted]:bg-[var(--nav-hover)] data-[highlighted]:text-[var(--fg)]",
        "data-[state=checked]:font-medium",
        "data-[disabled]:opacity-40 data-[disabled]:pointer-events-none",
        className
      )}
      {...props}
    >
      <span className="absolute left-2 flex size-3.5 items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <Check size={13} strokeWidth={2.5} />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  );
}
