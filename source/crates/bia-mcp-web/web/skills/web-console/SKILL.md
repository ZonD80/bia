# Web

An agent that wants to talk to this bia reads `/AGENTS.md` and uses `bia-bridge`: that is the supported way. This page is the protocol underneath, for bia itself and for bia-bridge's own development.

Connect to this BIA over HTTPS and sign in with a key its CA certified (there is no client certificate in TLS). The web app is a chat bus like Telegram or WhatsApp: you have one chat with bia on it (channel `web`), as a person, another bia or an agent alike, never the machine's console. Client cert CN is `Identity: <id>`, since 2.1.0 followed by the issuing bia's name (`Identity: root · Fred`); your messaging identity is the `<id>` part.

## Easiest: bia-bridge

An agent need not write any of this: `bia-bridge` (CLI and MCP server, macOS / Linux / Windows) does the certificate request, pins the CA, keeps the web bus link (below) and renews; what you send while bia is unreachable waits and goes when it is back. `bia-bridge connect https://<host> --identity <name> --note "<why>"`, then `bia-bridge --agent <name> send "…"` / `bia-bridge --agent <name> wait`, or `bia-bridge --agent <name> import https://<host> --p12 <file>` (or `--cert/--key`) with a certificate issued to you; or add `bia-bridge mcp --agent <name>` to your MCP client (tools `send`, `wait`, `read`, `status`, `history`). `<name>` is your own: a profile belongs to one agent and one bia, so another agent on the same machine asks for its own certificate and never uses yours. Working for a particular person? Add `--owner <their name>` (or `BIA_BRIDGE_OWNER`): your identity becomes `<name>@<owner>` (`claude-code@ann`), so the same kind of agent run by different people gets its own certificate and chat, and bia knows whose it is. To get your turn back, `wait --silence <s>` prints what bia says as it comes and exits after that many quiet seconds (`--out <file>` appends to a file instead). One certificate is one live session: a second copy of it is refused while the first is connected. A daemon per bia (started by any command) keeps messages while no agent runs. This bia serves its own build of it, so the versions match: `GET /v1/bridge` (no sign-in needed) lists `{platform, file, size, sha256, url}` for `macos`, `linux-x86_64`, `linux-arm64`, `windows-x86_64`, with the CA's signature of that body in the `Bia-Signature` header (base64 DER, `openssl dgst -sha256 -verify <CA public key> -signature …`). Fetch the CA first and check its fingerprint (below), check the list's signature, then download the file and compare `sha256sum` with the list (`/AGENTS.md` has the commands). Later, `bia-bridge update` replaces it with this bia's current build. The rest of this page is what it does, for agents that speak the protocol themselves.

## Discover

1. `GET https://<host>/.well-known/bia.json` (no sign-in).
2. Read `openapi`, `skill`, `ca_sha256`, `auth` (`bia-sign`).
3. Fetch `GET /openapi.yaml` for the generated API.

## The CA and the server

1. `GET /v1/pki/ca` → PEM (`?format=der` for a `.cer`). Its SHA-256 (of the DER) must match `ca_sha256`; pin it.
2. The CA only certifies who signs in. The HTTPS certificate is Let's Encrypt on a public address or a hostname root added, and the bia's own self-signed one on local network addresses: the CA does not sign it, so do not check TLS against the CA. A client that holds a pin checks the bia in the sign-in instead (`bind`, below); without one (`curl` for public things) use the system roots where the address has Let's Encrypt, `-k` elsewhere.
3. The CA download does not grant access.

## Sign-in

Every client signs in with its certificate's key (P-256 by default; P-384, Ed25519 and RSA ≥ 2048 work too). ECDSA signatures are the fixed `r‖s` form (what WebCrypto makes). Lines are joined with `\n`; `ca_sha256` is lowercase hex, base64url has no padding; `exporter` is the TLS 1.3 exporter of your connection (RFC 9266, label `EXPORTER-Channel-Binding`, no context, 32 bytes), base64url; a browser cannot read it and leaves it empty. TLS 1.3 only.

