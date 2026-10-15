# EasyDownVideo

Rust + Tauri 跨平台桌面视频下载器（个人工具）。

## 功能
- 直链视频（mp4/webm/mkv/mov/ts）多线程分块下载，断点续传
- HLS(m3u8) 下载：清晰度选择、AES-128 解密、内嵌 ffmpeg 无损转封装 mp4
- 可选 yt-dlp 解析网页视频
- 随时新增任务；每任务显示文件名/进度/速度/剩余时间；暂停/继续/重试/删除
- 设置中更改保存位置、线程数、同时任务数、外观（跟随系统/浅色/深色）

## 构建
npm install            # Tauri CLI
npm --prefix ui install
npm run tauri build    # 产出 .app + .dmg（macOS）
                       # 产物：target/release/bundle/macos/EasyDownVideo.app
                       #       target/release/bundle/dmg/EasyDownVideo_0.1.0_aarch64.dmg

## 开发
npm run tauri dev

## 测试
cargo test                  # core（只跑这个，快）
cargo test --workspace      # 连 src-tauri 一起
npm --prefix ui test        # 前端的 reducer 与格式化
