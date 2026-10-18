#!/usr/bin/env bash
# 取回内嵌的 ffmpeg（构建脚本在文件缺失时调用）。
#
# **为什么它们不在 git 里**：`windows-x86_64/ffmpeg.exe` 是 100.54 MB，**超过 GitHub
# 单文件 100 MB 的硬上限** —— 只要那个 blob 在历史里，push 就会被 pre-receive 钩子
# 拒掉（GH001: Large files detected）。macOS 那个 63 MB 不触发硬限，但两个加起来
# 压缩后约 67 MiB，每次推送都要重传一遍。所以两个都移出去，构建时按需取。
#
# 用法（一般由 build.rs 自动调用）：
#   scripts/fetch-ffmpeg.sh macos-aarch64
#   scripts/fetch-ffmpeg.sh windows-x86_64
#
# 校验值是 `vendor/ffmpeg/README.md` 那张表里的同一组。**对不上就失败**，
# 绝不悄悄换一个构建进来 —— 那样 `ensure_ffmpeg` 的「释放后自校验」会变成
# 「自己跟自己比」，永远通过。
set -euo pipefail

dir_name="${1:?用法: fetch-ffmpeg.sh <macos-aarch64|windows-x86_64>}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

case "$dir_name" in
  macos-aarch64)
    url="https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip"
    # 按后缀找条目，不写死路径：上游可能给条目套一层目录
    entry_pattern='(^|/)ffmpeg$' 
    out="$repo_root/vendor/ffmpeg/macos-aarch64/ffmpeg"
    want_bytes=66332880
    want_sha="2e11c6f90993cdb79fff84d3f90044d28316b310e75b3e030cfc9a54f2c9d384"
    ;;
  windows-x86_64)
    url="https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
    # **实测**：这个 zip 里的条目是 `ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe`，
    # 不是 README 里写的 `bin/ffmpeg.exe`（那是相对版本目录说的）。版本号会变，
    # 所以按后缀找；内容对不对由 SHA256 把关。
    entry_pattern='(^|/)ffmpeg\.exe$' 
    out="$repo_root/vendor/ffmpeg/windows-x86_64/ffmpeg.exe"
    want_bytes=105423872
    want_sha="3256173f3f8bffd7df12227c68adf68025edb1832273a9530688a7bb1ed8edec"
    ;;
  *)
    echo "未知 target：$dir_name（只支持 macos-aarch64 / windows-x86_64）" >&2
    exit 2
    ;;
esac

sha256_of() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  else echo "找不到 shasum / sha256sum，无法校验" >&2; exit 3
  fi
}

size_of() { wc -c <"$1" | tr -d ' '; }

# 已经就绪就什么都不做。注意这里**连 SHA256 一起验**：只比字节数的话，
# 一个被截断又补零的文件会被当成好的。
if [ -f "$out" ] && [ "$(size_of "$out")" = "$want_bytes" ]; then
  if [ "$(sha256_of "$out")" = "$want_sha" ]; then exit 0; fi
  echo "vendor/ffmpeg/$dir_name 的 SHA256 不符，重新下载" >&2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
zip="$tmp/ffmpeg.zip"

# gyan.dev 对单流限速到 20–30 KB/s（100 MB 要一个多小时），按 accept-ranges 分 8 段并发。
# **内层请求不能带 --retry / -C -**：README 实测过，那组合会把已下载的部分写坏
# （把 65MB 退回 21.9MB）。
fetch_ranged() {
  local url="$1" dest="$2" parts=8
  curl -sfL --max-time 60 -r 0-0 -D "$tmp/hdr.txt" -o /dev/null "$url"
  local len
  len="$(grep -i '^content-range:' "$tmp/hdr.txt" | tail -1 | sed 's#.*/##' | tr -d '\r')"
  case "$len" in ''|*[!0-9]*) echo "拿不到远端文件大小（content-range）" >&2; return 1;; esac
  echo "分 $parts 段下载，共 $len 字节" >&2

  local chunk=$(( (len + parts - 1) / parts ))
  local i start end pids=()
  for i in $(seq 0 $((parts - 1))); do
    start=$(( i * chunk ))
    end=$(( start + chunk - 1 ))
    [ "$end" -ge "$len" ] && end=$(( len - 1 ))
    [ "$start" -ge "$len" ] && break
    curl -sfL --max-time 1800 -r "$start-$end" -o "$tmp/part$i" "$url" &
    pids+=($!)
  done
  local p
  for p in "${pids[@]}"; do wait "$p" || { echo "有一段下载失败" >&2; return 1; }; done

  : >"$dest"
  for i in "${!pids[@]}"; do cat "$tmp/part$i" >>"$dest"; done
}

echo "下载 $url" >&2
if [ "$dir_name" = "windows-x86_64" ]; then
  fetch_ranged "$url" "$zip"
else
  curl -fL --retry 3 --retry-delay 2 --connect-timeout 20 -o "$zip" "$url"
fi

# 解出只要那一个条目。unzip 不一定有（Windows 上没有），bsdtar 到处都有。
list_entries() {
  if command -v unzip >/dev/null 2>&1; then unzip -Z1 "$1"
  else tar -tf "$1"; fi
}

entry="$(list_entries "$zip" | grep -E "$entry_pattern" | head -1)"
if [ -z "$entry" ]; then
  echo "压缩包里没找到匹配 $entry_pattern 的条目 —— 上游可能改了目录结构" >&2
  echo "现有条目（前 20 个）：" >&2
  list_entries "$zip" | head -20 >&2
  exit 4
fi

mkdir -p "$(dirname "$out")"
if command -v unzip >/dev/null 2>&1; then
  unzip -p "$zip" "$entry" >"$tmp/extracted"
else
  tar -xOf "$zip" "$entry" >"$tmp/extracted"
fi

if [ ! -s "$tmp/extracted" ]; then
  echo "条目 $entry 解出来是空的" >&2
  exit 4
fi

mv "$tmp/extracted" "$out"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) : ;; *) chmod +x "$out" ;; esac

got_bytes="$(size_of "$out")"
got_sha="$(sha256_of "$out")"
if [ "$got_bytes" != "$want_bytes" ] || [ "$got_sha" != "$want_sha" ]; then
  rm -f "$out"
  cat >&2 <<EOF
下载到的 ffmpeg 与预期不符，已删除：
  期望  $want_bytes 字节 / $want_sha
  实际  $got_bytes 字节 / $got_sha
多半是上游换了构建。确认新构建能用之后，把这两个值更新到
vendor/ffmpeg/README.md 和本脚本里 —— 不要直接放过。
EOF
  exit 5
fi

echo "已取回 vendor/ffmpeg/$dir_name/$entry（$got_bytes 字节）" >&2
