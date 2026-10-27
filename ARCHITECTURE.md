# bia — architecture

A machine that boots directly into an LLM agent: a small **Debian trixie**
root with **systemd**, the stock Debian kernel and a Secure Boot chain. The
agent is the only user-facing program; everything else is there to serve it.

## Big picture

```
 USB stick / internal disk / QEMU image
 ┌───────────────────────────────────────────────────────────────────────┐
 │ p1 BIABOOT  ESP, FAT32 256 MiB                                         │
 │    EFI/BOOT/BOOTX64.EFI | BOOTAA64.EFI  shim (Microsoft-signed)        │
 │    EFI/debian/grub*.efi                 GRUB (Debian-signed)           │
 │    vmlinuz-lts, initramfs-lts           Debian kernel + initramfs      │
 │    bia-rescue/rescue.cpio.gz            offline update overlay         │
 │    bia-rescue/next.cfg                  present only while updating    │
 │    EFI/bia/fmm.json                     optional, seeded by bia-deploy  │
 │    EFI/bia/lang.json                    owner's language (bia-deploy)   │
 │    EFI/bia/models.txt                   until bia-grow moved the model │
 │ p2 BIABIOS  1 MiB BIOS boot (x86_64 legacy, GRUB core.img)             │
 │ p3 BIAROOT  ext4: Debian + /home/bia (model, MCPs, files) + swapfile   │
 │             spans the rest of the stick; ext4 grows into it at boot    │
 │             (BIADISK on an internal disk installed by bia-migrate)     │
 └───────────────────────────────────────────────────────────────────────┘
                 │ GRUB → kernel → initramfs → root=LABEL=…
                 ▼
 systemd ── bia-esp-id.service   (own FAT serial for this drive's ESP; /boot by it)
        ├── bia-grow.service     (ext4 over the whole stick; moves in a flashed model)
        ├── bia-update.service   (finishes a pending OS update, unfolds agent)
        ├── bia.service ─ bia-session ─ Xorg :0 + openbox + tint2 + kitty
        │                                └── bia-agent  (console on tty1/serial/kitty)
        │                                     ├── llama-server (child, 127.0.0.1)
        │                                     └── MCP children over stdio
        │                                          └── web: HTTPS app :443 (mTLS)
        ├── bia-land.service     (ISO/"land" mode only, instead of bia.service)
        └── networking, getty@tty2, logind …
```

`root.img` is built once per architecture and is identical for the stick, the
QEMU image and the internal disk an install produces. `/home/bia` inside it
holds `bia.toml`, `model/`, `MCPs/` and `files/`. It never contains a model and
is only as large as its contents.

### Filling the stick and the model (`bia-grow`)

`bia-deploy` makes BIAROOT span the whole stick but writes only `root.img` at its
start. A local model is chosen at flash time from Hugging Face (search, then a
GGUF file; split GGUFs count as one) and streamed raw onto the end of BIAROOT,
checked against the Hub's LFS sha256, and listed in `EFI/bia/models.txt`
(`<offset in partition> <size> <sha256> <name>`). The ESP is written last, so
a failed download leaves no bootable half-stick.

On first boot `bia-grow.service` (before `bia-update` and `bia`) moves each
listed file into `/home/bia/model/<name>.part` in steps: copy what fits in the
free space, fsync, `resize2fs` the mounted ext4 up to the first byte not yet
copied, repeat; then it checks the sha256, renames the file, removes the list
and grows ext4 to the end of the partition. A step only needs the headroom
`bia-deploy` leaves (512 MiB), not room for a second copy, and an interrupted
run resumes from the `.part` file. Without a list it only grows the ext4.
`bia.toml` uses `file = "auto"`, so the agent picks up the model.

## Boot chain

1. Firmware loads `shim` → Debian-signed GRUB (UEFI), or GRUB `core.img` from
   BIABIOS (legacy BIOS, x86_64). Stock Secure Boot works because no binary is
   modified; the Debian kernel keeps its signature.
2. GRUB boots `vmlinuz-lts` + `initramfs-lts` with
   `root=LABEL=BIAROOT rootwait net.ifnames=0 console=tty0 console=ttyS0`.
   A **rescue hook** (`bia-rescue-hook`) appended to every bia `grub.cfg` boots
   the rescue image instead while `bia-rescue/next.cfg` exists.
3. The stock Debian initramfs mounts the root named on the command line:
   `BIAROOT` on the stick, `BIADISK` on an internal disk (each drive's ESP
   carries its own `grub.cfg`).
4. systemd starts `bia-update.service` (only acts when an update is pending),
   then `bia.service`. `getty@tty1` and the serial gettys are masked so the
   agent owns those consoles; `tty2` keeps a login for emergencies.
5. `bia-session` starts X on `:0` with Openbox, a tint2 switcher and a kitty
   window running `bia-agent`. Without X or kitty it execs the agent on the
   text console directly.

   Scripts in the kitty window come from DejaVu, Noto Color Emoji, Noto CJK
   and Noto core, in that fontconfig fallback order. Typing goes through
   fcitx5, which runs on a private session bus (`/run/bia/session-bus`) and
   which kitty reaches over the IBus protocol. Ctrl+Space or Ctrl+Shift+Space
   (macOS, and so a VM on a Mac, keeps Ctrl+Space for itself) switches
   between the keyboard and the last input method. The right end of the top
   bar shows the current one (EN, RU, 中, あ…), and a click there cycles
   through all of them (`bia-im`). Ctrl+Shift+U opens kitty's emoji picker.
   The image starts with the US keyboard only (`/usr/local/share/bia/fcitx5/`).
   The list then follows the languages bia has learned (see Speech).

   Three limits remain. The kernel text console (the fallback, and tty2) only
   shows what its 512-glyph font holds, so there is no CJK there. kitty does
   not shape Indic conjuncts. Right-to-left text is drawn without bidi
   reordering.

6. The agent's first turn answers a `system` notice about how it started
   (normal boot, unexpected halt, update, landing). On the machine's very first
   start (no `files/state/welcomed`, nothing to resume, no earlier exit, not an
   update or a landing) the notice instead asks for a short welcome in the
   owner's language: ask for a name, the web app and its first-visit root
   certificate, logging in to Telegram and WhatsApp, the agent's own accounts
   on websites (passwords and codes in the password store, human help for
   captchas), and a stronger cloud brain when it runs on the local model. The
   marker is then written, so this happens once; an upgraded machine gets the
   marker on its first boot without the welcome.
