//! "system" MCP: FMM (with Login with ChatGPT), HITL tasks and file links. Disks, network, curl and ss are Debian via bash.
//! The web server, its certificates and chats are the web MCP.
mod chatgpt;
mod fmm;
mod hitl;
#[path = "../../listen.rs"]
mod listen;

use bia_mcp::{arg_str_opt, arg_u32_opt, Server};
use serde_json::json;

fn main() {
    Server::new("system", env!("CARGO_PKG_VERSION"))
        .tool("fmm_configure", "Turn on a cloud model instead of the local one. api_key is required, except provider=chatgpt without a key, which switches back to the ChatGPT sign-in kept from fmm_chatgpt (no new sign-in). provider is openrouter, openai, anthropic, xai, deepseek, qwen or chatgpt; leave it out and it is detected from the key. model is that provider's id, for example x-ai/grok-4.6, gpt-5, claude-opus-4-6, grok-4.6, deepseek-chat, or qwen-max; leave it out for the provider's default. The new model is tested with this conversation before the switch; if that fails, or its first real reply fails, the old settings come back. Settings are saved and used again at next boot.",
            json!({"type": "object", "properties": {
                "provider": {"type": "string", "description": "optional: openrouter, openai, anthropic, xai, deepseek, qwen, or chatgpt (saved sign-in, no key). Detected from the key when left out."},
                "api_key": {"type": "string", "description": "the provider's secret key (not for provider=chatgpt)"},
                "model": {"type": "string", "description": "optional: the provider's model id, for example x-ai/grok-4.6"},
                "base_url": {"type": "string", "description": "optional. Only if the API address is not the default."},
                "ctx": {"type": "integer", "description": "optional. How many tokens of context. Default 131072."}}}),
            |_, a| fmm::configure(arg_str_opt(&a, "provider").unwrap_or(""), arg_str_opt(&a, "api_key").unwrap_or(""), arg_str_opt(&a, "model").unwrap_or(""), arg_str_opt(&a, "base_url"), arg_u32_opt(&a, "ctx")).map_err(|e| format!("{e:#}")))
        .tool("fmm_chatgpt", "Let the owner switch the cloud model to their ChatGPT plan (Plus, Pro, Business) with a Login with ChatGPT button: no API key. Serves a page with the button on bia (127.0.0.1) and a web console task page that shows bia's screen. You get a job id now and, right after, a system|fmm message: then open that page in Chrome with chrome_new_page (mcp/chrome) and tell the person with say to switch to Chrome and press Login with ChatGPT. After the sign-in the page lists the account's models and they press Continue; the result arrives as system|job:<id>. Never press the button or sign in yourself. Needs Chrome on bia: if it is missing the call fails: enable mcp/chrome, run chrome_install and follow it, then call again. Only when root asks.",
            json!({"type": "object", "properties": {
                "model": {"type": "string", "description": "optional: model to preselect, only if the user named one. Default: the gpt-6.1-sol model. The person can still pick another."},
                "email": {"type": "string", "description": "optional: the ChatGPT account's email, if the user said it"},
                "timeout_secs": {"type": "integer", "description": "optional: give up after this many seconds. Default 1800, 0 waits forever."}}}),
            chatgpt::login)
        .tool("fmm_back", "Go back to the cloud model settings from before the last switch (provider, key, model). Calling it again swaps back. Use it when a new model does not work.", json!({"type": "object", "properties": {}}),
            |_, _| fmm::back().map_err(|e| format!("{e:#}")))
        .tool("fmm_enable", "Turn the saved cloud model back on.", json!({"type": "object", "properties": {}}),
            |_, _| fmm::set_enabled(true).map_err(|e| format!("{e:#}")))
        .tool("fmm_disable", "Go back to the local model. The saved cloud settings stay.", json!({"type": "object", "properties": {}}),
            |_, _| fmm::set_enabled(false).map_err(|e| format!("{e:#}")))
        .tool("fmm_status", "Show the cloud model settings. The key is hidden.", json!({"type": "object", "properties": {}}),
            |_, _| Ok(fmm::status()))
        .tool("fmm_forget", "Delete the saved cloud model settings and the key.", json!({"type": "object", "properties": {}}),
            |_, _| Ok(fmm::forget()))
        .tool(
            "share_file",
            "Make a download link for a file under /home/bia/files. path example: out/report.pdf. Send the returned link with say. Do not change the link.",
            json!({"type": "object", "required": ["path"], "properties": {
                "path": {"type": "string", "description": "file under /home/bia/files, for example out/report.pdf"}
            }}),
            hitl::share,
        )
        .tool(
            "ask_human",
            "Ask the person to finish a step you cannot do, such as a captcha. You get a job id now. The result arrives later as system|job:<id>. Send the link from the log to the user with say.",
            json!({"type": "object", "required": ["title"], "properties": {
                "title": {"type": "string", "description": "short name the person will see"},
                "message": {"type": "string", "description": "what the person should do"},
                "timeout_secs": {"type": "integer", "description": "give up after this many seconds. 0 waits until they press Done."}
            }}),
            hitl::ask,
        )
        .run();
}
