# ffmpeg 静态构建来源

被 `include_bytes!` 编译期嵌入，运行期由 `core::ffmpeg::ensure_ffmpeg` 释放到 data 目录并校验 SHA256。

## 已入库的二进制（含校验值）

| target | 版本 | 字节数 | SHA256 |
| --- | --- | --- | --- |
| `macos-aarch64/ffmpeg` | 9.0.2 | 66332880 | `2e11c6f90993cdb79fff84d3f90044d28316b310e75b3e030cfc9a54f2c9d384` |
| `windows-x86_64/ffmpeg.exe` | 9.0.2 (essentials) | 105423872 | `3256173f3f8bffd7df12227c68adf68025edb1832273a9530688a7bb1ed8edec` |

记录校验值是为了让**日后重新 vendor 能判定是否拿到同一个二进制**：重新下载后先比对上表，不一致
说明上游换了构建，而不是"下载坏了"。

## 许可证：这两个构建都是 **GPL**

macOS 构建的 `ffmpeg -version` 里含 `--enable-gpl --enable-version3`（Windows 的 essentials 构建同为 GPL）。
本项目是**个人自用工具、不对外分发**，GPL 的义务由分发行为触发，因此当前无影响。
但若将来要分享或分发本应用，嵌入 GPL 的 ffmpeg 会把它一并卷入 —— 那时需换成 LGPL 构建
（去掉 `--enable-gpl`）或遵守 GPL 条款。这是**已知并有意接受**的状态，不是疏漏。

## 下载源

- macOS arm64: https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip
- Windows x86_64: https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip (取 bin/ffmpeg.exe)
- Linux amd64: https://ffmpeg.martin-riedl.de/redirect/latest/linux/amd64/release/ffmpeg.zip
- Linux arm64: https://ffmpeg.martin-riedl.de/redirect/latest/linux/arm64/release/ffmpeg.zip
- macOS x86_64: https://evermeet.cx/ffmpeg/ （下载后解压取 ffmpeg）

下载后放本目录对应 target 子目录，chmod +x（非 Windows），`ffmpeg -version` 验证，并把字节数与
SHA256 补进上表。

### 实测注意事项（踩过的坑，别再踩）

- **不要用 `curl -I` / HEAD 探测链接是否有效。** martin-riedl.de 对 HEAD 返回误导性的 404，而 GET 正常。
- **单流下载 gyan.dev 会被限速到 ~20–30 KB/s**（114MB 要 1–1.5 小时），且它不支持 `-C -`。
  可行的做法是按 `accept-ranges` 分 8 段并发取，约 2 分钟；取完用 ZIP 的 CRC 校验完整性。
- **`curl --retry` 配 `-C -` 会把已下载的部分写坏**（实测把 65MB 退回 21.9MB）。分段下载时内层不要带
  `--retry`。
- 解压后 zip 里可能还有 `ffprobe`，只保留 `ffmpeg` —— `include_bytes!` 只引用这一个路径。
