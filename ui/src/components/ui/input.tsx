import * as React from "react";
import { cn } from "@/lib/utils";

export function Input({ className, ...props }: React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cn(
        // 高度对齐 Button 的 md（h-8）：两者在同一条 flex 行里肩并肩，
        // 靠 padding 撑出来的 ~30px 会比 32px 的按钮矮一截，一眼看得出没对齐。
        "w-full min-w-0 h-8 rounded-lg border px-3 text-[13px] outline-none",
        "bg-[var(--bg-inset)] border-[var(--line-strong)] text-[var(--fg)]",
        "placeholder:text-[var(--fg-faint)]",
        "focus:border-[var(--accent-blue)] focus:ring-2 focus:ring-[var(--accent-blue-ring)]",
        className
      )}
      {...props}
    />
  );
}