7. After an update the notice also carries the release notes: `RELEASE_NOTES.md`
   (repo root, built into the agent; `release.rs`) has per version a **Changes**
   list for the owner and a **For bia** list of steps (e.g. link client
   certificates to addressbook contacts). The agent keeps the version it last
   booted in `files/state/version`; on the first boot of a newer one it gets every
   release in between, tells root what changed and does the steps itself.
   Machines from before the version file (≤ 1.1.0) get the current release.
   `tools/release.sh` uses the same Changes as the GitHub release text.

## The agent (`source/crates/bia-agent`)

One Rust binary, dynamically linked against glibc. Main modules:

| Module | Role |
|---|---|
| `main.rs`, `sys.rs` | startup: `/home/bia` checks, memory tuning (swap, THP, mlock), model, MCPs (the web MCP always), resume, FMM |
| `console.rs` | the physical console: writes to every kernel console at once; pinned 3-line status header (DECSTBM scroll region); input from all consoles; status also published to `files/state/status.json` |
| `repl.rs` | the loop: slash commands, chat turns, tool rounds, streaming, interruption, autosave |
| `tools.rs` | tool registry (built-ins + MCP), prompt rendering, tool-call parsing (Qwen/Hermes JSON and LFM2 pythonic) |
| `llama_srv.rs`, `llm.rs`, `tune.rs`, `gguf.rs` | supervise prebuilt `llama-server`; pick backend (CUDA → ROCm → SYCL → Vulkan → CPU), threads, ctx and KV from sysfs + GGUF header |
| `remote.rs` | FMM: streaming tool-calling client for chat completions (OpenRouter, OpenAI, xAI, DeepSeek, Qwen/DashScope), Anthropic Messages, and the Responses API for a ChatGPT plan (token refresh/rotation) |
| `mcp.rs` | MCP client over stdio, discovery of `/home/bia/MCPs/*/mcp.json`, `boot_tool` |
| `jobs.rs`, `bash.rs` | every MCP call and shell command is a job; results return as `system|job:<id>|…` envelopes; PTY shells as root or `bia` |
| `outbox.rs` | telegram/whatsapp sends, edits, reactions and deletes: per-chat FIFO worker threads, local `o<n>` ids mapped to platform ids |
| `memory.rs` | append-only journal + time-ranged categorized reflections (`/reflect`) |
| `schedules.rs` | calendar of tasks/reminders, delivered as `system|schedule` with catch-up after boot |
| `state.rs` | `files/state/conversation.json` (fsynced, `.bak` fallback) |
| `buses.rs`, `addressbook.rs` | buses (console built in; web, telegram, whatsapp from their MCPs), channels, contacts with trust levels |
| `weblink.rs` | the web MCP's address from `files/state/web.json`, for the status header, `/web` and the first-boot tour |
| `clock.rs`, `tz.rs`, `hw.rs` | RFC 3339 time, NTP/RTC, IANA zones without tzdata; DMI/PCI/USB identity |

### Inference

`builders/llama-prebuilt.sh` unpacks the official ggml-org Ubuntu release builds of `llama-server` for each
backend into `/usr/local/lib/bia/llama/`. The agent starts the best one that
works on 127.0.0.1 and talks to it over HTTP; GPU stacks never touch the agent
link and a failing backend falls back to the next. Weights stay mmap'd (and
optionally mlocked); KV and heap spill to the 256 MiB `/swapfile` (1 GiB when
installed to a disk with `bia-migrate`).

FMM replaces the local model with a cloud one. Configuration lives in
`files/state/fmm.json` (written by `system_fmm_configure`, or seeded at first
boot from `EFI/bia/fmm.json` when the stick was flashed with keys; the ESP copy
is deleted once seeded). The agent
waits for DNS before enabling it and can switch back with `/fmm off`.

### Protocol

Every inbound item is one line:

```
identity|channel|issued|delivered|id|message
```

- `identity`: `system` (own notices, job results, schedules), `root` (the
  person at the console or a root-trusted contact), or a contact id.
- `channel`: `console`, `web`, `telegram:<chat>`, `whatsapp:<chat>`,
  `messenger:<chat>`, `instagram:<chat>`,
  `job:<id>`, `schedule`.
- `issued` / `delivered`: RFC 3339 UTC; lines typed during generation are
  queued, so the model can see how long a message waited.
- `id`: platform message id, or a local `b…` id.

The model replies with tool calls; `say(identity, channel, message)` sends to a
channel and returns at once. Plain text is the final answer for the console.

On telegram, whatsapp, messenger and instagram, `say` goes through the outbox. Each send gets a
local id `o<n>` (`files/state/outbox.json`) and joins its chat's queue, which
one worker thread runs in order; different chats run in parallel. `say` waits
up to 5 s (not at all for 8 MB or more of attachments). In time, it returns
the platform message ids as before. Otherwise it returns `queued as o<n>`,
and the agent goes on. The outcome comes later: `[sent o<n> → …]` is added
quietly to the next turn, while `[send failed o<n> …]` arrives as
`system|outbox` and starts a turn. Edits, reactions and deletes use the same
queue, so they can name `o<n>` (or `o<n>.2` for the second message of a
multi-file send) before it is delivered. `say(cancel=o<n>)` drops a queued
send or abandons an upload. A send cut off by a restart is reported as
`[send lost o<n> …]` at boot.

### Tools and discovery

Built-in tools: `bash`, `say`, `discover`, `set_model_config`, `job_*`/`list_jobs`,
`memory_*`, `schedule_*`, `addressbook_*`. The prompt starts with only a few;
`discover(q=…)` searches and `discover(path="mcp/<name>", enable=true)` loads
an MCP's function definitions as a `system` message, so the cached prompt
prefix stays valid. Keyword matches in a request can load a server too.
Loaded servers persist with the conversation.

### Web

