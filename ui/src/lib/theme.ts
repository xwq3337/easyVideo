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
