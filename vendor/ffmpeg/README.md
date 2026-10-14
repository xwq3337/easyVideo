# ffmpeg 静态构建来源

- macOS arm64: https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip
- Windows x86_64: https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip (取 bin/ffmpeg.exe)
- Linux amd64: https://ffmpeg.martin-riedl.de/redirect/latest/linux/amd64/release/ffmpeg.zip
- Linux arm64: https://ffmpeg.martin-riedl.de/redirect/latest/linux/arm64/release/ffmpeg.zip
- macOS x86_64: https://evermeet.cx/ffmpeg/ （下载后解压取 ffmpeg）

下载后放本目录对应 target 子目录，chmod +x（非 Windows），并 `ffmpeg -version` 验证。
