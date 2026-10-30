//! OpenAPI document generated from the web app route table.
use serde_json::{json, Value};

/// Build OpenAPI 3.1 as YAML text (generated, not hand-maintained).
pub fn yaml(port: u16, ca_sha256: &str) -> String {
    let doc = document(port, ca_sha256);
    to_yaml(&doc)
}

pub fn document(port: u16, ca_sha256: &str) -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "bia web",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "HTTPS console and machine API. Auth is mTLS with certificates issued by this agent CA. Bootstrap (no client cert): download CA from /v1/pki/ca, pick up one-time PKCS#12 after root issues a cert."
        },
        "servers": [{"url": format!("https://0.0.0.0:{port}")}],
        "components": {
            "securitySchemes": {
                "mtls": {
                    "type": "mutualTLS",
                    "description": "Client certificate issued by this agent CA (CN = identity)."
                }
            },
            "schemas": {
                "WhoAmI": {
                    "type": "object",
                    "properties": {
                        "identity": {"type": "string"},
                        "role": {"type": "string", "enum": ["user", "agent"]},
                        "trust": {"type": "string", "enum": ["root", "normal"]},
                        "serial": {"type": "string"},
                        "not_after": {"type": "integer", "description": "Unix seconds"},
                        "days_left": {"type": "integer"},
                        "renew": {"type": "string", "description": "POST path to renew while not expired"}
                    }
                },
                "Discovery": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "version": {"type": "string"},
                        "openapi": {"type": "string"},
                        "skill": {"type": "string"},
                        "ca_sha256": {"type": "string"},
                        "ca": {"type": "string"},
                        "pickup": {"type": "string"},
                        "console_ws": {"type": "string"},
                        "auth": {"type": "string"},
                        "buses": {"type": "array", "items": {"type": "string"}}
                    }
                }
            }
        },
        "paths": {
            "/.well-known/bia.json": {
                "get": {
                    "tags": ["discovery"],
                    "summary": "Agent discovery (no client cert)",
                    "responses": {"200": {"description": "Discovery document", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Discovery"}}}}}
                }
            },
            "/openapi.yaml": {
                "get": {
                    "tags": ["discovery"],
                    "summary": "This OpenAPI document",
                    "responses": {"200": {"description": "OpenAPI 3.1 YAML"}}
                }
            },
            "/skills/web-console/SKILL.md": {
                "get": {
                    "tags": ["discovery"],
                    "summary": "Skill for peer agents",
                    "responses": {"200": {"description": "Markdown skill"}}
                }
            },
            "/v1/pki/ca": {
                "get": {
                    "tags": ["pki"],
                    "summary": "Download CA PEM (public, no auth)",
                    "responses": {
                        "200": {"description": "CA certificate PEM"}
                    }
                }
            },
            "/v1/pki/bootstrap": {
                "get": {
                    "tags": ["pki"],
                    "summary": "Whether an active root client cert exists (public)",
                    "responses": {
                        "200": {"description": "JSON with root_issued and issue path"}
                    }
                }
            },
            "/v1/pki/bootstrap/root": {
                "post": {
                    "tags": ["pki"],
                    "summary": "Issue the first root client PKCS#12 (public; 409 if root already active)",
                    "responses": {
                        "200": {"description": "PKCS#12 (password 12345678) for identity root"},
                        "409": {"description": "Root certificate already issued"}
                    }
                }
            },
            "/v1/pki/pickup/{token}": {
                "get": {
                    "tags": ["pki"],
                    "summary": "One-time PKCS#12 download (deleted after GET)",
                    "parameters": [{"name": "token", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {
                        "200": {"description": "PKCS#12 (password 12345678)"},
                        "404": {"description": "Missing, used, or expired"}
                    }
                }
            },
            "/v1/pki/requests": {
                "post": {
                    "tags": ["pki"],
                    "summary": "Ask for a client certificate for your own key (public): send a CSR. A person on this machine approves or denies it later; keep the returned id and poll it.",
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "required": ["identity", "csr"], "properties": {
                        "csr": {"type": "string", "description": "PEM certificate signing request for your key (EC P-256/P-384, Ed25519 or RSA >= 2048). Its subject is ignored."},
                        "identity": {"type": "string", "description": "letters, digits, - _ . @ (not root)"},
                        "role": {"type": "string", "enum": ["agent", "user"]},
                        "note": {"type": "string", "description": "who you are and why (shown to the approver)"},
                        "grant": {"type": "object", "description": "another bia only (mutual trust): a certificate for THIS bia's agent key (from GET /v1/pki/agent) signed by your CA, your CA, and your https address. Checked on arrival; on approval this bia keeps you as a peer it can call.", "properties": {
                            "cert": {"type": "string", "description": "PEM client certificate for this bia's agent key"},
                            "ca": {"type": "string", "description": "PEM of your CA"},
                            "url": {"type": "string", "description": "https://address[:port] of your web API"}
                        }}
                    }}}}},
                    "responses": {
                        "202": {"description": "{id (UUID, secret), status: pending, poll, cert, retry_after, expires_at, ca_sha256}"},
                        "400": {"description": "Bad identity, role or CSR"},
                        "429": {"description": "Too many requests waiting"}
                    }
                },
                "get": {
                    "tags": ["pki"],
                    "summary": "All certificate requests (root)",
                    "security": [{"mtls": []}],
                    "responses": {"200": {"description": "{requests: [{id, identity, role, status, created, expires_at?}]}"}}
                }
            },
            "/v1/pki/agent": {
                "get": {
                    "tags": ["pki"],
                    "summary": "This bia's agent name and a CSR for its agent key (public). Another bia signs the CSR with its own CA and sends that certificate as the grant of its certificate request: one approval then gives trust both ways.",
                    "responses": {"200": {"description": "{name, csr (PEM), ca_sha256, requests}"}}
                }
            },
            "/v1/pki/requests/{id}": {
                "get": {
                    "tags": ["pki"],
                    "summary": "Status of a certificate request by its id (public): pending, approved, denied, issued or expired. Poll every retry_after seconds while pending.",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}],
                    "responses": {
                        "200": {"description": "{id, identity, role, status, created, expires_at?, retry_after?, reason?, cert? (when approved)}"},
                        "404": {"description": "Unknown id"}
                    }
                }
            },
            "/v1/pki/requests/{id}/cert": {
                "get": {
                    "tags": ["pki"],
                    "summary": "Download the signed certificate followed by the CA, PEM (public; useless without your key)",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}],
                    "responses": {
                        "200": {"description": "PEM: your certificate (normal trust), then the CA"},
                        "403": {"description": "Denied"},
                        "409": {"description": "Still pending"},
                        "410": {"description": "Expired"}
                    }
                }
            },
            "/v1/pki/requests/{id}/approve": {
                "post": {
                    "tags": ["pki"],
                    "summary": "Approve a certificate request (root)",
                    "security": [{"mtls": []}],
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}],
                    "responses": {"200": {"description": "The request"}, "409": {"description": "Not pending"}}
                }
            },
            "/v1/pki/requests/{id}/deny": {
                "post": {
                    "tags": ["pki"],
                    "summary": "Deny a certificate request (root)",
                    "security": [{"mtls": []}],
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}}],
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "properties": {"reason": {"type": "string"}}}}}},
                    "responses": {"200": {"description": "The request"}, "409": {"description": "Not pending"}}
                }
            },
            "/v1/pki/renew": {
                "post": {
                    "tags": ["pki"],
                    "summary": "Renew the caller's still-valid client certificate for a new key the caller made: send a CSR. Returns the new certificate then the CA (PEM); the old serial is revoked. No key is made on the server.",
                    "security": [{"mtls": []}],
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "required": ["csr"], "properties": {"csr": {"type": "string", "description": "PEM CSR (EC P-256/P-384, Ed25519 or RSA >= 2048); subject ignored"}}}}}},
                    "responses": {"200": {"description": "PEM certificate + CA; headers X-Bia-Serial, X-Bia-Not-After"}, "400": {"description": "Missing or bad CSR"}, "403": {"description": "Certificate revoked or expired"}}
                }
            },
            "/v1/whoami": {
                "get": {
                    "tags": ["session"],
                    "summary": "Identity from client certificate",
                    "security": [{"mtls": []}],
                    "responses": {
                        "200": {"description": "Client identity", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/WhoAmI"}}}},
                        "401": {"description": "No valid client certificate"}
                    }
                }
            },
            "/v1/push/key": {
                "get": {
                    "tags": ["push"],
                    "summary": "VAPID public key (base64url) for PushManager.subscribe's applicationServerKey",
                    "security": [{"mtls": []}],
                    "responses": {"200": {"description": "{key}"}}
                }
            },
            "/v1/push/subscribe": {
                "post": {
                    "tags": ["push"],
                    "summary": "Store a Web Push subscription (PushSubscription.toJSON()) for the caller; bia then pushes new bia messages, calls and (root) human tasks while no tab of the caller is in front",
                    "security": [{"mtls": []}],
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "required": ["endpoint", "keys"], "properties": {"endpoint": {"type": "string"}, "keys": {"type": "object", "properties": {"p256dh": {"type": "string"}, "auth": {"type": "string"}}}}}}}},
                    "responses": {"200": {"description": "{ok}"}, "400": {"description": "Error text"}}
                }
            },
            "/v1/push/unsubscribe": {
                "post": {
                    "tags": ["push"],
                    "summary": "Forget one of the caller's subscriptions",
                    "security": [{"mtls": []}],
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "required": ["endpoint"], "properties": {"endpoint": {"type": "string"}}}}}},
                    "responses": {"200": {"description": "{ok, removed}"}}
                }
            },
            "/v1/push/subs": {
                "get": {
                    "tags": ["push"],
                    "summary": "The caller's subscribed browsers: id, device, service, created, expires, expired, last_ok, last_err, last_err_at, failing",
                    "security": [{"mtls": []}],
                    "responses": {"200": {"description": "{subs: [...]}"}}
                }
            },
            "/v1/push/remove": {
                "post": {
                    "tags": ["push"],
                    "summary": "Forget some of the caller's browsers by id (from /v1/push/subs)",
                    "security": [{"mtls": []}],
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "required": ["ids"], "properties": {"ids": {"type": "array", "items": {"type": "string"}}}}}}},
                    "responses": {"200": {"description": "{ok, removed}"}}
                }
            },
            "/v1/push/test": {
                "post": {
                    "tags": ["push"],
                    "summary": "Push a test notification to every subscribed browser of the caller",
                    "security": [{"mtls": []}],
                    "responses": {"200": {"description": "{ok, sent}"}, "502": {"description": "Push service error text"}}
                }
            },
            "/v1/status": {
                "get": {
                    "tags": ["root"],
                    "summary": "Agent status: model, state, context, version (root)",
                    "security": [{"mtls": []}],
                    "responses": {"200": {"description": "Status"}}
                }
            },
            "/v1/pki/certs": {
                "get": {
                    "tags": ["pki"],
                    "summary": "Issued client certificates (root)",
                    "security": [{"mtls": []}],
                    "responses": {"200": {"description": "{certs}"}}
                }
            },
            "/v1/pki/issue": {
                "post": {
                    "tags": ["pki"],
                    "summary": "Issue a client certificate; returns one-time pickup URLs (root)",
                    "security": [{"mtls": []}],
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "required": ["identity"], "properties": {
                        "identity": {"type": "string"},
                        "role": {"type": "string", "enum": ["user", "agent"]},
                        "trust": {"type": "string", "enum": ["root", "normal"]}
                    }}}}},
                    "responses": {"200": {"description": "{identity, serial, path, urls, expires_in}"}}
                }
            },
            "/v1/pki/revoke": {
                "post": {
                    "tags": ["pki"],
                    "summary": "Revoke a client certificate by serial (root)",
                    "security": [{"mtls": []}],
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "required": ["serial"], "properties": {"serial": {"type": "string"}}}}}},
                    "responses": {"200": {"description": "{revoked}"}}
                }
            },
            "/v1/fs/list": {
                "get": {
                    "tags": ["files"],
                    "summary": "List a directory by absolute path (root)",
                    "security": [{"mtls": []}],
                    "parameters": [{"name": "path", "in": "query", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "{path, home, entries, error}"}}
                }
            },
            "/v1/fs/get": {
                "get": {
                    "tags": ["files"],
                    "summary": "Download a file (Range) or a folder as a streamed tar (root)",
                    "security": [{"mtls": []}],
                    "parameters": [
                        {"name": "path", "in": "query", "required": true, "schema": {"type": "string"}},
                        {"name": "dl", "in": "query", "schema": {"type": "string", "enum": ["1"]}}
                    ],
                    "responses": {"200": {"description": "Bytes"}}
                }
            },
            "/v1/fs/put": {
                "post": {
                    "tags": ["files"],
                    "summary": "Write a file from the raw body; parent folders are created (root)",
                    "security": [{"mtls": []}],
                    "parameters": [
                        {"name": "path", "in": "query", "required": true, "schema": {"type": "string"}},
                        {"name": "overwrite", "in": "query", "schema": {"type": "string", "enum": ["1"]}}
                    ],
                    "responses": {"200": {"description": "{path, size}"}, "409": {"description": "Exists"}}
                }
            },
            "/v1/fs/op": {
                "post": {
                    "tags": ["files"],
                    "summary": "mkdir, touch, rename, move, copy, delete, chmod, du (root)",
                    "security": [{"mtls": []}],
                    "requestBody": {"content": {"application/json": {"schema": {"type": "object", "required": ["op"], "properties": {
                        "op": {"type": "string"},
                        "path": {"type": "string"},
                        "paths": {"type": "array", "items": {"type": "string"}},
                        "to": {"type": "string"},
                        "mode": {"type": "string"}
                    }}}}},
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/v1/files": {
                "get": {
                    "tags": ["files"],
                    "summary": "Legacy: list under /home/bia/files (root)",
                    "security": [{"mtls": []}],
                    "parameters": [{"name": "path", "in": "query", "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "Directory listing"}}
                }
            },
            "/v1/file": {
                "get": {
                    "tags": ["files"],
                    "summary": "Legacy: download under /home/bia/files (root)",
                    "security": [{"mtls": []}],
                    "parameters": [{"name": "path", "in": "query", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "File bytes"}}
                }
            },
            "/v1/hitl": {
                "get": {
                    "tags": ["hitl"],
                    "summary": "List pending human tasks",
                    "security": [{"mtls": []}],
                    "responses": {"200": {"description": "Tasks"}}
                }
            },
            "/v1/hitl/{id}": {
                "get": {
                    "tags": ["hitl"],
                    "summary": "Get one human task",
                    "security": [{"mtls": []}],
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "Task"}}
                }
            },
            "/v1/hitl/{id}/finish": {
                "post": {
                    "tags": ["hitl"],
                    "summary": "Complete a human task",
                    "security": [{"mtls": []}],
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/ws": {
                "get": {
                    "tags": ["chat"],
                    "summary": "The web bus (protocol 2): your one chat with bia, for people, other bias and agents alike. Client hello {proto: 2, instance, rx} first; then m {seq, m} / gone {seq, ids} from rx on, live changes, actions with cid → ack (send, edit, delete, react, pin, read, button, vote, close_poll, mute, hist, search), files pulled by the receiver (fget / binary chunks [sha256 32][offset u64][data] / fack / fdone), ping every 5 s (silent 15 s: closed), bye. One live session per identity (a second gets bye duplicate_key). See SKILL.md.",
                    "security": [{"mtls": []}],
                    "responses": {"101": {"description": "Switching Protocols"}, "429": {"description": "too many handshakes from this address"}}
                }
            },
            "/v1/bridge": {
                "get": {
                    "tags": ["bridge"],
                    "summary": "bia-bridge builds this bia serves (version, platform, size, sha256, url); public. GET /bridge/<file> downloads one.",
                    "responses": {"200": {"description": "the list"}}
                }
            },
            "/v1/call": {
                "get": {
                    "tags": ["chat"],
                    "summary": "Live voice call with bia on your web chat (?ring=<id> to answer bia's call). Binary 16 kHz mono s16le frames in; JSON {t:played|mute|hangup} in; bia's speech as binary [gen u32][rate u32][s16le] plus JSON ready/say/say_end/stop/caption/end out.",
                    "security": [{"mtls": []}],
                    "responses": {"101": {"description": "Switching Protocols"}}
                }
            },
            "/ws/term": {
                "get": {
                    "tags": ["root"],
                    "summary": "Live agent console (raw bytes, status header included); {t:line,v} types a console line (empty interrupts the model), {t:resize} (root)",
                    "security": [{"mtls": []}],
                    "responses": {"101": {"description": "Switching Protocols"}}
                }
            },
            "/ws/shell": {
                "get": {
                    "tags": ["root"],
                    "summary": "Root login shell on a PTY (binary frames; {t:resize} as text) (root)",
                    "security": [{"mtls": []}],
                    "responses": {"101": {"description": "Switching Protocols"}}
                }
            },
            "/gui": {
                "get": {
                    "tags": ["display"],
                    "summary": "noVNC of display :0 (VNC stays on 127.0.0.1:5900)",
                    "security": [{"mtls": []}],
                    "responses": {"302": {"description": "Redirect to noVNC"}}
                }
            },
            "/gui/ws": {
                "get": {
                    "tags": ["display"],
                    "summary": "WebSocket bridge to localhost VNC",
                    "security": [{"mtls": []}],
                    "responses": {"101": {"description": "Switching Protocols"}}
                }
            }
        },
        "x-bia-ca-sha256": ca_sha256,
    })
}

