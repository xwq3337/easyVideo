"use strict";

/* EasyVideo 视频捕获 - 弹窗逻辑 */

const DEFAULT_PORT = 27853;
const VIDEO_EXTENSIONS = new Set([
  "mp4",
  "webm",
  "flv",
  "mkv",
  "mov",
  "m4v",
  "avi",
  "3gp"
]);

const state = {
  tabId: null,
  tab: null,
  items: [],
  config: { port: DEFAULT_PORT, token: "" }
};

let elements = null;
let actionStatusTimer = null;

// --------------------------------------------------------------------------
// 通用工具
// --------------------------------------------------------------------------

function getElement(id) {
  return document.getElementById(id);
}

function readChromeLastError() {
  try {
    return chrome.runtime.lastError;
  } catch (error) {
    return null;
  }
}

function sendMessage(message) {
  return new Promise((resolve) => {
    try {
      chrome.runtime.sendMessage(message, (response) => {
        const error = readChromeLastError();
        if (error) {
          console.warn("[EasyVideo] 后台消息失败", error);
          resolve({
            ok: false,
            error: "扩展后台不可用，请重新加载扩展后重试。"
          });
          return;
        }
        resolve(response || { ok: false, error: "扩展没有返回结果。" });
      });
    } catch (error) {
      console.warn("[EasyVideo] 后台消息失败", error);
      resolve({
        ok: false,
        error: "扩展后台不可用，请重新加载扩展后重试。"
      });
    }
  });
}

function getActiveTab() {
  return new Promise((resolve) => {
    try {
      chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
        const error = readChromeLastError();
        if (error) {
          console.warn("[EasyVideo] 读取当前标签页失败", error);
          resolve(null);
          return;
        }
        resolve(Array.isArray(tabs) && tabs.length ? tabs[0] : null);
      });
    } catch (error) {
      console.warn("[EasyVideo] 读取当前标签页失败", error);
      resolve(null);
    }
  });
}

function normalizePort(value) {
  const port = typeof value === "number" ? value : Number(value);
  if (Number.isInteger(port) && port >= 1024 && port <= 65535) {
    return port;
  }
  return null;
}

function getHost(rawUrl) {
  try {
    return new URL(rawUrl).host;
  } catch (error) {
    return "";
  }
}

function getLastPathSegment(rawUrl) {
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

function getFileExtension(rawUrl) {
  const segment = getLastPathSegment(rawUrl);
  const lastDot = segment.lastIndexOf(".");
  if (lastDot === -1) {
    return "";
  }
  const extension = segment.slice(lastDot + 1).toLowerCase();
  return VIDEO_EXTENSIONS.has(extension) ? extension : "";
}

function getKindInfo(item) {
  if (item.kind === "hls") {
    return { label: "HLS", className: "kind-hls" };
  }
  const extension = getFileExtension(item.url);
  return {
    label: extension ? extension.toUpperCase() : "VIDEO",
    className: "kind-video"
  };
}

// 与 background.js 的 titleLooksLikeUrl 保持同一判断。直接打开媒体文件时，
// 标签页标题就是文档 URL；拿它当标题，同页每个清晰度看起来都一样。
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

function getDisplayTitle(item) {
  const pageTitle = typeof item.pageTitle === "string" ? item.pageTitle.trim() : "";
  // 有真正的页面标题就用它；标题只是 URL 时退回媒体文件名（FetchV 的做法），
  // 这样同一页的多个清晰度才能靠 `..._720p.m3u8` 区分开。
  if (pageTitle && !titleLooksLikeUrl(pageTitle, item.pageUrl || item.url)) {
    return pageTitle;
  }
  const segment = getLastPathSegment(item.url);
  if (segment) {
    return segment;
  }
  if (pageTitle) {
    return pageTitle;
  }
  const host = getHost(item.url);
  return host || "未命名视频";
}

function formatSize(value) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return "";
  }
  if (value < 1024) {
    return `${Math.round(value)} B`;
  }
  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1024;
  let unitIndex = 0;
  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex += 1;
  }
  const digits = size >= 100 ? 0 : size >= 10 ? 1 : 2;
  return `${size.toFixed(digits).replace(/\.0+$|(\.\d*[1-9])0+$/, "$1")} ${units[unitIndex]}`;
}

