// 从 egui 版的 format_size / format_speed 移植，行为保持一致：
// 1024 进制、保留一位小数、0 也要显示成 "0 B" 而不是空字符串。

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatSize(bytes: number): string {
  if (bytes <= 0) return "0 B";
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i += 1;
  }
  return i === 0 ? `${Math.round(v)} B` : `${v.toFixed(1)} ${UNITS[i]}`;
}

export function formatSpeed(bps: number): string {
  return `${formatSize(bps)}/s`;
}
