# bia release notes

Newest first. The agent carries this file: on the first boot after an update it reads every
release newer than the version it ran before, tells the user what changed (**Changes**) and
does the **For bia** steps itself. Write both for that reader: Changes in plain words for the
owner, For bia as concrete steps with tool names. Keep each item to one line.

## 1.4.0

### Changes
- bia reads the screen: it can list every word on it with the exact point to click it, in the languages Chrome is set to and English (Russian, German, Chinese, Arabic, Hindi and many more); the reading models download on first use.
- Folder uploads in Google Chrome work again: 1.3.3 said bia answers Chrome's "Upload N files to this site?" itself, but in Google Chrome that question ignores the keyboard, so uploads stalled there. Now bia waits for the question and reads its buttons, and clicks Upload.

### For bia
- New screen_ocr lists the words on screen (x,y,w,h for a region) with the x,y of each; use it first to click a button, link or label with screen_click, and screen_shot when the picture itself matters.
- Uploading a folder: after chrome_choose_files(paths=[folder]) its result lists the words of Chrome's "Upload N files to this site?" with x,y; screen_click its Upload button (right of Cancel), then wait for the page to show the files. This replaces 1.3.3's step that said not to click it.

## 1.3.4

### Changes
- A model that writes a tool call as plain text (for example "say|web|answered=[…]") is told that nothing ran and tries again; before, the messages it meant to answer stayed open while it believed they were closed.
- Asking for a function that is already enabled now shows how to call it, instead of "already enabled" again; models no longer loop on that search, also after a long conversation was compacted.

### For bia
- Call tools only as real tool calls; a reply that only writes one out (say|…, say(…), <invoke …>) does nothing. Check the unanswered: notice and answer or close what is still open.

## 1.3.3

### Changes
- An OS update writes every step to /bia-update/update.log (the download, packing up, the offline copy and the restore), as well as showing it on screen; bia reads that one file to follow the update, and it is kept afterwards.
- MCPs no longer fail to start after boot with "too many running jobs"; there is no limit on running jobs anymore.
- When the model gets stuck repeating itself (the same text again and again, or endless made-up tool calls in its thoughts), bia stops the reply, tells the model, and lets it try again; after three such loops in one turn it stops the turn.
- Switching the model to DeepSeek no longer fails with "Invalid assistant message: content or tool_calls must be set" (bia sent it an empty reply from the previous model).
- Folder uploads on websites go through without help: bia confirms Chrome's "Upload N files to this site?" itself. Sites can also download several files at once without Chrome asking "Download multiple files? Block / Allow".
- Notices about Chrome tabs opening and closing work again with newer Chrome versions.

### For bia
- During an OS update, follow it with tail -n 20 /bia-update/update.log instead of checking processes or mounts, if required.
- If an MCP failed to start with "too many running jobs", start it again with discover(path="mcp/<name>", restart=true).
- If switching to DeepSeek failed with "content or tool_calls must be set", tell the owner it works now and switch with system_fmm_configure when they want.
- Drop any habit of clicking Chrome's "Upload" or "Allow" prompts with screen tools; chrome_choose_files confirms a folder upload, and multiple downloads land in /home/bia/Downloads without asking.

## 1.3.2

### Changes
- bia can upload files on websites again: when a page opens its file picker (an upload button or a drop zone), bia gets a notice and picks the files itself, several at once, a whole folder, or a place to save a download. Before, it got stuck waiting on the picker.

### For bia
- To attach a file on a website, click the page's upload button or drop zone; when the [file dialog opened] notice arrives, call chrome_choose_files(paths=[absolute paths]) (or cancel=true), then wait until the page shows the file before sending the form.

## 1.3.1

### Changes
- Instagram voice messages, photos and videos arrive again; every one failed with "internal error … nil pointer dereference". The same fault hit files in Messenger chats that are not encrypted.
- Logging in to Instagram now connects straight away and says whether the connection is up; before, it reported success but stayed disconnected until the MCP was restarted.
- A link pasted on Telegram no longer comes with a false "attachment not received" note (its preview has nothing to download); polls, dice and stories are named instead.
- Telegram reaches a user or chat known only by its id more often (bia loads its contacts and recent chats first), and says how to get access when it still cannot.
- WhatsApp profile and group photos are sent smaller (640x640 JPEG without metadata), and a picture WhatsApp refuses gets a clear explanation.
- History on Messenger and Instagram shows a message that was only a file as [voice], [image] and so on, and the full log of these two is kept in logs/mcp-messenger.log and logs/mcp-instagram.log.

