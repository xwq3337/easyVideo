use std::path::{Path, PathBuf};

use crate::settings::sanitize_browser;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YtdlpFormat {
    pub id: String,
    pub ext: String,
    pub note: String,
    pub filesize: Option<u64>,
    pub url: String,
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
) -> anyhow::Result<Vec<YtdlpFormat>> {
    let args = probe_args(
        &cookies_file.to_string_lossy(),
        browser,
        impersonate,
        page_url,
    );
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new(exe).args(&args).output(),
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
