// Isolated world: forward the challenge; the service worker takes the origin from the sender, not the page.
document.addEventListener("bia-webauthn", (e) => {
  if (typeof e.detail !== "string") return;
  try {
    chrome.runtime.sendMessage({ type: "bia-webauthn", challenge: e.detail });
  } catch (err) {}
});
