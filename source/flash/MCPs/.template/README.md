# my-example

What it does, in two or three lines.

## Needs
- Debian packages: none (python3-minimal and curl ship with bia)
- Secrets: none (name each one here; read it from the secrets MCP or env, never store it in this folder)
- Services: none (e.g. OpenRouter API)

## Files
- `mcp.json`, `server.py`: the MCP (leave out mcp.json if the folder only holds scripts)
- `bin/`: scripts that schedules run (`/home/bia/MCPs/<name>/bin/…`)
- Data: `files/state/<name>/` (never in this folder)

## Rules for every bia feature
- One folder per feature in /home/bia/MCPs/<name>/, never named like a shipped MCP (use a `my-` prefix).
- Works on aarch64 and x86_64: Python on python3-minimal, shell, or native binaries for both arches
  in `"command": {"aarch64": "bin/aarch64/x", "x86_64": "bin/x86_64/x"}`. A venv or node_modules
  with compiled parts is rebuilt on start when missing, not kept here.
- No keys, chat ids, owner data or absolute paths to files/ inside the folder: it must be shareable as is.
- Test it: discover(restart=true) after edits, call each tool once; run each schedule script by hand.
