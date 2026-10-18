# bia release notes

Newest first. The agent carries this file: on the first boot after an update it reads every
release newer than the version it ran before, tells the user what changed (**Changes**) and
does the **For bia** steps itself. Write both for that reader: Changes in plain words for the
owner, For bia as concrete steps with tool names. Keep each item to one line.

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
