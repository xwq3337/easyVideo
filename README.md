# EasyDownVideo

Rust + Tauri 跨平台桌面视频下载器（个人工具）。

## 功能
- 直链视频（mp4/webm/mkv/mov/ts）多线程分块下载，断点续传
- HLS(m3u8) 下载：清晰度选择、AES-128 解密、内嵌 ffmpeg 无损转封装 mp4
- 可选 yt-dlp 解析网页视频
- 随时新增任务；每任务显示文件名/进度/速度/剩余时间；暂停/继续/重试/删除
- 设置中更改保存位置、线程数、同时任务数、外观（跟随系统/浅色/深色）
- 配套浏览器扩展：捕获网页中的 HLS/直链视频，连同 Referer/Cookie 等请求头发送给客户端下载

## 浏览器扩展
[`extension/`](extension/) 是一个 Manifest V3 扩展（Chrome / Edge）：

1. 打开 `chrome://extensions/`（Edge 为 `edge://extensions/`），开启“开发者模式”。
2. 点击“加载已解压的扩展程序”，选择本仓库的 `extension/` 目录。
3. 启动 easyVideo，进入 **设置 → 浏览器扩展**，复制访问令牌。
4. 点击扩展图标 → 右上角齿轮，填写端口（默认 `27853`）和令牌，测试连接后保存。
5. 在网页上播放视频，再打开扩展，点击捕获条目的“下载”。

扩展通过 `http://127.0.0.1:<端口>/api/v1/download` 与客户端通信，带
`Authorization: Bearer <令牌>`。接口只绑定回环地址、校验 Host 头并限制
请求头/请求体大小；令牌在设置页可随时重新生成。完整说明见
[extension/README.md](extension/README.md)。

## 构建
```shell
npm install            # Tauri CLI
npm --prefix ui install
npm run tauri build    # 产出 .app + .dmg（macOS，Apple 芯片）
                       # 产物：target/release/bundle/macos/Easy视频下载器.app
                       # target/release/bundle/dmg/Easy视频下载器_0.1.0_aarch64.dmg
```
## 开发
```shell
npm run tauri dev
```
## 测试
```shell
cargo test                  # core（只跑这个，快）
cargo test --workspace      # 连 src-tauri 一起
npm --prefix ui test        # 前端的 reducer 与格式化
```
