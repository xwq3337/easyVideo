//! 浏览器扩展的本地 HTTP 接口。
//!
//! 为什么不走 Chrome 的 native messaging：那需要在注册表里登记一个宿主程序，
//! 而且扩展 id 一变（加载未打包扩展时每次路径不同）就要重登一次。本地回环
//! HTTP 接口没有这些安装步骤，油猴脚本也能用同一套接口。
//!
//! 安全模型（三层，缺一不可）：
//! 1. **只绑 127.0.0.1** —— 外网根本连不上；
//! 2. **Host 头必须是本机名** —— 挡住 DNS rebinding（恶意域名解析到 127.0.0.1，
//!    浏览器发出的 Host 仍是恶意域名）；
//! 3. **随机令牌** —— 挡住任意网页对 `http://127.0.0.1:<port>` 发起的 CSRF
//!    POST。令牌只显示在本机设置页里，用户手动粘进扩展。
//!
//! 接口故意做得很小：`GET /api/v1/ping` 与 `POST /api/v1/download`。没有列表、
//! 没有删除 —— 那些已经由 Tauri IPC 提供给界面了，本地接口只需要"把捕获到的
//! URL 送进下载引擎"这一件事。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use easydownvideo::core::engine::Engine;
use easydownvideo::core::request;
use easydownvideo::settings::Settings;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_PORT: u16 = 27853;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);

const MIN_TOKEN_CHARS: usize = 16;

/// 落盘位置与设置文件同级：`<config_dir>/extension.json`。
///
/// **刻意不塞进 settings.json**：扩展配置要独立于下载设置保存（端口/令牌改了要
/// 立刻重启监听），而 Settings 的整个结构体是界面的 IPC 契约。混在一起会让每次
/// 保存扩展配置都触发一遍"重建引擎设置"，多一个不必要的耦合点。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionConfig {
    pub port: u16,
    pub token: String,
}

impl Default for ExtensionConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            token: String::new(),
        }
    }
}

impl ExtensionConfig {
    pub fn path() -> PathBuf {
        Settings::config_dir().join("extension.json")
    }

    /// 读配置；文件不存在、解析失败、令牌为空时补一个随机令牌并落盘。
    ///
    /// 解析失败直接回默认值是**有意**的：这个文件只影响浏览器扩展，损坏时最坏的
    /// 结果是用户要重新在扩展选项里粘一次令牌，不该让整个应用起不来。
    pub fn load_or_create() -> Self {
        let path = Self::path();
        let mut cfg: Self = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let mut dirty = false;
        if cfg.port < 1024 {
            cfg.port = DEFAULT_PORT;
            dirty = true;
        }
        if cfg.token.trim().len() < MIN_TOKEN_CHARS {
            cfg.token = generate_token();
            dirty = true;
        } else {
            cfg.token = cfg.token.trim().to_string();
        }
        if dirty {
            if let Err(e) = cfg.save() {
                tracing::warn!("写入扩展配置失败（本次运行仍可用）：{e:#}");
            }
        }
        cfg
    }

    /// 校验一份来自设置页的配置。错误文案直接给用户看。
    pub fn validated(port: u16, token: String) -> anyhow::Result<Self> {
        if !(1024..=65535).contains(&port) {
            anyhow::bail!("端口必须在 1024–65535 之间");
        }
        let token = token.trim().to_string();
        if token.chars().count() < MIN_TOKEN_CHARS {
            anyhow::bail!("访问令牌至少 {MIN_TOKEN_CHARS} 个字符");
        }
        if token.chars().any(|c| c.is_control()) {
            anyhow::bail!("访问令牌不能包含换行或控制字符");
        }
        Ok(Self { port, token })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create config dir")?;
        }
        let text = serde_json::to_string_pretty(self).context("serialize extension config")?;
        std::fs::write(&path, text).context("write extension config")
    }
}

pub fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionConfigDto {
    pub port: u16,
    pub token: String,
}

impl From<ExtensionConfig> for ExtensionConfigDto {
    fn from(c: ExtensionConfig) -> Self {
        Self {
            port: c.port,
            token: c.token,
        }
    }
}

