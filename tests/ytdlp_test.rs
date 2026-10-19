use easydownvideo::core::ytdlp::{
    cookie_jar_from_netscape, find_ytdlp_in, is_dash_fragment_url, parse_formats, probe_args,
    ytdlp_error_hint,
};
// `Jar::cookies()` 来自这个 trait，不 import 就只有 `set_cookies` 可见。
use reqwest::cookie::CookieStore;

#[test]
fn parse_formats_keeps_video_entries() {
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ytdlp_dump.json")).unwrap()
    ).unwrap();
    let formats = parse_formats(&json);
    // vcodec=none 的纯音频被剔除
    assert_eq!(formats.len(), 3);
    assert_eq!(formats[0].id, "18");
    assert_eq!(formats[0].note, "360p");
    assert_eq!(formats[0].filesize, Some(1_000_000));
    // 22 号格式是 `"filesize": null` + `"filesize_approx": 2000000` —— 真实 yt-dlp 就这么输出。
    // 必须回退到 approx；写成 `assert_eq!(filesize, None)` 等于把 bug 当成期望值钉死。
    assert_eq!(
        formats[1].filesize,
        Some(2_000_000),
        "filesize 为 null 时必须回退到 filesize_approx（or_else 不会被 Some(Null) 触发）"
    );
    // hls-1 没有 format_note，note 必须回退到 resolution
    assert_eq!(formats[2].note, "1920x1080", "format_note 缺失时应回退到 resolution");
    assert!(formats.iter().all(|f| f.url.starts_with("https://")));

    // 每个格式的 http_headers 必须带出来：B 站 CDN 没有页面 Referer 就是 403。
    // 同时也必须走 sanitize_headers（Accept-Encoding 会被剔除）。
    assert!(
        formats[0]
            .headers
            .contains(&("referer".into(), "https://example.com/watch".into())),
        "格式自带的 Referer 丢了，实际: {:?}",
        formats[0].headers
    );
    assert!(
        formats[0]
            .headers
            .contains(&("user-agent".into(), "FixtureUA/1.0".into()))
    );
    assert!(
        !formats[0]
            .headers
            .iter()
            .any(|(name, _)| name == "accept-encoding"),
        "外部来的 Accept-Encoding 必须被过滤掉"
    );
    // 18/22/hls-1 都声明了 acodec，说明自带音轨，不该再配一条。
    assert!(formats[0].has_audio);
    assert!(formats.iter().all(|f| f.audio.is_none()));
}

/// B 站的 DASH 格式是 video-only + 独立音轨。只保留 URL 会得到一个无声视频；
/// 必须选码率最高的 mp4 系音频配上去，并且两条轨各自的 Referer 都要留着。
#[test]
fn parse_formats_pairs_best_audio_with_video_only_formats() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "30016",
                "ext": "mp4",
                "vcodec": "avc1.640032",
                "acodec": "none",
                "filesize": 5000000,
                "url": "https://cdn.example.com/v/30016.m4s",
                "http_headers": {
                    "Referer": "https://www.bilibili.com/video/BV1xx",
                    "User-Agent": "bili-ua"
                }
            },
            {
                "format_id": "30216",
                "ext": "m4a",
                "vcodec": "none",
                "acodec": "mp4a.40.2",
                "abr": 64,
                "url": "https://cdn.example.com/a/30216.m4a"
            },
            {
                "format_id": "30280",
                "ext": "m4a",
                "vcodec": "none",
                "acodec": "mp4a.40.2",
                "abr": 192,
                "filesize": 200000,
                "url": "https://cdn.example.com/a/30280.m4a",
                "http_headers": { "Referer": "https://www.bilibili.com/video/BV1xx" }
            },
            {
                "format_id": "vp9",
                "ext": "webm",
                "vcodec": "vp9",
                "acodec": "none",
                "url": "https://cdn.example.com/v/vp9.webm"
            }
        ]
    });

    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 2, "纯音频不能混进候选列表: {formats:?}");

    let bili = &formats[0];
    assert!(bili.has_audio, "配上音频后 has_audio 必须为 true");
    let audio = bili.audio.as_ref().expect("纯视频格式必须配上音频");
    assert_eq!(
        audio.url, "https://cdn.example.com/a/30280.m4a",
        "必须选码率最高的音频"
    );
    assert_eq!(audio.filesize, Some(200_000));
    assert_eq!(
        audio.headers,
        vec![(
            "referer".to_string(),
            "https://www.bilibili.com/video/BV1xx".to_string()
        )],
        "音频轨自己的 Referer 不能丢"
    );

    // webm/VP9 不适合 -c copy 进 mp4，宁可不配也不产出一个坏容器。
    let vp9 = &formats[1];
    assert!(!vp9.has_audio);
    assert!(vp9.audio.is_none());
}

