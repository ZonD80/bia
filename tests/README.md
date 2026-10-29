# bia tests

`tests/run-all.sh` runs everything below and prints a summary (logs in `dist/test-logs/`).

| Stage | Test | What it proves |
|---|---|---|
| unit | `cargo test --workspace` | agent (web/console separation, reply targets, release notes, outbox, …), web MCP, PKI, MCPs |
| integration | `integration/web-mcp-e2e.py` | the web MCP binary over stdio + HTTPS + mTLS: first-visit root certificate, a chat both ways, identity rules (never `bia`), no console stream |
| integration | `integration/peer-link-e2e.py` | live peer links between two (and a copied third) web MCPs on localhost: trust in one approval, the link opening by itself, messages with the request chain and replies, a file and a resumed one, offline sends refused, disconnect and reconnect, bans (root's outranks the model's), bye with the reason on SIGTERM, a killed peer reported lost after 30 s, a copy with the same key refused, peer certificates good only for the link |
| integration | `integration/bridge-e2e.py` | bia-bridge against a real web MCP: key made locally and request filed, the daemon collects the certificate after approval (owner's message shown), messages and files both ways, edits, catch-up after the daemon was stopped, reconnect after bia restarts, MCP mode (send / wait / history), a denied request, several profiles |
| integration | `integration/ignore-e2e.py` | ignored chats through the messenger MCP with a fake bia-meta: ignored messages flagged and left unread; a wake pattern, a reply to bia and (with a voice note and speech models) a wake word spoken in a voice message deliver and mark read |
| integration | `integration/update-net-test.sh` | `bia-update`'s internet path in a container: manifest, download, sha256, unpack, version rules, refusals |
| e2e | `e2e/update-e2e.sh all internet disk flash rescue` | offline OS update through the rescue image, from a downloaded release, a flash stick or the disk; the agent survives |
| e2e | `e2e/peer-e2e.sh all` | two QEMU machines: bia-b asks bia-a's web API for a certificate, root approves, bia-b chats with bia-a over mTLS and renews (also the browser renewal path with `web/pki.js`); then the two bias set up trust, the peer link opens by itself, they pass a code word over it, a `/reboot` reaches the other side as `bye` at once and a hard reset is reported lost after ~30 s |
| e2e | `e2e/land-e2e.sh all` | land ISO install and carry fly/land between two machines |
| e2e | `e2e/img-e2e.sh all` | the release disk image boots, grows and runs (UEFI; `FIRMWARE=bios` on x86_64) |
| bios | `e2e/bios-e2e.sh all`, `e2e/bios-update-e2e.py` | legacy BIOS (SeaBIOS): install onto the disk, update and rescue |

Before the QEMU stages: `make build img iso ARCH=<arch>` (bios needs `ARCH=x86_64`).
`./fmm.json` (gitignored) gives the test agents a cloud model; never the slow local one.

Also here: `e2e/qemu-update-test.sh` (the same update by hand, `make run-update-*`),
`manual/telegram-smoke.sh` (a checklist to run inside a guest with a real Telegram account),
`mocks/mock-llm.py` (an OpenAI/Anthropic-compatible fake model for FMM without keys).

Build, image, QEMU and release tools live in `../tools/`.