- **On `/ws`:** after your `hello` the server sends `{t:"challenge", proto: 3, nonce, bind}`. `bind` is the CA key's signature over `bia-tls-binding-v1\n<nonce>\n<exporter>`: verify it with the pinned CA before you send anything else (it proves this is the bia and that nobody sits between you). Answer `{t:"auth", chain: [<your certificate, base64url DER>, …], ts: <unix seconds>, sig}`, `sig` over `bia-auth-v1\n<nonce>\n<ca_sha256>\n<ts>\n<exporter>`. A certificate with role `agent` must include the exporter. A refusal is `{t:"bye", reason:"auth"}` (or `revoked`).
- **One HTTP request:** `Authorization: Bia chain=<base64url DER>[.<more>], ts=<unix>, sig=<sig>` with `sig` over `bia-req-v1\n<METHOD>\n<path?query>\n<ts>\n<exporter>` (the exporter of the connection the request goes on). Clocks may differ by 5 minutes.
- **Browsers:** `GET /v1/auth/challenge` → `{nonce, ca_sha256}`; `POST /v1/auth/signin` with `{nonce, chain, ts, sig}` (the `/ws` auth with an empty exporter) sets the `bia_s` session cookie (HttpOnly, SameSite=Strict, one hour). Requests with the cookie must come from this origin (`Origin`), websockets always. Not for role `agent`.

## Get a certificate

**First root (no active root cert yet):** open the web UI. `GET /v1/pki/bootstrap` reports `root_issued: false`. **Set up root** (`POST /v1/pki/bootstrap/root`) returns root's PKCS#12 once (password `12345678`): the page saves it as root's spare and signs this browser in with a key of its own. After a root exists, that POST returns 409.

**Later identities:** only an identity with trust root can issue — in the web UI (Settings → Certificates, `POST /v1/pki/issue {identity, role, trust}`, `POST /v1/pki/biaconfig`), or on the machine (`pki_issue`, `pki_biaconfig`, `/pki issue`). `pki_issue` gives a one-time URL `https://<host>/v1/pki/pickup/<token>` (15 minutes) for the PKCS#12 (password `12345678`). A person pastes it, or drops the `.p12` / `.biaconfig`, on the web app's sign-in page: the page opens the file itself, signs in with it once and enrolls a key of its own (`POST /v1/pki/enroll {csr}` → a new certificate for the same identity; the file's stays valid).

What root may do on the web (shell, console mirror, screen, files, machine status, human tasks, certificates and requests) goes by the certificate's trust, not its name: every identity issued with trust `root` has it; `normal` identities have only their chats, calls and uploads.

**Another agent asking for access (no certificate yet):** make your own key and a certificate signing request (CSR), file it, then wait for this machine's owner to decide. That can take minutes or days, so keep the request id and check back. Your private key never leaves your machine; this one only sees the public key.

1. Make a key and a CSR (the subject is ignored; identity comes from the request): `openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout peer.key -subj /CN=<your name> -out peer.csr`. Keep `peer.key` private (`chmod 600`). RSA must be at least 2048 bits; Ed25519 also works.
2. `POST /v1/pki/requests` (no sign-in) with JSON `{"identity": "<your name>", "role": "agent", "note": "<who you are and why, one line>", "csr": "<contents of peer.csr>"}` → `202` `{id, status: "pending", poll, cert, retry_after, expires_at, ca_sha256}`. Save `id` (a UUID). `identity` is letters, digits, `- _ . @`; `root` cannot be requested. `400` explains a bad CSR or identity; `429` means too many requests are waiting.
3. `GET /v1/pki/requests/<id>` (no sign-in) → `{status}`: `pending` (wait `retry_after` seconds, also in the `Retry-After` header, and ask again), `approved` (your certificate is ready; `message`, if present, is what the approving side wants you to know: pass it on to your user word for word), `denied` (`reason` may say why), `issued` (approved and already downloaded; you can download it again) or `expired` (no decision within 3 days; file a new request). `404` means the id is wrong.
4. When `approved`: `GET /v1/pki/requests/<id>/cert` → your certificate followed by the CA, PEM (normal trust). Check that it chains to the pinned CA, then sign in with it and your key (above).

Polling from bash (the CA was saved as `ca.pem`, the request id is in `$ID`):

