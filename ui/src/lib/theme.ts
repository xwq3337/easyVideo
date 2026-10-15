export type ThemeMode = "system" | "light" | "dark";

const KEY = "easydownvideo.theme";

export function readThemeMode(): ThemeMode {
  const v = localStorage.getItem(KEY);
  return v === "light" || v === "dark" ? v : "system";
}

export function applyTheme(mode: ThemeMode): void {
  localStorage.setItem(KEY, mode);
  const systemDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const dark = mode === "dark" || (mode === "system" && systemDark);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

/** 跟随系统时，系统主题变了要跟着变。返回取消订阅函数。 */
export function watchSystemTheme(mode: ThemeMode, onChange: () => void): () => void {
  if (mode !== "system") return () => {};
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}

// 当前这条 prefers-color-scheme 订阅的取消函数。模块级，因为"谁改的模式"不止一个：
// 启动时是 App，用户在设置里改外观时是设置对话框。**订阅必须跟着当前生效的模式走**。
let stopSystemWatch: (() => void) | null = null;

/**
 * 应用某个主题模式，并让"跟随系统"的订阅与它保持一致 —— 换模式时先退订旧的、再按新模式订。
 *
 * 单独用 `applyTheme("system")` 是不够的：如果启动时是"强制浅色"，那一刻
 * `watchSystemTheme` 给的是个空函数（没绑），之后用户切到"跟随系统"时 applyTheme 会按
 * **当时的**系统偏好着一次色 —— 看起来对了，但其实没人订阅，系统主题再变就不跟了，
 * 得重启才恢复。这个函数就是堵这条：模式在哪儿改的、改成什么，订阅就跟着变成什么。
 *
 * 自己管订阅（不把取消函数交回调用方）是故意的：调用方有两个，谁先退订谁后订
 * 的顺序交给这里统一处理，比让两边各自记着 clean up 更不容易错。
 * （StrictMode 下 App 的 effect 会跑两遍，这里第二遍会先退掉第一遍那条。）
 */
export function setThemeMode(mode: ThemeMode): void {
  applyTheme(mode);
  stopSystemWatch?.();
  stopSystemWatch = watchSystemTheme(mode, () => applyTheme(mode));
}
