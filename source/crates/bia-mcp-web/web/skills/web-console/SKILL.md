# Web

Connect to this BIA over HTTPS with a client certificate issued by its CA. The web app is a chat bus like Telegram or WhatsApp: you are a person with your own chats on it (channel `web` for your main chat, `web:<chat>` for others), never the machine's console. Client cert CN is `Identity: <id>` (e.g. `Identity: root`); your messaging identity is the `<id>` part.

## Discover

1. `GET https://<host>/.well-known/bia.json` (no client cert).
2. Read `openapi`, `skill`, `ca_sha256`, `auth` (`mtls`).
3. Fetch `GET /openapi.yaml` for the generated API.

## Trust the CA

1. `GET /v1/pki/ca` → PEM (`?format=der` for a `.cer`); trust it for this host (fingerprint must match `ca_sha256`). From then on verify every request against it (`curl --cacert ca.pem`); do not use `-k`. The server certificate names this host's addresses.
2. The CA download does not grant access and does not issue a client certificate.
3. People get the CA inside their `.p12` (the web UI has no separate CA download), and the server sends it after its own certificate in the TLS handshake. Either way, check the fingerprint against `ca_sha256` before trusting it.

## Get a client certificate

**First root (no active root cert yet):** open the web UI without a client cert. `GET /v1/pki/bootstrap` reports `root_issued: false`. Click **Issue root certificate** (or `POST /v1/pki/bootstrap/root`) — the PKCS#12 is returned once. Import `root.p12` (password `12345678`), then reload. After a root exists, that POST returns 409.

**Later identities:** only an identity with trust root can issue — in the web UI (Settings → Certificates, `POST /v1/pki/issue {identity, role, trust}`), or on the machine (`pki_issue`, `/pki issue`). Bia returns a one-time HTTPS URL `https://<host>/v1/pki/pickup/<token>` (15 minutes). `GET` it once (no client cert) and import the PKCS#12 (password `12345678`).

What root may do on the web (shell, console mirror, screen, files, machine status, human tasks, certificates and requests) goes by the certificate's trust, not its name: every identity issued with trust `root` has it; `normal` identities have only their chats, calls and uploads.

**Another agent asking for access (no certificate yet):** make your own key and a certificate signing request (CSR), file it, then wait for this machine's owner to decide. That can take minutes or days, so keep the request id and check back. Your private key never leaves your machine; this one only sees the public key.

1. Make a key and a CSR (the subject is ignored; identity comes from the request): `openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout peer.key -subj /CN=<your name> -out peer.csr`. Keep `peer.key` private (`chmod 600`). RSA must be at least 2048 bits; Ed25519 also works.
2. `POST /v1/pki/requests` (no client cert) with JSON `{"identity": "<your name>", "role": "agent", "note": "<who you are and why, one line>", "csr": "<contents of peer.csr>"}` → `202` `{id, status: "pending", poll, cert, retry_after, expires_at, ca_sha256}`. Save `id` (a UUID). `identity` is letters, digits, `- _ . @`; `root` cannot be requested. `400` explains a bad CSR or identity; `429` means too many requests are waiting.
3. `GET /v1/pki/requests/<id>` (no client cert) → `{status}`: `pending` (wait `retry_after` seconds, also in the `Retry-After` header, and ask again), `approved` (your certificate is ready; `message`, if present, is what the approving side wants you to know: pass it on to your user word for word), `denied` (`reason` may say why), `issued` (approved and already downloaded; you can download it again) or `expired` (no decision within 3 days; file a new request). `404` means the id is wrong.
4. When `approved`: `GET /v1/pki/requests/<id>/cert` → your certificate followed by the CA, PEM (normal trust). Use it with your key: `curl --cacert ca.pem --cert peer.crt --key peer.key …`.

Polling from bash (the CA was saved as `ca.pem`, the request id is in `$ID`):

```sh
while :; do
  r=$(curl -s --cacert ca.pem https://<host>/v1/pki/requests/$ID)
  s=$(printf '%s' "$r" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')
  [ "$s" = pending ] || break
  sleep 15
done
echo "$r"   # status, and message or reason if the other side wrote one
[ "$s" = approved ] && curl -s --cacert ca.pem -o peer.crt https://<host>/v1/pki/requests/$ID/cert
curl -s --cacert ca.pem --cert peer.crt --key peer.key https://<host>/v1/whoami
```

The owner sees the request on the console (`/pki requests`, `/pki approve <id>`, `/pki deny <id>`), in the web UI (Settings → Certificates), or hears about it from bia and answers there.

**Another bia (mutual trust in one approval):** a bia does all of this with one tool, `web_peer_connect(url)`. It reads `GET /v1/pki/agent` (`{name, csr}`: this bia's agent name and a CSR for its agent key), signs that CSR with its own CA, and files its request with `"grant": {"cert": <that certificate>, "ca": <its CA>, "url": <its https address>}`. The grant is checked on arrival (it must be for this bia's agent key and signed by that CA). When the owner approves, the requester gets its certificate as above and this bia keeps the requester as a peer: both can now call each other (`web_peer_call`). The requester's IP address is in the request (`source`) and the notice; the grant's `url` is where it says it can be reached.

## Chats

Each identity has chats like Telegram: `main` is channel `web`, other chats are `web:<chat>`. Messages have ids (the same `b…` id bia sees in its envelope), replies, forwards, attachments, reactions, edits, pins, read marks, inline buttons and polls.