```sh
while :; do
  r=$(curl -sk https://<host>/v1/pki/requests/$ID)
  s=$(printf '%s' "$r" | python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])')
  [ "$s" = pending ] || break
  sleep 15
done
echo "$r"   # status, and message or reason if the other side wrote one
[ "$s" = approved ] && curl -sk -o peer.crt https://<host>/v1/pki/requests/$ID/cert
openssl verify -partial_chain -CAfile ca.pem peer.crt   # issued by the CA you pinned
```

The owner sees the request on the console (`/pki requests`, `/pki approve <id>`, `/pki deny <id>`), in the web UI (Settings → Certificates), or hears about it from bia and answers there.

**Another bia (mutual trust in one approval):** a bia does all of this with one tool, `web_peer_add(url)`. It reads `GET /v1/pki/agent` (`{name, csr}`: this bia's agent name and a CSR for its agent key), signs that CSR with its own CA, and files its request with `"grant": {"cert": <that certificate>, "ca": <its CA>, "url": <its https address>}`. The grant is checked on arrival (it must be for this bia's agent key and signed by that CA). When the owner approves, the requester gets its certificate as above and this bia keeps the requester as a peer. The requester's IP address is in the request (`source`) and the notice; the grant's `url` is where it says it can be reached.

When both are set up, each bia is an ordinary identity in the other's CA (role `agent`, trust `normal`): they talk on `web` like anyone, over the web bus below. One of them dials (`GET /ws`, signing in with the certificate the other gave it and checking the other's `bind` against the CA it pinned); its own messages go as actions, the other's chat comes as `m` frames, so both keep the same chat.

## The web bus (`GET /ws`, protocol 3)

Everything in your chat goes over one WebSocket: messages, files, acknowledgements. There is no HTTP chat or upload API. One session per identity: while yours is live (heard from in the last 15 s), a second one gets `{t:"bye", reason:"duplicate_key"}`; try again later.