/// `filesize_approx` 是估算值，绝不能冒充精确大小。
///
/// 旧实现把 `filesize.or(filesize_approx)` 存进 `filesize`，下载完成后拿它做
/// 逐字节校验 —— 一条完整下完的音轨永远对不上估算值，任务在重试三次后失败。
#[test]
fn audio_keeps_approximate_size_separate_from_exact() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "v",
                "ext": "mp4",
                "vcodec": "avc1",
                "acodec": "none",
                "url": "https://cdn.example.com/v.m4s"
            },
            {
                "format_id": "a",
                "ext": "m4a",
                "vcodec": "none",
                "acodec": "mp4a.40.2",
                "filesize": null,
                "filesize_approx": 1_978_315,
                "url": "https://cdn.example.com/a.m4a"
            }
        ]
    });
    let audio = parse_formats(&json)[0]
        .audio
        .clone()
        .expect("纯视频格式必须配上音频");
    assert_eq!(audio.filesize, None, "估算值不能存进精确大小字段");
    assert_eq!(audio.filesize_approx, Some(1_978_315));
}

/// HLS 音轨要按播放列表逐片下载，`download_track` 会把 m3u8 文本当成音频文件。
/// 这类格式不能参与配对；HLS 纯视频格式也应当诚实显示为“仅视频”。
#[test]
fn hls_formats_are_not_paired_with_audio() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "hls-v",
                "ext": "mp4",
                "vcodec": "avc1",
                "acodec": "none",
                "protocol": "m3u8_native",
                "url": "https://cdn.example.com/720p.m3u8"
            },
            {
                "format_id": "hls-a",
                "ext": "m4a",
                "vcodec": "none",
                "acodec": "mp4a.40.2",
                "protocol": "m3u8_native",
                "abr": 320,
                "url": "https://cdn.example.com/audio.m3u8"
            },
            {
                "format_id": "dash-a",
                "ext": "m4a",
                "vcodec": "none",
                "acodec": "mp4a.40.2",
                "protocol": "https",
                "abr": 128,
                "url": "https://cdn.example.com/audio.m4a"
            }
        ]
    });
    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 1);
    let hls = &formats[0];
    assert!(!hls.has_audio, "HLS 纯视频格式不能声称有声音");
    assert!(hls.audio.is_none(), "不能把播放列表当成音频轨配对");
}

/// 用户从开发者工具里复制出来的 `.m4s` 分片直链，必须被识别出来并给出可照做的提示，
/// 而不是丢给 yt-dlp 的 generic 提取器换回一句 `403 Forbidden`。
#[test]
fn dash_fragment_urls_are_recognized() {
    assert!(is_dash_fragment_url(
        "https://xy1x2x3x4x5.bilivideo.com/upgcxcode/42/64/42106487964/42106487964-1-100026.m4s?e=1"
    ));
    assert!(is_dash_fragment_url(
        "https://xy1x2x3x4x5.bilivideo.com/upgcxcode/42/64/42106487964/42106487964-1-30280.m4a"
    ));
    // 普通网页、普通 mp4 直链都不受影响
    assert!(!is_dash_fragment_url("https://www.bilibili.com/video/BV1xx"));
    assert!(!is_dash_fragment_url("https://example.com/a.mp4"));
    assert!(!is_dash_fragment_url(
        "https://other.example.com/audio/standalone.m4a"
    ));
}

#[test]
fn parse_formats_tolerates_missing_fields() {
    let json = serde_json::json!({ "formats": [ { "format_id": "x" } ] });
    assert!(parse_formats(&json).is_empty());
}

/// 探测命令行的构造。这一层刻意抽成纯函数：参数拼错是**静默**的 —— yt-dlp 只会
/// 多下一个无关的默认格式，或者干脆照旧被 Cloudflare 拦，界面上看不出区别。
#[test]
fn probe_args_always_asks_for_json_and_dumps_cookies() {
    let args = probe_args("/tmp/c.txt", None, false, "https://v.example/x", &[]);
    assert_eq!(
        args,
        vec![
            "-J",
            // 无论有没有开 cookie / 伪装，都要给 --cookies：yt-dlp 会把
            // **读到的 cookie jar 写回这个文件**（`--cookies` 的官方说明是
            // "read cookies from and dump cookie jar in"）。我们的下载客户端
            // 就是靠这个文件拿到浏览器 cookie 的，没有它就只能眼睁睁 403。
            "--cookies",
            "/tmp/c.txt",
            "https://v.example/x",
        ]
    );
}

