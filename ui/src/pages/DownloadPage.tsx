import { useState } from "react";
import TaskList from "../components/TaskList";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { normalizeUrl } from "../lib/url";
import { dispatch } from "../store/store";

export default function DownloadPage({ onAdd }: { onAdd: (url: string) => Promise<boolean> }) {
  const [url, setUrl] = useState("");
  const [adding, setAdding] = useState(false);

  async function submit() {
    if (adding) return; // 防连点：一次只提交一个，否则双击会拿同一条 URL 建两个任务
    const normalized = normalizeUrl(url);
    if (normalized === null) {
      if (url.trim() !== "") dispatch({ kind: "pushToast", text: "请输入 http(s) 链接" });
      return;
    }
    setAdding(true);
    try {
      // 失败时把 URL 留在框里：用户要重试/改的就是它，清掉等于逼他重新粘贴一次。
      if (await onAdd(normalized)) setUrl("");
    } finally {
      setAdding(false);
    }
  }

  return (
    <>
      {/* 左右内边距由 App 的滚动容器提供，这里只留纵向的 */}
      <div className="flex items-center gap-2 py-4">
        <Input
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void submit();
          }}
          placeholder="粘贴视频链接…"
          spellCheck={false}
        />
        <Button
          variant="primary"
          onClick={() => void submit()}
          disabled={adding || url.trim() === ""}
        >
          下载
        </Button>
      </div>
      <TaskList />
    </>
  );
}
