// bia chrome passkey proxy. Page automation is chrome-devtools-mcp over remote debugging.
// WebAuthn requests wait on the native host until the model approves or denies them; only then
// does the host sign from the secrets store. The host decides: nothing here can skip it.

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

// No timer: an approval takes as long as it takes. The host expires what nobody answers,
// the page's own timeout arrives as onRequestCanceled, and a lost port rejects everything.
function rpc(id, method, params) {
  return new Promise((resolve, reject) => {
    if (!port) {
      reject(new Error("native host down"));
      return;
    }
    pending.set(id, { resolve, reject });
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
  const info = {
    origin: sender.origin,
    top: sender.tab && sender.tab.url ? originOf(sender.tab.url) : "",
    conditional: msg.mediation === "conditional",
    known: true,
  };
  callers.set(challenge, info);
  setTimeout(() => callers.delete(challenge), 120000);
  const wait = callerWaits.get(challenge);
  if (wait) {
    callerWaits.delete(challenge);
    wait(info);
  }
});

function originOf(url) {
  try {
    return new URL(url).origin;
  } catch (e) {
    return "";
  }
}

function caller(challenge) {
  const key = b64urlNorm(challenge);
  const known = callers.get(key);
  if (known) return Promise.resolve(known);
  // The content script message can land just after the proxy event.
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      callerWaits.delete(key);
      focusedOrigin().then((origin) => resolve({ origin, top: origin, conditional: false, known: false }));
    }, 1500);
    callerWaits.set(key, (info) => {
      clearTimeout(timer);
      resolve(info);
    });
  });
}

function focusedOrigin() {
  return chrome.tabs.query({ active: true, lastFocusedWindow: true }).then((tabs) => {
    const tab = tabs[0];
    return tab && tab.url ? originOf(tab.url) : "";
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

// requestId -> {rpcId, canceled}; lets a page's abort or timeout retire the waiting request.
const running = new Map();

async function ceremony(req, method, params, complete) {
  const job = { rpcId: "e" + nextId++, canceled: false };
  running.set(req.requestId, job);
  try {
    const details = JSON.parse(req.requestDetailsJson);
    const who = await caller(details.challenge);
    if (job.canceled) return;
    const res = await rpc(job.rpcId, method, {
      origin: who.origin,
      topOrigin: who.top,
      known: who.known,
      conditional: who.conditional,
      timeoutMs: details.timeout || 0,
      challenge: details.challenge,
      ...params(details, who.origin),
    });
    if (job.canceled) return;
    const response = {
      clientDataJSON: res.clientDataJSON,
      authenticatorData: res.authenticatorData,
      ...(res.attestationObject
        ? {
            attestationObject: res.attestationObject,
            transports: ["internal"],
            publicKey: res.publicKey,
            publicKeyAlgorithm: res.publicKeyAlgorithm,
          }
        : { signature: res.signature, userHandle: res.userHandle || null }),
    };
    await complete({
      requestId: req.requestId,
      responseJson: JSON.stringify({
        id: res.credential_id,
        rawId: res.credential_id,
        type: "public-key",
        authenticatorAttachment: "platform",
        response,
        clientExtensionResults: {},
      }),
    });
  } catch (e) {
    if (!job.canceled) complete({ requestId: req.requestId, error: webauthnError(e) });
  } finally {
    running.delete(req.requestId);
  }
}

if (chrome.webAuthenticationProxy) {
  const proxy = chrome.webAuthenticationProxy;
  proxy.onIsUvpaaRequest.addListener((req) => {
    proxy.completeIsUvpaaRequest({ requestId: req.requestId, isUvpaa: true });
  });
  proxy.onCreateRequest.addListener((req) => {
    ceremony(req, "passkey_create", (details, origin) => {
      const algs = (details.pubKeyCredParams || []).map((p) => p.alg);
      if (algs.length && !algs.includes(-7)) throw new Error("NotSupportedError: only ES256 passkeys are supported");
      return {
        rpId: (details.rp && details.rp.id) || hostOf(origin),
        userName: (details.user && (details.user.name || details.user.displayName)) || "bia",
        userId: details.user && details.user.id,
        excludeCredentials: (details.excludeCredentials || []).map((c) => c.id).filter(Boolean),
      };
    }, (done) => proxy.completeCreateRequest(done));
  });
  proxy.onGetRequest.addListener((req) => {
    ceremony(req, "passkey_get", (details, origin) => ({
      rpId: details.rpId || hostOf(origin),
      allowCredentials: (details.allowCredentials || []).map((c) => c.id).filter(Boolean),
    }), (done) => proxy.completeGetRequest(done));
  });
  proxy.onRequestCanceled.addListener((requestId) => {
    const job = running.get(requestId);
    if (!job) return;
    job.canceled = true;
    const wait = pending.get(job.rpcId);
    if (wait) {
      pending.delete(job.rpcId);
      wait.reject(new Error("canceled"));
      post({ method: "passkey_cancel", params: { id: job.rpcId } });
    }
  });
  // Events reach an extension only after it becomes the active proxy.
  attachProxy();
}
