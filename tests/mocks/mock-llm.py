#!/usr/bin/env python3
"""Mock frontier-model server for testing FMM without keys.
Serves an OpenAI-compatible /v1/chat/completions and an Anthropic /v1/messages, both streaming.
Behaviour: if the last message is a user message and no tool result is present yet, it calls the
`system_time` tool (if offered); after a tool result it answers with text mentioning the result.
   mock-llm.py [port]   (default 9999)
"""
import json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _sse(self, events):
        self.send_response(200); self.send_header("Content-Type", "text/event-stream"); self.end_headers()
        for ev, data in events:
            if ev: self.wfile.write(f"event: {ev}\n".encode())
            self.wfile.write(f"data: {json.dumps(data) if data is not None else '[DONE]'}\n\n".encode()); self.wfile.flush(); time.sleep(0.02)
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0)); body = json.loads(self.rfile.read(n) or b"{}")
        auth = self.headers.get("Authorization", "") + self.headers.get("x-api-key", "")
        if "mock-key" not in auth:
            self.send_response(401); self.end_headers(); self.wfile.write(b'{"error":{"message":"bad key"}}'); return
        msgs = body.get("messages", [])
        tools = [t.get("function", t).get("name") for t in body.get("tools", [])]
        if self.path.endswith("/chat/completions"):
            last = msgs[-1] if msgs else {}
            wants_time = last.get("role") == "user" and "time" in str(last.get("content", "")).lower()
            if "system_time" in tools and wants_time:
                self._sse([(None, {"choices": [{"delta": {"content": "Let me check the clock. "}, "finish_reason": None}]}),
                           (None, {"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call_mock1", "function": {"name": "system_time", "arguments": ""}}]}, "finish_reason": None}]}),
                           (None, {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{}"}}]}, "finish_reason": None}]}),
                           (None, {"choices": [{"delta": {}, "finish_reason": "tool_calls"}], "usage": {"prompt_tokens": 120, "completion_tokens": 9}}), (None, None)])
            else:
                last_tool = next((m["content"] for m in reversed(msgs) if m.get("role") == "tool"), "")
                text = f"MOCK-OPENAI says: the machine reports {last_tool.splitlines()[0] if last_tool else 'nothing'}." if last_tool else "MOCK-OPENAI: hello from the mock frontier model."
                evs = [(None, {"choices": [{"delta": {"content": w + " "}, "finish_reason": None}]}) for w in text.split()]
                evs += [(None, {"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 150, "completion_tokens": len(text.split())}}), (None, None)]
                self._sse(evs)
        elif self.path.endswith("/v1/messages"):
            last = msgs[-1] if msgs else {}
            last_text = " ".join(b.get("text", "") for b in last.get("content", [])) if isinstance(last.get("content"), list) else str(last.get("content", ""))
            last_has_result = isinstance(last.get("content"), list) and any(b.get("type") == "tool_result" for b in last["content"])
            wants_time = last.get("role") == "user" and not last_has_result and "time" in last_text.lower()
            evs = [("message_start", {"type": "message_start", "message": {"usage": {"input_tokens": 100}}})]
            if "system_time" in tools and wants_time:
                evs += [("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                        ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Checking the clock. "}}),
                        ("content_block_stop", {"type": "content_block_stop", "index": 0}),
                        ("content_block_start", {"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "toolu_mock1", "name": "system_time", "input": {}}}),
                        ("content_block_delta", {"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{}"}}),
                        ("content_block_stop", {"type": "content_block_stop", "index": 1}),
                        ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 12}})]
            else:
                last = ""
                for m in reversed(msgs):
                    if isinstance(m.get("content"), list):
                        for b in m["content"]:
                            if b.get("type") == "tool_result": last = str(b.get("content", "")); break
                    if last: break
                text = f"MOCK-ANTHROPIC says: the machine reports {last.splitlines()[0] if last else 'nothing'}." if last else "MOCK-ANTHROPIC: hello from the mock frontier model."
                evs += [("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})]
                evs += [("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": w + " "}}) for w in text.split()]
                evs += [("content_block_stop", {"type": "content_block_stop", "index": 0}),
                        ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": len(text.split())}})]
            evs += [("message_stop", {"type": "message_stop"})]
            self._sse(evs)
        else:
            self.send_response(404); self.end_headers()

if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 9999
    print(f"mock LLM on :{port}"); ThreadingHTTPServer(("0.0.0.0", port), H).serve_forever()
