# Acknowledgements

bia stands on a lot of other people's work. Thank you to everyone behind
these projects. Each keeps its own license; binaries and images built from
this repository redistribute some of them (noted below) under their terms.

## Predecessors

bia is the successor of [MCC-H](https://github.com/MCC-H/mcc-h) and
[Yaaia](https://github.com/ZonD80/yaaia). It inherits MCC-H's idea of an agent
that sets up and runs a machine (bia can run another bia inside itself via
QEMU) and Yaaia's communication features (messengers, calls, contacts,
`passwords.json`).

## Operating system and boot

| Project | Used for |
|---|---|
| [Linux](https://kernel.org/) | the kernel (Debian `linux-image-*` package) |
| [Debian](https://www.debian.org/) (trixie) | the root filesystem, packages, signed kernel, `debootstrap`, build container (bookworm) |
| [systemd](https://systemd.io/) | init, services, logind |
| [shim](https://github.com/rhboot/shim) | Microsoft-signed first-stage loader for Secure Boot |
| [GNU GRUB](https://www.gnu.org/software/grub/) | UEFI and legacy BIOS boot loader, rescue-image hook |
| [initramfs-tools](https://salsa.debian.org/kernel-team/initramfs-tools) | initramfs, root-selection hooks |
| [e2fsprogs](https://e2fsprogs.sourceforge.net/), [dosfstools](https://github.com/dosfstools/dosfstools), [exfatprogs](https://github.com/exfatprogs/exfatprogs), [ntfs-3g](https://github.com/tuxera/ntfs-3g) | filesystems on sticks and disks |
| [rsync](https://rsync.samba.org/) | offline OS update and install copies |
| [efibootmgr](https://github.com/rhboot/efibootmgr) | BootNext handling for updates |
| [ifupdown](https://salsa.debian.org/debian/ifupdown), [dhcpcd](https://github.com/NetworkConfiguration/dhcpcd), [iproute2](https://wiki.linuxfoundation.org/networking/iproute2), [wpa_supplicant](https://w1.fi/wpa_supplicant/), [iw](https://wireless.wiki.kernel.org/en/users/documentation/iw) | networking and Wi-Fi |
| [Mesa](https://www.mesa3d.org/) / Vulkan loader, [oneAPI Level Zero](https://github.com/oneapi-src/level-zero) | GPU access for inference |
| [ALSA](https://www.alsa-project.org/) | audio |

## Desktop and remote display

| Project | Used for |
|---|---|
| [X.Org Server](https://www.x.org/) | display `:0` |
| [Openbox](http://openbox.org/) | window manager |
| [tint2](https://gitlab.com/o9000/tint2) | window switcher bar |
| [kitty](https://sw.kovidgoyal.net/kitty/) | terminal hosting the agent console |
| [x11vnc](https://github.com/LibVNC/x11vnc), [noVNC](https://novnc.com/) | remote view of the desktop from the web console |
| [xdotool](https://github.com/jordansissel/xdotool), [scrot](https://github.com/resurrecting-open-source-projects/scrot) | screen MCP input and screenshots |
| [DejaVu fonts](https://dejavu-fonts.github.io/), [Noto Color Emoji](https://github.com/googlefonts/noto-emoji) | fonts |
| [Google Chrome](https://www.google.com/chrome/) | browser driven by the chrome MCP (installed on the target on request) |

## AI, speech and media

| Project | Used for |
|---|---|
| [llama.cpp](https://github.com/ggml-org/llama.cpp) / [ggml](https://github.com/ggml-org/ggml) | local inference; official prebuilt `llama-server` (CPU, CUDA, ROCm, SYCL, Vulkan) |
| [GGUF](https://github.com/ggml-org/ggml/blob/master/docs/gguf.md) | model file format (header parsed by the agent) |
| [Qwen3](https://github.com/QwenLM/Qwen3) (Qwen3-4B-Instruct-2507) | default local model in `bia.toml` |
| [Liquid AI LFM2 / LFM2.5](https://www.liquid.ai/) | supported model family (pythonic tool-call format) |
| [OpenRouter](https://openrouter.ai/), [OpenAI API](https://platform.openai.com/), [Anthropic API](https://docs.anthropic.com/) | Frontier Model Mode providers |
| [Model Context Protocol](https://modelcontextprotocol.io/) | tool server protocol for every MCP |
| [chrome-devtools-mcp](https://github.com/ChromeDevTools/chrome-devtools-mcp) | page automation inside the chrome MCP |
| [Node.js](https://nodejs.org/) | runtime for chrome-devtools-mcp |
| [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) | on-device speech-to-text, text-to-speech, VAD, language identification, denoising |
| [Whisper](https://github.com/openai/whisper) (tiny bundled; base/small on demand) | speech recognition and spoken language identification |
| [Piper](https://github.com/rhasspy/piper) (en_US-lessac-medium bundled; other voices on demand) | speech synthesis voices |
| [Kokoro](https://huggingface.co/hexgrad/Kokoro-82M) (optional) | multilingual speech synthesis voice |
| [Silero VAD](https://github.com/snakers4/silero-vad) | voice activity detection |
| [GTCRN](https://github.com/Xiaobin-Rong/gtcrn) | speech denoising |
| [WeSpeaker](https://github.com/wenet-e2e/wespeaker) (CAM++, optional) | speaker embedding for the opt-in speaker check |
| [FFmpeg](https://ffmpeg.org/), [Opus](https://opus-codec.org/) | call audio encoding / decoding |

## Messaging

| Project | Used for |
|---|---|
| [ferogram](https://crates.io/crates/ferogram) | Telegram MTProto user client |
| [tgcalls](https://github.com/ankit-chaubey/tgcalls) | Telegram voice calls (vendored with a patch in `source/vendor/tgcalls`) |
| [NTgCalls](https://github.com/pytgcalls/ntgcalls) | native call media library, LGPL-3.0 (vendored binding in `source/vendor/ntgcalls`, `libntgcalls.so` shipped) |
| [whatsapp-rust](https://crates.io/crates/whatsapp-rust) | WhatsApp linked-device client and VoIP |
| [mautrix-meta](https://github.com/mautrix/meta) (AGPL-3.0) | Messenger and Instagram protocols in `bia-meta`: messagix, instameow, media download; bia's message handling follows its connectors |
| [whatsmeow](https://github.com/tulir/whatsmeow) (MPL-2.0) | Messenger's end-to-end encrypted chats (as a registered device) |
| [modernc.org/sqlite](https://gitlab.com/cznic/sqlite) | pure-Go SQLite for `bia-meta`'s keys and chat index |
| [fbchat-v2](https://github.com/m008v/fbchat-v2) (Apache-2.0) | studied for Messenger's E2EE media and reconnect handling |

## Go

[Go](https://go.dev/) builds `bia-meta` (toolchain fetched and checksum-checked by
`builders/go-env.sh`). Direct dependencies besides mautrix-meta and whatsmeow:
[zerolog](https://github.com/rs/zerolog), [coder/websocket](https://github.com/coder/websocket),
[google/uuid](https://github.com/google/uuid), [golang.org/x/image](https://pkg.go.dev/golang.org/x/image),
[protobuf](https://pkg.go.dev/google.golang.org/protobuf). The full tree is in `source/go/bia-meta/go.sum`.

## Rust ecosystem

Rust ([rust-lang.org](https://www.rust-lang.org/)) and Cargo build everything
in `source/crates` and `installers`. Direct dependencies include:

- [serde](https://serde.rs/) / [serde_json](https://github.com/serde-rs/json), [toml](https://github.com/toml-rs/toml), [anyhow](https://github.com/dtolnay/anyhow), [regex](https://github.com/rust-lang/regex), [libc](https://github.com/rust-lang/libc)
- [tokio](https://tokio.rs/), [tracing](https://github.com/tokio-rs/tracing), [log](https://github.com/rust-lang/log)
- [rustls](https://github.com/rustls/rustls), [ring](https://github.com/briansmith/ring), [rcgen](https://github.com/rustls/rcgen), [x509-parser](https://github.com/rusticata/x509-parser), [p12](https://crates.io/crates/p12), [rustls-pemfile](https://github.com/rustls/pemfile), [time](https://github.com/time-rs/time)
- [ureq](https://github.com/algesten/ureq) — HTTP client for FMM and downloads
- RustCrypto: [sha1](https://github.com/RustCrypto/hashes), [sha2](https://github.com/RustCrypto/hashes), [hmac](https://github.com/RustCrypto/MACs), [p256](https://github.com/RustCrypto/elliptic-curves), [rsa](https://github.com/RustCrypto/RSA); [sha1_smol](https://github.com/mitsuhiko/sha1-smol), [rand](https://github.com/rust-random/rand), [base64](https://github.com/marshallpierce/rust-base64)
- [qrcode](https://github.com/kennytm/qrcode-rust), [image](https://github.com/image-rs/image) — WhatsApp pairing QR
- [gpt](https://github.com/Quyzi/gpt), [uuid](https://github.com/uuid-rs/uuid), [fatfs](https://github.com/rafalh/rust-fatfs), [walkdir](https://github.com/BurntSushi/walkdir), [clap](https://github.com/clap-rs/clap) — image and stick writers
- [egui / eframe](https://github.com/emilk/egui) — bia-deploy GUI
- [mold](https://github.com/rui314/mold) — linker in the build container

The full dependency tree with versions is in `Cargo.lock`.

## Development and website

| Project | Used for |
|---|---|
| [Docker](https://www.docker.com/) | reproducible Linux build container on macOS |
| [QEMU](https://www.qemu.org/) + [EDK2 / OVMF](https://github.com/tianocore/edk2) | running and end-to-end testing images |
| [Python](https://www.python.org/) | test drivers (serial, QEMU monitor, mock LLM) |
| [Nuxt](https://nuxt.com/) / [Vue](https://vuejs.org/) | project website |
| [Cloudflare Pages](https://pages.cloudflare.com/) / [Wrangler](https://developers.cloudflare.com/workers/wrangler/) | website hosting and deploys |

## Compatibility credits

- `passwords.json` follows the [Yaaia](https://github.com/ZonD80/yaaia) v2 format, extended with HOTP and passkeys.
- 1Password CSV / JSON / CXF import and export for the secrets MCP.
- WebAuthn / FIDO2 software passkeys follow the [W3C WebAuthn](https://www.w3.org/TR/webauthn-2/) spec; OTP follows RFC 4226 / RFC 6238.
