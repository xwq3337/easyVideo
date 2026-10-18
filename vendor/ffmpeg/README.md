# ffmpeg 静态构建

被 `include_bytes!` 编译期嵌入，运行期由 `core::ffmpeg::ensure_ffmpeg` 释放到 data 目录并校验 SHA256。

## **这两个二进制不在 git 里**

它们由 `scripts/fetch-ffmpeg.sh` 按需取回，`build.rs` 在文件缺失时自动调用它（首次构建需要联网）。

为什么移出去：`windows-x86_64/ffmpeg.exe` 是 **100.54 MB**，**超过 GitHub 单文件 100 MB 的硬上限**
—— 只要那个 blob 在历史里，`git push` 就会被 pre-receive 钩子拒掉：

```
remote: error: File vendor/ffmpeg/windows-x86_64/ffmpeg.exe is 100.54 MB;
                this exceeds GitHub's file size limit of 100.00 MB
remote: error: GH001: Large files detected.
```

macOS 那个 63 MB 不触发硬限，但两个加起来压缩后约 67 MiB，每次推送都要重传一遍。所以两个都移出去。
`.gitignore` 里对应两行；目录本身与这份 README 仍然入库（它们是构建契约的一部分）。

## 校验值（构建的判据）

| target | 版本 | 字节数 | SHA256 |
| --- | --- | --- | --- |
| `macos-aarch64/ffmpeg` | 9.0.2 | 66332880 | `2e11c6f90993cdb79fff84d3f90044d28316b310e75b3e030cfc9a54f2c9d384` |
| `windows-x86_64/ffmpeg.exe` | 9.0.2 (essentials) | 105423872 | `3256173f3f8bffd7df12227c68adf68025edb1832273a9530688a7bb1ed8edec` |

脚本里硬编码了同一组值，**对不上就失败并删掉下载物** —— 绝不悄悄换一个构建进来。若只是"下载坏了"，
重跑即可；若上游换了构建，这里和 `scripts/fetch-ffmpeg.sh` 两处要一起更新。

## 下载源

| target | URL | zip 内条目 |
| --- | --- | --- |
| macOS arm64 | https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip | `ffmpeg` |
| Windows x86_64 | https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip | `ffmpeg-<版本>-essentials_build/bin/ffmpeg.exe` |
| Linux amd64 | https://ffmpeg.martin-riedl.de/redirect/latest/linux/amd64/release/ffmpeg.zip | `ffmpeg` |
| Linux arm64 | https://ffmpeg.martin-riedl.de/redirect/latest/linux/arm64/release/ffmpeg.zip | `ffmpeg` |
| macOS x86_64 | https://evermeet.cx/ffmpeg/ | 解压取 `ffmpeg` |

**Windows 的条目路径带版本目录**（实测 `ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe`），
所以脚本按后缀 `(^|/)ffmpeg\.exe$` 找，不写死路径 —— 上游升版不会让它直接失效。

手动取：`scripts/fetch-ffmpeg.sh macos-aarch64`（或 `windows-x86_64`）。

**Windows 构建需要 bash**（Git Bash / MSYS2）：`build.rs` 是用 `bash` 跑上面那个脚本的。
`build.rs` 会主动找 Git for Windows / MSYS2 自带的 bash，**不会直接用 PATH 里的 `bash`**
——Windows 的 PATH 通常先命中 `C:\Windows\System32\bash.exe`（WSL 启动器），没装 WSL
发行版时它以 1 退出且一行脚本都不执行，看起来像下载失败。找不到时可用环境变量
`EASYVIDEO_BASH` 指定 bash 的完整路径。若没有 bash，编译会在 build script 阶段停下并说明原因。

## 许可证：这两个构建都是 **GPL**

macOS 构建的 `ffmpeg -version` 里含 `--enable-gpl --enable-version3`（Windows 的 essentials 构建同为 GPL）。
本项目是**个人自用工具、不对外分发**，GPL 的义务由分发行为触发，因此当前无影响。
但若将来要分享或分发本应用，嵌入 GPL 的 ffmpeg 会把它一并卷入 —— 那时需换成 LGPL 构建
（去掉 `--enable-gpl`）或遵守 GPL 条款。这是**已知并有意接受**的状态，不是疏漏。

## 实测注意事项（踩过的坑，别再踩）

- **不要用 `curl -I` / HEAD 探测链接是否有效。** martin-riedl.de 对 HEAD 返回误导性的 404，而 GET 正常。
- **单流下载 gyan.dev 会被限速到 ~20–30 KB/s**（约 110 MB 要 1–1.5 小时），且它不支持 `-C -`。
  脚本按 `accept-ranges` 分 8 段并发取（约 2 分钟）。
- **`curl --retry` 配 `-C -` 会把已下载的部分写坏**（实测把 65MB 退回 21.9MB）。分段下载时内层
  不要带 `--retry`。
- zip 里还有 `ffprobe`（gyan.dev 那份）—— 只解 `ffmpeg` 那一个条目，`include_bytes!` 只引用它。
- macOS 那个 zip 解压后**恰好是 66332880 字节**，与上表逐字节吻合；下载完可以直接比对。