### For bia
- If Instagram voice messages or photos failed for you with "nil pointer dereference", ask the people who sent them to send them again.
- If you set up a Chrome tab or an MCP restart as a workaround for "Instagram is not connected yet", drop it; instagram_login and instagram_status are enough now.
- Try whatsapp_set_photo again if it failed with 406; if it still fails, tell the owner the photo has to be set from the phone.

## 1.3.0

### Changes
- Backups (fold) and restores (unfold) now refuse to run without root rights instead of quietly leaving out files only root can read, such as keys and trusted peers. A backup made from a schedule or a script as user bia missed those files before.
- User bia can now use sudo without a password, so a schedule or script running as bia can run a backup with sudo.
- Job logs in files/logs/jobs are deleted after 30 days, and new ones belong to bia instead of root.
- bia can now run the groups it administers on Telegram, WhatsApp, Messenger and Instagram: show a group's settings, make or remove admins (on Telegram with the exact rights and a title), set what members may do (on Telegram per action, slow mode, protected content; on WhatsApp and encrypted Messenger groups who may send, edit the info or add people), join approval and requests, disappearing messages, the title, description and photo, and on Telegram restrict or ban one member for a while.
- Updating from a bia stick works again when the computer finds the stick first. Every bia drive had the same boot partition name and ID, so the computer could read the stick's boot partition instead of its own disk's. It then booted normally instead of running the update, and /boot could come from the stick. Each drive now gives its boot partition an ID of its own at startup and mounts /boot by that ID. An update also changes the ID of a stick's boot partition when it is the same as the disk's.
- New: Facebook Messenger and Instagram direct messages. bia signs in to facebook.com or instagram.com in its own Chrome (with the password and codes it keeps in its secrets), then takes the session from there and reads and answers its chats like on Telegram and WhatsApp: encrypted Messenger chats too, photos, videos, files, voice messages (transcribed; it can answer by voice), replies, edits, reactions, unsending, groups, bans and blocks. There are no calls on these two. Meta does not offer this officially, so it may ask for a security check in the browser now and then.

### For bia
- If a schedule or script of yours makes backups as user bia (its own tar, or the carry binary), run it as root: put sudo in front of its command, or set the schedule's user to root (schedule_update user=root). Check the next nightly archive includes files/state/pki.
- Drop any manual cleanup of old job logs and any chown of root-owned files you added for backups: neither is needed now.
- /boot is now mounted by its own ID instead of the name BIABOOT (see /etc/fstab); the first startup after this update sets that up and logs it (journalctl -u bia-esp-id). Leave it as it is. If an earlier update from a stick failed, you can delete what is left of it in /bia-update (this startup removes it too).
- mcp/messenger and mcp/instagram are new. Mention them to the owner once; log in only when asked: sign in at facebook.com / instagram.com in your Chrome (chrome MCP; passwords and codes from secrets), then messenger_login / instagram_login.

## 1.2.2

### Changes
- bia no longer gets stuck with "HTTP 400 … tool_calls must be followed by tool messages" (seen with DeepSeek). When bia used up its tool rounds in one turn, its last calls stayed without an answer, and the model's service refused every later request. Such a conversation now heals itself on the next request; nothing is lost.
- bia no longer stops a long task after a fixed number of tool rounds: every 100 rounds (was a hard stop after 32) it posts you a short status and goes on for as long as the task needs. To stop it or change course, just write to it; your message reaches it between steps.
- When the model's reply is cut off by its service's output limit in the middle of a tool call, bia no longer runs that tool with empty arguments: nothing runs, and the model is told to call it again with shorter arguments.
- bia can now adjust how it runs with one tool, set_model_config: its context size, after how many tool rounds it posts you a status, and the sampling of the local model (temperature and similar). After each change it makes a short test request and undoes the change if the model stops answering. Changes are kept across restarts.
- In the web app, everyone you gave a certificate with trust root now has the root options too: the root shell, the live console, the screen, files, machine status, certificates and requests from other agents, and notifications for human tasks. Before, only the certificate named root had them. People with trust normal still have only their own chats.

### For bia
- set_context is now set_model_config (context=<tokens>); it also takes max_tool_rounds and the local model's sampling. If your notes or schedules mention set_context, update them.

## 1.2.1

