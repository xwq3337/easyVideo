import { useEffect, useState } from "react";
import Toolbar from "./components/Toolbar";
import TaskList from "./components/TaskList";
import SettingsDialog from "./components/SettingsDialog";
import ChoiceDialog from "./components/ChoiceDialog";
import StatusBar from "./components/StatusBar";
import Toasts from "./components/Toasts";
import * as ipc from "./lib/ipc";
import { startBridge, hydrate, loadSettings, run } from "./store/store";
import { readThemeMode, setThemeMode } from "./lib/theme";

export default function App() {
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => {
    // brief 的 Step 10 只调了 applyTheme 一次；可 Step 11 第 5 条要求"把系统切到深色、窗口立刻变深"，
    // 那就得订阅 prefers-color-scheme。setThemeMode 把"上色 + 订阅"绑成一件事，
    // 设置对话框改外观时也走它 —— 否则从"强制浅色"切到"跟随系统"会只上色、不订阅（见报告）。
    setThemeMode(readThemeMode());
    void startBridge();
    void loadSettings();
  }, []);

  /** 返回是否真的建了任务，Toolbar 据此决定清不清输入框。 */
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
    <div className="flex flex-col h-full">
      <Toolbar onAdd={addUrl} onOpenSettings={() => setSettingsOpen(true)} />
      <main className="flex-1 overflow-y-auto">
        <TaskList />
      </main>
      <ChoiceDialog />
      <StatusBar version="v0.1.0" />
      <Toasts />
      <SettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </div>
  );
}