function showActionStatus(text, isError) {
  if (!elements) {
    return;
  }
  if (actionStatusTimer) {
    window.clearTimeout(actionStatusTimer);
    actionStatusTimer = null;
  }
  elements.actionStatus.textContent = text;
  elements.actionStatus.classList.remove("hidden", "is-error", "is-success");
  elements.actionStatus.classList.add(isError ? "is-error" : "is-success");

  if (!isError) {
    actionStatusTimer = window.setTimeout(() => {
      if (!elements) {
        return;
      }
      elements.actionStatus.classList.add("hidden");
      elements.actionStatus.textContent = "";
      actionStatusTimer = null;
    }, 3200);
  }
}

function setConnection(connectionState, text, detail) {
  if (!elements) {
    return;
  }
  elements.connection.dataset.state = connectionState;
  elements.connectionText.textContent = text;
  elements.connection.title = detail || "";
}

function updateConfigUi() {
  if (!elements) {
    return;
  }
  const hasToken = Boolean(state.config.token);
  elements.tokenWarning.classList.toggle("hidden", hasToken);
  elements.warningOptions.textContent = "去设置";
  elements.footerPort.textContent = `端口 ${state.config.port}`;
}

// --------------------------------------------------------------------------
// 数据加载
// --------------------------------------------------------------------------

async function refreshConnection() {
  setConnection("checking", "正在连接…", "");
  const response = await sendMessage({ type: "testConnection" });
  if (response && response.ok) {
    const appName = response.app || "easyVideo";
    const detail = response.version
      ? `${appName} ${response.version}`
      : appName;
    setConnection("connected", "已连接", detail);
  } else {
    setConnection(
      "error",
      "未连接",
      (response && response.error) || "无法连接 easyVideo 客户端"
    );
  }
}

async function loadItems() {
  if (state.tabId === null) {
    state.items = [];
    renderItems();
    return;
  }

  const response = await sendMessage({
    type: "getItems",
    tabId: state.tabId
  });

  if (response && Array.isArray(response.items)) {
    state.items = response.items;
  } else {
    state.items = [];
    if (response && response.error) {
      showActionStatus(response.error, true);
    }
  }
  renderItems();
}

// --------------------------------------------------------------------------
// 渲染
// --------------------------------------------------------------------------

function createItemError() {
  const error = document.createElement("div");
  error.className = "item-error";
  error.setAttribute("role", "alert");
  error.hidden = true;
  return error;
}