/// 本地接口的生命周期管理。
///
/// **令牌与监听器分开管理**：令牌放进 `RwLock`，每次请求读取当前值；改令牌只写这
/// 把锁，不重启监听。这样"重新生成令牌"是纯粹的原子更新 —— 如果连令牌变化都重启
/// 监听，同端口重绑要先等旧 task 从 accept 里退出，中间存在一个必然的
/// `AddrInUse` 竞态窗口。只有**端口变化**才真的重启监听。
#[derive(Clone)]
pub struct LocalApi {
    inner: Arc<Inner>,
}

struct Inner {
    rt: Handle,
    engine: Engine,
    token: Arc<std::sync::RwLock<String>>,
    state: Mutex<Running>,
}

struct Running {
    /// 实际监听成功的端口；`0` 表示当前没有监听器。
    port: u16,
    /// 设置页最近一次保存的端口。监听失败时它仍与 `extension.json` 一致，
    /// 这样界面不会把用户刚填的端口显示成旧值（或 0）。
    configured_port: u16,
    cancel: Option<CancellationToken>,
}

impl LocalApi {
    pub fn new(rt: Handle, engine: Engine) -> Self {
        Self {
            inner: Arc::new(Inner {
                rt,
                engine,
                token: Arc::new(std::sync::RwLock::new(String::new())),
                state: Mutex::new(Running {
                    port: 0,
                    configured_port: DEFAULT_PORT,
                    cancel: None,
                }),
            }),
        }
    }

    /// 启动监听，或在端口未变时只更新令牌。
    ///
    /// 先同步 bind 再装进 runtime：端口被占用时这条命令会当场返回错误，
    /// 用户能看到"端口已被占用"，而不是界面显示成功、日志里躺着一条 error。
    pub fn start(&self, port: u16, token: String) -> anyhow::Result<()> {
        // 先记录"期望配置"：即使接下来 bind 失败（端口被占用），设置页读到的也
        // 是用户刚保存的端口/令牌，与已经落盘的 extension.json 保持一致。
        let already_listening = {
            let mut state = self.inner.state.lock().unwrap();
            state.configured_port = port;
            // 只有真正监听在该端口上，才算"无需重启"。
            state.cancel.is_some() && state.port == port
        };
        *self.inner.token.write().unwrap() = token;
        if already_listening {
            return Ok(());
        }

        // 监听器换到新端口。旧监听可能还占着**旧**端口，但那个端口与这里请求的不同；
        // 同端口的情况已被上面的分支拦下，所以不存在与旧 listener 的重绑冲突。
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", port))
            .with_context(|| format!("无法监听 127.0.0.1:{port}（端口可能已被占用）"))?;
        std_listener
            .set_nonblocking(true)
            .context("set nonblocking")?;
        let listener = {
            let _guard = self.inner.rt.enter();
            TcpListener::from_std(std_listener).context("register listener with tokio")?
        };

        let mut state = self.inner.state.lock().unwrap();
        if let Some(old) = state.cancel.take() {
            old.cancel();
        }
        state.port = port;
        let cancel = CancellationToken::new();
        state.cancel = Some(cancel.clone());

        let engine = self.inner.engine.clone();
        let token = self.inner.token.clone();
        self.inner
            .rt
            .spawn(serve(listener, port, engine, token, cancel));
        Ok(())
    }

    /// 当前配置（设置页读它）。
    ///
    /// 端口返回的是**期望值**而不是 `Running::port`：监听失败时设置页仍应显示
    /// 用户刚保存的端口，否则“重新生成令牌”还会拿 0 去 bind。
    pub fn config(&self) -> (u16, String) {
        let state = self.inner.state.lock().unwrap();
        let token = self.inner.token.read().unwrap().clone();
        (state.configured_port, token)
    }
}

