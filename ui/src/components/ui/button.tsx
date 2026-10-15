import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

// 主按钮用**胶囊**——这是 apple.com 最 signature 的形状。次级按钮用 6px 圆角，
// 两者的差距本身就是层级信号，不必再加颜色区分。
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-colors disabled:pointer-events-none disabled:opacity-40 outline-none",
  {
    variants: {
      variant: {
        primary:
          // 强调色走变量，**不写死 hex**。写死的话深色模式会一直用浅色那支，
          // spec §5.1 定的深色强调色就成了死代码。
          // hover/active 用亮度滤镜而不是另外两个 hex：变量是唯一来源，滤镜天然跟着主题走。
          "bg-[var(--accent-blue)] text-white hover:brightness-110 active:brightness-95 rounded-full",
        secondary:
          "border border-[var(--line-strong)] text-[var(--fg)] hover:bg-[var(--bg-subtle)] rounded-md",
        ghost:
          "text-[var(--fg-muted)] hover:bg-[var(--bg-subtle)] rounded-md",
        danger: "text-[var(--accent-err)] hover:bg-[var(--bg-subtle)] rounded-md",
      },
      size: {
        sm: "h-7 px-3 text-[12px]",
        md: "h-8 px-4 text-[13px]",
        icon: "size-7",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  }
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

export function Button({ className, variant, size, ...props }: ButtonProps) {
  return <button className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}

export { buttonVariants };