function createCaptureItem(item) {
  const article = document.createElement("article");
  article.className = "capture-item";

  const kind = getKindInfo(item);
  const badge = document.createElement("span");
  badge.className = `kind-badge ${kind.className}`;
  badge.textContent = kind.label;

  const body = document.createElement("div");
  body.className = "item-body";

  const title = document.createElement("div");
  title.className = "item-title";
  title.textContent = getDisplayTitle(item);
  title.title = item.url || "";
  body.appendChild(title);

  const meta = document.createElement("div");
  meta.className = "item-meta";
  const host = getHost(item.url);
  if (host) {
    const hostSpan = document.createElement("span");
    hostSpan.className = "item-host";
    hostSpan.textContent = host;
    meta.appendChild(hostSpan);
  }

  const sizeText = formatSize(item.size);
  if (sizeText) {
    if (meta.childNodes.length) {
      const dot = document.createElement("span");
      dot.className = "meta-dot";
      dot.setAttribute("aria-hidden", "true");
      dot.textContent = "·";
      meta.appendChild(dot);
    }
    const sizeSpan = document.createElement("span");
    sizeSpan.className = "item-size";
    sizeSpan.textContent = sizeText;
    meta.appendChild(sizeSpan);
  }

  if (!meta.childNodes.length) {
    const fallback = document.createElement("span");
    fallback.className = "item-host";
    fallback.textContent = "视频流";
    meta.appendChild(fallback);
  }
  body.appendChild(meta);

  const downloadButton = document.createElement("button");
  downloadButton.type = "button";
  downloadButton.className = "button button-small button-primary item-download";
  downloadButton.textContent = "下载";
  downloadButton.setAttribute("aria-label", `下载 ${getDisplayTitle(item)}`);

  const error = createItemError();

  downloadButton.addEventListener("click", () => {
    handleItemDownload(item, downloadButton, error).catch(() => {
      downloadButton.disabled = false;
      downloadButton.textContent = "下载";
      error.textContent = "发送失败，请重试。";
      error.hidden = false;
    });
  });

  article.append(badge, body, downloadButton, error);
  return article;
}

function renderItems() {
  if (!elements) {
    return;
  }
  elements.itemList.replaceChildren();

  if (state.items.length) {
    const fragment = document.createDocumentFragment();
    state.items.forEach((item) => {
      fragment.appendChild(createCaptureItem(item));
    });
    elements.itemList.appendChild(fragment);
  }

  const count = state.items.length;
  elements.itemCount.textContent = count ? `${count} 个` : "";
  elements.emptyState.classList.toggle("hidden", count !== 0);
}

// --------------------------------------------------------------------------
// 交互
// --------------------------------------------------------------------------

async function handleItemDownload(item, button, error) {
  error.hidden = true;
  error.textContent = "";
  button.disabled = true;
  button.textContent = "发送中…";

  const response = await sendMessage({ type: "download", item });
  if (response && response.ok) {
    button.textContent = "已发送";
    button.classList.add("is-success");
    window.setTimeout(() => {
      if (!button.isConnected) {
        return;
      }
      button.disabled = false;
      button.textContent = "下载";
      button.classList.remove("is-success");
    }, 1800);
    return;
  }

  button.disabled = false;
  button.textContent = "下载";
  error.textContent = (response && response.error) || "发送失败，请重试。";
  error.hidden = false;
}

async function handleRefresh() {
  elements.refreshButton.disabled = true;
  elements.refreshButton.textContent = "刷新中…";
  try {
    await Promise.all([loadItems(), refreshConnection()]);
  } finally {
    elements.refreshButton.disabled = false;
    elements.refreshButton.textContent = "刷新";
  }
}

async function handleClear() {
  if (state.tabId === null) {
    showActionStatus("无法读取当前标签页。", true);
    return;
  }

  elements.clearButton.disabled = true;
  elements.clearButton.textContent = "清空中…";
  try {
    const response = await sendMessage({
      type: "clear",
      tabId: state.tabId
    });
    if (response && response.ok) {
      state.items = [];
      renderItems();
      showActionStatus("已清空当前页面的捕获记录。", false);
    } else {
      showActionStatus(
        (response && response.error) || "清空失败，请重试。",
        true
      );
    }
  } finally {
    elements.clearButton.disabled = false;
    elements.clearButton.textContent = "清空";
  }
}

async function handleDownloadPage() {
  if (state.tabId === null) {
    showActionStatus("无法读取当前标签页。", true);
    return;
  }

  const button = elements.downloadPageButton;
  button.disabled = true;
  button.textContent = "发送中…";

  try {
    const response = await sendMessage({
      type: "downloadPage",
      tabId: state.tabId
    });
    if (response && response.ok) {
      button.textContent = "已发送";
      button.classList.add("is-success");
      showActionStatus("已发送当前页面到 easyVideo。", false);
      window.setTimeout(() => {
        if (!button.isConnected) {
          return;
        }
        button.disabled = false;
        button.textContent = "下载当前页面";
        button.classList.remove("is-success");
      }, 1800);
    } else {
      showActionStatus(
        (response && response.error) || "发送失败，请重试。",
        true
      );
      button.disabled = false;
      button.textContent = "下载当前页面";
    }
  } catch (error) {
    showActionStatus("发送失败，请重试。", true);
    button.disabled = false;
    button.textContent = "下载当前页面";
  }
}