#[test]
fn probe_args_adds_browser_cookies_and_impersonation_when_enabled() {
    let args = probe_args("/tmp/c.txt", Some("chrome"), true, "https://v.example/x", &[]);
    assert!(
        args.windows(2).any(|w| w == ["--cookies-from-browser", "chrome"]),
        "选了浏览器就必须带上 --cookies-from-browser，实际: {args:?}"
    );
    assert!(
        args.windows(2).any(|w| w == ["--impersonate", "chrome"]),
        "开了伪装就必须带上 --impersonate，实际: {args:?}"
    );
    assert!(
        !args.iter().any(|a| a.contains("generic:impersonate")),
        "不要退回 generic:impersonate：它在没装 curl_cffi 时**静默失效**，\
         用户看到的仍是同一个 403，只会以为开关坏了。实际: {args:?}"
    );
}

/// 这条是**注入防护**的回归测试。浏览器名来自设置页，会被原样拼进命令行：
/// 放行任意字符串就等于给了"在值里写 `-` 开头的东西，让 yt-dlp 当成新参数解析"的路。
#[test]
fn probe_args_drops_unknown_browser_names() {
    let args = probe_args("/tmp/c.txt", Some("chrome --exec rm -rf /"), true, "https://v.example/x", &[]);
    assert!(
        !args.iter().any(|a| a.contains("rm -rf")),
        "可疑的值不能被拼进命令行，实际: {args:?}"
    );
    assert!(!args.iter().any(|a| a == "--cookies-from-browser"));

    // 合法名字（大小写、两侧空格不同）规范化之后要保留
    assert!(
        probe_args("/tmp/c.txt", Some(" Chrome "), false, "u", &[])
            .windows(2)
            .any(|w| w == ["--cookies-from-browser", "chrome"]),
        "合法的浏览器名要能识别，别把功能也一起挡掉"
    );
}

/// 这条测的是**隐私**，不是功能。
///
/// `--cookies-from-browser chrome` 会把浏览器里**所有站点**的 cookie 都倒进那个 jar 文件。
/// 如果原样拼成一个 `Cookie:` 头贴到我们发的每个请求上，用户随手粘一个第三方直链，
/// 就会把邮箱、网盘、银行的 cookie 一起送给那个站点。所以必须按域分发 ——
/// 交给 reqwest 的 Jar，它是按 cookie 自己的 domain/path 决定发给谁的。
#[test]
fn cookie_jar_scopes_each_cookie_to_its_own_domain() {
    // 真机上 yt-dlp 写出来的就是这个形状（含它自己那两行 # 头）。
    let jar = cookie_jar_from_netscape(
        "# Netscape HTTP Cookie File\n\
         # This file is generated by yt-dlp.  Do not edit.\n\
         \n\
         v.example.com\tFALSE\t/\tFALSE\t0\tsession\txyz\n\
         .other.com\tTRUE\t/\tFALSE\t0\tsecret\tS\n",
    );

    let got = jar
        .cookies(&url::Url::parse("https://v.example.com/a.ts").unwrap())
        .expect("属于该域的 cookie 必须发出去");
    assert_eq!(got.to_str().unwrap(), "session=xyz");

    assert!(
        jar.cookies(&url::Url::parse("https://evil.example.org/").unwrap())
            .is_none(),
        "不属于该域的 cookie 一个都不能发出去"
    );

    // 前导点表示覆盖子域
    assert!(
        jar.cookies(&url::Url::parse("https://www.other.com/").unwrap())
            .is_some(),
        "`.other.com` 必须覆盖其子域"
    );
}

/// Netscape 格式用 `#HttpOnly_` 前缀标记 HttpOnly cookie。按"`#` 开头就是注释"一刀切，
/// 会把这类整行丢掉 —— 而丢掉的正是**会话 cookie**，最要紧的那一批。
#[test]
fn http_only_lines_are_not_swallowed_as_comments() {
    let jar = cookie_jar_from_netscape(
        "#HttpOnly_v.example.com\tFALSE\t/\tTRUE\t0\tcf_clearance\tabc\n",
    );
    let got = jar
        .cookies(&url::Url::parse("https://v.example.com/").unwrap())
        .expect("HttpOnly 的 cookie 同样要能带出去");
    assert_eq!(got.to_str().unwrap(), "cf_clearance=abc");
}

#[test]
fn empty_or_cookieless_jar_yields_no_cookies() {
    // 从没读过浏览器 cookie 时，yt-dlp 只写两行注释。此时必须一个 cookie 都不发。
    for text in ["", "# Netscape HTTP Cookie File\n\n", "not\ta\treal\tjar\n"] {
        let jar = cookie_jar_from_netscape(text);
        assert!(
            jar.cookies(&url::Url::parse("https://v.example.com/").unwrap())
                .is_none(),
            "空 jar 不该发出任何 cookie，输入: {text:?}"
        );
    }
    // 值里带 = 也要原样保留（base64 cookie 很常见）
    let jar = cookie_jar_from_netscape("v.example.com\tFALSE\t/\tFALSE\t0\ttok\ta=b=c\n");
    assert_eq!(
        jar.cookies(&url::Url::parse("https://v.example.com/").unwrap())
            .unwrap()
            .to_str()
            .unwrap(),
        "tok=a=b=c"
    );
}

