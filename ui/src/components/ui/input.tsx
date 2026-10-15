import * as React from "react";
import { cn } from "@/lib/utils";

export function Input({ className, ...props }: React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cn(
        "w-full min-w-0 rounded-lg border px-3 py-1.5 text-[13px] outline-none",
        "bg-[var(--bg-inset)] border-[var(--line-strong)] text-[var(--fg)]",
        "placeholder:text-[var(--fg-faint)]",
        "focus:border-[var(--accent-blue)] focus:ring-2 focus:ring-[var(--accent-blue-ring)]",
        className
      )}
      {...props}
    />
  );
}
