// Page world: announce each WebAuthn call's challenge so the extension learns the calling origin.
(() => {
  const creds = navigator.credentials;
  if (!creds || !creds.create || !creds.get) return;
  function b64url(buf) {
    const bytes = buf instanceof ArrayBuffer ? new Uint8Array(buf) : new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
    let s = "";
    for (const b of bytes) s += String.fromCharCode(b);
    return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }
  function announce(options) {
    try {
      const ch = options && options.publicKey && options.publicKey.challenge;
      if (ch) document.dispatchEvent(new CustomEvent("bia-webauthn", { detail: b64url(ch) }));
    } catch (e) {}
  }
  const create = creds.create.bind(creds);
  const get = creds.get.bind(creds);
  creds.create = function (options) {
    announce(options);
    return create(options);
  };
  creds.get = function (options) {
    announce(options);
    return get(options);
  };
})();