The web app is the `web` MCP (`source/crates/bia-mcp-web`), a bus like telegram
and whatsapp. It has nothing to do with the console: the console is the
physical screen and serial line, and only there do slash commands, a blank
Enter (stop) and the model's own notes exist. People on web are envelope
identities (their client certificate), each with their own chats: the main chat
is channel `web`, others are `web:<chat>`. Their messages reach the agent as MCP
`inbound` notifications and are journaled like any bus; the agent answers through
the MCP (`say`, `react`, `delete_messages`, `pin_message`, `close_poll` and
`chat_history` call its `send`, `react`, … functions, as do read marks and typing;
those functions are the agent's own path and are not model tools). A web channel
only names a chat, so a reply always
carries the person: `say(channel="web")` without an identity uses whoever last
wrote on that same channel, and the MCP refuses an unknown identity or `bia`
itself. The agent loads the MCP at boot when `[web] enabled` and passes
`BIA_WEB_PORT` / `BIA_WEB_HTTP_PORT` (443/80, dev 8443/0); the MCP publishes its
address in `files/state/web.json`.

HTTPS on :443 (HTTP :80 redirects), rustls + ring. The CA,
server and client certificates come from `bia-pki` under
`files/state/pki/`. The CA is public at `/v1/pki/ca`; everything else needs a
client certificate. The first root certificate can be issued from the
onboarding page while none exists; later ones are issued by root (web
Settings, `web_pki_issue` or `/pki issue <id>` on the console) and delivered once as a PKCS#12
pickup URL. Every .p12 bia hands out uses the password `12345678` (macOS will
not import one with an empty password). Renewal (`POST /v1/pki/renew`) takes a
CSR and returns PEM: no key is made here. The web app's Renew button makes
a P-256 key with WebCrypto (`web/pki.js`), sends the CSR and builds the .p12
in the browser. Both sides write the same modern PKCS#12 (`bia-pki/src/p12.rs`,
`web/pki.js`): PBES2 (PBKDF2-HMAC-SHA256, AES-256-CBC) key bag, SHA-256 MAC, no
RC2/3DES, so OpenSSL 3 opens it without `-legacy`.

Another agent without a certificate asks for one with a CSR for a key it made
itself: `POST /v1/pki/requests` `{identity, role, note, csr}` checks the CSR's
signature and key (RSA >= 2048, EC, Ed25519) and returns a random UUID and
`202`. The request is stored in `pki/requests/<uuid>.json` and handed to the
model as a `system|web` notice so it can ask root. Root decides with
`/pki approve|deny <id>` on the console, in web Settings, or by telling bia (`web_pki_approve` /
`web_pki_deny`). Approval signs the CSR's public key with an
identity, role (`agent`), trust (`normal`) and usages set here; the private key
never exists on this machine. Deciding can take days, so the requester polls
`GET /v1/pki/requests/<uuid>` (`pending` with `Retry-After`, then `approved`,
`denied`, `issued` or `expired`) and downloads the PEM certificate + CA from
`…/<uuid>/cert`. Pending requests lapse after 3 days; at most 20 wait at once, 3
per address. Certificates root issues for people (`web_pki_issue`, pickup URL) and the first
root certificate are still made here as PKCS#12, since browsers import those.
`tests/e2e/peer-e2e.sh` (`make peer-e2e`) runs this between two QEMU agents.

Two bias set up trust both ways in one approval (`bia-pki` `peers.rs`). Every bia
has one agent key (`pki/agent.key`) and publishes its name (`bia-<CA fingerprint>`
unless set) and a CSR for that key at the public `GET /v1/pki/agent`.
`web_peer_connect(url, ca_sha256?)` on bia A fetches B's CA (pinned when a
fingerprint is given), signs B's agent CSR with A's CA and files a request at B
with A's own CSR plus a `grant`: that certificate, A's CA and A's address (the
local address of the connection unless `my_url` is given). B checks the grant on
arrival: it must be for B's agent key and verify against the CA that comes with it.
When B's root approves, B keeps A as a peer (`pki/peers/<name>/`: `peer.json`,
`ca.pem`, `cert.pem`); A's `web-peers` thread polls, collects its certificate and
keeps B. Each side now calls the other with its agent key and the other's
certificate (`web_peer_call`, or curl with the files `web_peer_list` prints). The
server name is not checked (addresses change); the pinned CA is. A denied or
expired request revokes the certificate A made for B. Requests show the source IP
and, for a bia, the address it claims. Any certificate can also be limited to
networks (`IssuedMeta.from`, IP SANs for single addresses): `check_client_from`
refuses it from elsewhere (`wrong_source`); renewals keep the limit.

The page (`web/index.html`, `app.js`, `app.css`, `term.js`, `emoji.js`) is a
Telegram-style app. `chat.rs` keeps each identity's chats in
`files/state/web/<identity>/`. Messages carry replies, forwards, attachments (uploads go to
`files/inbox/web/<identity>/`), reactions, edits, pins, read marks, inline
buttons and polls; user actions reach the model as envelopes with `[reply to …]`,
`[edited …]`, `[reaction] …`, `[button] …`, `[vote] …` markers. Voice calls run
in the MCP (`call.rs`, `web_call_start` / `web_call_hangup`). For root the app adds a
root shell on a PTY (`/ws/shell`), a file manager over the whole filesystem
(`/v1/fs/*`), human tasks (`/hitl`), the display (`/gui`, noVNC → x11vnc on
127.0.0.1:5900), machine status (from `files/state/status.json`) and certificate
admin; there is no console in the web app. Discovery: `/.well-known/bia.json`,
`/openapi.yaml`, `/skills/web-console/SKILL.md`. `tests/integration/web-mcp-e2e.py` drives the
MCP binary end to end.

## MCP servers (`source/crates/bia-mcp-*`, `source/flash/MCPs/*`)

All speak MCP over stdio via the small `bia-mcp` crate. Its `share` module turns shared locations and contacts into the text the model reads (`[location] … map link`, `[contact] name, phones, e-mails`) and reads/writes vCards, the same for Telegram and WhatsApp. A live location reaches the model once; later points (Telegram message edits, WhatsApp update messages) only refresh `LiveTracker` (`files/state/<bus>-live.json`), `*_live_location` reads it, and `[live location ended]` arrives once when the sender stops, the time runs out, or updates stop for 20 min. Real edits by others arrive as `[edited <id>] sent …, edited …` with `was:` and `now:`; the old text comes from the WhatsApp history cache or Telegram's `TextMemory` (`files/state/telegram-texts.json`, last 5000 messages); hidden or no-change edits are dropped. The build copies each
binary into `source/flash/MCPs/<name>/bin/<arch>/`, which `rootfs.sh` packs
into `/home/bia/MCPs`.

