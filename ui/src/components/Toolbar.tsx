import { useState } from "react";
import WindowControls from "./WindowControls";
import { normalizeUrl } from "../lib/url";
import { dispatch } from "../store/store";

interface Props {
  /** 返回是否真的建了任务 —— 决定清不清输入框（失败时留住，用户能改一改再试）。 */
  onAdd: (url: string) => Promise<boolean>;
  onOpenSettings: () => void;
}

export default function Toolbar({ onAdd, onOpenSettings }: Props) {
  const [url, setUrl] = useState("");
  const [adding, setAdding] = useState(false);

  async function submit() {
    if (adding) return; // 防连点：一次只提交一个，否则双击会拿同一条 URL 建两个任务
    const normalized = normalizeUrl(url);
    if (normalized === null) {
      // 空输入按旧行为静默忽略；非空但不是 http(s)（egui 版原话）才提示，且**不建任务**。
      if (url.trim() !== "") dispatch({ kind: "pushToast", text: "请输入 http(s) 链接" });
      return;
    }
    setAdding(true);
    try {
      // 失败（add_url 走 run，失败只落一条 toast）时把 URL 留在框里：
      // 用户要重试/改的就是它，清掉等于逼他重新粘贴一次。成功才清空。
      if (await onAdd(normalized)) setUrl("");
    } finally {
      setAdding(false);
    }
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
        disabled={adding || url.trim() === ""}
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
