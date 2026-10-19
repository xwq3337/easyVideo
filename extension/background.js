"use strict";

/* ==========================================================================
 * EasyVideo 视频捕获 - Manifest V3 Service Worker
 * 负责：监听媒体请求、按标签页维护捕获列表、转发下载请求到本地客户端。
 * ========================================================================== */

// --------------------------------------------------------------------------
// 常量与配置
// --------------------------------------------------------------------------

const DEFAULT_PORT = 27853;
const DEFAULT_TOKEN = "";
const MAX_ITEMS_PER_TAB = 50;
const CAPTURES_STORAGE_KEY = "captures";
// 每个标签页最近一次主文档请求的头。只对"下载当前页面"有意义：yt-dlp 解析页面
// （B 站、需要登录的站点）时要用它带上 Cookie / Referer / UA，否则解析出来的
// CDN 直链在下载阶段仍会 403。
const PAGE_HEADERS_STORAGE_KEY = "pageHeaders";
const API_PREFIX = "/api/v1";
const FETCH_TIMEOUT_MS = 15000;
const PING_TIMEOUT_MS = 5000;

const CAPTURE_TYPES = [
  "main_frame",
  "sub_frame",
  "media",
  "xmlhttprequest",
  "other"
];

const REQUEST_FILTER = {
  urls: ["<all_urls>"],
  types: CAPTURE_TYPES
};

const DIRECT_VIDEO_EXTENSIONS = new Set([
  "mp4",
  "webm",
  "flv",
  "mkv",
  "mov",
  "m4v",
  "avi",
  "3gp"
]);

// 这些扩展名明确是分片/字幕/资源，即使响应头是视频类型也不捕获。
const EXCLUDED_SEGMENT_EXTENSIONS = new Set([
  "ts",
  "m4s",
  "m4a",
  "aac",
  "vtt",
  "jpg",
  "jpeg",
  "png",
  "gif",
  "webp",
  "js",
  "css"
]);

// 发送给 easyVideo 前需要剔除的请求头。Cookie、Referer、Origin、
// User-Agent、Authorization 以及自定义头会保留。
const DROPPED_REQUEST_HEADERS = new Set([
  "host",
  "content-length",
  "connection",
  "transfer-encoding",
  "range",
  "if-none-match",
  "if-modified-since",
  "if-match",
  "if-unmodified-since",
  "if-range",
  "accept-encoding",
  "content-encoding",
  "te",
  "trailer",
  "upgrade",
  "proxy-authorization"
]);

const NO_TOKEN_MESSAGE =
  "尚未配置访问令牌，请打开扩展选项，粘贴 easyVideo 设置 → 浏览器扩展 中显示的令牌。";

// --------------------------------------------------------------------------
// 状态：捕获列表 + 请求头缓存
// --------------------------------------------------------------------------

// capturesByTab: { "<tabId>": { items: [...], updatedAt: number } }
let capturesByTab = null;
let stateLoadPromise = null;
let stateOperationChain = Promise.resolve();

// pageHeadersByTab: { "<tabId>": [{ name, value }, ...] }
let pageHeadersByTab = null;
let pageHeadersLoadPromise = null;

// requestId -> [{ name, value }]
const requestHeadersByRequestId = new Map();

// --------------------------------------------------------------------------
// 基础工具
// --------------------------------------------------------------------------

function normalizePort(value) {
  const port = typeof value === "number" ? value : Number(value);
  // 与客户端一致：低于 1024 的端口需要特权，扩展这边提前拒绝，
  // 免得用户填了个客户端永远保存不了的值。
  if (Number.isInteger(port) && port >= 1024 && port <= 65535) {
    return port;
  }
  return null;
}

function toTabId(value) {
  const tabId = typeof value === "number" ? value : Number(value);
  return Number.isInteger(tabId) && tabId >= 0 ? tabId : null;
}