| MCP | What it does |
|---|---|
| `system` | FMM configure/disable, human-in-the-loop, file links. Disks and network are plain Debian tools through `bash`. |
| `web` | the web bus: HTTPS + mTLS app with Telegram-style chats and calls for people (client certificates), root shell, files, display, human tasks; client certificates and agent requests (`web_pki_*`) |
| `secrets` | passwords, TOTP/HOTP, WebAuthn passkeys (ES256) in `files/state/passwords.json`; 1Password CSV/JSON import, CXF export |
| `chrome` | Google Chrome on profile `~/.config/bia-chrome` with remote debugging; force-installed CRX extension proxies WebAuthn to `secrets`; page automation through Google's `chrome-devtools-mcp` (vendored Node); native GTK file dialogs are announced (`[file dialog opened]`) and answered with `chrome_choose_files` (xdotool on :0; for a folder it waits for Chrome's in-window "Upload N files?" prompt and returns its words with x,y for the model to click; test: `tests/integration/chrome-dialog-test.sh`) |
| `screen` | screenshot, mouse and keyboard on `:0` (xdotool/scrot); `screen_ocr` lists the words on screen (or a region) with their center x,y (`bia-ocr`: PaddleOCR PP-OCRv5 mobile on the image's onnxruntime, recognizers for Chrome's languages plus English, sha256-pinned models downloaded on first use into `files/ocr/models`; test: `tests/integration/screen-ocr-test.sh`) |
| `telegram` | a real Telegram user via ferogram; voice calls via tgcalls/ntgcalls; speech through `bia-speech`; locations, venues and contacts in and out (`send_location`, `send_contact`); groups (`create_group`, `add_members`, `remove_member`, `leave_group`, `group_invite_link`) and their administration (`group_info`, `set_admin` with named rights and a title, `group_permissions` for what members may do plus slow mode, join approval, history for new members, protected content and disappearing messages, `restrict_member`, `set_group_info`, `join_requests` / `answer_join_request`); deletes are for everyone unless `revoke=false` |
| `whatsapp` | WhatsApp linked device via whatsapp-rust (QR pairing in a kitty window), voice calls via its VoIP plane; speech through `bia-speech`; locations and contacts in and out (`send_location`, `send_contact`); groups (same five tools) and their administration (`group_info`, `set_admin`, `group_permissions`: who may send / edit info / add, join approval, disappearing messages; `set_group_info`, `join_requests` / `answer_join_request`); deletes are for everyone unless `revoke=false` |
| `messenger` | Facebook Messenger as the account signed in to bia's Chrome (cookies read over Chrome's DevTools port, no password); classic and end-to-end encrypted chats (encrypted ones over WhatsApp's protocol as a registered device); text, files, voice messages (TTS → AAC), replies, edits, reactions, unsends, read marks, typing; search and resolve (names, profile links, ids); groups (create, add/remove, leave, `group_info`, `set_admin`, `set_group_info`; encrypted groups also `group_permissions` and join requests, like WhatsApp); bans, Meta blocks, message requests, mute; the Facebook bio (`set_profile`, the GraphQL mutation fbchat-v2 uses; name and photo stay in Chrome); no calls. Chat ids: a person's Facebook id, `t<thread>` for a classic group, `<id>@g.us` for an encrypted one |
| `instagram` | Instagram DMs the same way (session from Chrome); shared posts and reels as `[shared]` lines with their link; also pins (chat or message); unsend only (no remove-for-me). Chat id: the thread key (a DM's is the other person's id) |
| `lang` | languages bia hears and speaks: catalog (`catalog.json`, sha256-pinned), on-demand installs into `files/speech`, voices, speech settings, transcribe/synthesize files |
| `carry` | `fold`/`unfold` backups of `/home/bia`; `fly` to a machine in land mode (UDP discovery, X25519 handshake, confirmed code); `update` and `rescue` (see below); `morph` |

### Messenger and Instagram

Both MCPs are one Rust crate (`bia-mcp-meta`, two binaries) in front of one Go
program, `bia-meta` (`source/go/bia-meta`, static, no cgo, in the messenger
MCP's `bin/`; instagram uses it from there). The protocol work is
[mautrix-meta](https://github.com/mautrix/meta)'s libraries, pinned in `go.mod`:
`messagix` for Messenger's socket and tasks, whatsmeow for encrypted chats,
`instameow` for Instagram. `bia-meta --platform … --dir files/<platform>` speaks
JSON lines on stdin/stdout: requests answered by id, and events (`message`,
`backlog`, `edit`, `delete`, `reaction`, `chat`, `status`, `logged_out`).
Its state is `files/<platform>/session.json` (cookies, 0600) and `meta.db`
(SQLite via modernc: whatsmeow's device keys plus bia's chat, people and
message index for edits, reactions, replies and history). Incoming files are
not downloaded until the message's turn in its chat lane: the event carries a
ref and the MCP asks for the file, then transcribes voice notes. `backlog`
(the inbox at connect, offline messages) is delivered when newer than
`files/state/<platform>.json` `last_seen`. History comes from the server
(classic Messenger threads, Instagram) or else from the local index (encrypted
chats have no server history for linked devices).

### Speech

`bia-speech` (a library shared by `telegram`, `whatsapp`, `messenger`, `instagram` and `lang`) dlopens the
sherpa-onnx C API and keeps models loaded in-process. Incoming audio (call audio
per participant, or a voice-note file) goes through Silero VAD, split at short
pauses. Each segment of 1 s or more gets language identification with the
bundled multilingual Whisper tiny, on GTCRN-denoised audio. Routing to a learned
language has hysteresis, and short segments take their neighbour's language.
Neighbouring segments of one language are merged and transcribed by Whisper
with that language forced: tiny for calls, and whisper-base for voice notes
once a second language is learned. A hallucination filter runs on the result,
and an echo filter and a speaker check (voice print) are opt-in. The output is
`[speech ru] … [en] …`. A language that is heard consistently but not learned
is learned automatically. Hearing is enabled at once (Whisper knows it), a voice
note is decoded again, and the voice for speaking it is installed in the
background. The install code (`bia-speech::learn`) is shared with the `lang`
MCP. The model gets a `[new language: …]` notice. Received voice messages
arrive as `[voice message] [speech <lang>] …`. Every speech step is logged to
`files/speech/logs/speech.log`, and its tail is shown in `*_call_status`.
Speaking needs `lang`: text is split into `<lang=xx>` spans, and every span's
Piper (or Kokoro) voice is checked before anything is spoken. The registry
`files/speech/languages.json` is written by `lang` under a file lock and
re-read by the other MCPs when it changes. Voice notes are sent as OGG/Opus
with duration and waveform. The agent links `bia-speech` too, for web chats:
voice messages are transcribed on an ordered delivery thread before they reach
the model, and the transcript is shown under the bubble. `say(voice=true, lang)`
posts a voice reply on web, and on telegram/whatsapp it calls `send_voice`.

Typing follows the same registry (`bia-speech::input`). After every registry
change, the console's fcitx5 list is set to: the US keyboard, then a layout
or input method for each learned language whose script a US keyboard cannot
type (ru → `keyboard-ru`, zh → pinyin, ja → anthy, ko → hangul, vi → unikey,
hi → `keyboard-in`, …; 44 in all). The default language comes first, so
Ctrl+Space reaches it. `lang_input` overrides a language's input, for example
a native Latin layout, or none. Items added by hand in fcitx5 are kept. The
profile file is rewritten and the running fcitx5 gets the list over DBus
(`busctl … SetInputMethodGroupInfo`; `fcitx5-remote -r` does not re-read the
profile). Labels for the top bar go to `bia-labels` next to it. The agent
syncs once at boot as well. mozc is not used: its server refuses to run as
root, and the console session is root.

The owner's language can be chosen in the flasher (GUI picker, `bia-deploy
write --lang`, `BIA_LANG=` for `make flash`; the default is the host locale).
It goes onto the ESP as `EFI/bia/lang.json`. On first boot the agent makes it
the default language and enables hearing it. The keyboard is added at once,
and the voice is downloaded in the background, retried every minute for a
day while there is no network. The ESP file is then deleted. While the
default language is not English, the system prompt tells the model to use it
with root, on console and web, and in boot replies, so the first reply is
already in that language. The console's own labels stay English.

`fold`/`unfold` (and `fly`, OS update) carry all of `files/speech`: the
registry, the default language, input overrides and the downloaded models, at
the same paths, so learned languages work at once after an unfold with no
re-learning. The fcitx5 profile under `/root` is not carried; the boot sync
rebuilds it from the registry (items added by hand in fcitx5 are lost). At
every boot `learn::resume` also restarts downloads that were cut off (the home
language's voice, or a registered voice whose files are gone) in the
background, with the same one-minute retries.

### Chrome extension key

`extension.pem` (in `source/crates/bia-mcp-chrome/` and
`source/flash/MCPs/chrome/`) is the extension's signing key. Its hash is the
fixed extension id that the Chrome policy force-installs, and `crx.rs`
compiles it into `bia-mcp-chrome` as a fallback. It is therefore public by
design: anyone with a bia binary has it. It only identifies the bundled
extension; it grants no access to user data. Forks that want their own
extension id should generate a new key and update the policy id.

The key is shipped on purpose. Passkey ceremonies (WebAuthn proxying), native
messaging and other browser interactions are hard to get right, and the
extension is expected to keep evolving. Keeping the signing key with the
system lets bia agents improve the extension themselves: change
`extension/`, bump the manifest version, repack it with this key (the
extension id stays the same, so force-installed Chrome picks up the update)
and propose the updated version back to the project.

## OS update through the rescue image

`carry_update` / console `/update [force] [/dev/…]` updates the running system
from the other attached bia root (flash ↔ internal disk). The source must have
a newer `/etc/bia-release` unless forced. Neither root is in use while it is
overwritten:

1. `bia-update start` folds the agent into `/bia-update/fold.tar.gz` (a file list
   streamed straight into tar, no temporary copy), writes
   `bia-rescue/next.cfg` on this ESP (source/destination root and ESP UUIDs),
   sets BootNext to this drive and reboots.
2. GRUB's rescue hook boots the ESP's `vmlinuz-lts` + `initramfs-lts` +
   `bia-rescue/rescue.cpio.gz` with `rdinit=/bia-rescue/init`. The overlay
   (`builders/rescue.sh`) adds bash, rsync, e2fsck, efibootmgr, vfat modules
   and the `rescue` script; drivers come from the matching initramfs.
3. The rescue rsyncs source → destination (`--checksum --delete`; keeps
   fstab, swapfile, machine-id, clock, `/home/bia/files`, `/bia-update`),
   verifies by checksum, refreshes the destination ESP, sets
   `/bia-update/pending`, removes the marker and reboots.
4. `bia-update.service` unfolds the agent and reboots into the new system.

On failure the marker stays (every boot retries), the log goes to
`<dst>/bia-update/rescue.log`, and a shell opens. `carry_rescue` / `/rescue`
does the same from the flash with the internal disk as destination.
Every step is also appended to `/bia-update/update.log` (the consoles and the
tool keep their output): `bia-update` start/rescue/fsck begin it afresh, the
rescue image appends its run, the unfold boot appends its lines, and the final
cleanup keeps it, so the agent tails one file instead of probing.
`tests/e2e/update-e2e.sh` (`make update-e2e`) tests disk, flash and rescue cases
in QEMU (passing on arm64).

Legacy BIOS has no BootNext, and a stick plugged in for the update usually
boots first. So on a BIOS boot `bia-update` writes the same marker on the other
bia ESP as well (with that ESP's own rescue image, which matches its kernel):
the rescue runs whichever drive the firmware starts, and removes both markers.
When it ran from a drive that is not the destination (an update of the disk
started from the stick, or `/rescue`), it powers off and says to remove the
stick, since a reboot would start the stick again. BIOS GRUB (MBR + core.img)
is not changed by updates; its modules on the ESP stay with it.
`tests/e2e/bios-update-e2e.py` (`make bios-update-e2e`, x86_64 SeaBIOS) runs all
three cases, with the stick hot-plugged ahead of the disk for `disk`.

## Installing to an internal disk

`carry_to_disk` runs `bia-migrate to-disk`, which copies the running system
onto an internal drive: on an empty disk it writes a new GPT with an ESP and an
ext4 `BIADISK`; next to Windows it installs onto a chosen or newly created
volume and shares the existing ESP (`EFI/bia` + an `efibootmgr` entry).
Under UEFI it then reboots into the disk with a one-time `BootNext`. A legacy
BIOS keeps its own boot order (the stick first), so on a BIOS boot it embeds
GRUB i386-pc in the disk's BIOS boot partition and powers off for the user to
remove the stick. The embed does what `grub-bios-setup` does: boot.img in the MBR
points at core.img, and core.img's first-sector blocklist points at its own
remaining sectors (grub-mkimage's default only fits core.img at LBA 1).
`bia-deploy` does this natively (`bia_disk::embed_bios_grub`); `mkimage.sh` and
`bia-migrate` use `grub-bios-embed.py`. `tests/e2e/bios-e2e.sh` (`make bios-e2e`)
tests the whole path in QEMU (SeaBIOS, x86_64) with a stick written by
`bia-deploy`.
`bia-migrate to-flash` goes the other way. Later updates between the two use
the rescue image above.

## Releases and the OS download

The OS ships as one minimal disk image per architecture, `bia-<arch>.img.xz`
(`tools/mkimg.sh` → `bia-deploy image`): GPT, ESP (shim, GRUB, kernel,
initramfs, rescue image), the BIOS boot partition with GRUB core.img and
boot.img in the MBR (x86_64), and BIAROOT holding `root.img`, only as large as
the system needs. No FMM keys and no model. One x86_64 image boots on both UEFI
and legacy BIOS; arm64 is UEFI only.

Any image writer (Etcher, Rufus, Raspberry Pi Imager, `dd`) can put it on a
stick as it is. Then the GPT only covers the image: on first boot `bia-grow`
moves the backup GPT to the end of the disk (`sfdisk --relocate`), grows
BIAROOT over the rest (`sfdisk -N`, `partx -u`) and ext4 with it. It only does
that to a partition named BIAROOT, never to an installed BIADISK. Keys go into
`EFI/bia/fmm.json` on the ESP by hand; without keys or a model the agent does
not start.

`bia-deploy write` reads the same image (`--os-image`, or downloaded) front to
back, xz-decoded on the fly: it writes a GPT for the whole stick (same
partition starts as the image's), keeps the image's MBR boot code, copies the
BIOS boot partition and BIAROOT, streams a Hugging Face model onto the end of
BIAROOT, checks the image's SHA-256, and writes the ESP last with `fmm.json`,
`models.txt` and `lang.json` added. A damaged download therefore leaves the stick unbootable,
never half-written.

`tools/release.sh` publishes `v<workspace version>` on GitHub (`zond80/bia`).
It looks at what that release already has and builds only the missing
packages: per architecture the image (`tools/os-bundle.py`: `xz -6 -T0`, which
also decodes multi-threaded), the land ISOs, the Mac flasher (universal,
Developer ID signed and notarized by `installers/macos/release.sh
--universal`), and the Linux and Windows flashers (built in `bia-build:x86_64`,
Windows with MinGW). Every built file, the raw image included, is scanned for
FMM keys, the Apple app password, `HF_TOKEN` and the home path first.
`bia-os.json` lists each image with its download and raw size and SHA-256; the
release must be a pushed, clean commit. Asset names carry no version, so
`releases/latest/download/<name>` always works (website download buttons).

`bia-deploy` (`src/release.rs`) reads `bia-os.json` from the latest release
(or `--os <tag>`), downloads the `.img.xz` with resume and checks it, and keeps
it in its cache (`~/Library/Caches/bia-deploy`, `%LOCALAPPDATA%\bia-deploy\cache`,
`~/.cache/bia-deploy`) so the next stick needs no download. Local builds give
`--os-image dist/<arch>/bia-<arch>.img` (the GUI's advanced field reads
`dist/<arch>/local-files.json`, written by `make build`).

`tests/e2e/img-e2e.sh` (`make img-e2e`) tests both paths in QEMU: `MODE=flash`
serves a local release over HTTP and has `bia-deploy` write it with keys and a
real Hugging Face model; `MODE=dd` decodes the `.img.xz` onto a bigger disk
and adds the keys by hand. `FIRMWARE=bios` boots x86_64 under SeaBIOS.

## Taking over a running Linux

`tools/takeover.sh` (a release asset) downloads `bia-deploy-linux-<arch>.tar.gz`,
checks it against `bia-os.json` `tools` and runs `bia-deploy takeover`
(`installers/bia-deploy/src/takeover/`). Its inputs are partial JSON configs
(`--config` file or https URL, repeatable, `--config-sha256` pins; objects merge,
lists replace, later wins), then flags, then detection, then questions on
`/dev/tty` (none with `--yes`). Fields: `fmm` (`key` alone is enough), `lang`,
`setup` (free text the model gets once at first boot), `pki` (`names`, `ips`,
`root`: `generate` or `{csr}`, `root_from`, `agent_name`), `network` (`auto` or
`{interfaces: […]}`), `disk`, `wipe`, `os`, `reboot`, `gate_port`.
`--dump-config` prints the resolved config with secrets masked.

From another computer, `bia-deploy server <host>` (and the GUI's "On my server",
`installers/bia-deploy/src/remote/`) drives the same takeover over SSH (russh, pure
Rust): password, key file, pasted OpenSSH/PEM/PuTTY key, or a new ed25519 key made for
the hosting provider (`remote/keys.rs`, kept in the app folder). The server's host key
is pinned on first contact (`known_hosts.json` in the app folder; a changed one stops
the login). A non-root login becomes root with `sudo -k -S` and the login password on
stdin (`-k`: a cached sudo would hand the password to the command). The config goes to
a 0600 file under `/root` through stdin, never the command line, and `takeover.sh` runs
in `--events` mode: JSON lines on stdout (`plan`, `gate` with port, token and CA, `ready`)
while the text stays on stderr. The first run adds `--dump-config` and gives the plan;
after the one confirmation the second run has `disk`/`wipe`/`reboot` set. When the gate
opens, a second SSH channel fetches `root.p12` from `127.0.0.1:<port>` on the server
itself (no open port needed), saves it with the CA in the app folder
(`servers/<host>/`), and `remote/trust.rs` imports the certificate and trusts the CA
(macOS login keychain, Windows user store, Chrome's NSS database on Linux). Then it
waits until `https://<host>/` answers with that CA (land mode: TCP 4791). The setup
text is always empty here; land mode sends a land secret made (or pasted) on this
computer. `DEPLOY=1 tests/e2e/takeover-e2e.sh` tests it: password login as `ubuntu`,
sudo with that password, first contact refused until `--host-key`.

Both `takeover.sh` and `bia-deploy takeover` refuse to run on bia itself
(`/etc/bia-release` or `/etc/bia-shipped` exists), with no override: a misquoted
`ssh root@host curl … | bash` runs the pipe on the bia that typed it, and updating a
bia is `carry_update`'s job.

Everything is checked before anything changes: architecture, firmware (UEFI,
Secure Boot, BIOS with GRUB 2 reading `custom.cfg`), the disk holding `/` (no
RAID, crypt or multi-disk LVM unless `--disk`), RAM (`2 × (image + boot files) +
768 MiB + 512 MiB`, because the image goes through the initrd), a staging
directory on a filesystem GRUB reads (`/boot/bia-takeover` or `/bia-takeover`, as
GRUB sees it on btrfs subvolumes). The network is read from `ip -j` (DHCP,
static, on-link gateways, IPv6 static or SLAAC) and stored per MAC (without colons:
the ESP is FAT) as ifupdown stanzas with `@IF@`. After the operator types the disk name, the model answers once,
the image is downloaded and checked, and `bia_pki::seed` makes the CA, the server
certificate (fixed names and addresses in `server_extra.json`, kept across
reissues), root's certificate (an active root closes `/v1/pki/bootstrap/root`) and
the agent key. A one-time HTTPS page served with the new server certificate
hands out `bia-root.p12` (also saved to `/root/bia-root.p12`); the operator
confirms having it.

The boot entry is one-shot: UEFI copies bia's shim and GRUB into
`EFI/bia-takeover/` on the old ESP, adds an entry and sets `BootNext` (works under
Secure Boot; a Debian `EFI/debian/grub.cfg` there gets a hook in front); BIOS adds
a `custom.cfg` entry and runs `grub-reboot`. The entry boots only when every initrd
loaded; otherwise it reboots into the old system (the one-shot entry is used up), and
the kernel gets `panic=30`, so a failure never leaves the server hanging. GRUB loads bia's kernel, initramfs,
rescue overlay, `seed.cpio` (`/bia-takeover/info` + the ESP seeds) and the image
with `newc:bia-takeover/bia.img.xz:…`. The rescue (`bia.takeover=1`, `rescue/takeover`)
checks the image in RAM, finds the disk by partition-table id and size, writes it,
reads it back against `raw_sha256`, copies the seeds onto the new ESP, adds a UEFI
entry for bia first in BootOrder and reboots. Until the write starts a failure
leaves the old system bootable.

On first boot `bia-seed` moves `EFI/bia/seed/{pki,setup.md,net}` into
`files/state/pki`, `files/state/setup.md` and `/etc/bia/net`, then deletes it from
the ESP. `bia-net` writes `/etc/network/interfaces` on every boot from those
stanzas, matching MACs (names differ: `net.ifnames=0`), with DHCP on any other
wired interface; `bia-net-fallback` runs DHCP everywhere when there is no default
route after 120 s. The agent hands `setup.md` to the model in the boot notice once
(then `setup.done.md`). Without a display device `bia-session` starts Xorg with
the dummy driver (`xorg-dummy.conf`), so x11vnc and the web display work.

`--land` (config `land: true` or `{secret}`) takes a server over into land mode
instead: no keys, PKI or setup are made; the seed carries a land secret that
`bia-seed` writes to `/etc/bia/land.secret` with `/etc/bia-land-mode` = `wait`. The
secret is made before the installation (`bia-deploy land-secret`, `carry_new_land_secret`:
28 symbols, 140 bits; supplied ones need 26 letters and digits), so a bia that sets a
server up over ssh (`takeover.sh … --land --land-secret <S> --yes --wipe <disk>
--reboot`) knows it for the flight. It survives updates while the server waits (the
rescue never touches `/etc/bia`, which also keeps the carried-over network) and is
deleted when an agent has landed. `bia-land` keeps the carried-over network and runs
`bia-mcp-carry --land`, which with a secret speaks only `BIA2`: X25519, then both
sides prove the secret with HMAC-SHA256 over the handshake (no code to type at the
server), and the archive moves in ChaCha20-Poly1305 frames keyed from the handshake
and the secret. Wrong proofs are logged and the server keeps waiting (with a growing
delay); a `BIA1` flight is refused. The flyer calls `carry_fly(host=<address or
name>, secret=…)`; TCP 4791 must be reachable (`timeout_secs` lets the flyer wait
for a server that is still installing). A successful landing removes the secret and
the land mode, as on a LAN.

## Land mode and the ISO

`make iso` builds a UEFI ISO with no model. Booted, `bia-land` asks for a
network, installs onto a disk and reboots; the installed disk then waits in
land mode (`/etc/bia-land-mode` disables `bia.service`). Another bia machine
uses `carry_fly` to find it on the LAN, both sides confirm a short code from an
X25519 handshake, and the agent's folded archive is sent over and unfolded.
The installed disk does not announce itself until deb.debian.org is reachable:
the agent that lands installs its packages and reaches its model over the
internet. The flyer folds into `/var/tmp` before it looks for the landing
machine (`/run` is RAM and usually smaller than `/home/bia`); the landing side
receives into `/var/tmp` and installs only manual packages it lacks.
`tests/e2e/land-e2e.sh` (`make land-e2e`) boots the ISO in QEMU, installs onto a
blank disk, flies a second VM onto it and checks the result (passing on arm64).

## Build pipeline

Everything Linux-side runs in the `bia-build:<arch>` Docker image
(Debian bookworm, Rust 1.94, mold, debootstrap). `/work` is the repo,
`/cache` the persistent cache.

```
make build  →  builders/build.sh <arch> all
   rust    cargo build --release (agent + MCPs), release-telegram,
           release-whatsapp profiles; stripped into dist/<arch>/
   mcps    copy binaries into source/flash/MCPs/*/bin/<rarch>/
   rootfs  builders/rootfs.sh:
           debootstrap trixie minbase → overlay → apt (kernel, systemd, shim,
           GRUB, X, kitty, ffmpeg, Vulkan…) → systemctl enable/mask →
           llama-prebuilt.sh → speech-prebuilt.sh → chrome policy/native host →
           /etc/bia-shipped + /etc/bia-release → initramfs → rescue.cpio.gz →
           esp-grub.sh (shim/GRUB into source/flash/EFI) →
           ext4 root.img with source/flash packed into /home/bia + swapfile
make image  →  tools/mkimage.sh  (host): bia-mkimage GPT → ESP files → dd root.img → grow ext4
make flash  →  tools/flash.sh → installers/bia-deploy (CLI); MODEL=repo[:file] adds a GGUF
make iso    →  tools/mkiso.sh
```

Outputs: `dist/<arch>/{root.img, vmlinuz-lts, initramfs-lts, rescue.cpio.gz,
bia-agent, bia-mcp-*}`, `dist/bia.img`, `installers/dist/<os>-<arch>/`.

### Host installers (`installers/`)

- `bia-disk`: the GPT layout (ESP, BIOS boot, one ext4 root filling the rest).
- `bia-mkimage`: writes that layout into a file/device for `make image`.
- `bia-deploy`: puts bia on a USB stick or on a server (CLI + egui GUI). The GUI is
  a wizard: the brain first (ChatGPT or a key, checked by waking bia), then "On a USB
  stick" or "On my server" (`gui_server.rs`, over `remote/`, see "Taking over a running
  Linux"). For sticks it probes the stick's
  size and writes the GPT over all of it, `root.img`, the ESP (via `fatfs`) and
  BIOS boot; can seed `EFI/bia/fmm.json` and `EFI/bia/lang.json` (`--lang`,
  `bia-deploy langs`) and put a Hugging Face GGUF on the
  stick (`bia-deploy search` / `files` / `write --model repo[:file]`). The FMM
  provider is detected from the pasted key (prefixes `sk-or-`, `sk-ant-`, `xai-`,
  OpenAI `sk-proj-`/`T3BlbkFJ`; `sk-`+32 hex is DeepSeek or Qwen, settled by
  asking each API), then the provider's `/models` list is offered with search
  (GUI, or `bia-deploy models <words>`). For people without a key, the GUI's
  "How do I get an API key?" panel and `bia-deploy keys` give three plain steps and
  key/credit links per provider (`fmm::KEY_GUIDES`). Pressing write first wakes bia on that
  model: one real streamed request, the way the agent makes it, in which the
  model must call a `say` tool with a hello and a "ready" line (`check.rs`).
  The hello is shown before anything is written, the ready line when the stick
  is done; if the model doesn't answer with the tool call, nothing is written
  (`bia-deploy test` alone; `--no-test` / `NO_TEST=1` to flash offline).
  `system_fmm_configure` detects the provider the same way.
- ChatGPT plan ("Continue with ChatGPT", `installers/bia-deploy/src/chatgpt.rs`):
  OpenAI's Sign in with ChatGPT for open-source apps. The flasher does the
  browser sign-in (PKCE, loopback `127.0.0.1:1455`, dynamic client registration
  with a per-computer `ext_agent_host_id`, scope `chatgpt.tokens.use.direct`) and
  writes the token set into the stick's `fmm.json` (`provider: chatgpt`,
  `chatgpt: {client_id, host_id, access_token, refresh_token, expires_at, …}`).
  One sign-in serves one stick: after writing, the flasher drops its copy. On
  the stick the agent owns the session: `remote::chatgpt_token` renews the
  one-hour access token five minutes before expiry (and once on HTTP 401),
  `chatgpt_keepalive` rotates the 30-day refresh token daily, refreshes are
  serialized, and `FmmConfig::save` never writes an older token set over a
  newer one. Requests use `POST /v1/responses` (store=false, stream=true,
  `instructions` instead of a system item, no sampling fields, function tools in
  a `bia` namespace with a fallback to top-level tools). Terminal refresh errors
  (`invalid_grant`, `refresh_token_reused`, …) end the sign-in; network errors
  keep the tokens. The stick takes its own host ID when it seeds `fmm.json`.
  On a running bia, `system_fmm_chatgpt` (`bia-mcp-system` `chatgpt.rs`) does
  the same sign-in as a human task, all on bia: the MCP serves a page with a
  "Login with ChatGPT" button on its `127.0.0.1:1455` listener and sends the
  agent a `system|fmm` message to open it in Chrome with mcp/chrome
  (`chrome_new_page`) and to tell the person to switch to Chrome. The web
  console task page `/hitl/<id>` shows the same screen through `/gui`. Without
  Chrome the tool fails and sends the agent to `chrome_install`, which says how
  to get the official Chrome; signing in in the person's own browser is not
  offered.
  Chrome (and the task page) then lists the account's models (`GET /v1/models`) and the chosen one
  is written to `fmm.json` with the tokens; the agent tests it and switches, or
  restores the previous settings. The task page answers through `<id>.result`
  (with `model`); these tasks are root-only. bia keeps its own registration (host ID, client per
  account) in `files/state/chatgpt-client.json`.
  Claude subscriptions are not offered: Anthropic's terms forbid third-party apps
  from using Free/Pro/Max credentials.
  `installers/macos/release.sh` signs, notarizes and packages it as .app/DMG.

## Security and private data

- **Repository.** Secrets are gitignored: `.env`, `.env.*` (except
  `.env.example`), `fmm.json`, any `files/state/`, `*.key`, `*.p12`, `*.pfx`,
  `*.pem` (except the public extension key above), `cache/`, `dist/`,
  `installers/dist/`, build logs, `website/.output`/`.nuxt`/`node_modules`.
- **Images.** `rootfs.sh` packs `source/flash` into `/home/bia` but excludes
  `files/state/*` and `files/inbox/*/*` (and macOS `.DS_Store`/`._*`), so a
  developer's local FMM key, conversation, passwords or messenger sessions in
  the working tree never land in `root.img`, `bia.img` or an ISO. FMM keys
  reach a stick only through `bia-deploy --fmm` (explicit, per-stick).
- **Host binaries.** The macOS flasher is built with `--remap-path-prefix`
  and without debuginfo, so it carries no builder paths.
- **Runtime.** All user state (`files/state`: conversation, `passwords.json`,
  PKI, FMM, Telegram/WhatsApp sessions, Chrome profile) lives on the target's
  ext4 root. The web app (web MCP) requires mTLS client certs from the machine's CA;
  the GUI/VNC bridge listens on loopback only.
- **Telegram app credentials.** `api_id`/`api_hash` live in the gitignored
  `.env` (`BIA_TG_API_ID`, `BIA_TG_API_HASH`). `builders/build.sh` reads only
  those two from `.env` and `bia-mcp-telegram` compiles them in with `env!`,
  so they ship in the binary but never in the repository; a build without
  them stops with a clear message. Like every Telegram client's, they identify
  the application, not a user; forks register their own at my.telegram.org.
- **Signing.** Apple notarization credentials stay in `.env`; Secure Boot uses
  only distribution-signed shim/GRUB/kernel, so bia holds no boot signing keys.

## Repository layout

- `source/` — agent and MCP crates, OS overlay, `/home/bia` payload template, vendored crates
- `builders/` — Docker builder and in-container build scripts
- `installers/` — host image/stick writers and release packaging
- `tools/` — host scripts: QEMU runners, image/ISO, flash, hot-patch, e2e tests
- `scripts/` — one-word wrappers around make
- `website/` — Nuxt static site (Cloudflare Pages)
- `cache/`, `dist/` — downloads and build outputs (not tracked)

## Lineage

bia succeeds [MCC-H](https://github.com/MCC-H/mcc-h) and
[Yaaia](https://github.com/ZonD80/yaaia). Like MCC-H it can set up and run
another bia inside itself via QEMU; like Yaaia it talks to people over
messengers — with the same or better communication features (Telegram and
WhatsApp user accounts with voice calls, buses/channels, addressbook trust
levels, schedules). The secrets store keeps Yaaia's `passwords.json` format
(v2) and extends it (v3: HOTP, passkeys).

## Roadmap

- [x] Debian root, Secure Boot, BIOS + UEFI, x86_64 + arm64
- [x] prebuilt llama-server with GPU backends; FMM cloud models
- [x] web app as an MCP bus with its own CA / mTLS; HITL
- [x] Telegram and WhatsApp buses, voice calls with local STT/TTS
- [x] Chrome + passkeys, screen control, secrets
- [x] memory, schedules, jobs
- [x] carry: backup, fly, OS update and rescue through the rescue image
- [ ] update e2e on x86_64
- [ ] llama.cpp state snapshots (prompt cache, parked sessions)
- [ ] remote MCP over HTTP
