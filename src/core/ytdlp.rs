use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::sanitize_browser;

/// yt-dlp 给某个格式规定的请求头（`Referer` / `User-Agent` / `Origin`…）。
///
/// **这些头不能丢。** B 站等站点的 CDN 直链在没有页面 Referer 时一律返回
/// `403 text/html`，yt-dlp 的 `-J` 输出里每个格式都带着自己的 `http_headers`，
/// 正是为了这种"页面能解析、直链却挑 Referer"的站点。选择格式时只留 URL、
/// 把这些头扔掉，下载就会变成 `[generic] … 403 Forbidden`。
pub type FormatHeaders = Vec<(String, String)>;

/// 纯视频格式配套的音频轨。DASH（B 站默认）把音视频拆成两个文件，
/// 只下视频会得到无声的 mp4，所以必须把音频一起下下来再用 ffmpeg 合成。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct YtdlpAudio {
    pub url: String,
    pub ext: String,
    /// yt-dlp 明确给出的精确大小。**只有它能用来判定"文件已下完"**。
    pub filesize: Option<u64>,
    /// yt-dlp 的估算值（输出里带 `~` 的那种）。只能用于进度总量，
    /// 拿它做逐字节的完整性校验会让一条完全正常的音轨永远对不上。
    #[serde(default)]
    pub filesize_approx: Option<u64>,
    #[serde(default)]
    pub headers: FormatHeaders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YtdlpFormat {
    pub id: String,
    pub ext: String,
    pub note: String,
    pub filesize: Option<u64>,
    pub url: String,
    /// 该格式自己的请求头，选中后必须随任务一起走。
    pub headers: FormatHeaders,
    /// 纯视频格式配套的音频；`None` 表示该格式自带音轨（或没找到可配的音频）。
    pub audio: Option<YtdlpAudio>,
    /// 选中后最终是否会有声音。界面用它标出"仅视频"的格式。
    pub has_audio: bool,
}

/// 能直接 `-c copy` 进 mp4 的视频/音频容器。B 站的视频轨 `ext` 是 `mp4`
/// （URL 路径以 `.m4s` 结尾），音频轨是 `m4a`。
const MP4_VIDEO_EXTS: &[&str] = &["mp4", "m4s", "m4v", "mov"];
const MP4_AUDIO_EXTS: &[&str] = &["m4a", "mp4", "aac"];

fn http_headers(format: &serde_json::Value) -> FormatHeaders {
    let Some(map) = format.get("http_headers").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let raw: Vec<(String, String)> = map
        .iter()
        .filter_map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), value.to_string()))
        })
        .collect();
    // yt-dlp 的 `http_headers` 同样经过外部数据这一层（它可能来自站点的 JSON），
    // 与扩展捕获的头共用同一套过滤：去掉 Host/Accept-Encoding/Range 之类。
    crate::core::request::sanitize_headers(&raw)
}

