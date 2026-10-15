/**
 * 把用户粘贴的东西整成一条 http(s) URL；整不出来返回 null。
 *
 * 刻意宽松 —— 缺 scheme 的 `example.com/v.mp4`、`www.bilibili.com` 都能补上 `https://`。
 * 只有**明确不是 http(s)** 的才拒绝（`ftp://`、`file://`、`javascript:` 之类），
 * 对应 egui 版那句「请输入 http(s) 链接」。空串也返回 null（调用方按"没输入"处理，不报错）。
 */
export function normalizeUrl(raw: string): string | null {
  const v = raw.trim();
  if (v === "") return null;

  // 已经写了 scheme 的：只放行 http / https。要求 `://` 是为了不把 `host:port`
  // 里的冒号误判成 scheme（那样 `example.com:8080/x` 会被当成 scheme `example.com` 拒掉）。
  const scheme = /^([a-zA-Z][a-zA-Z0-9+.-]*):\/\//.exec(v);
  if (scheme) {
    const s = scheme[1].toLowerCase();
    return s === "http" || s === "https" ? v : null;
  }

  // 没写 scheme：看起来像主机名就补 https://。
  //   域名（可带端口/路径/查询）：example.com、example.com:8080/v
  //   纯 host:port（含 IP）：localhost:3000、1.2.3.4:8080/x
  const looksLikeHost =
    /^[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}(?::\d+)?(?:[/?#]\S*)?$/.test(v) ||
    /^[a-zA-Z0-9.-]+:\d+(?:[/?#]\S*)?$/.test(v);
  return looksLikeHost ? `https://${v}` : null;
}