### Changes
- Backups made by bia now include every key and password file again. Some files written while bia was running (passwords saved since the last restart, the web app's certificate keys, the notification key and the list of browsers that get notifications, land secrets) belonged to the system administrator until the next restart, so a backup skipped them with a warning. If such a key had been lost, you would have had to turn browser notifications on again. Now they belong to bia right away.

### For bia
- If a backup you made earlier warned that it could not read files under files/state (for example web/push/vapid.pk8, pki/*.key or passwords.json), make a new backup now: those files are included from this version on.

## 1.2.0

### Changes
- The web app can now notify you when bia writes, calls or needs you, even when no bia tab is open. Turn it on with the banner in the side bar or in Settings → Notifications; on iPhone and iPad, first add bia to the Home Screen and open it from there. Nothing is sent while you have bia in front of you, and the message text is encrypted on the way.
- bia no longer thinks it can only move itself to computers on the same network: it now sees right away that carry_fly takes the address (and land secret) of a server on the internet.
- Settings → Notifications lists every browser that gets your notifications, shows whether delivery works, and lets you remove old or broken ones. Expired ones are removed by themselves.

### For bia
- When someone uses the web app and has not turned notifications on, tell them once that Settings → Notifications lets them get your messages with the tab closed (on iPhone: Add to Home Screen first).

## 1.1.10

### Changes
- Telegram and WhatsApp: when you reply to a message (or quote part of it), bia now sees which message you mean, who wrote it and its text. Before, it got only your words and had to guess what "and this?" was about.
- After an update or a restart, bia now reports back in the chat where you (or anyone else) were talking to it just before, instead of only on its own screen. If it promised to check something after the reboot, it tells you the result there.

## 1.1.9

### Changes
- Logs of scheduled tasks are now deleted after 14 days. Before, every run left its log file behind forever. Only these run logs are deleted; messages, memory and other files are not touched.
- The cache of recent Telegram message texts (used to show what an edited message said before) moved out of /home/bia to /var/cache/bia. It is readable only by root, so backups made as user bia skipped it with a warning; now it is no longer in the backed-up folder at all.
- Everything bia builds for itself now lives in one folder per feature in /home/bia/MCPs, works on both kinds of machine (ARM and x86) and keeps your data and keys outside that folder, so a feature can be shared with other bias or people as is. bia starts each one from a template (MCPs/.template).
- bia's instructions are up to date again. Updates kept the instructions from the day bia was first installed (in /home/bia/bia.toml), so newer rules, for example about web chats, voice and languages, never reached it. bia now uses its current built-in instructions unless you wrote your own in bia.toml.
- bia's instructions are half as long (about 2,500 characters instead of 5,200) with the same rules, so every request costs less. bia also no longer gets stuck searching for a function again and again: such a search now loads that function, and any repeated call is answered with what to do next.

### For bia
- Old files in files/state/schedule-logs are now deleted by bia itself: drop any cleanup you set up for them.
- files/state/telegram-texts.json is gone from /home/bia (moved to /var/cache/bia): remove any backup exclude or workaround you added for it, and never run backups as root to include it.
- Move what you already built into MCPs/<name>/ folders as your prompt now says (see MCPs/.template/README.md): each script outside MCPs (e.g. in files/) that a schedule runs, with its helpers; move its data to files/state/<name>/, point the schedule at the new path, run it once by hand to check, then tell root what you moved.

## 1.1.8

### Changes
- Telegram and WhatsApp: a message now shows as read as soon as bia receives it. Before, a voice message showed as read only after bia had downloaded and transcribed it, so the sender waited without knowing whether it arrived.
- Telegram, WhatsApp and the web app handle each incoming message on its own: downloading a file or transcribing a voice message no longer holds up other chats (on Telegram it held up everything, including incoming calls; in the web app, other people's messages). Within one chat, messages still reach bia in the order they were sent: a text sent right after a voice message comes after that voice message's transcript, never before it.
- Telegram and WhatsApp: when a photo, voice message or file cannot be downloaded, bia is now told so and can ask for it again. Before, such a message without text was dropped silently.

### For bia
- "[<photo|voice message|file…> not received: <reason>; …]" on a Telegram or WhatsApp message means its attachment could not be downloaded: ask the sender to send it again if you need it.

## 1.1.7

### Changes
- A takeover now also shows its plan, progress and result on every other terminal and on the server's consoles (the provider's web or serial console), as `bia takeover: …` lines, so whoever watches the server sees what happens. The gate link and the land secret stay on the terminal that runs it.
- bia now checks its disk at every boot, before using it, and repairs what it finds by itself. Before, the boot never checked it, so errors such as a damaged system file ("Bad message") stayed until someone repaired them by hand. On a healthy disk this costs a fraction of a second.
- A full disk check on request: ask bia to check and repair its disk, or type /fsck on the console. bia restarts into its rescue system for a few minutes, checks the disk while it is not in use, and tells you the result when it is back.
- An update no longer stops at the disk-space step when the system's df program is damaged.
- A server taken over with takeover.sh now shows "bia disk" in its boot menu and firmware boot list instead of "bia flash", like any other bia installed on a disk.
- After an MCP is updated or edited, bia can restart it on purpose (discover with restart=true) and then knows its new functions. The model gets an MCP's instructions again only when they really changed (bia keeps a fingerprint of what it gave the model, also across restarts), so an update that changes nothing it sees costs no extra tokens. Before, bia kept the old function list until it restarted itself.
- Web app on a phone: the chat header has its back arrow again (to the chat list and Console, Root shell, Files, Human tasks, Display, Settings), and a long status such as the model name stays on one line instead of covering the chat.
- bia now knows when someone opens or closes the web app, and may greet them when it fits (for example when it has news for them or they were away long). Quick reconnects of a phone and extra tabs are not reported.
- bia no longer hangs at boot while its MCPs start (for example Telegram or WhatsApp restoring their session over a slow network): they start in the background as jobs, and bia is ready for you right away.

### For bia
- If you see filesystem errors ("Bad message", EXT4 checksum errors in dmesg, tune2fs state "clean with errors"), tell root and call carry_fsck: it checks and repairs the disk offline and reports at the next boot. Do not build your own boot entries, init= scripts or one-shot units for fsck.
- After a repair: check lost+found and dpkg --verify; carry_update reinstalls the whole system if system files were damaged.
- After you update or edit an MCP, call discover(path="mcp/<name>", restart=true): it starts the new version and tells you its functions only if they changed. When a result says "mcp/<name> changed", use the definitions in it from then on. Running MCPs are not restarted by themselves when their files change.
- At boot, MCPs start in the background as jobs (mcp_start); each result arrives as system|job:<id>: started (callable now, with its new definitions only if they changed) or why not. Until then, calls to that MCP answer that it is still starting (wait for the job; job_logs shows progress, job_kill gives the start up). say to a Telegram or WhatsApp chat meanwhile is queued and goes out when that MCP is up; if it does not start, each queued message comes back as [send failed …] on system|outbox.

## 1.1.6

### Changes
- A takeover now continues only after root's certificate (bia-root.p12) was downloaded from its one-time page, in a browser or with curl or wget; copying the file with scp no longer counts. A download counts only once the whole file was sent. If the page cannot start (its port is taken), the takeover stops before changing anything. Its output shows ready curl and wget commands, and an ssh one for when the port is blocked from outside.
- bia's tool descriptions are shorter: each MCP says in a sentence or two what it is for, and the details come with each tool, its results and its errors.

### For bia
- When you take over a server without --land, download root's certificate from the gate page the takeover prints, instead of copying it with scp: in a second bash job run `curl -fk -o /home/bia/files/<address>-root.p12 https://<address>:8443/<token>/root.p12` (if port 8443 is blocked: `ssh root@<address> 'curl -fsk https://127.0.0.1:8443/<token>/root.p12' > /home/bia/files/<address>-root.p12`); then the takeover continues by itself. Never cat or print that file: it holds root's key. Tell root where it is.
- To make a server a land for yourself, call carry_new_land_secret(host=<address>): it now returns the exact steps (ssh check, takeover command with the secret filled in, carry_fly).

## 1.1.5

### Changes
- A takeover now waits until root's certificate (bia-root.p12) has left the server, from the one-time page or copied with scp, and only then continues and reboots by itself. Before, an unattended takeover could reboot right away and erase the only copy, leaving no way into the new bia. Its output says plainly what to fetch and how; `gate_wait` now only sets how long to wait before giving up without changing anything.
- After a takeover, the server's carried-over network now always applies at the first boot. Before, the network card could come up with plain DHCP before the carried-over settings were in place: servers with a fixed address (for example a /32 with its gateway outside, as at OVH or Hetzner) ended up with only a 169.254.x.x address and an empty /etc/resolv.conf.
- The network no longer waits 15 seconds at every boot after a takeover, and the DHCP fallback no longer mistakes a 169.254.x.x route for a working network.
- The console's network messages show the real method (static, dhcp) instead of always "dhcp".

### For bia
- When you take over a server without --land, takeover.sh waits for root's certificate: in a second bash job run `scp root@<address>:/root/bia-root.p12 /home/bia/files/<address>-root.p12`, then the takeover continues by itself. Never cat or print that file: it holds root's key. Tell root where it is.
- Nothing else to do.

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