async fn serve(
    listener: TcpListener,
    port: u16,
    engine: Engine,
    token: Arc<std::sync::RwLock<String>>,
    cancel: CancellationToken,
) {
    tracing::info!("浏览器扩展接口已监听 http://127.0.0.1:{port}");
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((mut stream, _)) => {
                        let engine = engine.clone();
                        // 每个连接读一次当前令牌：改令牌后，下一个请求立即用新值。
                        let token = token.read().unwrap().clone();
                        tokio::spawn(async move {
                            match tokio::time::timeout(
                                CONNECTION_TIMEOUT,
                                handle_connection(&mut stream, &token, &engine),
                            )
                            .await
                            {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => tracing::debug!("扩展接口请求失败: {e:#}"),
                                Err(_) => tracing::debug!("扩展接口请求超时"),
                            }
                        });
                    }
                    Err(e) => tracing::warn!("扩展接口 accept 失败: {e}"),
                }
            }
        }
    }
    tracing::info!("浏览器扩展接口已停止（端口 {port}）");
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct HttpResponse {
    status: u16,
    reason: &'static str,
    body: String,
}

impl HttpResponse {
    fn json(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            reason: reason_phrase(status),
            body: value.to_string(),
        }
    }

    fn empty(status: u16) -> Self {
        Self {
            status,
            reason: reason_phrase(status),
            body: String::new(),
        }
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    }
}

async fn handle_connection(
    stream: &mut TcpStream,
    token: &str,
    engine: &Engine,
) -> anyhow::Result<()> {
    let Some(req) = read_request(stream).await? else {
        return Ok(());
    };
    // DNS rebinding 防线：浏览器发出的 Host 是攻击者域名时直接拒绝。
    let host_ok = req
        .headers
        .get("host")
        .map(|h| host_is_local(h))
        .unwrap_or(false);
    let response = if !host_ok {
        HttpResponse::json(
            403,
            serde_json::json!({ "ok": false, "error": "只接受来自本机的请求" }),
        )
    } else {
        route(&req, token, engine)
    };
    write_response(stream, response).await
}

fn route(req: &HttpRequest, token: &str, engine: &Engine) -> HttpResponse {
    let path = req.path.split('?').next().unwrap_or("");
    match (req.method.as_str(), path) {
        ("OPTIONS", _) => HttpResponse::empty(204),
        ("GET", "/api/v1/ping") => HttpResponse::json(
            200,
            serde_json::json!({
                "ok": true,
                "app": "easyVideo",
                "version": env!("CARGO_PKG_VERSION"),
            }),
        ),
        ("POST", "/api/v1/download") => {
            if !authorized(&req.headers, token) {
                return HttpResponse::json(
                    401,
                    serde_json::json!({ "ok": false, "error": "访问令牌无效" }),
                );
            }
            let body: DownloadBody = match serde_json::from_slice(&req.body) {
                Ok(body) => body,
                Err(e) => {
                    return HttpResponse::json(
                        400,
                        serde_json::json!({ "ok": false, "error": format!("请求体不是合法 JSON: {e}") }),
                    )
                }
            };
            if !request::is_downloadable_url(&body.url) {
                return HttpResponse::json(
                    400,
                    serde_json::json!({ "ok": false, "error": "只支持 http/https 链接" }),
                );
            }
            let headers: Vec<(String, String)> = body
                .headers
                .into_iter()
                .map(|h| (h.name, h.value))
                .collect();
            let id = engine.add_url_with_headers(body.url, body.filename, headers);
            HttpResponse::json(200, serde_json::json!({ "ok": true, "id": id }))
        }
        _ => HttpResponse::json(404, serde_json::json!({ "ok": false, "error": "未知接口" })),
    }
}

#[derive(Deserialize)]
struct DownloadBody {
    url: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    headers: Vec<HeaderPair>,
}

#[derive(Deserialize)]
struct HeaderPair {
    name: String,
    value: String,
}

fn authorized(headers: &HashMap<String, String>, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let provided = headers
        .get("authorization")
        .and_then(|v| {
            let lower = v.to_ascii_lowercase();
            lower
                .strip_prefix("bearer ")
                .map(|_| v["bearer ".len()..].trim())
        })
        .or_else(|| headers.get("x-easyvideo-token").map(|v| v.trim()));
    match provided {
        Some(value) => constant_time_eq(value.as_bytes(), token.as_bytes()),
        None => false,
    }
}

