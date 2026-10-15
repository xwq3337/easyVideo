import { useState } from "react";
import WindowControls from "./WindowControls";

interface Props {
  onAdd: (url: string) => Promise<void>;
  onOpenSettings: () => void;
  busy?: boolean;
}

export default function Toolbar({ onAdd, onOpenSettings, busy }: Props) {
  const [url, setUrl] = useState("");

  async function submit() {
    const v = url.trim();
    if (!v) return;
    await onAdd(v);
    setUrl("");
  }

  return (
    <header
      // 拖拽区挂在**背景**上，不挂在输入框上 —— 否则框里选不中文字。
      // 左侧 78px 留给 macOS 的红绿灯（titleBarStyle: Overlay 时它们浮在内容上）。
      data-tauri-drag-region
      className="flex items-center gap-2 pl-[78px] pr-0 py-2.5 border-b select-none"
      style={{ borderColor: "var(--line)", background: "var(--bg)" }}
    >
      <input
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void submit();
        }}
        placeholder="粘贴视频链接…"
        spellCheck={false}
        // select-text：header 上的 select-none 会被输入框继承，WebKit 下框里就选不中文字了；
        // 显式把输入框改回可选中（brief 没写这一句，是我们加的，理由见报告）。
        className="flex-1 min-w-0 rounded-md px-2.5 py-1.5 outline-none border select-text"
        style={{
          background: "var(--bg-inset)",
          borderColor: "var(--line-strong)",
          color: "var(--fg)",
        }}
      />
      <button
        onClick={() => void submit()}
        disabled={busy || url.trim() === ""}
        className="rounded-md px-3 py-1.5 font-medium disabled:opacity-40 transition-opacity"
        style={{ background: "var(--fill)", color: "var(--btn-fg)" }}
      >
        下载
      </button>
      <button
        onClick={onOpenSettings}
        aria-label="设置"
        className="rounded-md px-2.5 py-1.5 transition-colors"
        style={{ color: "var(--fg-muted)" }}
      >
        ⚙
      </button>
      <WindowControls />
    </header>
  );
}
