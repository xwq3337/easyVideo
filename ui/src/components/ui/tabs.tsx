import * as TabsPrimitive from "@radix-ui/react-tabs";
import { cn } from "@/lib/utils";

export const Tabs = TabsPrimitive.Root;

/** 分段控件的外壳：整块浅灰底，靠内层选中项"浮起"来表达当前值。 */
export function TabsList({ className, ...props }: TabsPrimitive.TabsListProps) {
  return (
    <TabsPrimitive.List
      className={cn("inline-flex items-center gap-0.5 rounded-lg p-0.5", className)}
      style={{ background: "var(--bg-inset)" }}
      {...props}
    />
  );
}

export function TabsTrigger({ className, ...props }: TabsPrimitive.TabsTriggerProps) {
  return (
    <TabsPrimitive.Trigger
      className={cn(
        "rounded-[6px] px-3 py-1 text-[12px] transition-colors outline-none",
        // 未选中是灰字；选中项白底 + 深字 + 极淡投影 —— 这正是 macOS 分段控件的样子，
        // 也是 Apple 表达"三选一"最标准的控件。
        "text-[var(--fg-muted)] hover:text-[var(--fg)]",
        "data-[state=active]:bg-[var(--bg)] data-[state=active]:text-[var(--fg)]",
        "data-[state=active]:shadow-[0_1px_2px_rgba(0,0,0,.08)] data-[state=active]:font-medium",
        className
      )}
      {...props}
    />
  );
}
