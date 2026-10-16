import { useEffect, useState } from "react";
import Sidebar, { pageTitle, type Page } from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import DownloadPage from "./pages/DownloadPage";
import VideoOpPage from "./pages/VideoOpPage";
import SettingsPage from "./pages/SettingsPage";
import ChoiceDialog from "./components/ChoiceDialog";
import Toasts from "./components/Toasts";
import WindowControls from "./components/WindowControls";
import * as ipc from "./lib/ipc";
import { startBridge, hydrate, loadSettings, run } from "./store/store";
import { readThemeMode, setThemeMode } from "./lib/theme";

export default function App() {
  // 三个页面用 useState 切就够了，不引 react-router —— 桌面 App 没有 URL 的需求。
  const [page, setPage] = useState<Page>({ kind: "download" });
  // 侧边栏「视频操作」那一组展开没有。放这里而不是 Sidebar 内部：三个页面是条件渲染的，
  // 切页会卸载整棵子树，放里面一次切页就重置了（与下面的 url 同理）。
  const [subOpen, setSubOpen] = useState(false);

  // 输入框的内容**放在这里，不放 DownloadPage 里**：三个页面是条件渲染的，
  // 切去设置页会让下载页整棵子树卸载，写了一半、还没提交的链接就跟着没了
  // （旧版工具条常驻，所以这是这次改版引入的行为回退）。拿到 App 上之后，
  // 切页只是换渲染的子树，输入框里的字原样还在。
  const [url, setUrl] = useState("");

  useEffect(() => {
    setThemeMode(readThemeMode());
    void startBridge();
    void loadSettings();
  }, []);

  /** 返回是否真的建了任务，DownloadPage 据此决定清不清输入框。 */
  async function addUrl(url: string): Promise<boolean> {
    // run() 失败时只加一条提示，不抛出去（命令级失败不打断操作）
    if ((await run(ipc.addUrl(url))) === undefined) return false;
    // 立刻拉一次：add_url 返回 id 之前 core 已经在发这个任务的事件，
    // 只靠事件的话第一帧会晚到（store 里那条"未知 id 触发重同步"是兜底，不是主路径）。
    // hydrate 自带 catch，失败也只落一条 toast，不会把 rejection 抛回这里。
    await hydrate();
    return true;
  }

  return (
    <div className="flex h-full">
      <Sidebar
        page={page}
        onNavigate={setPage}
        subOpen={subOpen}
        onToggleSub={() => setSubOpen((v) => !v)}
      >
        <StatusBar version="v0.1.0" />
      </Sidebar>

      <main className="flex-1 min-w-0 flex flex-col">
        {/* 非 macOS 的自绘窗口键挂在这里；macOS 上 WindowControls 按设计不渲染 */}
        <header
          // 同样必须 `"deep"`：裸属性只让 header 自身的留白可拖，
          // 标题文字 `<h1>` 盖住的那部分拖不动。按钮仍然可点（drag.js:58）。
          data-tauri-drag-region="deep"
          className="flex items-center justify-between px-6 pt-5 pb-3 select-none"
        >
          <h1 className="text-[22px] font-semibold" style={{ letterSpacing: "-0.01em" }}>
            {pageTitle(page)}
          </h1>
          <WindowControls />
        </header>

        {/* 左右内边距放在**滚动容器**上，不要塞进 DownloadPage 内部的包里。
            TaskList 的空状态靠 `h-full flex items-center justify-center` 做垂直居中，
            而百分比高度需要一个**确定高度**的父级；中间多包一层 auto 高的 div，
            h-full 就退化成 auto，那句"粘贴一个视频链接"会贴着输入行而不是居中。 */}
        <div className="flex-1 overflow-y-auto px-6">
          {page.kind === "download" && (
            <DownloadPage url={url} onUrlChange={setUrl} onAdd={addUrl} />
          )}
          {page.kind === "videoOp" && <VideoOpPage op={page.op} />}
          {page.kind === "settings" && <SettingsPage />}
        </div>
      </main>

      <ChoiceDialog />
      <Toasts />
    </div>
  );
}