/// 令牌比较不走提前返回：本地接口的时序差异虽然难利用，但代价只是几行代码。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn host_is_local(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    if host == "127.0.0.1" || host == "localhost" || host == "::1" {
        return true;
    }
    // IPv6 字面量带方括号：`[::1]` 或 `[::1]:27853`
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next() == Some("::1");
    }
    let name = host.rsplit_once(':').map(|(n, _)| n).unwrap_or(&host);
    name == "127.0.0.1" || name == "localhost"
}

async fn read_request(stream: &mut TcpStream) -> anyhow::Result<Option<HttpRequest>> {
    let mut buf = Vec::with_capacity(4096);
    let header_end = loop {
        if let Some(pos) = find_bytes(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > MAX_HEADER_BYTES {
            anyhow::bail!("请求头超过 {MAX_HEADER_BYTES} 字节");
        }
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            anyhow::bail!("请求在头部中途结束");
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    if header_end > MAX_HEADER_BYTES {
        anyhow::bail!("请求头超过 {MAX_HEADER_BYTES} 字节");
    }
    let head = std::str::from_utf8(&buf[..header_end]).context("请求头不是 UTF-8")?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    if method.is_empty() || path.is_empty() {
        anyhow::bail!("无法解析请求行");
    }

    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        anyhow::bail!("请求体超过 {MAX_BODY_BYTES} 字节");
    }
    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let want = (content_length - body.len()).min(8192);
        let mut chunk = vec![0u8; want];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            anyhow::bail!("请求体在预期长度之前结束");
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    Ok(Some(HttpRequest {
        method,
        path,
        headers,
        body,
    }))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn write_response(stream: &mut TcpStream, response: HttpResponse) -> anyhow::Result<()> {
    let body = response.body.as_bytes();
    let head = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Authorization, Content-Type, X-EasyVideo-Token\r\n\
         Access-Control-Allow-Private-Network: true\r\n\
         Access-Control-Max-Age: 86400\r\n\
         \r\n",
        response.status,
        response.reason,
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_is_exact() {
        assert!(authorized(
            &HashMap::from([("authorization".into(), "Bearer abcdefghijklmnop".into())]),
            "abcdefghijklmnop"
        ));
        assert!(!authorized(
            &HashMap::from([("authorization".into(), "Bearer wrongwrongwrong".into())]),
            "abcdefghijklmnop"
        ));
        assert!(!authorized(&HashMap::new(), "abcdefghijklmnop"));
        assert!(!authorized(
            &HashMap::from([("authorization".into(), "Bearer x".into())]),
            ""
        ));
    }

    #[test]
    fn host_header_must_be_loopback() {
        assert!(host_is_local("127.0.0.1:27853"));
        assert!(host_is_local("localhost:27853"));
        assert!(host_is_local("[::1]:27853"));
        assert!(!host_is_local("evil.example:27853"));
        assert!(!host_is_local("127.0.0.1.evil.example:27853"));
    }

    #[test]
    fn config_validation_rejects_bad_ports_and_short_tokens() {
        assert!(ExtensionConfig::validated(0, "x".repeat(64)).is_err());
        assert!(ExtensionConfig::validated(80, "x".repeat(64)).is_err());
        assert!(ExtensionConfig::validated(27853, "short".into()).is_err());
        let cfg = ExtensionConfig::validated(27853, "  a".repeat(10)).unwrap();
        assert_eq!(cfg.port, 27853);
        assert!(!cfg.token.starts_with(' '));
    }

    #[test]
    fn config_dto_uses_camel_case_keys() {
        let j = serde_json::to_value(ExtensionConfigDto {
            port: 27853,
            token: "t".into(),
        })
        .unwrap();
        assert_eq!(j, serde_json::json!({ "port": 27853, "token": "t" }));
    }

    fn raw_request(
        method: &str,
        path: &str,
        host: &str,
        token: Option<&str>,
        body: &str,
    ) -> String {
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n");
        if let Some(token) = token {
            req.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ));
        req
    }

    async fn roundtrip(port: u16, raw: String) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(raw.as_bytes()).await.unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).await.unwrap();
        out
    }

    /// 监听失败（端口被占用）时，`config()` 仍要报告用户刚保存的配置。
    /// 否则设置页显示端口 0，`regenerate_extension_token` 还会拿 0 去 bind。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_start_keeps_configured_values() {
        let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = occupied.local_addr().unwrap().port();

        let data_dir = std::env::temp_dir().join(format!("easydownvideo-ext-cfg-{port}"));
        std::fs::create_dir_all(&data_dir).unwrap();
        let engine = Engine::new(Handle::current(), Settings::default(), data_dir.clone());

        let api = LocalApi::new(Handle::current(), engine);
        let token = "configured-token-0123456789".to_string();
        assert!(api.start(port, token.clone()).is_err());
        assert_eq!(api.config(), (port, token));

        drop(occupied);
        std::fs::remove_dir_all(data_dir).ok();
    }

    /// 端到端跑一遍真实的 TCP 请求：Host 校验、令牌、CORS 预检、URL 白名单。
    ///
    /// 这些是安全边界，纯函数单测覆盖不到「HTTP 层有没有真的调用它们」。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_layer_enforces_host_and_token() {
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = std_listener.local_addr().unwrap().port();
        std_listener.set_nonblocking(true).unwrap();
        let listener = TcpListener::from_std(std_listener).unwrap();

        let data_dir = std::env::temp_dir().join(format!("easydownvideo-ext-test-{port}"));
        std::fs::create_dir_all(&data_dir).unwrap();
        let engine = Engine::new(Handle::current(), Settings::default(), data_dir.clone());
        let token = Arc::new(std::sync::RwLock::new("test-token-0123456789".to_string()));
        let cancel = CancellationToken::new();
        let server = tokio::spawn(serve(listener, port, engine, token.clone(), cancel.clone()));

        let host = format!("127.0.0.1:{port}");

        // ping 不需要令牌。
        let resp = roundtrip(port, raw_request("GET", "/api/v1/ping", &host, None, "")).await;
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
        assert!(resp.contains("\"app\":\"easyVideo\""), "{resp}");

        // DNS rebinding 防线：Host 不是本机名，即便连的就是 127.0.0.1 也拒绝。
        let resp = roundtrip(
            port,
            raw_request("GET", "/api/v1/ping", "evil.example", None, ""),
        )
        .await;
        assert!(resp.starts_with("HTTP/1.1 403"), "{resp}");

        // CORS 预检不要求令牌（预检本来就不能带 Authorization）。
        let resp = roundtrip(
            port,
            raw_request("OPTIONS", "/api/v1/download", &host, None, ""),
        )
        .await;
        assert!(resp.starts_with("HTTP/1.1 204"), "{resp}");

        // 没有令牌不能下载。
        let ok_body = r#"{"url":"https://example.com/v.mp4"}"#;
        let resp = roundtrip(
            port,
            raw_request("POST", "/api/v1/download", "localhost", None, ok_body),
        )
        .await;
        assert!(resp.starts_with("HTTP/1.1 401"), "{resp}");

        // 令牌正确，但非 http/https 一律拒绝（避免本地接口变成任意协议入口）。
        let bad_body = r#"{"url":"ftp://example.com/v.mp4"}"#;
        let resp = roundtrip(
            port,
            raw_request(
                "POST",
                "/api/v1/download",
                "localhost",
                Some("test-token-0123456789"),
                bad_body,
            ),
        )
        .await;
        assert!(resp.starts_with("HTTP/1.1 400"), "{resp}");
        assert!(resp.contains("http/https"), "{resp}");

        // 换令牌后立即生效：监听没有重启，旧令牌应当马上失效。
        *token.write().unwrap() = "rotated-token-987654321".to_string();
        let resp = roundtrip(
            port,
            raw_request(
                "POST",
                "/api/v1/download",
                "localhost",
                Some("test-token-0123456789"),
                bad_body,
            ),
        )
        .await;
        assert!(resp.starts_with("HTTP/1.1 401"), "{resp}");
        let resp = roundtrip(
            port,
            raw_request(
                "POST",
                "/api/v1/download",
                "localhost",
                Some("rotated-token-987654321"),
                bad_body,
            ),
        )
        .await;
        assert!(resp.starts_with("HTTP/1.1 400"), "{resp}");

        cancel.cancel();
        let _ = server.await;
        std::fs::remove_dir_all(data_dir).ok();
    }
}
