# bia — boot straight into an agent

bia turns a USB stick (or a disk, or a VM image) into a computer whose whole
user interface is an LLM agent. Plug it in, boot, and the first thing on the
screen is a prompt that talks to an LLM (a cloud model, or optionally one
running on that machine). The agent
owns the box: it can run shell commands, manage disks and the network, drive
Chrome, see and click the desktop, keep passwords and passkeys, answer you on
Telegram or WhatsApp (including voice calls), Messenger or Instagram, schedule work, remember things,
and update or move itself to another machine.

- **Cloud brain.** Frontier Model Mode (FMM) runs the agent on a cloud model
  (OpenRouter, OpenAI, Anthropic, Gemini, xAI, DeepSeek or Qwen) with your own key,
  or your ChatGPT Plus/Pro plan through "Continue with ChatGPT" in the flasher.
  At the moment the cheapest option is DeepSeek V4.1.
- **Local model, optionally.** A GGUF model can run through llama.cpp on the
  machine itself (CPU, or CUDA / ROCm / SYCL / Vulkan when a GPU is there).
  Weights are mmap'd, so RAM + SSD act as one pool.
- **One Debian system.** Debian *trixie* minbase + systemd, Secure Boot via the
  stock Microsoft-signed shim and Debian-signed GRUB, x86_64 and arm64, UEFI
  and legacy BIOS (x86_64).
- **Tools are MCP servers.** Each capability is a drop-in folder with an
  `mcp.json` and a binary; the model discovers and loads them on demand.

See [ARCHITECTURE.md](ARCHITECTURE.md) for how it works inside.

### Get bia

