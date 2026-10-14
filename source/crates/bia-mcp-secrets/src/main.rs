//! "secrets" MCP: passwords, TOTP/HOTP, WebAuthn passkeys. Store: $BIA_FILES/state/passwords.json
use bia_mcp::{arg_bool, arg_str, arg_str_opt, Server};
use bia_mcp_secrets::store;
use serde_json::json;

fn main() {
    Server::new("secrets", env!("CARGO_PKG_VERSION"))
        .tool(
            "list",
            "List saved secrets. Shows name and type only. Does not show the password or the code. Types are string, totp, hotp, and passkey.",
            json!({"type": "object", "properties": {}}),
            |_, _| store::list(),
        )
        .tool(
            "get",
            "Read one secret by id. id is the name from secrets_list. A password comes back as text. A totp or hotp comes back as the current login code. A passkey comes back without its private key. Set raw to true only for a backup of the stored seed.",
            json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string", "description": "name or id from secrets_list"},
                "raw": {"type": "boolean", "description": "true returns the stored seed instead of the current code. Default false."}
            }}),
            |_, a| store::get(arg_str(&a, "id")?, arg_bool(&a, "raw", false)),
        )
        .tool(
            "set",
            "Save a secret. description is the name you will use later. value is the password, or an otpauth:// URL for a login code. type is string, totp, or hotp. Omit type for a password. Refuses to overwrite the same name unless force is true.",
            json!({"type": "object", "required": ["description", "value"], "properties": {
                "description": {"type": "string", "description": "name, for example GitHub token or GitHub 2FA"},
                "type": {"type": "string", "description": "string, totp, or hotp. Default string."},
                "value": {"type": "string", "description": "the password, or an otpauth:// URL"},
                "force": {"type": "boolean", "description": "true overwrites a secret with the same name. Default false."}
            }}),
            |_, a| store::set(arg_str(&a, "description")?, arg_str_opt(&a, "type").unwrap_or(""), arg_str(&a, "value")?, arg_bool(&a, "force", false)),
        )
        .tool(
            "delete",
            "Delete one secret. id is the name from secrets_list.",
            json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string"}
            }}),
            |_, a| store::delete(arg_str(&a, "id")?),
        )
        .tool(
            "import",
            "Import a 1Password-style login CSV (Title,URL,Username,Password,OTPAuth,Notes), or legacy 1Password unencrypted JSON. Pass path under /home/bia/files, or json with pasted text. force true overwrites names that already exist.",
            json!({"type": "object", "properties": {
                "json": {"type": "string", "description": "pasted CSV or legacy 1Password JSON. Omit if you pass path."},
                "path": {"type": "string", "description": "file under /home/bia/files, for example inbox/web/export.csv"},
                "force": {"type": "boolean", "description": "true overwrites names that already exist. Default false."}
            }}),
            |_, a| {
                let force = arg_bool(&a, "force", false);
                if let Some(p) = arg_str_opt(&a, "path").filter(|s| !s.trim().is_empty()) {
                    return store::import_path(p, force);
                }
                if let Some(j) = arg_str_opt(&a, "json").filter(|s| !s.trim().is_empty()) {
                    return store::import_json(j, force);
                }
                Err("pass json= (paste) or path= (file under /home/bia/files)".into())
            },
        )
        .tool(
            "export",
            "Export one login group to a 1Password-compatible CSV under /home/bia/files. id is a name from secrets_list; matching username/password/TOTP siblings are merged into one row. Cleartext. Passkeys are noted only (not portable via CSV).",
            json!({"type": "object", "required": ["id", "path"], "properties": {
                "id": {"type": "string", "description": "name or id from secrets_list"},
                "path": {"type": "string", "description": "file under /home/bia/files, for example outbox/gmail.csv (.csv added if missing)"}
            }}),
            |_, a| store::export_path(arg_str(&a, "id")?, arg_str(&a, "path")?),
        )
        .tool(
            "passkey_create",
            "Create a passkey for one website. description is the name you will use later. rp_id is the site name, for example github.com. The private key is saved and is not printed.",
            json!({"type": "object", "required": ["description", "rp_id"], "properties": {
                "description": {"type": "string", "description": "name, for example GitHub passkey"},
                "rp_id": {"type": "string", "description": "site name, for example github.com"},
                "user_name": {"type": "string", "description": "account name. Default bia."},
                "user_id": {"type": "string", "description": "optional account id"},
                "challenge": {"type": "string", "description": "the challenge the website just showed, if you have it"},
                "origin": {"type": "string", "description": "optional. Default is https:// plus rp_id."},
                "force": {"type": "boolean", "description": "true overwrites a passkey with the same name"}
            }}),
            |_, a| store::passkey_create(
                arg_str(&a, "description")?,
                arg_str(&a, "rp_id")?,
                arg_str_opt(&a, "user_name").unwrap_or("bia"),
                arg_str_opt(&a, "user_id"),
                arg_str_opt(&a, "challenge"),
                arg_str_opt(&a, "origin"),
                arg_bool(&a, "force", false),
            ),
        )
        .tool(
            "passkey_assert",
            "Sign in with a saved passkey. id is the passkey name. challenge is the challenge the website just showed. Returns the signature the page needs.",
            json!({"type": "object", "required": ["id", "challenge"], "properties": {
                "id": {"type": "string", "description": "passkey name from secrets_list"},
                "challenge": {"type": "string", "description": "the challenge the website just showed"},
                "origin": {"type": "string", "description": "optional page address"},
                "rp_id": {"type": "string", "description": "optional site name, if it differs from the saved one"}
            }}),
            |_, a| store::passkey_assert(
                arg_str(&a, "id")?,
                arg_str(&a, "challenge")?,
                arg_str_opt(&a, "origin"),
                arg_str_opt(&a, "rp_id"),
            ),
        )
        .run();
}
