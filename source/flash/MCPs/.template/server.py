#!/usr/bin/python3
"""Minimal MCP server (stdio, one JSON-RPC message per line) for bia.

Runs on python3-minimal, which bia ships on both arches: json, os, subprocess,
pathlib, datetime and ssl are there; urllib and http.client are not, so call
web APIs with curl (see http_get). Copy this folder, rename it, edit TOOLS.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

NAME = Path(__file__).resolve().parent.name
# Data lives outside the MCP folder, so the folder can be shared as is.
STATE = Path(os.environ.get("BIA_FILES", "/home/bia/files")) / "state" / NAME


def http_get(url, headers=None):
    cmd = ["curl", "-fsS", "--retry", "3", "--retry-delay", "5", "--max-time", "30", url]
    for k, v in (headers or {}).items():
        cmd += ["-H", f"{k}: {v}"]
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def hello(args):
    return f"Hello, {args.get('name', 'world')}!"


TOOLS = {
    "example_hello": (
        hello,
        "Greet someone by name.",
        {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
    ),
}


def reply(id_, result=None, error=None):
    msg = {"jsonrpc": "2.0", "id": id_}
    if error is not None:
        msg["error"] = {"code": -32000, "message": error}
    else:
        msg["result"] = result
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def main():
    for line in sys.stdin:
        try:
            req = json.loads(line)
        except ValueError:
            continue
        method, id_ = req.get("method"), req.get("id")
        if id_ is None:
            continue  # notification
        if method == "initialize":
            reply(id_, {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                        "serverInfo": {"name": NAME, "version": "0.1.0"}})
        elif method == "tools/list":
            reply(id_, {"tools": [{"name": n, "description": d, "inputSchema": s}
                                  for n, (_, d, s) in TOOLS.items()]})
        elif method == "tools/call":
            p = req.get("params", {})
            tool = TOOLS.get(p.get("name"))
            if not tool:
                reply(id_, error=f"unknown tool {p.get('name')}")
                continue
            try:
                text, err = str(tool[0](p.get("arguments") or {})), False
            except Exception as e:
                text, err = f"error: {e}", True
            reply(id_, {"content": [{"type": "text", "text": text}], "isError": err})
        else:
            reply(id_, error=f"unsupported method {method}")


if __name__ == "__main__":
    main()
