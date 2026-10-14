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
 │    EFI/bia/fmm.json                     optional, seeded by bia-flash  │
 │    EFI/bia/models.txt                   until bia-grow moved the model │
 │ p2 BIABIOS  1 MiB BIOS boot (x86_64 legacy, GRUB core.img)             │
 │ p3 BIAROOT  ext4: Debian + /home/bia (model, MCPs, files) + swapfile   │
 │             spans the rest of the stick; ext4 grows into it at boot    │
 │             (BIADISK on an internal disk installed by bia-migrate)     │
 └───────────────────────────────────────────────────────────────────────┘
                 │ GRUB → kernel → initramfs → root=LABEL=…
                 ▼
 systemd ── bia-grow.service     (ext4 over the whole stick; moves in a flashed model)
        ├── bia-update.service   (finishes a pending OS update, unfolds agent)
        ├── bia.service ─ bia-session ─ Xorg :0 + openbox + tint2 + kitty
        │                                └── bia-agent  (console on tty1/serial/kitty)
        │                                     ├── llama-server (child, 127.0.0.1)
        │                                     ├── MCP children over stdio
        │                                     └── HTTPS web console :443 (mTLS)
        ├── bia-land.service     (ISO/"land" mode only, instead of bia.service)
        └── networking, getty@tty2, logind …
