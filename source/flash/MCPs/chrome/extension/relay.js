// Isolated world: forward the challenge; the service worker takes the origin from the sender, not the page.
document.addEventListener("bia-webauthn", (e) => {
  if (typeof e.detail !== "string") return;
  try {
    const d = JSON.parse(e.detail);
    chrome.runtime.sendMessage({ type: "bia-webauthn", challenge: String(d.challenge), mediation: String(d.mediation) });
  } catch (err) {}
});

// Calls: a tab opened with chrome_call_open hands its audio to the service worker and gets
// bia's voice back. Other tabs are told they are not such a tab and send nothing.
(() => {
  let armed = false;
  let port = null;
  function toPage(msg) {
    document.dispatchEvent(new CustomEvent("bia-voice-in", { detail: JSON.stringify(msg) }));
  }
  function connect() {
    try {
      port = chrome.runtime.connect({ name: "bia-voice" });
    } catch (e) {
      port = null;
      return;
    }
    port.onMessage.addListener(toPage);
    port.onDisconnect.addListener(() => {
      port = null;
    });
  }
  document.addEventListener("bia-voice-out", (e) => {
    if (!armed || typeof e.detail !== "string") return;
    if (!port) connect();
    try {
      if (port) port.postMessage(e.detail);
    } catch (err) {
      port = null;
    }
  });
  try {
    chrome.runtime.sendMessage({ type: "bia-voice-hello" }, (reply) => {
      void chrome.runtime.lastError;
      armed = !!(reply && reply.armed);
      if (armed) connect();
      toPage({ t: "armed", on: armed });
    });
  } catch (e) {
    toPage({ t: "armed", on: false });
  }
})();