- `GET /v1/chats` — your chats (`{t:"hello", me, chats, drafts, typing}`).
- `GET /v1/chats/<chat>/messages?limit=&before=|after=|around=<id>` — pages of messages.
- `GET /v1/search?q=&chat=` — search text and file names.
- `POST /v1/upload?name=<file>` (raw body) → `{path}` under `inbox/web/<identity>/`; then send it in `files`.
- `GET /v1/att?chat=&id=&i=[&dl=1]` — an attachment (Range supported).
- `POST /v1/chats/action` with one action, or the same JSON over the WebSocket:
  - `{t:"send", chat, text, reply_to?, files?: [paths], kind?: "voice"}`
  - `{t:"edit", chat, id, text}` · `{t:"delete", chat, ids}` · `{t:"react", chat, id, emoji}` (toggles)
  - `{t:"pin", chat, id, on}` · `{t:"read", chat, upto}` · `{t:"forward", chat, ids, to}`
  - `{t:"button", chat, id, row, col}` · `{t:"vote", chat, id, options: [index…]}`
  - `{t:"chat_new", title}` · `{t:"chat_rename", chat, title}` · `{t:"chat_mute", chat, on}` · `{t:"chat_clear", chat}` · `{t:"chat_delete", chat}`
- `GET /ws` — WebSocket. First frame `{t:"hello"}`, then events: `msg`, `upd`, `del`, `read`, `clear`, `chat`, `chat_del`, `typing`, `draft` (streamed answer, `d` = delta), `draft_end`. Send actions with a `cid`; replies are `{t:"ack", cid, ok, r|error}`. Send `{t:"ping"}` at least every 2 minutes.

What bia receives: your text with `[reply to <id> from <who>: "…"]` / `[forwarded from …]` markers and a `files:` list; edits, deletions, reactions, button presses and votes arrive as `[edited <id>]`, `[deleted] …`, `[reaction] …`, `[button] …`, `[vote] …`. Legacy `{t:"line", v, files}` still sends to the main chat.

## Root only

- `GET /v1/call?chat=<id>[&ring=<id>]` — WebSocket voice call with bia. Send binary frames of 16 kHz mono s16le (20 ms), and JSON `{t:"played", gen}` after an utterance finished playing, `{t:"mute", on}`, `{t:"hangup"}`. You receive `{t:"ready"}`, `{t:"say", gen, text}`, binary `[gen u32 LE][rate u32 LE][s16le…]` audio, `{t:"say_end", gen}`, `{t:"stop", gen}` (drop queued audio), `{t:"caption", who, text}` and `{t:"end", reason}`. The chat WebSocket announces bia's calls as `{t:"ring", chat, id}` (answer by opening the call with `ring=<id>`, decline with the action `call_decline`) and `{t:"ring_end", chat}`.
- `GET /ws/term?rows=&cols=` — the live agent console (raw terminal bytes, status header included). Send `{t:"line", v}` to type a console line (an empty line interrupts the model), `{t:"resize", rows, cols}`.
- `GET /ws/shell?rows=&cols=` — a root login shell on a PTY: binary frames are keystrokes and output; `{t:"resize", rows, cols}` as text.
- Files (absolute paths): `GET /v1/fs/list?path=`, `GET /v1/fs/get?path=[&dl=1]` (folders stream as tar), `POST /v1/fs/put?path=&overwrite=1` (raw body), `POST /v1/fs/op {op: mkdir|touch|rename|move|copy|delete|chmod|du, …}`. Legacy `/v1/files`, `/v1/file` are relative to `/home/bia/files`.
- Certificates: `GET /v1/pki/certs`, `POST /v1/pki/issue`, `POST /v1/pki/revoke {serial}`. Requests from other agents: `GET /v1/pki/requests`, `POST /v1/pki/requests/<id>/approve`, `POST /v1/pki/requests/<id>/deny {reason?}`. Status: `GET /v1/status`.
- HITL: `/v1/hitl`, `/hitl/{id}` (page embeds `/gui`), `POST /v1/hitl/{id}/finish {ok, note}`.
- Display: `/gui` bridges to VNC on `127.0.0.1:5900`. Never open `:5900` on the LAN.

## Everyone

- `GET /v1/whoami` — identity from the cert, `days_left`, `renew` path.
- `POST /v1/pki/renew` `{"csr": "<PEM>"}` — while your cert is still valid, make a new key and CSR yourself (as when requesting) and send the CSR; you get the new certificate followed by the CA (PEM), same identity, role and trust. The old serial is revoked. No key is ever made on this machine. Example: `openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout new.key -subj /CN=me -out new.csr` then `curl --cacert ca.pem --cert peer.crt --key peer.key -H 'Content-Type: application/json' -d "$(python3 -c 'import json;print(json.dumps({"csr":open("new.csr").read()}))')" -o new.crt https://<host>/v1/pki/renew`, and switch to `new.crt` + `new.key`. The web app's **Renew** button does the same in the browser and saves a .p12 (password `12345678`).

## Buses

Channels include `web` / `web:<chat>` (your chats in this HTTPS app), `telegram:<id>`, `whatsapp:<phone>`, and `console` (the machine's physical screen and serial line, not reachable over the network). Cert identity maps to addressbook trust (`root` / `normal`). Bia replies on your channel with your identity: your chats are yours alone.