```

`root.img` is built once per architecture and is identical for the stick, the
QEMU image and the internal disk an install produces. `/home/bia` inside it
holds `bia.toml`, `model/`, `MCPs/` and `files/`. It never contains a model and
is only as large as its contents.

### Filling the stick and the model (`bia-grow`)

`bia-flash` makes BIAROOT span the whole stick but writes only `root.img` at its
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
`bia-flash` leaves (512 MiB), not room for a second copy, and an interrupted
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

## The agent (`source/crates/bia-agent`)

One Rust binary, dynamically linked against glibc. Main modules:

| Module | Role |
|---|---|
| `main.rs`, `sys.rs` | startup: `/home/bia` checks, memory tuning (swap, THP, mlock), web console, model, MCPs, resume, FMM |
| `console.rs` | writes to every kernel console at once; pinned 3-line status header (DECSTBM scroll region); input from all consoles |
| `repl.rs` | the loop: slash commands, chat turns, tool rounds, streaming, interruption, autosave |
| `tools.rs` | tool registry (built-ins + MCP), prompt rendering, tool-call parsing (Qwen/Hermes JSON and LFM2 pythonic) |
| `llama_srv.rs`, `llm.rs`, `tune.rs`, `gguf.rs` | supervise prebuilt `llama-server`; pick backend (CUDA → ROCm → SYCL → Vulkan → CPU), threads, ctx and KV from sysfs + GGUF header |
| `remote.rs` | FMM: streaming tool-calling client for chat completions (OpenRouter, OpenAI, xAI, DeepSeek, Qwen/DashScope), Anthropic Messages, and the Responses API for a ChatGPT plan (token refresh/rotation) |
| `mcp.rs` | MCP client over stdio, discovery of `/home/bia/MCPs/*/mcp.json`, `boot_tool` |
| `jobs.rs`, `bash.rs` | every MCP call and shell command is a job; results return as `system|job:<id>|…` envelopes; PTY shells as root or `bia` |
| `memory.rs` | append-only journal + time-ranged categorized reflections (`/reflect`) |
| `schedules.rs` | calendar of tasks/reminders, delivered as `system|schedule` with catch-up after boot |
| `state.rs` | `files/state/conversation.json` (fsynced, `.bak` fallback) |
| `buses.rs`, `addressbook.rs` | buses (console, web, telegram, whatsapp), channels, contacts with trust levels |
| `web.rs`, `webchat.rs`, `openapi.rs`, `hitl.rs` | HTTPS + mTLS web console, Telegram-style web chats, `/openapi.yaml`, human-in-the-loop tasks |
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
  `job:<id>`, `schedule`.
- `issued` / `delivered`: RFC 3339 UTC; lines typed during generation are
  queued, so the model can see how long a message waited.
- `id`: platform message id, or a local `b…` id.

The model replies with tool calls; `say(identity, channel, message)` sends to a
channel and returns at once. Plain text is the final answer for the console.

### Tools and discovery

Built-in tools: `bash`, `say`, `discover`, `set_context`, `job_*`/`list_jobs`,
`memory_*`, `schedule_*`, `addressbook_*`. The prompt starts with only a few;
`discover(q=…)` searches and `discover(path="mcp/<name>", enable=true)` loads
an MCP's function definitions as a `system` message, so the cached prompt
prefix stays valid. Keyword matches in a request can load a server too.
Loaded servers persist with the conversation.

### Web console

HTTPS on :443 (HTTP :80 redirects) inside the agent, rustls + ring. The CA,
server and client certificates come from `bia-pki` under
`files/state/pki/`. The CA is public at `/v1/pki/ca`; everything else needs a
client certificate. The first root certificate can be issued from the
onboarding page while none exists; later ones are issued by root (web
Settings, `pki_issue` or `/pki issue <id>`) and delivered once as a PKCS#12
pickup URL. Every .p12 bia hands out uses the password `12345678` (macOS will
not import one with an empty password). Renewal (`POST /v1/pki/renew`) takes a
CSR and returns PEM: no key is made here. The web console's Renew button makes
a P-256 key with WebCrypto (`web/pki.js`), sends the CSR and builds the .p12
in the browser. Both sides write the same modern PKCS#12 (`bia-pki/src/p12.rs`,
`web/pki.js`): PBES2 (PBKDF2-HMAC-SHA256, AES-256-CBC) key bag, SHA-256 MAC, no
RC2/3DES, so OpenSSL 3 opens it without `-legacy`.

Another agent without a certificate asks for one with a CSR for a key it made
itself: `POST /v1/pki/requests` `{identity, role, note, csr}` checks the CSR's
signature and key (RSA >= 2048, EC, Ed25519) and returns a random UUID and
`202`. The request is stored in `pki/requests/<uuid>.json`, shown on the console
and handed to the model as a system notice so it can ask root. Root decides with
`/pki approve|deny <id>`, in web Settings, or by telling bia (`pki_approve` /
`pki_deny` in the system MCP). Approval signs the CSR's public key with an
identity, role (`agent`), trust (`normal`) and usages set here; the private key
never exists on this machine. Deciding can take days, so the requester polls
`GET /v1/pki/requests/<uuid>` (`pending` with `Retry-After`, then `approved`,
`denied`, `issued` or `expired`) and downloads the PEM certificate + CA from
`…/<uuid>/cert`. Pending requests lapse after 3 days; at most 20 wait at once, 3
per address. Certificates root issues for people (`pki_issue`, pickup URL) and the first
root certificate are still made here as PKCS#12, since browsers import those.
`tools/peer-e2e.sh` (`make peer-e2e`) runs this between two QEMU agents.

The page (`web/index.html`, `app.js`, `app.css`, `term.js`, `emoji.js`) is a
Telegram-style app. `webchat.rs` keeps each identity's chats in
`files/state/web/<identity>/`: the main chat is channel `web`, others are
`web:<chat>`. Messages carry replies, forwards, attachments (uploads go to
`files/inbox/web/<identity>/`), reactions, edits, pins, read marks, inline
buttons and polls; user actions reach the model as envelopes with `[reply to …]`,
`[edited …]`, `[reaction] …`, `[button] …`, `[vote] …` markers, and the model
uses `say(reply_to, files, buttons, poll, edit)`, `react`,
`delete_messages`, `pin_message`, `close_poll` and `chat_history`. Streamed
answers show live as drafts. For root the app adds the live agent console
(`/ws/term`, the same byte stream as the screen, status header included), a
root shell on a PTY (`/ws/shell`), a file manager over the whole filesystem
(`/v1/fs/*`), human tasks (`/hitl`), the display (`/gui`, noVNC → x11vnc on
127.0.0.1:5900) and certificate admin. Discovery: `/.well-known/bia.json`,
`/openapi.yaml`, `/skills/web-console/SKILL.md`.

## MCP servers (`source/crates/bia-mcp-*`, `source/flash/MCPs/*`)

All speak MCP over stdio via the small `bia-mcp` crate. Its `share` module turns shared locations and contacts into the text the model reads (`[location] … map link`, `[contact] name, phones, e-mails`) and reads/writes vCards, the same for Telegram and WhatsApp. A live location reaches the model once; later points (Telegram message edits, WhatsApp update messages) only refresh `LiveTracker` (`files/state/<bus>-live.json`), `*_live_location` reads it, and `[live location ended]` arrives once when the sender stops, the time runs out, or updates stop for 20 min. Real edits by others arrive as `[edited <id>] sent …, edited …` with `was:` and `now:`; the old text comes from the WhatsApp history cache or Telegram's `TextMemory` (`files/state/telegram-texts.json`, last 5000 messages); hidden or no-change edits are dropped. The build copies each
binary into `source/flash/MCPs/<name>/bin/<arch>/`, which `rootfs.sh` packs
into `/home/bia/MCPs`.

| MCP | What it does |
|---|---|
| `system` | FMM configure/disable, web console link, human-in-the-loop, client-cert issue. Disks and network are plain Debian tools through `bash`. |
| `secrets` | passwords, TOTP/HOTP, WebAuthn passkeys (ES256) in `files/state/passwords.json`; 1Password CSV/JSON import, CXF export |
| `chrome` | Google Chrome on profile `~/.config/bia-chrome` with remote debugging; force-installed CRX extension proxies WebAuthn to `secrets`; page automation through Google's `chrome-devtools-mcp` (vendored Node) |
| `screen` | screenshot, mouse and keyboard on `:0` (xdotool/scrot) |
| `telegram` | a real Telegram user via ferogram; voice calls via tgcalls/ntgcalls; speech through `bia-speech`; locations, venues and contacts in and out (`send_location`, `send_contact`) |
| `whatsapp` | WhatsApp linked device via whatsapp-rust (QR pairing in a kitty window), voice calls via its VoIP plane; speech through `bia-speech`; locations and contacts in and out (`send_location`, `send_contact`) |
| `lang` | languages bia hears and speaks: catalog (`catalog.json`, sha256-pinned), on-demand installs into `files/speech`, voices, speech settings, transcribe/synthesize files |
| `carry` | `fold`/`unfold` backups of `/home/bia`; `fly` to a machine in land mode (UDP discovery, X25519 handshake, confirmed code); `update` and `rescue` (see below); `morph` |

### Speech

`bia-speech` (a library shared by `telegram`, `whatsapp` and `lang`) dlopens the
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
`tools/update-e2e.sh` (`make update-e2e`) tests disk, flash and rescue cases
in QEMU (passing on arm64).

Legacy BIOS has no BootNext, and a stick plugged in for the update usually
boots first. So on a BIOS boot `bia-update` writes the same marker on the other
bia ESP as well (with that ESP's own rescue image, which matches its kernel):
the rescue runs whichever drive the firmware starts, and removes both markers.
When it ran from a drive that is not the destination (an update of the disk
started from the stick, or `/rescue`), it powers off and says to remove the
stick, since a reboot would start the stick again. BIOS GRUB (MBR + core.img)
is not changed by updates; its modules on the ESP stay with it.
`tools/bios-update-e2e.py` (`make bios-update-e2e`, x86_64 SeaBIOS) runs all
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
`bia-flash` does this natively (`bia_disk::embed_bios_grub`); `mkimage.sh` and
`bia-migrate` use `grub-bios-embed.py`. `tools/bios-e2e.sh` (`make bios-e2e`)
tests the whole path in QEMU (SeaBIOS, x86_64) with a stick written by
`bia-flash`.
`bia-migrate to-flash` goes the other way. Later updates between the two use
the rescue image above.

## Releases and the OS download

The OS ships as one minimal disk image per architecture, `bia-<arch>.img.xz`
(`tools/mkimg.sh` → `bia-flash image`): GPT, ESP (shim, GRUB, kernel,
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

`bia-flash write` reads the same image (`--os-image`, or downloaded) front to
back, xz-decoded on the fly: it writes a GPT for the whole stick (same
partition starts as the image's), keeps the image's MBR boot code, copies the
BIOS boot partition and BIAROOT, streams a Hugging Face model onto the end of
BIAROOT, checks the image's SHA-256, and writes the ESP last with `fmm.json` and
`models.txt` added. A damaged download therefore leaves the stick unbootable,
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

`bia-flash` (`src/release.rs`) reads `bia-os.json` from the latest release
(or `--os <tag>`), downloads the `.img.xz` with resume and checks it, and keeps
it in its cache (`~/Library/Caches/bia-flash`, `%LOCALAPPDATA%\bia-flash\cache`,
`~/.cache/bia-flash`) so the next stick needs no download. Local builds give
`--os-image dist/<arch>/bia-<arch>.img` (the GUI's advanced field reads
`dist/<arch>/local-files.json`, written by `make build`).

`tools/img-e2e.sh` (`make img-e2e`) tests both paths in QEMU: `MODE=flash`
serves a local release over HTTP and has `bia-flash` write it with keys and a
real Hugging Face model; `MODE=dd` decodes the `.img.xz` onto a bigger disk
and adds the keys by hand. `FIRMWARE=bios` boots x86_64 under SeaBIOS.

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
`tools/land-e2e.sh` (`make land-e2e`) boots the ISO in QEMU, installs onto a
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
make flash  →  tools/flash.sh → installers/bia-flash (CLI); MODEL=repo[:file] adds a GGUF
make iso    →  tools/mkiso.sh
```

Outputs: `dist/<arch>/{root.img, vmlinuz-lts, initramfs-lts, rescue.cpio.gz,
bia-agent, bia-mcp-*}`, `dist/bia.img`, `installers/dist/<os>-<arch>/`.

### Host installers (`installers/`)

- `bia-disk`: the GPT layout (ESP, BIOS boot, one ext4 root filling the rest).
- `bia-mkimage`: writes that layout into a file/device for `make image`.
- `bia-flash`: cross-platform USB writer (CLI + egui GUI). Probes the stick's
  size and writes the GPT over all of it, `root.img`, the ESP (via `fatfs`) and
  BIOS boot; can seed `EFI/bia/fmm.json` and put a Hugging Face GGUF on the
  stick (`bia-flash search` / `files` / `write --model repo[:file]`). The FMM
  provider is detected from the pasted key (prefixes `sk-or-`, `sk-ant-`, `xai-`,
  OpenAI `sk-proj-`/`T3BlbkFJ`; `sk-`+32 hex is DeepSeek or Qwen, settled by
  asking each API), then the provider's `/models` list is offered with search
  (GUI, or `bia-flash models <words>`). For people without a key, the GUI's
  "How do I get an API key?" panel and `bia-flash keys` give three plain steps and
  key/credit links per provider (`fmm::KEY_GUIDES`). Pressing write first wakes bia on that
  model: one real streamed request, the way the agent makes it, in which the
  model must call a `say` tool with a hello and a "ready" line (`check.rs`).
  The hello is shown before anything is written, the ready line when the stick
  is done; if the model doesn't answer with the tool call, nothing is written
  (`bia-flash test` alone; `--no-test` / `NO_TEST=1` to flash offline).
  `system_fmm_configure` detects the provider the same way.
- ChatGPT plan ("Continue with ChatGPT", `installers/bia-flash/src/chatgpt.rs`):
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
  reach a stick only through `bia-flash --fmm` (explicit, per-stick).
- **Host binaries.** The macOS flasher is built with `--remap-path-prefix`
  and without debuginfo, so it carries no builder paths.
- **Runtime.** All user state (`files/state`: conversation, `passwords.json`,
  PKI, FMM, Telegram/WhatsApp sessions, Chrome profile) lives on the target's
  ext4 root. The web console requires mTLS client certs from the agent's CA;
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
- [x] web console with agent CA / mTLS; HITL
- [x] Telegram and WhatsApp buses, voice calls with local STT/TTS
- [x] Chrome + passkeys, screen control, secrets
- [x] memory, schedules, jobs
- [x] carry: backup, fly, OS update and rescue through the rescue image
- [ ] update e2e on x86_64
- [ ] llama.cpp state snapshots (prompt cache, parked sessions)
- [ ] remote MCP over HTTP
