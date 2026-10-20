# bia release notes

Newest first. The agent carries this file: on the first boot after an update it reads every
release newer than the version it ran before, tells the user what changed (**Changes**) and
does the **For bia** steps itself. Write both for that reader: Changes in plain words for the
owner, For bia as concrete steps with tool names. Keep each item to one line.

## 1.1.4

### Changes
- takeover.sh (and bia-flash takeover) now refuse to run on a machine that already runs bia, and change nothing there. Before, a takeover command meant for another server could erase bia itself when it ran on bia by mistake, for example through ssh with missing quotes. To update bia, use the update (carry_update, or /update on the console).
- Telegram and WhatsApp messages now reach bia with the time they were written. Before, bia saw when it received them, so messages from while it was off all looked as if they had just arrived. Their message history now shows the time of each message too.
- bia's messages on Telegram and WhatsApp now show their formatting: bold, italic, strikethrough, code, code blocks, links, headings, lists and quotes. Before, people saw the raw markdown, such as `**bold**`. If Telegram refuses the formatting, the message is sent as plain text, so it still arrives.
- Passkeys in Chrome work reliably again. bia itself made a folder for its internal connections private when it started, so Chrome's passkey helper could not reach bia and sites reported an error. That folder now stays open to Chrome, and bia repairs the connection by itself if it ever breaks. chrome_status now says why the passkey add-on is not connected.
- The web app no longer offers a separate CA download: every certificate file (.p12) bia hands out already contains bia's certificate authority, “bia agent CA”. Import the .p12 and trust that CA in the same place (the sign-in page shows how for each system). The takeover page offers only bia-root.p12 now, and the web app sends its full certificate chain.
- In the web app, bold and italic text that contains code (for example **quota `drive/v3/about`:**) now shows formatted instead of with raw asterisks.

### For bia
- When you take over a server over ssh, first check with `ssh root@<address> 'hostname; cat /etc/os-release'` that you reach that server, then put the whole takeover command, the pipe into bash too, inside single quotes: `ssh root@<address> 'curl -fsSL …/takeover.sh | bash -s -- …'`. If it answers "this machine already runs bia", it ran on you: fix the quoting or the address. Never use takeover to update a bia; use carry_update.
- If you made your own fix for the passkey bridge (for example /etc/tmpfiles.d/bia-chrome.conf or a chmod of /run/bia), remove it: this version ships /etc/tmpfiles.d/bia.conf. When chrome_status shows connected false, read not_connected_because before changing anything.

## 1.1.3

### Changes
- bia can now replace a running Linux, such as a rented server (VPS): `curl -fsSL https://github.com/zond80/bia/releases/latest/download/takeover.sh | sudo bash`. It asks for the API key, language, and optional setup instructions for bia (or reads them from a config file or URL, also partial ones). It keeps the server's network settings and makes bia's certificates before anything is overwritten. You download root's certificate from a one-time page first; then the server reboots into bia. bia writes itself onto the disk from memory and starts with your keys.
- A server can also be taken over in land mode (`takeover.sh … --land`): it becomes an empty bia that waits for one of your bias to fly its agent there over the internet. It uses a land secret instead of keys: make it beforehand (`bia-flash land-secret`, or ask your bia for a new land secret), pass it with `--land-secret`, and later tell your bia "fly to <server address> with land secret …". The server keeps the secret through updates while it waits and deletes it once an agent has landed. Both sides prove the secret, nobody has to type a code at the server, and the agent travels encrypted.
- Two bias can now trust each other in one step: bia connects to the other one and gives it a certificate, and asks for one back. Once the other bia's owner approves, both can call each other's web API. Ask bia to "connect to the bia at https://…", or use /pki connect on the console.
- Certificate requests show the IP address they came from, and for another bia the address it says it can be reached at.
- A certificate can be limited to the addresses it may be used from; it is refused anywhere else.
- Machines without any display device (many cloud servers) now get a virtual screen, so the web app's display view works there too.

### For bia
- Nothing to do. You can now prepare a land yourself: when root gives you ssh access to a server, follow the steps in the carry description (carry_new_land_secret, ssh, takeover.sh --land, carry_fly). ssh is installed now. carry_fly takes host (an address or name on the internet) and secret (a land secret root gives you) to fly to a server that was taken over in land mode. You now have web_peer_connect, web_peer_list, web_peer_call and web_peer_remove (mcp/web) for mutual trust with other bias. Use them when root asks you to connect to another bia.

## 1.1.2

### Changes
- The model's thoughts show on the console again (in gray) with a ChatGPT subscription, with Claude models and with OpenRouter; before, these providers sent no reasoning at all or only empty blocks.
- Picking the wrong certificate in the browser (another bia's, an expired or a revoked one) now opens a page that says so and how to choose again, instead of Chrome's ERR_SSL_DECRYPT_ERROR_ALERT.
- The web app now uses the potato icon from the website.
- Switching the cloud model is safer: the new model is first tried with the real conversation, and if it fails then, or on its first real reply, bia goes back to the previous settings by itself. "Go back to the previous model" (or /fmm back on the console) swaps to the settings from before the last switch.
- Switching away from a ChatGPT subscription keeps its sign-in, so going back to it needs no new Login with ChatGPT. A new model also no longer inherits the previous model's context size, which made every request to a smaller model fail.
- On the machine's console, /call <function> runs any tool yourself: it asks for each value (for example the API key for system_fmm_configure), shows the call and runs it after you confirm. Use it to fix the cloud model when bia itself cannot.

## 1.1.1

### Changes
- The web app is now its own part of bia (the web MCP), a chat bus like Telegram and WhatsApp. It no longer shows the machine's console or a stop button; slash commands work only on the physical console.
- Web replies now always go to the person who wrote in that chat. In 1.1.0 a reply could end up in the wrong web chat and never reach the person. Sending to a web chat that does not exist is refused instead of silently opening a new one (web_send new_chat=true opens one on purpose).
- After a reboot or crash, messages that came while bia was off are answered right away; before, the boot notice could make bia reply only "ready for commands" and skip them.
- Telegram and WhatsApp: bia can create groups, add and remove members, leave groups and make invite links.
- Deleting messages on Telegram and WhatsApp now deletes them for everyone by default; say so if you want to keep the other person's copy.
- Web certificates and other agents' access requests moved from the system tools to the web tools (web_pki_*).
- web_pki_approve takes an optional message that the requesting agent reads with the approval (a welcome, a code word, anything root wants it told).
- Updates over the internet: with no flash stick attached, "update bia" downloads the newest release from GitHub, checks it and installs it the same way as from a stick (a stick is still used first when one is attached). Newer now means a higher version number.

### For bia
- Link certificates to contacts: discover(path='mcp/web', enable=true), then web_pki_list. For every active certificate whose identity has no addressbook contact with that id, addressbook_upsert(id=<identity>, name=<their name if you know it, else the identity>, trust='root' for root, else 'normal'). If you know the person's Telegram or WhatsApp chat, addressbook_link it to that contact.
- Recover replies that 1.1.0 put in the wrong web chat: if /home/bia/files/state/web/bia/ exists, read its messages; for each one where you can tell from your conversation who it was meant for, send it again with say to that person (web_chats lists who has web chats). Tell root which ones you could not place, then delete that folder.
- Use web_pki_issue, web_pki_list, web_pki_revoke, web_pki_requests, web_pki_approve and web_pki_deny from now on; the system_pki_* tools are gone.