function handleOpenOptions(event) {
  event.preventDefault();
  try {
    const result = chrome.runtime.openOptionsPage();
    if (result && typeof result.catch === "function") {
      result.catch((error) => {
        console.warn("[EasyVideo] 打开设置页失败", error);
        showActionStatus("无法打开设置页。", true);
      });
    }
  } catch (error) {
    console.warn("[EasyVideo] 打开设置页失败", error);
    showActionStatus("无法打开设置页。", true);
  }
}

function bindEvents() {
  elements.refreshButton.addEventListener("click", () => {
    handleRefresh().catch((error) => {
      console.warn("[EasyVideo] 刷新失败", error);
      showActionStatus("刷新失败，请重试。", true);
    });
  });

  elements.clearButton.addEventListener("click", () => {
    handleClear().catch((error) => {
      console.warn("[EasyVideo] 清空失败", error);
      showActionStatus("清空失败，请重试。", true);
    });
  });

  elements.downloadPageButton.addEventListener("click", () => {
    handleDownloadPage().catch((error) => {
      console.warn("[EasyVideo] 下载当前页面失败", error);
      showActionStatus("发送失败，请重试。", true);
      elements.downloadPageButton.disabled = false;
      elements.downloadPageButton.textContent = "下载当前页面";
    });
  });

  elements.openOptions.addEventListener("click", handleOpenOptions);
  elements.warningOptions.addEventListener("click", handleOpenOptions);
}

// --------------------------------------------------------------------------
// 初始化
// --------------------------------------------------------------------------

async function init() {
  elements = {
    connection: getElement("connection"),
    connectionText: getElement("connectionText"),
    openOptions: getElement("openOptions"),
    tokenWarning: getElement("tokenWarning"),
    tokenWarningText: getElement("tokenWarningText"),
    warningOptions: getElement("warningOptions"),
    refreshButton: getElement("refreshButton"),
    clearButton: getElement("clearButton"),
    downloadPageButton: getElement("downloadPageButton"),
    actionStatus: getElement("actionStatus"),
    itemCount: getElement("itemCount"),
    emptyState: getElement("emptyState"),
    itemList: getElement("itemList"),
    footerPort: getElement("footerPort")
  };

  bindEvents();
  setConnection("checking", "正在连接…", "");

  const [tab, configResponse] = await Promise.all([
    getActiveTab(),
    sendMessage({ type: "getConfig" })
  ]);

  state.tab = tab;
  state.tabId =
    tab && Number.isInteger(tab.id) && tab.id >= 0 ? tab.id : null;

  if (configResponse && !configResponse.error) {
    const port = normalizePort(configResponse.port);
    state.config = {
      port: port || DEFAULT_PORT,
      token:
        typeof configResponse.token === "string"
          ? configResponse.token.trim()
          : ""
    };
  }
  updateConfigUi();

  if (state.tabId === null) {
    showActionStatus("无法读取当前标签页。", true);
    renderItems();
    setConnection("error", "未连接", "无法读取当前标签页");
    return;
  }

  await Promise.all([loadItems(), refreshConnection()]);
}

document.addEventListener("DOMContentLoaded", () => {
  init().catch((error) => {
    console.warn("[EasyVideo] 初始化失败", error);
    if (elements) {
      showActionStatus("扩展初始化失败，请重新打开弹窗。", true);
      setConnection("error", "未连接", "初始化失败");
    }
  });
});