#[test]
fn cloudflare_403_error_message_points_at_the_fix() {
    // 用户看到的原文就是这一串（yt-dlp 自己给的提示）。既然它已经指明了出路，
    // 我们就不该把这行原始 stderr 原样丢给用户 —— 补一句能直接照着做的。
    let hint = ytdlp_error_hint(
        "ERROR: [generic] Got HTTP Error 403 caused by Cloudflare anti-bot challenge; \
         try again with --extractor-args \"generic:impersonate\"",
    )
    .expect("Cloudflare 403 必须有对应提示");
    assert!(hint.contains("伪装"), "应指向设置里的伪装开关，实际: {hint}");
    assert!(hint.contains("cookie"), "应同时提到 cookie，实际: {hint}");

    // 与 CF 无关的报错不要乱加提示
    assert_eq!(ytdlp_error_hint("ERROR: Unsupported URL: https://x/"), None);
}

/// yt-dlp 找不到伪装目标时的**真实**报错原文（本机 yt-dlp 2026.08.19 实测，
/// 用 `--impersonate Chrome-999` 触发；没装 curl_cffi 时走的是同一条 raise）。
///
/// **注意它压根没提 curl_cffi** —— 只有 "You may be missing dependencies" 这句。
/// 所以这条测试必须用真实文本：编一句带 curl_cffi 的假消息来自我满足的话，
/// 线上匹配不上，用户仍然只看到一句没头没脑的 "is not available"。
#[test]
fn impersonate_unavailable_error_names_the_missing_dependency() {
    let real = "Traceback (most recent call last):\n\
                yt_dlp.utils.YoutubeDLError: Impersonate target \"chrome\" is not available. \
                Use --list-impersonate-targets to see available targets. \
                You may be missing dependencies required to support this target.";
    let hint = ytdlp_error_hint(real).expect("伪装不可用必须有对应提示");
    assert!(hint.contains("curl_cffi"), "要点名依赖，实际: {hint}");
}

#[test]
fn bilibili_412_error_message_points_at_the_fix() {
    // 用户实际看到的原文（B 站风控时 yt-dlp 就这么输出）。
    let hint = ytdlp_error_hint(
        "ERROR: [BiliBili] 1GJ411x7h7: Unable to download webpage: \
         HTTP Error 412: Precondition Failed",
    )
    .expect("B 站 412 必须有对应提示");
    assert!(hint.contains("风控"), "要点明是风控，实际: {hint}");
    assert!(hint.contains("cookie"), "应指向设置里的浏览器 cookie，实际: {hint}");
    assert!(hint.contains("yt-dlp"), "应提到更新 yt-dlp，实际: {hint}");

    // 非 B 站站点同样可能 412，措辞不能硬说成 B 站。
    let other = ytdlp_error_hint("ERROR: Unable to download webpage: HTTP Error 412: Precondition Failed")
        .expect("任意站点的 412 都该给提示");
    assert!(!other.contains("B 站"), "非 B 站报错不应冒充 B 站，实际: {other}");

    // 视频 ID 里恰好带 412，不是 HTTP 412 —— 不能误报成风控。
    assert_eq!(
        ytdlp_error_hint("ERROR: [BiliBili] BV1xx412yy: Video unavailable"),
        None,
        "不能拿裸数字 412 当 HTTP 状态码匹配"
    );
}

#[test]
fn find_ytdlp_scans_path_in_order() {
    // **不能**写成 `let _ = find_ytdlp();` —— 那是断言不了任何东西的空测试。
    // PATH 扫描必须测真行为：跳过没有该文件的目录、在后面的目录里找到、找不到返回 None。
    // 把 PATH 字符串作为参数传入纯函数，测试才能在不改全局环境（会与并行测试打架）的前提下驱动它。
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let target = b.path().join("bin");
    std::fs::create_dir_all(&target).unwrap();
    let exe = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
    std::fs::write(target.join(exe), b"stub").unwrap();

    // 前面的目录里没有该文件，必须继续往后找
    let path_var = std::env::join_paths([a.path(), &target]).unwrap();
    let expected = target.join(exe);
    assert_eq!(
        find_ytdlp_in(&path_var).as_deref(),
        Some(expected.as_path()),
        "应跳过不含该文件的目录，在后面的目录里找到"
    );

    // 完全没有该文件 -> None（而不是 panic）
    let none_var = std::env::join_paths([a.path()]).unwrap();
    assert_eq!(find_ytdlp_in(&none_var), None);
}
