"use strict";

/* EasyVideo 视频捕获 - 设置页逻辑 */

const DEFAULT_PORT = 27853;
let elements = null;

// --------------------------------------------------------------------------
// 工具
// --------------------------------------------------------------------------

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

function normalizePort(value) {
  const port = typeof value === "number" ? value : Number(value);
  if (Number.isInteger(port) && port >= 1024 && port <= 65535) {
    return port;
  }
  return null;
}

function showStatus(text, type) {
  if (!elements) {
    return;
  }
  elements.status.hidden = false;
  elements.status.classList.remove(
    "hidden",
    "is-error",
    "is-success",
    "is-neutral"
  );
  elements.status.classList.add(
    type === "error"
      ? "is-error"
      : type === "success"
        ? "is-success"
        : "is-neutral"
  );
  elements.status.textContent = text;
}

function setButtonBusy(button, busy, busyLabel, normalLabel) {
  button.disabled = busy;
  button.textContent = busy ? busyLabel : normalLabel;
}

function readFormValues() {
  const port = normalizePort(elements.portInput.value);
  if (port === null) {
    showStatus("端口必须是 1024-65535 之间的整数。", "error");
    elements.portInput.focus();
    return null;
  }
  return {
    port,
    token: elements.tokenInput.value.trim()
  };
}

function writeLocalConfig(config) {
  return new Promise((resolve) => {
    try {
      chrome.storage.local.set(config, () => {
        const error = readChromeLastError();
        if (error) {
          console.warn("[EasyVideo] 保存设置失败", error);
          resolve(false);
          return;
        }
        resolve(true);
      });
    } catch (error) {
      console.warn("[EasyVideo] 保存设置失败", error);
      resolve(false);
    }
  });
}

// --------------------------------------------------------------------------
// 页面操作
// --------------------------------------------------------------------------

async function loadSettings() {
  const response = await sendMessage({ type: "getConfig" });
  if (response && !response.error) {
    const port = normalizePort(response.port);
    if (port !== null) {
      elements.portInput.value = String(port);
    }
    elements.tokenInput.value =
      typeof response.token === "string" ? response.token : "";
  } else if (response && response.error) {
    showStatus(`无法读取已保存的设置：${response.error}`, "error");
  }
}

async function handleTestConnection() {
  const values = readFormValues();
  if (!values) {
    return;
  }

  const button = elements.testConnectionButton;
  setButtonBusy(button, true, "测试中…", "测试连接");
  showStatus(`正在测试端口 ${values.port}…`, "neutral");

  try {
    const response = await sendMessage({
      type: "testConnection",
      port: values.port
    });
    if (response && response.ok) {
      const appName = response.app || "easyVideo";
      const version = response.version ? ` ${response.version}` : "";
      showStatus(
        `连接成功：${appName}${version}（端口 ${values.port}）`,
        "success"
      );
    } else {
      showStatus(
        (response && response.error) || "无法连接 easyVideo 客户端。",
        "error"
      );
    }
  } finally {
    setButtonBusy(button, false, "测试中…", "测试连接");
  }
}

async function handleSave(event) {
  event.preventDefault();
  const values = readFormValues();
  if (!values) {
    return;
  }

  const button = elements.saveButton;
  setButtonBusy(button, true, "保存中…", "保存");

  try {
    const saved = await writeLocalConfig(values);
    if (!saved) {
      showStatus("保存失败，请重试。", "error");
      return;
    }

    elements.portInput.value = String(values.port);
    elements.tokenInput.value = values.token;
    if (values.token) {
      showStatus("设置已保存。", "success");
    } else {
      showStatus(
        "设置已保存，但访问令牌为空，下载前请先粘贴 easyVideo 中显示的令牌。",
        "neutral"
      );
    }
  } finally {
    setButtonBusy(button, false, "保存中…", "保存");
  }
}

function handleToggleToken() {
  const willReveal = elements.tokenInput.type === "password";
  elements.tokenInput.type = willReveal ? "text" : "password";
  elements.toggleTokenButton.textContent = willReveal ? "隐藏" : "显示";
  elements.toggleTokenButton.setAttribute(
    "aria-pressed",
    willReveal ? "true" : "false"
  );
}

function bindEvents() {
  elements.settingsForm.addEventListener("submit", (event) => {
    handleSave(event).catch((error) => {
      console.warn("[EasyVideo] 保存设置失败", error);
      showStatus("保存失败，请重试。", "error");
    });
  });

  elements.testConnectionButton.addEventListener("click", () => {
    handleTestConnection().catch((error) => {
      console.warn("[EasyVideo] 测试连接失败", error);
      showStatus("测试连接失败，请重试。", "error");
    });
  });

  elements.toggleTokenButton.addEventListener("click", handleToggleToken);
}

// --------------------------------------------------------------------------
// 初始化
// --------------------------------------------------------------------------

async function init() {
  elements = {
    settingsForm: document.getElementById("settingsForm"),
    portInput: document.getElementById("port"),
    tokenInput: document.getElementById("token"),
    toggleTokenButton: document.getElementById("toggleTokenButton"),
    testConnectionButton: document.getElementById("testConnectionButton"),
    saveButton: document.getElementById("saveButton"),
    status: document.getElementById("status")
  };

  elements.portInput.value = String(DEFAULT_PORT);
  bindEvents();
  await loadSettings();
}

document.addEventListener("DOMContentLoaded", () => {
  init().catch((error) => {
    console.warn("[EasyVideo] 设置页初始化失败", error);
    if (elements) {
      showStatus("设置页初始化失败，请刷新页面重试。", "error");
    }
  });
});
