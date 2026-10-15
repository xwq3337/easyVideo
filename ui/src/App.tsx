import { useEffect, useState } from "react";
import Sidebar, { type Page } from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import DownloadPage from "./pages/DownloadPage";
import VideoOpsPage from "./pages/VideoOpsPage";
import SettingsPage from "./pages/SettingsPage";
import ChoiceDialog from "./components/ChoiceDialog";
import Toasts from "./components/Toasts";
import WindowControls from "./components/WindowControls";
import * as ipc from "./lib/ipc";
import { startBridge, hydrate, loadSettings, run } from "./store/store";
import { readThemeMode, setThemeMode } from "./lib/theme";

const TITLES: Record<Page, string> = {
  download: "下载",
  videoOps: "视频操作",
  settings: "设置",
};

export default function App() {
  // 三个页面用 useState 切就够了，不引 react-router —— 桌面 App 没有 URL 的需求。
  const [page, setPage] = useState<Page>("download");

  useEffect(() => {
    setThemeMode(readThemeMode());
    void startBridge();
    void loadSettings();
  }, []);

  /** 返回是否真的建了任务，DownloadPage 据此决定清不清输入框。 */
  async function addUrl(url: string): Promise<boolean> {
    if ((await run(ipc.addUrl(url))) === undefined) return false;
    await hydrate();
    return true;
  }

  return (
    <div className="flex h-full">
      <Sidebar page={page} onNavigate={setPage}>
        <StatusBar version="v0.1.0" />
      </Sidebar>

      <main className="flex-1 min-w-0 flex flex-col">
        {/* 非 macOS 的自绘窗口键挂在这里；macOS 上 WindowControls 按设计不渲染 */}
        <header
          data-tauri-drag-region
          className="flex items-center justify-between px-6 pt-5 pb-3 select-none"
        >
          <h1 className="text-[22px] font-semibold" style={{ letterSpacing: "-0.01em" }}>
            {TITLES[page]}
          </h1>
          <WindowControls />
        </header>

        <div className="flex-1 overflow-y-auto">
          {page === "download" && <DownloadPage onAdd={addUrl} />}
          {page === "videoOps" && <VideoOpsPage />}
          {page === "settings" && <SettingsPage />}
        </div>
      </main>

      <ChoiceDialog />
      <Toasts />
    </div>
  );
}