fn ext_is(value: &serde_json::Value, allowed: &[&str]) -> bool {
    value
        .get("ext")
        .and_then(|v| v.as_str())
        .map(|ext| allowed.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 该格式是不是 HLS/m3u8 播放列表而不是一个可直接下载的媒体文件。
///
/// HLS 分片要按播放列表逐片下载，`download_track` 只会把整个响应写成一个文件 ——
/// 把 m3u8 当音频轨下下来，得到的是一段播放列表文本。
fn is_hls_format(value: &serde_json::Value) -> bool {
    value
        .get("protocol")
        .and_then(|v| v.as_str())
        .map(|p| p.to_ascii_lowercase().contains("m3u8"))
        .unwrap_or(false)
}

fn quality_score(value: &serde_json::Value) -> u64 {
    for key in ["abr", "tbr"] {
        if let Some(n) = value.get(key).and_then(|v| v.as_f64()) {
            if n.is_finite() && n > 0.0 {
                return n as u64;
            }
        }
    }
    value
        .get("filesize")
        .and_then(|v| v.as_u64())
        .or_else(|| value.get("filesize_approx").and_then(|v| v.as_u64()))
        .unwrap_or(0)
}

/// 从 yt-dlp 的音频格式里挑一条最适合并进 mp4 的。
///
/// 只接受 `m4a`/`mp4`/`aac`：VP9/Opus 的 webm 音频虽然 ffmpeg 也能勉强塞进 mp4，
/// 但播放器兼容性差得多，而 B 站、YouTube 这类 DASH 站点都提供 m4a 轨。
fn best_audio(formats: &[&serde_json::Value]) -> Option<YtdlpAudio> {
    formats
        .iter()
        .filter(|f| f.get("vcodec").and_then(|v| v.as_str()) == Some("none"))
        // HLS 音轨同样要按播放列表下载，不能当普通文件整条拉取。
        .filter(|f| !is_hls_format(f))
        .filter(|f| ext_is(f, MP4_AUDIO_EXTS))
        .filter_map(|f| {
            let url = f.get("url").and_then(|v| v.as_str())?;
            Some((
                quality_score(f),
                YtdlpAudio {
                    url: url.to_string(),
                    ext: f
                        .get("ext")
                        .and_then(|v| v.as_str())
                        .unwrap_or("m4a")
                        .to_ascii_lowercase(),
                    // 精确值和估算值必须分开存：前者可以用于完整性校验，
                    // 后者只配用来估算进度。
                    filesize: f.get("filesize").and_then(|v| v.as_u64()),
                    filesize_approx: f.get("filesize_approx").and_then(|v| v.as_u64()),
                    headers: http_headers(f),
                },
            ))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, audio)| audio)
}

pub fn find_ytdlp() -> Option<PathBuf> {
    find_ytdlp_in(std::env::var_os("PATH")?)
}

/// 纯函数：扫描给定的 PATH 字符串。拆出来是为了可测 —— 直接驱动 `find_ytdlp()` 就得改全局
/// 环境变量，那会与并行跑的其它测试互相干扰（env 是进程级的）。
pub fn find_ytdlp_in(path_var: impl AsRef<std::ffi::OsStr>) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["yt-dlp.exe", "youtube-dl.exe", "yt-dlp", "youtube-dl"]
    } else {
        &["yt-dlp", "youtube-dl"]
    };
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn parse_formats(json: &serde_json::Value) -> Vec<YtdlpFormat> {
    let mut out = Vec::new();
    let Some(formats) = json.get("formats").and_then(|f| f.as_array()) else {
        return out;
    };
    // 所有纯视频格式共用同一条音轨：DASH 站点（B 站、YouTube）的音轨是独立文件，
    // 而且任何一档视频都能配最好的一档音频。
    let all: Vec<&serde_json::Value> = formats.iter().collect();
    let audio = best_audio(&all);
    for f in formats {
        let vcodec = f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("");
        if vcodec == "none" {
            continue; // 纯音频
        }
        let Some(url) = f.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        let note = f
            .get("format_note")
            .and_then(|v| v.as_str())
            .or_else(|| f.get("resolution").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        // `acodec` 明确为 `none` 才是纯视频。该字段缺失时按"自带音轨"处理：
        // 老的 fixture / 部分 extractor 不输出它，贸然配对反而会拼出双音轨。
        let video_only = f.get("acodec").and_then(|v| v.as_str()) == Some("none");
        // HLS 视频轨由 `run_hls` 下载，它不认识“配套音频”这回事；给这种格式
        // 配一条音频只会让界面承诺有声音、实际产物却没有。HLS 纯视频格式
        // 老老实实标成"仅视频"。
        let paired = if video_only && !is_hls_format(f) && ext_is(f, MP4_VIDEO_EXTS) {
            audio.clone()
        } else {
            None
        };
        out.push(YtdlpFormat {
            id: f.get("format_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            ext: f.get("ext").and_then(|v| v.as_str()).unwrap_or("mp4").to_string(),
            note,
            // 必须**先 `and_then` 再 `or_else`**。真实 yt-dlp 对很多格式输出的是
            // `"filesize": null`，此时 `f.get("filesize")` 返回的是 `Some(Value::Null)` 而**不是**
            // `None`，于是 `or_else` 永远不触发，`as_u64()` 在 Null 上得到 None ——
            // `filesize_approx` 被白白丢掉，而且恰好发生在最需要它的时候。
            filesize: f
                .get("filesize")
                .and_then(|v| v.as_u64())
                .or_else(|| f.get("filesize_approx").and_then(|v| v.as_u64())),
            url: url.to_string(),
            headers: http_headers(f),
            has_audio: !video_only || paired.is_some(),
            audio: paired,
        });
    }
    out
}

/// 构造一次探测的命令行。抽成纯函数是因为拼错的参数**不会报错**：yt-dlp 要么照旧
/// 被 Cloudflare 拦下，要么静默换一个格式，界面上完全看不出区别。
///
/// `cookies_file` 总会带上 —— yt-dlp 的 `--cookies` 既是输入也是输出（官方说明：
/// "read cookies from and dump cookie jar in"）。我们的下载客户端没有自己的浏览器
/// cookie 读取实现，靠的就是 yt-dlp 把 jar 落在这个文件里。
pub fn probe_args(
    cookies_file: &str,
    browser: Option<&str>,
    impersonate: bool,
    url: &str,
    headers: &[(String, String)],
) -> Vec<String> {
    let mut args = vec![
        "-J".to_string(),
        "--cookies".to_string(),
        cookies_file.to_string(),
    ];
    // 白名单过滤：这个值来自设置页，会被原样拼进命令行。放行任意字符串等于给了
    // 用户一条"以 - 开头就能追加任意 yt-dlp 参数"的路。
    if let Some(b) = browser.and_then(sanitize_browser) {
        args.push("--cookies-from-browser".to_string());
        args.push(b);
    }
    if impersonate {
        // 用 `--impersonate chrome` 而不是 403 报错里建议的
        // `--extractor-args generic:impersonate`：实测两者都能把 UA 换成 Chrome
        // （`--print-traffic` 验证过），但**依赖缺失时的表现完全不同** ——
        // `generic:impersonate` 在没装 curl_cffi 时会静默失效（`_check_impersonate_target`
        // 在没有任何 handler 支持时直接 return），用户看到的还是同一个 403，
        // 只会以为"开关坏了"；`--impersonate` 则直接报
        // `Impersonate target "chrome" is not available. ...`，界面就能照实说清原因。
        args.push("--impersonate".to_string());
        args.push("chrome".to_string());
    }
    // 扩展捕获到的 Referer/Cookie 等头原样交给 yt-dlp。很多站点的网页地址本身
    // 能解析，但真正的媒体 URL 要求 Referer —— 不传的话 yt-dlp 拿到直链后 403。
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("user-agent") {
            args.push("--user-agent".to_string());
            args.push(value.clone());
        } else {
            args.push("--add-headers".to_string());
            args.push(format!("{name}: {value}"));
        }
    }
    args.push(url.to_string());
    args
}

/// 把 yt-dlp 写出来的 Netscape cookie jar 读成一个**按域分发**的 cookie jar。
///
/// ## 为什么不能只拼一个 `Cookie:` 头
///
/// `--cookies-from-browser chrome` 倒出来的是浏览器里**所有站点**的 cookie。拼成
/// 单个头贴到我们发的每个请求上，等于把用户的邮箱/网盘/银行 cookie 一并送给每一个
/// 被下载的站点 —— 用户随手粘一个第三方直链就中招。交给 reqwest 的 `Jar`，它按每条
/// cookie 自己的 domain/path 决定发给谁，这才是"cookie 本来该怎么工作"。
///
/// ## 为什么 `#HttpOnly_` 不能当注释跳过
///
/// Netscape 格式用 `#HttpOnly_` 前缀标记 HttpOnly cookie，而 `#` 开头的行看着就像注释。
/// 一刀切会把这类整行丢掉 —— 丢掉的恰恰是会话 cookie，最要紧的那一批。
pub fn cookie_jar_from_netscape(text: &str) -> reqwest::cookie::Jar {
    let jar = reqwest::cookie::Jar::default();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // `#HttpOnly_` 是标记，不是注释；剥掉前缀后剩下的仍是完整的 7 列。
        let line = match line.strip_prefix("#HttpOnly_") {
            Some(rest) => rest,
            // 其余 `#` 开头的才是 yt-dlp 自己写的文件头注释
            None if line.starts_with('#') => continue,
            None => line,
        };
        // 7 列制表符分隔：`域 含子域 路径 仅https 过期 名 值`
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let (domain, path, secure, name) = (f[0].trim(), f[2].trim(), f[3].trim(), f[5].trim());
        if domain.is_empty() || name.is_empty() || !path.starts_with('/') {
            continue;
        }
        // 值本身可能含制表符（罕见但合法），用剩下的列拼回去
        let value = f[6..].join("\t");
        // 拼成一条 Set-Cookie，再给一个"这条 cookie 来自哪"的 URL —— Jar 靠后者
        // 推断域的匹配方式（带前导点 = 覆盖子域）。
        let scheme = if secure.eq_ignore_ascii_case("true") {
            "https"
        } else {
            "http"
        };
        let origin = format!("{scheme}://{}{}", domain.trim_start_matches('.'), path);
        let set_cookie = format!("{name}={value}; Domain={domain}; Path={path}");
        if let Ok(u) = url::Url::parse(&origin) {
            jar.add_cookie_str(&set_cookie, &u);
        }
    }
    jar
}

/// 这个地址是不是"拿不到页面上下文就必然 403"的 DASH 分片直链。
///
/// B 站 CDN 的 `.m4s`（视频轨）与 `.m4a`（音频轨）只有在带页面 Referer 时才放行，
/// 而且链接带时效签名。用户从开发者工具里复制这样一条贴进来时，探测拿到的是
/// `403 text/html`，会被归成"网页"再交给 yt-dlp 的 generic 提取器，最后报出
/// 一句对用户毫无帮助的 `[generic] … 403 Forbidden`。与其绕这一圈，不如直接说清楚。
pub fn is_dash_fragment_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let path = parsed.path().to_ascii_lowercase();
    let bilibili_cdn = host == "bilivideo.com"
        || host.ends_with(".bilivideo.com")
        || host.ends_with(".bilivideo.cn");
    path.ends_with(".m4s") || (bilibili_cdn && path.ends_with(".m4a"))
}