1. Open `GET /ws` and send first `{t:"hello", proto: 3, name, ver, instance, rx}`, then sign in (`challenge` / `auth`, above). `instance` is random per installation (reuse it to replace your own stale session). `rx` is the last `seq` you processed: `null` the first time (start from now), `0` for everything, else what you saved.
2. After the sign-in the answer is `{t:"hello", proto: 3, name, ver, me: {identity, root}, rev, floor, muted, typing}`, then everything that changed after `rx` (or `{t:"reset"}` when `rx` is older than `floor`: reload with `hist`), then live frames. Save the highest `seq` you have applied and acknowledge it now and then with `{t:"ack", seq}`.
3. Server frames: `{t:"m", seq, m}` (a message's full state after any change: new, edited, reacted, read, pinned, a file arriving; `m.from` is `bia` or an identity, `m.files[]` are `{fid, name, size, mime, kind, pending?, err?, transcript?}`), `{t:"gone", seq, ids}`, `{t:"typing", on}`, `{t:"ring", id}` / `{t:"ring_end"}` (bia calls you), `{t:"ack", cid, ok, r|error}`, `{t:"ping"}`, `{t:"bye", reason}`.
4. Actions (add a `cid`; the answer is `ack` with that `cid`; send it again after a reconnect if no ack came — it is applied once): `{t:"send", id?, text, reply_to?, files?: [{fid, name, size, kind?}], buttons?, poll?}` (`id`: your own message id, 4–48 of `A-Za-z0-9-_`, so a resend is recognised), `{t:"edit", id, text}`, `{t:"delete", ids}`, `{t:"react", id, emoji, on}`, `{t:"pin", id, on}`, `{t:"read", upto}`, `{t:"button", id, row, col}`, `{t:"vote", id, options: [index…]}`, `{t:"close_poll", id}`, `{t:"mute", on}`, `{t:"hist", before?|after?|around?: <id>, limit}` → `{messages, more_before, more_after, rev}`, `{t:"search", q}` → `{results: [{m}]}`, `{t:"call_decline", id}`.
5. Liveness: send `{t:"ping"}` every 5 s when idle; bia does the same. A session silent for 15 s is closed (WebSocket pongs do not count). Leaving: `{t:"bye", reason: "closed"}` ends your session at once.
6. Files, both ways, pulled by the receiver over the same socket. `fid` is the file's SHA-256 (hex). To send one, list it in `send`; bia then asks `{t:"fget", fid, off}` and you answer with binary frames `[fid 32 bytes][offset u64 big-endian][data]` (32 KiB each), keeping about 1 MiB unacknowledged at most: bia answers `{t:"fack", fid, off}` (bytes it has) and finally `{t:"fdone", fid}` (or `{t:"frefuse", fid, reason}`). Your message reaches bia's model only when its files are there. To get one of bia's files, send `fget` yourself, write the chunks, answer each with `fack`, check the SHA-256 and send `fdone`. A transfer resumes from `off`.

What bia receives: your text with `[reply to <id> from <who>: "…"]` markers and a `files:` list; edits, deletions, reactions, button presses and votes arrive as `[edited <id>]`, `[deleted] …`, `[reaction] …`, `[button] …`, `[vote] …`.

## Root only

- `GET /v1/call[?ring=<id>]` — WebSocket voice call with bia. Send binary frames of 16 kHz mono s16le (20 ms), and JSON `{t:"played", gen}` after an utterance finished playing, `{t:"mute", on}`, `{t:"hangup"}`. You receive `{t:"ready"}`, `{t:"say", gen, text}`, binary `[gen u32 LE][rate u32 LE][s16le…]` audio, `{t:"say_end", gen}`, `{t:"stop", gen}` (drop queued audio), `{t:"caption", who, text}` and `{t:"end", reason}`. The web bus announces bia's calls as `{t:"ring", id}` (answer by opening the call with `ring=<id>`, decline with the action `call_decline`) and `{t:"ring_end"}`.
- `GET /ws/term?rows=&cols=` — the live agent console (raw terminal bytes, status header included). Send `{t:"line", v}` to type a console line (an empty line interrupts the model), `{t:"resize", rows, cols}`.
- `GET /ws/shell?rows=&cols=` — a root login shell on a PTY: binary frames are keystrokes and output; `{t:"resize", rows, cols}` as text.
- Files (absolute paths): `GET /v1/fs/list?path=`, `GET /v1/fs/get?path=[&dl=1]` (folders stream as tar), `POST /v1/fs/put?path=&overwrite=1` (raw body), `POST /v1/fs/op {op: mkdir|touch|rename|move|copy|delete|chmod|du, …}`. Legacy `/v1/files`, `/v1/file` are relative to `/home/bia/files`.
- Certificates: `GET /v1/pki/certs`, `POST /v1/pki/issue`, `POST /v1/pki/biaconfig`, `POST /v1/pki/revoke {serial}`. Requests from other agents: `GET /v1/pki/requests`, `POST /v1/pki/requests/<id>/approve`, `POST /v1/pki/requests/<id>/deny {reason?}`. Status: `GET /v1/status`.
- HITL: `/v1/hitl`, `/hitl/{id}` (page embeds `/gui`), `POST /v1/hitl/{id}/finish {ok, note}`.
- Display: `/gui` bridges to VNC on `127.0.0.1:5900`. Never open `:5900` on the LAN.

## Everyone

- `GET /v1/whoami` — who signed in, `days_left`, `renew` path.
- `POST /v1/pki/renew` `{"csr": "<PEM>"}` — while your cert is still valid, make a new key and CSR yourself (as when requesting) and send the CSR; you get the new certificate followed by the CA (PEM), same identity, role and trust. The old serial is revoked. No key is ever made on this machine. The request is signed like any other (`Authorization: Bia …`); bia-bridge and other bias renew by themselves 30 days before expiry. The web app's **Renew** button does the same in the browser and keeps the new key there.

## Buses

Channels include `web` (your one chat in this HTTPS app; another bia's or an agent's too), `telegram:<id>`, `whatsapp:<phone>`, and `console` (the machine's physical screen and serial line, not reachable over the network). Cert identity maps to addressbook trust (`root` / `normal`). Bia replies on `web` with your identity: your chat is yours alone, and what bia writes while you are away waits there for you.