pub fn discovery(port: u16, ca_sha256: &str) -> Value {
    json!({
        "name": "bia",
        "version": env!("CARGO_PKG_VERSION"),
        "openapi": "/openapi.yaml",
        "skill": "/skills/web-console/SKILL.md",
        "ca_sha256": ca_sha256,
        "ca": "/v1/pki/ca",
        "pickup": "/v1/pki/pickup/{token}",
        "request": "/v1/pki/requests",
        "console_ws": "/ws",
        "auth": "mtls",
        "buses": ["console", "web", "telegram", "whatsapp"],
        "chat_ws": "/ws",
        "chat_protocol": 2,
        "bridge": {
            "about": "bia-bridge: a ready client for agents (CLI + MCP server); connect once, the owner approves, then send / wait",
            "list": "/v1/bridge",
            "download": "/bridge/<file>",
            "files": crate::bridge::list()["files"],
            "mcp": {"command": "bia-bridge", "args": ["mcp"]}
        },
        "peer_proto": 1,
        "port": port,
    })
}

fn to_yaml(v: &Value) -> String {
    // Minimal JSON→YAML for our generated tree (objects/arrays/scalars).
    fn emit(out: &mut String, v: &Value, indent: usize) {
        let pad = "  ".repeat(indent);
        match v {
            Value::Object(m) => {
                if m.is_empty() {
                    out.push_str("{}");
                    return;
                }
                let mut first = true;
                for (k, val) in m {
                    if !first {
                        out.push('\n');
                    }
                    first = false;
                    out.push_str(&pad);
                    out.push_str(k);
                    out.push(':');
                    match val {
                        Value::Object(_) | Value::Array(_) => {
                            out.push('\n');
                            emit(out, val, indent + 1);
                        }
                        _ => {
                            out.push(' ');
                            emit_scalar(out, val);
                        }
                    }
                }
            }
            Value::Array(a) => {
                if a.is_empty() {
                    out.push_str(&pad);
                    out.push_str("[]");
                    return;
                }
                for (i, item) in a.iter().enumerate() {
                    if i > 0 {
                        out.push('\n');
                    }
                    out.push_str(&pad);
                    out.push_str("- ");
                    match item {
                        Value::Object(_) | Value::Array(_) => {
                            out.push('\n');
                            emit(out, item, indent + 1);
                        }
                        _ => emit_scalar(out, item),
                    }
                }
            }
            _ => emit_scalar(out, v),
        }
    }
    fn emit_scalar(out: &mut String, v: &Value) {
        match v {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::String(s) => {
                if s.is_empty()
                    || s.contains(':')
                    || s.contains('#')
                    || s.contains('\n')
                    || s.starts_with(' ')
                {
                    out.push('"');
                    out.push_str(&s.replace('\\', "\\\\").replace('"', "\\\""));
                    out.push('"');
                } else {
                    out.push_str(s);
                }
            }
            _ => out.push_str("null"),
        }
    }
    let mut out = String::new();
    emit(&mut out, v, 0);
    out.push('\n');
    out
}
