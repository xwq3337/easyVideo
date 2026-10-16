use std::path::Path;

/// 从 `ffmpeg -i <file>` 的 stderr 里抠出时长（秒）。
///
/// **为什么要写这个**：内嵌的只有 ffmpeg，没有 ffprobe（`vendor/ffmpeg/` 下就一个二进制）。
/// 不带输出文件时 ffmpeg 会以非零码退出，那是正常的 —— 我们要的就是它打在 stderr 上的信息。
///
/// 拿不到时返回 `None` 而不是 0：调用方据此把进度退化成不确定态。
/// 返回 0 会让进度条要么永远停在 0%、要么在除法里炸掉。
pub fn parse_duration(stderr: &str) -> Option<f64> {
    let line = stderr.lines().find(|l| l.contains("Duration:"))?.trim();
    let rest = line.split("Duration:").nth(1)?.trim();
    let stamp = rest.split(',').next()?.trim();
    // 格式固定是 HH:MM:SS.ss，三段冒号分隔。
    // `N/A` 会在这里被拆成 1 段而落空。
    let parts: Vec<&str> = stamp.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    // 秒那一段**必须带小数点**。ffmpeg 永远打两位小数，所以这是对已知格式的形状校验：
    // 少了它，"Duration: 00:00:0" 这种被截断的行会三段都 parse 成功、得到 Some(0.0) ——
    // 一个假时长比拿不到时长更坏，它会让进度条除出一个无意义的数。
    if !parts[2].contains('.') {
        return None;
    }
    let h: f64 = parts[0].trim().parse().ok()?;
    let m: f64 = parts[1].trim().parse().ok()?;
    let s: f64 = parts[2].trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// 真去跑一次 `ffmpeg -i`。失败、超时、解析不出来都是 `None` —— 这个函数不产生错误，
/// 因为"拿不到时长"不该阻断任务。
pub async fn media_duration(ffmpeg: &Path, input: &Path) -> Option<f64> {
    // 不带输出文件：ffmpeg 打印完流信息就以非零码退出，这正是我们要的
    let out = tokio::process::Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .output()
        .await
        .ok()?;
    parse_duration(&String::from_utf8_lossy(&out.stderr))
}
