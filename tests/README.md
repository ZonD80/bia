# bia tests

`tests/run-all.sh` runs everything below and prints a summary (logs in `dist/test-logs/`).

| Stage | Test | What it proves |
|---|---|---|
| unit | `cargo test --workspace` | agent (web/console separation, reply targets, release notes, outbox, …), web MCP, PKI, MCPs |
| integration | `integration/web-mcp-e2e.py` | the web MCP binary over stdio + HTTPS + mTLS: first-visit root certificate, a chat both ways, identity rules (never `bia`), no console stream |
| integration | `integration/peer-link-e2e.py` | live peer links between two (and a copied third) web MCPs on localhost: trust in one approval, the link opening by itself, messages with the request chain and replies, a file and a resumed one, offline sends refused, disconnect and reconnect, bans (root's outranks the model's), bye with the reason on SIGTERM, a killed peer reported lost after 30 s, a copy with the same key refused, peer certificates good only for the link |
| integration | `integration/bridge-e2e.py` | bia-bridge against a real web MCP: key made locally and request filed, the daemon collects the certificate after approval (owner's message shown), messages and files both ways, edits, catch-up after the daemon was stopped, reconnect after bia restarts, MCP mode (send / wait / history), a denied request, several profiles |
| integration | `integration/ignore-e2e.py` | ignored chats through the messenger MCP with a fake bia-meta: ignored messages flagged and left unread; a wake pattern, a reply to bia and (with a voice note and speech models) a wake word spoken in a voice message deliver and mark read |
| integration | `integration/members-e2e.py` | who wrote a message and who is in a group, without accounts: the messenger MCP with a fake bia-meta gives each message the sender's id and the group flag, sends a group's member list (bia left out) once before its first message, and turns joins and leaves into member events (bia's own kept back); MAX's control messages (add, remove, join by link, leave) become joins and leaves |
| integration | `integration/queue-e2e.py` | the waiting queue on the real agent: a scripted model with a 16k context and a fake telegram bus flooding it with 150 group messages plus a few important ones. Every message reaches the model in a working step (none is reflected away unseen), reflections never come twice in a row, root groups, root contacts, contacts, strangers and replies come before the flood in that order, the most important chat comes last in its batch, another group is not starved, the `[queue]` notice and `queue_take` work, the loop guard keeps 20% free, a very long message is cut and kept whole, a stranger joining a root group is reported and `chat_level` holds it, no group is linked to a person, read receipts go out when the model gets each message (queued ones stay unread) |
| integration | `integration/ticker-e2e.py` | the unanswered ticker on the real agent: a contact's request while the model runs a long bash job and a mixed group never stops talking. The first nudge comes at 15s and lists the job, the second 30s later, the model's status and answer reach the chat and the answer closes it; group talk (a root contact's too) is never nudged and nothing is sent to the group, a message naming bia there is |
| integration | `integration/max-mcp-e2e.py` | the max MCP with the real `bia_max.py` and PyMax against a fake MAX server: logins (QR, code, password), sends, inbound lines, catch-up, a revoked session, and voice calls with real WebRTC audio to a fake phone (with `BIA_SPEECH_DIR`, real voices and listening too). Not the real MAX servers |
| integration | `integration/screen-desktop-e2e.py` | the screen MCP's desktop tools on Xvfb, Openbox and AT-SPI with a real GTK app: snapshot, refs, acting through the app, stale refs, windows, the clipboard |
| integration | `integration/chrome-cert-e2e.py` | the chrome MCP and real Chromium against an HTTPS server that demands a client certificate: import, CA trust, the picker dialog answered through accessibility, per-site rules, restore after an OS update, removal |
| integration | `integration/chrome-passkey-e2e.py` | the chrome MCP and the real extension in Chromium against a page calling `navigator.credentials`: nothing is created or signed before `chrome_passkey_answer`; deny, approve, abort, quoted site words, choosing between passkeys |
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
