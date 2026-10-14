// bia chrome passkey proxy. Page automation is chrome-devtools-mcp over remote debugging.
// WebAuthn ceremonies are completed from the secrets store on the native host.

const HOST = "com.bia.chrome";

let port = null;
let nextId = 1;
const pending = new Map();
let retryMs = 1000;
let retryTimer = null;

function connect() {
  if (port) return;
  if (retryTimer) {
    clearTimeout(retryTimer);
    retryTimer = null;
  }
  try {
    port = chrome.runtime.connectNative(HOST);
  } catch (e) {
    port = null;
    scheduleConnect();
    return;
  }
  const opened = Date.now();
  port.onMessage.addListener((msg) => {
    retryMs = 1000;
    const wait = pending.get(msg && msg.id);
    if (!wait) return;
    pending.delete(msg.id);
    if (msg.error) wait.reject(new Error(msg.error));
    else wait.resolve(msg.result);
  });
  port.onDisconnect.addListener(() => {
    void chrome.runtime.lastError;
    port = null;
    for (const wait of pending.values()) wait.reject(new Error("native host disconnected"));
    pending.clear();
    // The host exits at once when the MCP is not running; back off instead of respawning every second.
    if (Date.now() - opened < 2000) retryMs = Math.min(retryMs * 2, 5000);
    else retryMs = 1000;
    scheduleConnect();
  });
}

function scheduleConnect() {
  if (retryTimer) return;
  retryTimer = setTimeout(() => {
    retryTimer = null;
    connect();
  }, retryMs);
}

function post(msg) {
  if (port) port.postMessage(msg);
}

function rpc(method, params) {
  return new Promise((resolve, reject) => {
    if (!port) {
      reject(new Error("native host down"));
      return;
    }
    const id = "e" + nextId++;
    const timer = setTimeout(() => {
      if (pending.has(id)) {
        pending.delete(id);
        reject(new Error("passkey timed out"));
      }
    }, 15000);
    pending.set(id, {
      resolve: (v) => { clearTimeout(timer); resolve(v); },
      reject: (e) => { clearTimeout(timer); reject(e); },
    });
    post({ id, method, params });
  });
}

chrome.alarms.create("bia-keepalive", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener(() => { if (!port) connect(); });
connect();

// ---- WebAuthn proxy ----------------------------------------------------------------------

// Content scripts report {origin, challenge} for every navigator.credentials call; the proxy
// event carries only the options, so the challenge ties the two together.
const callers = new Map();
const callerWaits = new Map();

function b64urlNorm(s) {
  return String(s || "").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

chrome.runtime.onMessage.addListener((msg, sender) => {
  if (!msg || msg.type !== "bia-webauthn" || !sender.origin) return;
  const challenge = b64urlNorm(msg.challenge);
  if (!challenge) return;
  const info = { origin: sender.origin, tabId: sender.tab ? sender.tab.id : null, at: Date.now() };
  callers.set(challenge, info);
  setTimeout(() => callers.delete(challenge), 120000);
  const wait = callerWaits.get(challenge);
  if (wait) {
    callerWaits.delete(challenge);
    wait(info);
  }
});

function callerOrigin(challenge) {
  const key = b64urlNorm(challenge);
  const known = callers.get(key);
  if (known) return Promise.resolve(known.origin);
  // The content script message can land just after the proxy event.
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      callerWaits.delete(key);
      focusedOrigin().then(resolve);
    }, 1500);
    callerWaits.set(key, (info) => {
      clearTimeout(timer);
      resolve(info.origin);
    });
  });
}

function focusedOrigin() {
  return chrome.tabs.query({ active: true, lastFocusedWindow: true }).then((tabs) => {
    const tab = tabs[0];
    try {
      return new URL(tab && tab.url ? tab.url : "").origin;
    } catch (e) {
      return "";
    }
  });
}

function hostOf(origin) {
  try {
    return new URL(origin).hostname;
  } catch (e) {
    return "";
  }
}

// Host errors look like "NotAllowedError: reason"; keep the DOMException name for the page.
function webauthnError(e) {
  const text = String(e && e.message ? e.message : e);
  const m = /^(NotAllowedError|InvalidStateError|NotSupportedError|SecurityError):\s*(.*)$/.exec(text);
  return m ? { name: m[1], message: m[2] } : { name: "NotAllowedError", message: text };
}

async function attachProxy() {
  try {
    const err = await chrome.webAuthenticationProxy.attach();
    // An error string also comes back when this extension is already the proxy.
    if (err && !/already/i.test(err)) console.warn("webAuthenticationProxy.attach:", err);
  } catch (e) {
    console.warn("webAuthenticationProxy.attach:", e);
  }
}

if (chrome.webAuthenticationProxy) {
  chrome.webAuthenticationProxy.onIsUvpaaRequest.addListener((req) => {
    chrome.webAuthenticationProxy.completeIsUvpaaRequest({ requestId: req.requestId, isUvpaa: true });
  });
  chrome.webAuthenticationProxy.onCreateRequest.addListener(async (req) => {
    try {
      const details = JSON.parse(req.requestDetailsJson);
      const origin = await callerOrigin(details.challenge);
      const algs = (details.pubKeyCredParams || []).map((p) => p.alg);
      if (algs.length && !algs.includes(-7)) throw new Error("NotSupportedError: only ES256 passkeys are supported");
      const res = await rpc("passkey_create", {
        rpId: (details.rp && details.rp.id) || hostOf(origin),
        userName: (details.user && (details.user.name || details.user.displayName)) || "bia",
        userId: details.user && details.user.id,
        challenge: details.challenge,
        origin,
        excludeCredentials: (details.excludeCredentials || []).map((c) => c.id).filter(Boolean),
      });
      const response = res && res.response ? res.response : res;
      chrome.webAuthenticationProxy.completeCreateRequest({
        requestId: req.requestId,
        credential: JSON.stringify({
          id: res.id,
          rawId: res.rawId || res.id,
          type: "public-key",
          authenticatorAttachment: "platform",
          response,
          clientExtensionResults: {},
        }),
      });
    } catch (e) {
      chrome.webAuthenticationProxy.completeCreateRequest({ requestId: req.requestId, error: webauthnError(e) });
    }
  });
  chrome.webAuthenticationProxy.onGetRequest.addListener(async (req) => {
    try {
      const details = JSON.parse(req.requestDetailsJson);
      const origin = await callerOrigin(details.challenge);
      const res = await rpc("passkey_get", {
        rpId: details.rpId || hostOf(origin),
        challenge: details.challenge,
        origin,
        allowCredentials: (details.allowCredentials || []).map((c) => c.id).filter(Boolean),
      });
      const response = res && res.response ? res.response : res;
      chrome.webAuthenticationProxy.completeGetRequest({
        requestId: req.requestId,
        credential: JSON.stringify({
          id: res.id,
          rawId: res.rawId || res.id,
          type: "public-key",
          authenticatorAttachment: "platform",
          response,
          clientExtensionResults: {},
        }),
      });
    } catch (e) {
      chrome.webAuthenticationProxy.completeGetRequest({ requestId: req.requestId, error: webauthnError(e) });
    }
  });
  // Events reach an extension only after it becomes the active proxy.
  attachProxy();
}