/// 把 yt-dlp 的失败原因翻成一句用户能照做的话。
///
/// 原始 stderr 还是要显示的（那是唯一的事实来源），但不能只有它：这两类错误的
/// 出路都在**本程序的设置页**里，用户对着 yt-dlp 的命令行提示是找不到的。
pub fn ytdlp_error_hint(stderr: &str) -> Option<String> {
    let low = stderr.to_ascii_lowercase();
    // 先判这条：它是"开了伪装却没装依赖"的直接后果。若不先判，下面按 403 分支
    // 给出的建议（"去打开伪装"）恰好是用户已经打开的那个开关，等于原地打转。
    if low.contains("curl_cffi") || low.contains("impersonate") && low.contains("not available") {
        return Some(
            "yt-dlp 的伪装功能需要 curl_cffi 依赖，当前没装。可执行 `pip install -U yt-dlp curl_cffi` \
             （或用发行版自带的完整包，如 `yt-dlp[default]`）后再试。"
                .to_string(),
        );
    }
    if low.contains("403")
        && (low.contains("cloudflare") || low.contains("anti-bot") || low.contains("challenge"))
    {
        return Some(
            "站点用了 Cloudflare 人机验证。到「设置」里选一个已登录该站的浏览器读取 cookie，\
             并打开「伪装浏览器指纹」；伪装需要 yt-dlp 装有 curl_cffi。"
                .to_string(),
        );
    }
    // B 站的风控返回 412（不是 403）。它通常不是"链接失效"，而是这次请求被判定为
    // 自动化流量 —— 换一个已登录的浏览器 cookie 往往立刻恢复。
    //
    // 锚定 `precondition failed` / `http error 412`，不能只判 `low.contains("412")`：
    // 报错里还带着视频 ID（如 `BV1xx412yy`），按裸数字匹配会把"视频不存在"之类的
    // 错误也解释成风控。
    if low.contains("precondition failed") || low.contains("http error 412") {
        return Some(if low.contains("bilibili") {
            "B 站触发了风控（HTTP 412）。请稍后重试，并到「设置」里选一个已登录 B 站的\
             浏览器读取 cookie；同时确认 yt-dlp 已更新到较新版本。"
                .to_string()
        } else {
            "站点返回 HTTP 412（Precondition Failed），通常是在对自动化请求做风控。\
             请稍后重试，并到「设置」里选一个已登录该站的浏览器读取 cookie；\
             同时确认 yt-dlp 已更新到较新版本。"
                .to_string()
        });
    }
    None
}