function isHttpUrl(rawUrl) {
  if (typeof rawUrl !== "string") {
    return false;
  }
  try {
    const parsed = new URL(rawUrl);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch (error) {
    return false;
  }
}

function logWarning(message, error) {
  if (error) {
    console.warn("[EasyVideo]", message, error);
  } else {
    console.warn("[EasyVideo]", message);
  }
}

function readChromeLastError() {
  try {
    return chrome.runtime.lastError;
  } catch (error) {
    return null;
  }
}

// --------------------------------------------------------------------------
// chrome.storage 封装（所有回调都读取 lastError）
// --------------------------------------------------------------------------

function storageGet(areaName, keys) {
  return new Promise((resolve) => {
    let area = null;
    try {
      area = chrome.storage && chrome.storage[areaName];
    } catch (error) {
      area = null;
    }
    if (!area || typeof area.get !== "function") {
      resolve({});
      return;
    }
    try {
      area.get(keys, (result) => {
        const error = readChromeLastError();
        if (error) {
          logWarning(`读取 ${areaName} 存储失败`, error);
          resolve({});
          return;
        }
        resolve(result || {});
      });
    } catch (error) {
      logWarning(`读取 ${areaName} 存储失败`, error);
      resolve({});
    }
  });
}

function storageSet(areaName, values) {
  return new Promise((resolve) => {
    let area = null;
    try {
      area = chrome.storage && chrome.storage[areaName];
    } catch (error) {
      area = null;
    }
    if (!area || typeof area.set !== "function") {
      resolve(false);
      return;
    }
    try {
      area.set(values, () => {
        const error = readChromeLastError();
        if (error) {
          logWarning(`写入 ${areaName} 存储失败`, error);
          resolve(false);
          return;
        }
        resolve(true);
      });
    } catch (error) {
      logWarning(`写入 ${areaName} 存储失败`, error);
      resolve(false);
    }
  });
}

// --------------------------------------------------------------------------
// URL / Content-Type 分类
// --------------------------------------------------------------------------

function classifyUrl(rawUrl) {
  if (!isHttpUrl(rawUrl)) {
    return null;
  }
  try {
    const path = new URL(rawUrl).pathname.toLowerCase();
    if (path.endsWith(".m3u8")) {
      return "hls";
    }
    // 刻意不捕获 `.mpd`：easyVideo 的下载引擎只支持 HLS 与直链，
    // 原始 DASH manifest 交给 yt-dlp 也基本解析不了。仅捕获它能给用户的
    // 只有一个注定失败的“下载”按钮，不如让“下载当前页面”走 yt-dlp。
    const lastDot = path.lastIndexOf(".");
    if (lastDot !== -1) {
      const extension = path.slice(lastDot + 1);
      if (DIRECT_VIDEO_EXTENSIONS.has(extension)) {
        return "video";
      }
    }
  } catch (error) {
    return null;
  }
  return null;
}

function classifyContentType(rawContentType) {
  if (typeof rawContentType !== "string" || !rawContentType.trim()) {
    return null;
  }
  const mime = rawContentType.split(";")[0].trim().toLowerCase();
  if (
    mime === "application/vnd.apple.mpegurl" ||
    mime === "application/x-mpegurl" ||
    mime === "application/mpegurl" ||
    mime === "audio/mpegurl" ||
    mime === "audio/x-mpegurl"
  ) {
    return "hls";
  }
  if (mime.startsWith("video/") && mime !== "video/mp2t") {
    return "video";
  }
  return null;
}

// 有些下载端点把视频以 application/octet-stream 返回，URL 上也没有扩展名，
// 唯一能判断类型的线索就是 Content-Disposition 里的文件名。
function classifyContentDisposition(rawValue) {
  if (typeof rawValue !== "string" || !rawValue.trim()) {
    return null;
  }

  let name = "";
  const extended = rawValue.match(/filename\*\s*=\s*(?:UTF-8'')?([^;]+)/i);
  if (extended) {
    name = extended[1].trim().replace(/^["']|["']$/g, "");
    try {
      name = decodeURIComponent(name);
    } catch (error) {
      // 编码不合法就按原样取扩展名。
    }
  }
  if (!name) {
    const plain = rawValue.match(/filename\s*=\s*("([^"]*)"|[^;]+)/i);
    if (plain) {
      name = (plain[2] !== undefined ? plain[2] : plain[1])
        .trim()
        .replace(/^["']|["']$/g, "");
    }
  }
  if (!name) {
    return null;
  }

  const lastDot = name.lastIndexOf(".");
  if (lastDot === -1) {
    return null;
  }
  const extension = name.slice(lastDot + 1).toLowerCase();
  if (extension === "m3u8") {
    return "hls";
  }
  return DIRECT_VIDEO_EXTENSIONS.has(extension) ? "video" : null;
}

function hasExcludedSegmentExtension(rawUrl) {
  if (!isHttpUrl(rawUrl)) {
    return false;
  }
  try {
    const path = new URL(rawUrl).pathname.toLowerCase();
    for (const extension of EXCLUDED_SEGMENT_EXTENSIONS) {
      if (path.endsWith(`.${extension}`)) {
        return true;
      }
    }
  } catch (error) {
    return false;
  }
  return false;
}

function mergeKinds(previousKind, nextKind) {
  // 清单类型优先：同一个 URL 先以直链出现、后来响应头表明是播放列表时，
  // 记录必须升级成 hls，否则会被当成普通视频文件直接下载一份播放列表文本。
  if (nextKind === "hls" || previousKind === "hls") {
    return "hls";
  }
  return nextKind || previousKind || "video";
}

function extensionFromVideoUrl(rawUrl) {
  if (!isHttpUrl(rawUrl)) {
    return "";
  }
  try {
    const path = new URL(rawUrl).pathname;
    const segment = path.slice(path.lastIndexOf("/") + 1);
    const lastDot = segment.lastIndexOf(".");
    if (lastDot === -1) {
      return "";
    }
    const extension = segment.slice(lastDot + 1).toLowerCase();
    return DIRECT_VIDEO_EXTENSIONS.has(extension) ? extension : "";
  } catch (error) {
    return "";
  }
}

function pathSegmentFromUrl(rawUrl) {
  if (!isHttpUrl(rawUrl)) {
    return "";
  }
  try {
    const path = new URL(rawUrl).pathname;
    const segment = path.slice(path.lastIndexOf("/") + 1);
    if (!segment) {
      return "";
    }
    try {
      return decodeURIComponent(segment);
    } catch (error) {
      return segment;
    }
  } catch (error) {
    return "";
  }
}

// 去掉路径段最后一段扩展名：`skate_4k_720p.m3u8` -> `skate_4k_720p`。
function mediaStemFromUrl(rawUrl) {
  return pathSegmentFromUrl(rawUrl).replace(/\.[^.]+$/, "");
}

// 标签页标题是不是"URL 本身"。
///
// 直接打开 `.m3u8` / `.mp4` 时，Chrome 把标题设成文档 URL（常常省略协议，
// 形如 `host/path/file.m3u8`）。这类标题不是名字：用它在列表里显示，同页捕获到的
// 每个清晰度看起来一模一样；用它当文件名，几个变体会得到同一个名字。
function titleLooksLikeUrl(title, pageUrl) {
  const value = String(title || "").trim();
  if (!value) {
    return false;
  }
  if (/^https?:\/\//i.test(value)) {
    return true;
  }
  let host = "";
  let bare = "";
  let pathAndQuery = "";
  try {
    const parsed = new URL(pageUrl);
    host = parsed.host;
    bare = `${parsed.host}${parsed.pathname}`;
    pathAndQuery = `${parsed.pathname}${parsed.search}`;
  } catch (error) {
    return false;
  }
  // 没有 document.title 的媒体文档，Chrome 用"去掉协议、解码百分号转义"的显示 URL
  // 当标签页标题；IDN 主机显示成 Unicode，而 URL.host 是 punycode。两种形态都要认。
  let displayBare = bare;
  let displayPath = pathAndQuery;
  try {
    displayBare = decodeURI(bare);
    displayPath = decodeURI(pathAndQuery);
  } catch (error) {
    // 百分号转义不合法时只比较原始形态
  }
  const stripSlash = (text) => text.replace(/\/+$/, "");
  // 主机对不上（punycode vs Unicode）时路径仍然一致，单独比路径。
  const slash = value.indexOf("/");
  const valuePath = slash === -1 ? "" : value.slice(slash);
  return (
    stripSlash(value) === stripSlash(bare) ||
    stripSlash(value) === stripSlash(displayBare) ||
    value.startsWith(`${bare}?`) ||
    value.startsWith(`${displayBare}?`) ||
    // 只有 punycode 主机才会显示成 Unicode 而整串对不上；此时路径仍然一致。
    (host.includes("xn--") &&
      valuePath !== "" &&
      (valuePath === displayPath || valuePath === pathAndQuery)) ||
    // 标题被改写成同域 URL（页面地址已变化）时也能认出来；带空格的正常标题
    // 即使以域名开头也不当成 URL。
    (host !== "" && value.startsWith(`${host}/`) && !/\s/.test(value))
  );
}

// --------------------------------------------------------------------------
// 文件名建议
// --------------------------------------------------------------------------

function sanitizeFilenameBase(rawName) {
  let name = String(rawName || "")
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/[<>:"/\\|?*]/g, " ")
    .replace(/\s+/g, " ")
    .trim();

  // 若标题本身以媒体扩展名结尾，先去掉，避免生成 xxx.mp4.mp4。
  name = name.replace(
    /\.(m3u8|mp4|webm|flv|mkv|mov|m4v|avi|3gp)\s*$/i,
    ""
  );

  name = name.replace(/^[.\s]+|[.\s]+$/g, "");

  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i.test(name)) {
    name = `_${name}`;
  }
  return name;
}

function chooseFileExtension(kind, rawUrl) {
  if (kind === "hls") {
    return ".mp4";
  }
  const videoExtension = extensionFromVideoUrl(rawUrl);
  return videoExtension ? `.${videoExtension}` : ".mp4";
}

function suggestFilename(kind, url, pageTitle, pageUrl) {
  const extension = chooseFileExtension(kind, url);
  // 页面标题通常是更好的名字；但当标题就是 URL 本身（直接打开媒体文件的页面），
  // 必须退回媒体 URL 自己的文件名 —— 否则同一页捕获的每个清晰度都会重名。
  const title = String(pageTitle || "");
  let base = titleLooksLikeUrl(title, pageUrl || url)
    ? ""
    : sanitizeFilenameBase(title);
  if (!base) {
    base = sanitizeFilenameBase(mediaStemFromUrl(url));
  }
  if (!base) {
    base = "easyVideo-video";
  }

  const maxBaseLength = Math.max(1, 100 - extension.length);
  base = Array.from(base).slice(0, maxBaseLength).join("");
  base = base.replace(/[.\s]+$/g, "");
  if (!base) {
    base = "easyVideo-video";
  }
  return `${base}${extension}`;
}

// --------------------------------------------------------------------------
// 请求头清洗
// --------------------------------------------------------------------------

function sanitizeHeaders(headers) {
  if (!Array.isArray(headers)) {
    return [];
  }

  const sanitized = [];
  for (const header of headers) {
    if (!header || typeof header.name !== "string") {
      continue;
    }
    if (typeof header.value !== "string") {
      continue;
    }

    const name = header.name.trim();
    const value = header.value;
    if (!name || name.includes("\r") || name.includes("\n")) {
      continue;
    }
    if (value.includes("\r") || value.includes("\n")) {
      continue;
    }

    const lowerName = name.toLowerCase();
    if (lowerName.startsWith(":")) {
      continue;
    }
    if (
      DROPPED_REQUEST_HEADERS.has(lowerName) ||
      lowerName.startsWith("proxy-")
    ) {
      continue;
    }

    sanitized.push({ name, value });
  }
  return sanitized;
}

// --------------------------------------------------------------------------
// 捕获状态持久化与串行更新
// --------------------------------------------------------------------------

function normalizeStoredItem(rawItem) {
  if (!rawItem || typeof rawItem.url !== "string") {
    return null;
  }
  if (!isHttpUrl(rawItem.url)) {
    return null;
  }
  const kind = ["hls", "video"].includes(rawItem.kind)
    ? rawItem.kind
    : classifyUrl(rawItem.url);
  if (!kind) {
    return null;
  }

  const timestamp =
    typeof rawItem.ts === "number" && Number.isFinite(rawItem.ts)
      ? rawItem.ts
      : Date.now();
  const size =
    typeof rawItem.size === "number" && Number.isFinite(rawItem.size) &&
    rawItem.size >= 0
      ? Math.floor(rawItem.size)
      : null;

  return {
    url: rawItem.url,
    kind,
    contentType:
      typeof rawItem.contentType === "string" ? rawItem.contentType : "",
    size,
    pageUrl: typeof rawItem.pageUrl === "string" ? rawItem.pageUrl : "",
    pageTitle: typeof rawItem.pageTitle === "string" ? rawItem.pageTitle : "",
    headers: sanitizeHeaders(rawItem.headers),
    ts: timestamp
  };
}

function normalizeStoredCaptures(rawCaptures) {
  const normalized = {};
  if (!rawCaptures || typeof rawCaptures !== "object") {
    return normalized;
  }

  for (const [tabId, bucket] of Object.entries(rawCaptures)) {
    if (!/^\d+$/.test(tabId) || !bucket || !Array.isArray(bucket.items)) {
      continue;
    }
    const items = [];
    for (const rawItem of bucket.items) {
      const item = normalizeStoredItem(rawItem);
      if (item && !items.some((existing) => existing.url === item.url)) {
        items.push(item);
      }
      if (items.length >= MAX_ITEMS_PER_TAB) {
        break;
      }
    }
    if (!items.length) {
      continue;
    }
    items.sort((left, right) => right.ts - left.ts);
    normalized[tabId] = {
      items,
      updatedAt:
        typeof bucket.updatedAt === "number" && Number.isFinite(bucket.updatedAt)
          ? bucket.updatedAt
          : Date.now()
    };
  }
  return normalized;
}

function ensureStateLoaded() {
  if (capturesByTab !== null) {
    return Promise.resolve(capturesByTab);
  }
  if (!stateLoadPromise) {
    stateLoadPromise = storageGet("session", CAPTURES_STORAGE_KEY).then(
      (result) => {
        capturesByTab = normalizeStoredCaptures(result[CAPTURES_STORAGE_KEY]);
        return capturesByTab;
      }
    );
  }
  return stateLoadPromise;
}

function saveCaptures() {
  if (capturesByTab === null) {
    return Promise.resolve(false);
  }
  return storageSet("session", {
    [CAPTURES_STORAGE_KEY]: capturesByTab
  });
}

function mutateCaptures(mutator) {
  const operation = stateOperationChain.then(async () => {
    await ensureStateLoaded();
    const result = await mutator(capturesByTab);
    await saveCaptures();
    return result;
  });
  stateOperationChain = operation.catch((error) => {
    logWarning("更新捕获状态失败", error);
  });
  return operation;
}

async function readItems(tabId) {
  await stateOperationChain;
  await ensureStateLoaded();
  const bucket = capturesByTab[String(tabId)];
  if (!bucket || !Array.isArray(bucket.items)) {
    return [];
  }
  return bucket.items.map((item) => ({
    ...item,
    headers: item.headers.map((header) => ({ ...header }))
  }));
}

function addCapture(tabId, url, kind, patch) {
  const extra = patch || {};
  return mutateCaptures((captures) => {
    const key = String(tabId);
    let bucket = captures[key];
    if (!bucket || !Array.isArray(bucket.items)) {
      bucket = { items: [], updatedAt: 0 };
      captures[key] = bucket;
    }

    const now = Date.now();
    const existingIndex = bucket.items.findIndex((entry) => entry.url === url);
    let item;
    let created = false;

    if (existingIndex !== -1) {
      item = bucket.items.splice(existingIndex, 1)[0];
      item.kind = mergeKinds(item.kind, kind);
      if (extra.contentType) {
        item.contentType = extra.contentType;
      }
      if (
        typeof extra.size === "number" &&
        Number.isFinite(extra.size) &&
        extra.size >= 0
      ) {
        item.size = Math.floor(extra.size);
      }
      if (extra.headers && extra.headers.length) {
        item.headers = extra.headers;
      }
      item.ts = now;
      bucket.items.unshift(item);
    } else {
      created = true;
      item = {
        url,
        kind: mergeKinds(null, kind || classifyUrl(url)),
        contentType:
          typeof extra.contentType === "string" ? extra.contentType : "",
        size:
          typeof extra.size === "number" &&
          Number.isFinite(extra.size) &&
          extra.size >= 0
            ? Math.floor(extra.size)
            : null,
        pageUrl: typeof extra.pageUrl === "string" ? extra.pageUrl : "",
        pageTitle:
          typeof extra.pageTitle === "string" ? extra.pageTitle : "",
        headers: sanitizeHeaders(extra.headers),
        ts: now
      };
      bucket.items.unshift(item);
    }

    if (bucket.items.length > MAX_ITEMS_PER_TAB) {
      bucket.items.length = MAX_ITEMS_PER_TAB;
    }
    bucket.updatedAt = now;

    return {
      created,
      count: bucket.items.length,
      pageUrl: item.pageUrl,
      pageTitle: item.pageTitle
    };
  });
}

function updateCapturePageInfo(tabId, url, pageUrl, pageTitle) {
  if (!pageUrl && !pageTitle) {
    return Promise.resolve();
  }
  return mutateCaptures((captures) => {
    const bucket = captures[String(tabId)];
    if (!bucket || !Array.isArray(bucket.items)) {
      return;
    }
    const item = bucket.items.find((entry) => entry.url === url);
    if (!item) {
      return;
    }
    if (pageUrl) {
      item.pageUrl = pageUrl;
    }
    if (pageTitle) {
      item.pageTitle = pageTitle;
    }
    bucket.updatedAt = Date.now();
  });
}

function updateCaptureHeaders(tabId, url, headers) {
  if (!Array.isArray(headers) || !headers.length) {
    return Promise.resolve();
  }
  return mutateCaptures((captures) => {
    const bucket = captures[String(tabId)];
    if (!bucket || !Array.isArray(bucket.items)) {
      return;
    }
    const item = bucket.items.find((entry) => entry.url === url);
    if (!item) {
      return;
    }
    item.headers = headers;
    bucket.updatedAt = Date.now();
  });
}

function clearTabCaptures(tabId, updateBadge) {
  return mutateCaptures((captures) => {
    const key = String(tabId);
    if (captures[key]) {
      delete captures[key];
    }
  }).then(() => {
    if (updateBadge) {
      setBadgeCount(tabId, 0);
    }
  });
}

// --------------------------------------------------------------------------
// 主文档请求头缓存（供"下载当前页面"使用）
// --------------------------------------------------------------------------

function normalizeStoredPageHeaders(rawPageHeaders) {
  const normalized = {};
  if (!rawPageHeaders || typeof rawPageHeaders !== "object") {
    return normalized;
  }
  for (const [tabId, headers] of Object.entries(rawPageHeaders)) {
    if (!/^\d+$/.test(tabId)) {
      continue;
    }
    const sanitized = sanitizeHeaders(headers);
    if (sanitized.length) {
      normalized[tabId] = sanitized;
    }
  }
  return normalized;
}

function ensurePageHeadersLoaded() {
  if (pageHeadersByTab !== null) {
    return Promise.resolve(pageHeadersByTab);
  }
  if (!pageHeadersLoadPromise) {
    pageHeadersLoadPromise = storageGet("session", PAGE_HEADERS_STORAGE_KEY).then(
      (result) => {
        pageHeadersByTab = normalizeStoredPageHeaders(
          result[PAGE_HEADERS_STORAGE_KEY]
        );
        return pageHeadersByTab;
      }
    );
  }
  return pageHeadersLoadPromise;
}

function savePageHeaders() {
  if (pageHeadersByTab === null) {
    return Promise.resolve(false);
  }
  return storageSet("session", {
    [PAGE_HEADERS_STORAGE_KEY]: pageHeadersByTab
  });
}

// 与捕获列表共用同一条操作链：两者都写 session storage，顺序执行最简单。
function mutatePageHeaders(mutator) {
  const operation = stateOperationChain.then(async () => {
    await ensurePageHeadersLoaded();
    const result = await mutator(pageHeadersByTab);
    await savePageHeaders();
    return result;
  });
  stateOperationChain = operation.catch((error) => {
    logWarning("更新页面请求头缓存失败", error);
  });
  return operation;
}

function setPageHeaders(tabId, headers) {
  const sanitized = sanitizeHeaders(headers);
  return mutatePageHeaders((pageHeaders) => {
    if (sanitized.length) {
      pageHeaders[String(tabId)] = sanitized;
    } else {
      delete pageHeaders[String(tabId)];
    }
  });
}

function clearPageHeaders(tabId) {
  return mutatePageHeaders((pageHeaders) => {
    delete pageHeaders[String(tabId)];
  });
}

async function getPageHeaders(tabId) {
  await stateOperationChain;
  await ensurePageHeadersLoaded();
  const headers = pageHeadersByTab[String(tabId)];
  return Array.isArray(headers)
    ? headers.map((header) => ({ ...header }))
    : [];
}

// --------------------------------------------------------------------------
// 标签页信息与徽标
// --------------------------------------------------------------------------

function getTabInfo(tabId) {
  return new Promise((resolve) => {
    try {
      chrome.tabs.get(tabId, (tab) => {
        const error = readChromeLastError();
        if (error || !tab) {
          resolve(null);
          return;
        }
        resolve({
          url: typeof tab.url === "string" ? tab.url : "",
          title: typeof tab.title === "string" ? tab.title : ""
        });
      });
    } catch (error) {
      resolve(null);
    }
  });
}

function setBadgeCount(tabId, count) {
  if (!Number.isInteger(tabId) || tabId < 0) {
    return;
  }
  const text = count > 0 ? String(count) : "";
  try {
    chrome.action.setBadgeBackgroundColor(
      { tabId, color: "#2F6FED" },
      () => {
        readChromeLastError();
      }
    );
    chrome.action.setBadgeText({ tabId, text }, () => {
      readChromeLastError();
    });
  } catch (error) {
    // 标签页可能已经关闭，忽略即可。
  }
}

function queueCapture(tabId, url, kind, patch) {
  addCapture(tabId, url, kind, patch)
    .then((result) => {
      if (!result) {
        return;
      }
      setBadgeCount(tabId, result.count);
      if (!result.pageUrl && !result.pageTitle) {
        getTabInfo(tabId)
          .then((tabInfo) => {
            if (!tabInfo || (!tabInfo.url && !tabInfo.title)) {
              return undefined;
            }
            return updateCapturePageInfo(tabId, url, tabInfo.url, tabInfo.title);
          })
          .catch((error) => {
            logWarning("读取标签页信息失败", error);
          });
      }
    })
    .catch((error) => {
      logWarning("记录视频请求失败", error);
    });
}

// --------------------------------------------------------------------------
// webRequest 监听
// --------------------------------------------------------------------------

function handleBeforeRequest(details) {
  if (!details || !Number.isInteger(details.tabId) || details.tabId < 0) {
    return;
  }
  // 新的主文档开始加载：旧页面的头立即作废。**必须在这里清**，不能放到
  // tabs.onUpdated 里 —— 那个事件与 webRequest 的先后顺序没有保证，
  // 有可能把随后 onBeforeSendHeaders 刚存下的新头又清掉。
  if (details.type === "main_frame") {
    clearPageHeaders(details.tabId).catch((error) => {
      logWarning("清理页面请求头失败", error);
    });
  }
  const kind = classifyUrl(details.url);
  if (!kind) {
    return;
  }
  queueCapture(details.tabId, details.url, kind, {});
}

function handleBeforeSendHeaders(details) {
  if (!details || typeof details.requestId !== "string") {
    return;
  }

  const headers = sanitizeHeaders(details.requestHeaders);
  requestHeadersByRequestId.set(details.requestId, headers);

  if (!Number.isInteger(details.tabId) || details.tabId < 0) {
    return;
  }
  // 主文档的头单独留一份给"下载当前页面"：yt-dlp 需要它来带登录态解析。
  // 这里不要求 URL 像视频 —— 页面 URL 本来就不是视频地址。
  if (details.type === "main_frame") {
    setPageHeaders(details.tabId, headers).catch((error) => {
      logWarning("记录页面请求头失败", error);
    });
  }
  if (!classifyUrl(details.url)) {
    return;
  }
  updateCaptureHeaders(details.tabId, details.url, headers).catch((error) => {
    logWarning("更新请求头失败", error);
  });
}

function getResponseHeader(responseHeaders, headerName) {
  if (!Array.isArray(responseHeaders)) {
    return "";
  }
  const wanted = headerName.toLowerCase();
  for (const header of responseHeaders) {
    if (
      header &&
      typeof header.name === "string" &&
      header.name.toLowerCase() === wanted &&
      typeof header.value === "string"
    ) {
      return header.value;
    }
  }
  return "";
}

function parseContentLength(rawValue) {
  if (typeof rawValue !== "string" || !rawValue.trim()) {
    return null;
  }
  const value = Number(rawValue.trim());
  if (!Number.isFinite(value) || value < 0) {
    return null;
  }
  return Math.floor(value);
}

function handleHeadersReceived(details) {
  if (!details || !Number.isInteger(details.tabId) || details.tabId < 0) {
    return;
  }

  const contentType = getResponseHeader(
    details.responseHeaders,
    "content-type"
  );
  const contentLength = parseContentLength(
    getResponseHeader(details.responseHeaders, "content-length")
  );
  const kindFromContentType = classifyContentType(contentType);
  const kindFromDisposition = classifyContentDisposition(
    getResponseHeader(details.responseHeaders, "content-disposition")
  );
  const kindFromUrl = classifyUrl(details.url);

  // 拒绝 .ts、.m4s、.m4a、.aac、.vtt 等分片/资源，避免仅凭
  // 响应头把视频分片误当成可下载文件。
  if (!kindFromUrl && hasExcludedSegmentExtension(details.url)) {
    return;
  }
  if (!kindFromContentType && !kindFromDisposition && !kindFromUrl) {
    return;
  }

  const headers = requestHeadersByRequestId.get(details.requestId) || [];
  queueCapture(
    details.tabId,
    details.url,
    kindFromContentType || kindFromDisposition || kindFromUrl,
    {
      contentType,
      size: contentLength,
      headers
    }
  );
}

function handleRequestFinished(details) {
  if (!details || typeof details.requestId !== "string") {
    return;
  }
  requestHeadersByRequestId.delete(details.requestId);
}

chrome.webRequest.onBeforeRequest.addListener(
  handleBeforeRequest,
  REQUEST_FILTER
);

chrome.webRequest.onBeforeSendHeaders.addListener(
  handleBeforeSendHeaders,
  REQUEST_FILTER,
  ["requestHeaders", "extraHeaders"]
);

chrome.webRequest.onHeadersReceived.addListener(
  handleHeadersReceived,
  REQUEST_FILTER,
  ["responseHeaders"]
);

chrome.webRequest.onCompleted.addListener(
  handleRequestFinished,
  REQUEST_FILTER
);

chrome.webRequest.onErrorOccurred.addListener(
  handleRequestFinished,
  REQUEST_FILTER
);

// --------------------------------------------------------------------------
// 标签页生命周期
// --------------------------------------------------------------------------

chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (
    changeInfo &&
    changeInfo.status === "loading" &&
    typeof changeInfo.url === "string"
  ) {
    clearTabCaptures(tabId, true).catch((error) => {
      logWarning("清理标签页捕获记录失败", error);
    });
  }
});

chrome.tabs.onRemoved.addListener((tabId) => {
  clearTabCaptures(tabId, false).catch((error) => {
    logWarning("清理已关闭标签页的捕获记录失败", error);
  });
  clearPageHeaders(tabId).catch((error) => {
    logWarning("清理已关闭标签页的请求头失败", error);
  });
});

// --------------------------------------------------------------------------
// easyVideo 客户端 API
// --------------------------------------------------------------------------

async function getConfig() {
  const stored = await storageGet("local", ["port", "token"]);
  const port = normalizePort(stored.port) || DEFAULT_PORT;
  const token =
    typeof stored.token === "string" ? stored.token.trim() : DEFAULT_TOKEN;
  return { port, token };
}

function readJsonResponse(response) {
  return response
    .json()
    .then((data) => data)
    .catch(() => null);
}

function fetchWithTimeout(url, options, timeoutMs) {
  const controller = new AbortController();
  const timer = setTimeout(() => {
    controller.abort();
  }, timeoutMs);

  return fetch(url, {
    ...options,
    signal: controller.signal
  }).finally(() => {
    clearTimeout(timer);
  });
}

function appBaseUrl(port) {
  return `http://127.0.0.1:${port}`;
}

function connectionErrorMessage(port, timedOut) {
  if (timedOut) {
    return `连接 easyVideo 超时（请确认客户端已启动，端口 ${port}）`;
  }
  return `无法连接 easyVideo（请确认客户端已启动，端口 ${port}）`;
}

function serverErrorMessage(data, fallback) {
  if (data && typeof data.error === "string" && data.error.trim()) {
    return `easyVideo 返回错误：${data.error.trim()}`;
  }
  return fallback;
}

async function testConnection(portOverride) {
  const config = await getConfig();
  const port = normalizePort(portOverride) || config.port;

  try {
    const response = await fetchWithTimeout(
      `${appBaseUrl(port)}${API_PREFIX}/ping`,
      { method: "GET" },
      PING_TIMEOUT_MS
    );
    const data = await readJsonResponse(response);

    if (!response.ok) {
      return {
        ok: false,
        error:
          response.status === 401 || response.status === 403
            ? "令牌无效（请在扩展选项里重新粘贴）"
            : serverErrorMessage(
                data,
                `无法连接 easyVideo（客户端返回 HTTP ${response.status}，端口 ${port}）`
              )
      };
    }
    if (!data || data.ok !== true) {
      return {
        ok: false,
        error: serverErrorMessage(data, "easyVideo 返回了无法识别的响应。")
      };
    }
    return {
      ok: true,
      app: typeof data.app === "string" ? data.app : "easyVideo",
      version: typeof data.version === "string" ? data.version : ""
    };
  } catch (error) {
    return {
      ok: false,
      error: connectionErrorMessage(port, error && error.name === "AbortError")
    };
  }
}

async function sendDownloadRequest(payload, config) {
  if (!config.token) {
    return { ok: false, error: NO_TOKEN_MESSAGE };
  }

  try {
    const response = await fetchWithTimeout(
      `${appBaseUrl(config.port)}${API_PREFIX}/download`,
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${config.token}`
        },
        body: JSON.stringify(payload)
      },
      FETCH_TIMEOUT_MS
    );
    const data = await readJsonResponse(response);

    if (response.status === 401 || response.status === 403) {
      return {
        ok: false,
        error: "令牌无效（请在扩展选项里重新粘贴）"
      };
    }
    if (!response.ok) {
      return {
        ok: false,
        error: serverErrorMessage(
          data,
          `easyVideo 下载请求失败（HTTP ${response.status}）`
        )
      };
    }
    if (!data || data.ok !== true) {
      return {
        ok: false,
        error: serverErrorMessage(data, "easyVideo 未确认下载请求。")
      };
    }
    return { ok: true, id: data.id };
  } catch (error) {
    return {
      ok: false,
      error: connectionErrorMessage(
        config.port,
        error && error.name === "AbortError"
      )
    };
  }
}

async function downloadItem(item) {
  if (!item || !isHttpUrl(item.url)) {
    return { ok: false, error: "无效的视频地址。" };
  }

  const config = await getConfig();
  if (!config.token) {
    return { ok: false, error: NO_TOKEN_MESSAGE };
  }

  const kind = item.kind === "hls" ? "hls" : "video";
  const filename = suggestFilename(
    kind,
    item.url,
    item.pageTitle,
    item.pageUrl || item.url
  );
  const payload = {
    url: item.url,
    filename,
    headers: sanitizeHeaders(item.headers)
  };
  return sendDownloadRequest(payload, config);
}

async function downloadCurrentPage(tabId) {
  const tabInfo = await getTabInfo(tabId);
  if (!tabInfo || !isHttpUrl(tabInfo.url)) {
    return {
      ok: false,
      error: "当前标签页不是可下载的网页（仅支持 http/https 页面）。"
    };
  }

  const config = await getConfig();
  if (!config.token) {
    return { ok: false, error: NO_TOKEN_MESSAGE };
  }

  // 把主文档的 Cookie/Referer/UA 一并交给 yt-dlp。缺少登录态时，B 站等站点
  // 可能解析不出高清格式；缺少页面 Referer 时，解析出的 CDN 直链下载会 403。
  const headers = await getPageHeaders(tabId);
  return sendDownloadRequest(
    {
      url: tabInfo.url,
      filename: suggestFilename(
        "video",
        tabInfo.url,
        tabInfo.title,
        tabInfo.url
      ),
      headers
    },
    config
  );
}

// --------------------------------------------------------------------------
// 消息接口
// --------------------------------------------------------------------------

async function handleMessage(message) {
  if (!message || typeof message !== "object") {
    return { ok: false, error: "无效的请求。" };
  }

  switch (message.type) {
    case "getItems": {
      const tabId = toTabId(message.tabId);
      if (tabId === null) {
        return { items: [] };
      }
      try {
        return { items: await readItems(tabId) };
      } catch (error) {
        logWarning("读取捕获列表失败", error);
        return { items: [] };
      }
    }

    case "clear": {
      const tabId = toTabId(message.tabId);
      if (tabId === null) {
        return { ok: false, error: "无效的标签页。" };
      }
      await clearTabCaptures(tabId, true);
      return { ok: true };
    }

    case "download":
      return downloadItem(message.item);

    case "downloadPage": {
      const tabId = toTabId(message.tabId);
      if (tabId === null) {
        return { ok: false, error: "无效的标签页。" };
      }
      return downloadCurrentPage(tabId);
    }

    case "testConnection":
      return testConnection(message.port);

    case "getConfig":
      return getConfig();

    default:
      return { ok: false, error: "未知操作。" };
  }
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  let result;
  try {
    result = handleMessage(message);
  } catch (error) {
    logWarning("处理消息失败", error);
    sendResponse({ ok: false, error: "扩展内部错误，请重试。" });
    return false;
  }

  Promise.resolve(result)
    .then((response) => {
      try {
        sendResponse(response || { ok: false, error: "扩展未返回结果。" });
      } catch (error) {
        // 发送端（弹窗）可能已经关闭，忽略。
      }
    })
    .catch((error) => {
      logWarning("处理消息失败", error);
      try {
        sendResponse({ ok: false, error: "扩展内部错误，请重试。" });
      } catch (sendError) {
        // 忽略已经关闭的消息通道。
      }
    });

  return true;
});
