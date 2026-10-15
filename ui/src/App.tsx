import { useEffect, useState } from "react";
import Toolbar from "./components/Toolbar";
import TaskList from "./components/TaskList";
import StatusBar from "./components/StatusBar";
import Toasts from "./components/Toasts";
import * as ipc from "./lib/ipc";
import { startBridge, hydrate, loadSettings, run } from "./store/store";
import { applyTheme, readThemeMode, watchSystemTheme } from "./lib/theme";

export default function App() {
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    const mode = readThemeMode();
    applyTheme(mode);
    // brief 的 Step 10 只调了 applyTheme 一次；可 Step 11 第 5 条要求"把系统切到深色、窗口立刻变深"，
    // 那就得订阅 prefers-color-scheme。brief 自己给了 watchSystemTheme，正是干这个的，接上去（见报告）。
    const stopWatching = watchSystemTheme(mode, () => applyTheme(mode));
    void startBridge();
    void loadSettings();
    return stopWatching;
  }, []);

  async function addUrl(url: string) {
    // run() 失败时只加一条提示，不抛出去（命令级失败不打断操作）
    if ((await run(ipc.addUrl(url))) === undefined) return;
    // 立刻拉一次：add_url 返回 id 之前 core 已经在发这个任务的事件，
    // 只靠事件的话第一帧会晚到（store 里那条"未知 id 触发重同步"是兜底，不是主路径）。
    await hydrate();
  }

  return (
    <div className="flex flex-col h-full">
      <Toolbar onAdd={addUrl} onOpenSettings={() => setSettingsOpen(true)} />
      <main className="flex-1 overflow-y-auto">
        <TaskList />
      </main>
      <StatusBar version="v0.1.0" />
      <Toasts />
      {settingsOpen && <div />}
    </div>
  );
}