/// 探测网页可用的下载格式。
///
/// `cookies_file` 由调用方给定（应用数据目录下的固定文件），用来接收 yt-dlp 写回的
/// cookie jar —— 调用方在本函数返回后就该去读它，供自己的下载客户端使用。
pub async fn fetch_formats(
    exe: &Path,
    page_url: &str,
    cookies_file: &Path,
    browser: Option<&str>,
    impersonate: bool,
    headers: &[(String, String)],
) -> anyhow::Result<Vec<YtdlpFormat>> {
    // `--cookies` 是读+写：目录不存在时 yt-dlp 在退出阶段写 jar 会抛
    // `FileNotFoundError`，把真正的失败原因（如站点 403/412）盖在 traceback 后面。
    // 生产环境里 data_dir 通常已存在，但全新安装时不能指望这一点。
    if let Some(parent) = cookies_file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("无法创建 cookie 目录 {}: {e}", parent.display()))?;
    }
    let args = probe_args(
        &cookies_file.to_string_lossy(),
        browser,
        impersonate,
        page_url,
        headers,
    );
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        crate::core::process::hidden_command(exe).args(&args).output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("yt-dlp 解析超时（120s）"))??;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(match ytdlp_error_hint(&stderr) {
            Some(hint) => anyhow::anyhow!("yt-dlp 失败: {stderr}\n{hint}"),
            None => anyhow::anyhow!("yt-dlp 失败: {stderr}"),
        });
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    Ok(parse_formats(&json))
}
