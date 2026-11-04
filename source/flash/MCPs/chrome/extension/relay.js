// Isolated world: forward the challenge; the service worker takes the origin from the sender, not the page.
document.addEventListener("bia-webauthn", (e) => {
  if (typeof e.detail !== "string") return;
  try {
    const d = JSON.parse(e.detail);
    chrome.runtime.sendMessage({ type: "bia-webauthn", challenge: String(d.challenge), mediation: String(d.mediation) });
  } catch (err) {}
});