- **Computer (Mac, Windows, Linux):** bia deploy writes a USB stick or puts bia on a
  server: [latest release](https://github.com/ZonD80/bia/releases/latest), or
  [ownbia.com/download](https://ownbia.com/download).
- **Android:** bia deploy puts bia on a server from the phone:
  [Google Play](https://play.google.com/store/apps/details?id=biadeploy.android), or the
  [APK](https://github.com/ZonD80/bia/releases/latest/download/bia-deploy-android.apk) from the latest release.
- **iPhone:** coming soon to the App Store.
- **A Linux server over ssh:** `curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | sudo bash`

### Lineage

bia is the successor of two earlier projects:

- **[MCC-H](https://github.com/MCC-H/mcc-h)** — bia can set up and run
  another bia inside itself via QEMU, so one agent can build, boot and drive
  a whole second machine.
- **[Yaaia](https://github.com/ZonD80/yaaia)** — bia has the same (and in
  places better) communication features: Telegram and WhatsApp as a real
  user, voice calls, contacts with trust levels, channels, schedules, and a
  Yaaia-compatible `passwords.json`.

## Concept

A normal OS boots into a shell or a desktop and you start programs. bia boots
into a single long-running conversation. The agent (`bia-agent`) is a systemd
service on tty1 (and every serial console). Every
input — your typing, a Telegram or web chat message, a calendar reminder, a finished
background job, a boot notice — reaches the model as one line:

```
identity|channel|issued|delivered|id|message
root|console|2026-10-09T10:00:00Z|2026-10-09T10:00:02Z|b17|which disks are here?
```

The model answers by calling tools (`bash`, `say`, `discover`, MCP functions…);
plain text is the final answer, printed on the console. The conversation is
saved after every turn and resumed after a reboot, so the machine carries on
where it left off.

## Repository layout

```
.
├── Cargo.toml / Cargo.lock    Rust workspace (agent, MCPs, installers)
├── Makefile                   every build/run entry point (see below)
├── ARCHITECTURE.md            design notes
├── fmm.json.example           template for local FMM keys  (copy → fmm.json, gitignored)
├── .env.example               Apple codesign/notarize vars (copy → .env, gitignored)
│
├── source/
│   ├── crates/
│   │   ├── bia-agent/         the agent: console UI, REPL, tool registry, llama-server
│   │   ├── bia-bridge/        client for other agents: cert request, chat link daemon, CLI + MCP
│   │   │                      supervisor, FMM client, memory, schedules
│   │   ├── bia-mcp/           tiny MCP-over-stdio server library used by every MCP
│   │   ├── bia-pki/           agent-owned CA, server/client certs, CSR requests, PKCS#12 pickup
│   │   ├── bia-mcp-system/    FMM config, human-in-the-loop, file links
│   │   ├── bia-mcp-web/       the web bus: HTTPS + mTLS app (chats, calls, files, shell, display), PKI
│   │   ├── bia-mcp-secrets/   passwords, TOTP/HOTP, WebAuthn passkeys, 1Password import
│   │   ├── bia-mcp-chrome/    Google Chrome lifecycle, passkey extension, devtools MCP
│   │   ├── bia-mcp-screen/    screenshot + mouse/keyboard on display :0
│   │   ├── bia-mcp-telegram/  Telegram user account (ferogram) + voice calls (tgcalls)
│   │   ├── bia-mcp-whatsapp/  WhatsApp linked device (whatsapp-rust) + voice calls
│   │   ├── bia-mcp-meta/      Messenger and Instagram DMs (two MCPs) in front of bia-meta
│   ├── go/bia-meta/           Go helper on mautrix-meta: messagix, whatsmeow (encrypted chats), instameow
│   │   ├── bia-mcp-carry/     backup (fold/unfold), machine-to-machine "fly", OS update
│   │   ├── bia-mcp-lang/      languages heard/spoken: catalog, verified installs, settings
│   │   ├── bia-speech/        shared on-device speech: VAD, language ID, Whisper, voices
│   │   └── listen.rs          shared "who owns this port" helper
│   ├── flash/                 template packed into /home/bia on the target
│   │   ├── bia.toml           agent configuration (model, sampling, prompt, web)
│   │   ├── MCPs/<name>/       mcp.json (+ bin/<arch>/ filled by the build)
│   │   ├── model/             *.gguf (not in git)
│   │   └── files/             agent data (state/ and inbox/ are never shipped)
│   ├── os/overlay/            files laid over the Debian root: systemd units,
│   │                          bia-session (X + kitty), bia-update, bia-land,
│   │                          bia-migrate, rescue image scripts, initramfs config
│   └── vendor/                patched tgcalls / ntgcalls crates
│
├── builders/                  run inside the Docker build container
│   ├── Dockerfile             Debian bookworm + Rust 1.94 + debootstrap
│   ├── build.sh               rust | mcps | rootfs | all
│   ├── rootfs.sh              debootstrap → overlay → apt → initramfs → root.img
│   ├── llama-prebuilt.sh      official llama.cpp llama-server builds per backend
│   ├── speech-prebuilt.sh     sherpa-onnx, Whisper tiny, Silero VAD, GTCRN, Piper, ntgcalls
│   └── rescue.sh              rescue.cpio.gz overlay used for offline OS updates
│
├── installers/
│   ├── bia-disk/              GPT layout library (ESP + BIOS boot + ext4 root)
│   ├── bia-mkimage/           host tool: write that layout into an image
│   ├── bia-deploy/            USB stick writer and server takeover over SSH, CLI + egui GUI
│   ├── macos/                 codesign / notarize / DMG release script
│   └── dist/<os>-<arch>/      built installer binaries (not in git)
│
├── tools/                     host-side scripts: QEMU runners, image/ISO builders,
│                              flash writer, MCP hot-patching, e2e update test,
│                              serial / QEMU-monitor drivers, mock LLM
├── scripts/                   one-word wrappers (build_and_run_from_flash, make_iso…)
├── website/                   Nuxt 4 static site, deployed to Cloudflare Pages
├── cache/                     downloads, cargo cache, build trees  (not in git)
└── dist/                      build outputs: <arch>/root.img, <arch>/bia-<arch>.img, bia.img, ISOs  (not in git)
```

## Requirements

- macOS on Apple Silicon is the reference host (Linux hosts work for most
  targets). Docker Desktop (or another Docker engine) with `linux/arm64` and
  `linux/amd64` platforms.
- QEMU with EDK2 firmware for `make run`.
- A host Rust toolchain for `bia-mkimage` / `bia-deploy`.
- For local inference, a GGUF from Hugging Face picked when flashing
  (`MODEL=repo[:file.gguf] make flash`, or the flasher GUI); otherwise an FMM
  key. Images never contain a model.

## Build

```bash
make builder                 # once per ARCH: the Debian build container
make build                   # agent + MCPs + Debian root  -> dist/arm64/root.img + bia-arm64.img
make img                     # just the minimal disk image dist/<arch>/bia-<arch>.img (Etcher/dd-able)
ARCH=x86_64 make builder build
make image                   # dist/bia.img (16 GiB QEMU flash image) from dist/<arch>
make run                     # boot it in QEMU (arm64 + hvf on Apple Silicon)
make run-serial              # same, headless, serial console in this terminal
make flash                   # write a USB stick with bia-deploy (x86_64 by default)
MODEL=unsloth/Qwen3-4B-Instruct-2507-GGUF make flash   # … plus a GGUF from Hugging Face
make iso                     # installer ISO without a model (x86_64 by default)
MODE=dd make img-e2e         # QEMU test of the image (MODE=flash|dd, FIRMWARE=uefi|bios, ARCH)
NOTARIZE=0 make flasher      # bia-deploy CLI + GUI; on macOS also .app / DMG
tools/make-icons.sh          # website favicon/icons + flasher .icns/.ico/window icon from brand/potato.svg (macOS, needs Chrome)
tools/release.sh             # GitHub release v<version>: builds only what is missing (OS per arch, flashers), notarizes, uploads
make update-e2e              # QEMU end-to-end test of the rescue-image OS update
make land-e2e                # QEMU end-to-end test: land ISO install, then fly/land
make peer-e2e                # QEMU end-to-end test: two agents set up trust, then talk over the live peer link
make bios-e2e                # QEMU end-to-end test on legacy BIOS: boot the stick, install to disk
make bios-update-e2e         # /update and /rescue through the rescue image on legacy BIOS
make patch-mcp MCP=chrome    # rebuild one MCP and hot-patch existing images
```

`make build` runs in stages that can be called on their own:
`make rust`, `make mcps`, `make rootfs`.

### Secrets during development

Two local files are gitignored and never committed:

| File | Used by | Contents |
|---|---|---|
| `fmm.json` (from `fmm.json.example`) | `make flash`, `bia-deploy --fmm` | cloud model provider, API key, model |
| `.env` (from `.env.example`) | `make flasher` on macOS; `make build` | Apple ID / app password for notarization; Telegram `BIA_TG_API_ID` / `BIA_TG_API_HASH` (required for the telegram MCP, compiled into it) |

FMM keys are written onto the stick's ESP (`EFI/bia/fmm.json`) only when you
flash with them; they are never baked into `root.img`, `bia.img` or an ISO.
Runtime data the agent creates (`files/state/`: conversation, passwords,
Telegram/WhatsApp sessions, PKI) lives on the target machine only.

## Using it

The top of the screen is a status header (version, arch, model, RAM/swap/
context meters, tokens/s, what the agent is doing). Your input is green after
`->`, the agent's answer cyan after `<-`, tool calls yellow.

```
-> /help                               list console commands
-> which disks are here?               bash: lsblk
-> use openrouter with key sk-or-…     system_fmm_configure
-> use my ChatGPT Plus subscription     system_fmm_chatgpt (Login with ChatGPT page)
-> log into my telegram, +34…          telegram_login → telegram_submit_code
-> every morning at 8 send me the news schedule_create
-> /update                             OS update through the rescue image
```

Press Enter on an empty line to interrupt the model. The same session is
available in a browser at `https://<machine-ip>/` with a client certificate
the agent issues (`/pki issue`).

Other agents (Claude Code, Claude Desktop, Cursor, scripts) talk to a bia with
`bia-bridge` (release assets for macOS, Linux, Windows; `source/crates/bia-bridge`):
`bia-bridge connect https://<machine-ip> --identity <name>`, the owner approves, then
`bia-bridge send` / `bia-bridge wait`, or `claude mcp add bia -- bia-bridge mcp`.

## Adding an MCP

Create `source/flash/MCPs/<name>/mcp.json`:

```json
{ "name": "<name>", "version": "1.0", "description": "what it is for (shown to the model)",
  "command": { "aarch64": "bin/aarch64/<binary>", "x86_64": "bin/x86_64/<binary>" },
  "keywords": ["words", "that", "load", "it"],
  "examples": ["root|console|do x -> tool(arg='v')"],
  "boot_tool": "restore" }
```

and put the binaries under `bin/<arch>/`. The agent starts it over stdio
(standard MCP: `initialize`, `tools/list`, `tools/call`) and exposes its tools
as `<name>_<tool>`. Rust MCPs can use the `bia-mcp` crate.

## Status

Working: x86_64 and arm64 builds, UEFI + Secure Boot (shim), legacy BIOS on
x86_64, local llama.cpp with GPU backends, FMM, web app (an MCP bus) with mTLS,
Telegram and WhatsApp (text + voice), Messenger and Instagram DMs, Chrome + passkeys, screen control,
secrets, memory, schedules, backup / fly / OS update through the rescue image
(e2e-tested on arm64).

Open: x86_64 run of the update e2e suite, llama.cpp state snapshots, remote
MCP over HTTP.

## License

GPL-3.0 — see [LICENSE.md](LICENSE.md). Copyright © 2026 ZonD80 aka Aleksei
Borodin. Third-party projects are listed in
[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md).
