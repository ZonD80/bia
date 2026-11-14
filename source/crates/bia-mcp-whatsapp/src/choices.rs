//! Messages that offer options: polls, buttons, lists, template and native-flow buttons.
//!
//! `offer` turns one into text with numbered options (what the model reads) plus a
//! `Choice` (what `whatsapp_choose` needs to answer it). Choices are kept per message in
//! `whatsapp/choices.json`; `answer` builds the reply message for a picked option.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use whatsapp_rust::prelude::*;

/// Newest choices kept on disk.
const KEEP: usize = 400;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Opt {
    pub text: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub desc: String,
    /// A row of a native-flow list (answered differently from a native-flow button).
    #[serde(default)]
    pub row: bool,
    /// Position among the buttons of its message (templates answer with it).
    #[serde(default)]
    pub index: u32,
    /// Card of a carousel the button sits on.
    #[serde(default)]
    pub card: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Choice {
    /// poll | buttons | list | template | flow
    pub kind: String,
    #[serde(default)]
    pub title: String,
    pub options: Vec<Opt>,
    /// Poll: how many options one vote may hold (0 = any number).
    #[serde(default)]
    pub max: u32,
    /// Poll: hex of the message secret votes are encrypted with.
    #[serde(default)]
    pub secret: String,
    /// Who sent the message (the poll's creator).
    #[serde(default)]
    pub from_jid: String,
    #[serde(default)]
    pub ts: i64,
}

fn s(o: &Option<String>) -> &str {
    o.as_deref().map(str::trim).unwrap_or("")
}

fn push_line(out: &mut Vec<String>, line: &str) {
    let line = line.trim();
    if !line.is_empty() {
        out.push(line.to_string());
    }
}

fn numbered(out: &mut Vec<String>, options: &[Opt], from: usize) {
    for (n, o) in options.iter().enumerate().skip(from) {
        if o.desc.is_empty() {
            out.push(format!("{}. {}", n + 1, o.text));
        } else {
            out.push(format!("{}. {} — {}", n + 1, o.text, o.desc));
        }
    }
}

/// The poll of a message, whichever version of the field carries it.
fn poll_of(base: &wa::Message) -> Option<&wa::message::PollCreationMessage> {
    base.poll_creation_message
        .as_option()
        .or_else(|| base.poll_creation_message_v2.as_option())
        .or_else(|| base.poll_creation_message_v3.as_option())
        .or_else(|| base.poll_creation_message_v5.as_option())
        .or_else(|| base.poll_creation_message_v6.as_option())
        .or_else(|| base.poll_creation_message_v4.as_option().and_then(|w| w.message.as_option()).and_then(poll_of))
}

fn poll(p: &wa::message::PollCreationMessage) -> (String, Choice) {
    let options: Vec<Opt> = p
        .options
        .iter()
        .filter_map(|o| o.option_name.clone())
        .filter(|n| !n.trim().is_empty())
        .map(|text| Opt { text, ..Default::default() })
        .collect();
    let quiz = p.correct_answer.is_set();
    let max = if quiz { 1 } else { p.selectable_options_count.unwrap_or(0) };
    let pick = match max {
        0 => "pick any".to_string(),
        1 => "pick one".to_string(),
        n => format!("pick up to {n}"),
    };
    let mut out = vec![format!("[{}] {} ({pick})", if quiz { "quiz" } else { "poll" }, s(&p.name))];
    numbered(&mut out, &options, 0);
    let choice = Choice { kind: "poll".into(), title: s(&p.name).to_string(), options, max, ..Default::default() };
    (out.join("\n"), choice)
}

fn buttons(b: &wa::message::ButtonsMessage) -> (String, Choice) {
    use wa::message::buttons_message::Header;
    let mut out = Vec::new();
    if let Some(Header::Text(t)) = &b.header {
        push_line(&mut out, t);
    }
    push_line(&mut out, s(&b.content_text));
    push_line(&mut out, s(&b.footer_text));
    let options: Vec<Opt> = b
        .buttons
        .iter()
        .enumerate()
        .filter_map(|(i, x)| {
            let text = x.button_text.as_option().map(|t| s(&t.display_text).to_string()).unwrap_or_default();
            (!text.is_empty()).then(|| Opt { text, id: s(&x.button_id).to_string(), index: i as u32, ..Default::default() })
        })
        .collect();
    if !options.is_empty() {
        out.push("[buttons]".into());
        numbered(&mut out, &options, 0);
    }
    (out.join("\n"), Choice { kind: "buttons".into(), title: s(&b.content_text).to_string(), options, ..Default::default() })
}

fn list(l: &wa::message::ListMessage) -> (String, Choice) {
    let mut out = Vec::new();
    push_line(&mut out, s(&l.title));
    push_line(&mut out, s(&l.description));
    push_line(&mut out, s(&l.footer_text));
    let mut options: Vec<Opt> = Vec::new();
    let mut rows = Vec::new();
    for sec in &l.sections {
        let from = options.len();
        for r in &sec.rows {
            let text = s(&r.title).to_string();
            if text.is_empty() {
                continue;
            }
            options.push(Opt {
                text,
                id: s(&r.row_id).to_string(),
                desc: s(&r.description).to_string(),
                index: options.len() as u32,
                ..Default::default()
            });
        }
        if options.len() > from {
            if !s(&sec.title).is_empty() {
                rows.push(format!("{}:", s(&sec.title)));
            }
            numbered(&mut rows, &options, from);
        }
    }
    if !options.is_empty() {
        out.push(format!("[list] {}", s(&l.button_text)).trim().to_string());
        out.extend(rows);
    }
    (out.join("\n"), Choice { kind: "list".into(), title: s(&l.title).to_string(), options, ..Default::default() })
}

fn hydrated(t: &wa::message::template_message::HydratedFourRowTemplate) -> (String, Choice) {
    use wa::hydrated_template_button::HydratedButton;
    use wa::message::template_message::hydrated_four_row_template::Title;
    let mut out = Vec::new();
    if let Some(Title::HydratedTitleText(x)) = &t.title {
        push_line(&mut out, x);
    }
    push_line(&mut out, s(&t.hydrated_content_text));
    push_line(&mut out, s(&t.hydrated_footer_text));
    let mut options: Vec<Opt> = Vec::new();
    let mut links = Vec::new();
    for (i, b) in t.hydrated_buttons.iter().enumerate() {
        match &b.hydrated_button {
            Some(HydratedButton::QuickReplyButton(q)) if !s(&q.display_text).is_empty() => options.push(Opt {
                text: s(&q.display_text).to_string(),
                id: s(&q.id).to_string(),
                index: b.index.unwrap_or(i as u32),
                ..Default::default()
            }),
            Some(HydratedButton::UrlButton(u)) => links.push(format!("- {}: {}", s(&u.display_text), s(&u.url))),
            Some(HydratedButton::CallButton(c)) => links.push(format!("- {}: {}", s(&c.display_text), s(&c.phone_number))),
            _ => {}
        }
    }
    if !options.is_empty() {
        out.push("[buttons]".into());
        numbered(&mut out, &options, 0);
    }
    out.extend(links);
    (out.join("\n"), Choice { kind: "template".into(), title: s(&t.hydrated_content_text).to_string(), options, ..Default::default() })
}

/// One interactive message (or one card of a carousel): its text lines, and its
/// native-flow buttons and list rows added to `options`.
fn interactive_into(m: &wa::message::InteractiveMessage, card: Option<u32>, out: &mut Vec<String>, options: &mut Vec<Opt>) {
    use wa::message::interactive_message::InteractiveMessage as Payload;
    if let Some(h) = m.header.as_option() {
        push_line(out, s(&h.title));
        push_line(out, s(&h.subtitle));
    }
    if let Some(b) = m.body.as_option() {
        push_line(out, s(&b.text));
    }
    if let Some(f) = m.footer.as_option() {
        push_line(out, s(&f.text));
    }
    match &m.interactive_message {
        Some(Payload::NativeFlowMessage(nf)) => {
            let mut picks = Vec::new();
            let mut links = Vec::new();
            for (i, b) in nf.buttons.iter().enumerate() {
                let p: serde_json::Value = serde_json::from_str(b.button_params_json.as_deref().unwrap_or("")).unwrap_or_default();
                let get = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
                let name = s(&b.name);
                match name {
                    "quick_reply" => {
                        let from = options.len();
                        options.push(Opt { text: get("display_text"), id: get("id"), index: i as u32, card, ..Default::default() });
                        numbered(&mut picks, options, from);
                    }
                    "single_select" => {
                        push_line(&mut picks, &format!("[list] {}", get("title")));
                        for sec in p.get("sections").and_then(|v| v.as_array()).into_iter().flatten() {
                            let from = options.len();
                            let str_of = |r: &serde_json::Value, k: &str| r.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
                            for r in sec.get("rows").and_then(|v| v.as_array()).into_iter().flatten() {
                                let text = [str_of(r, "header"), str_of(r, "title")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
                                if text.is_empty() {
                                    continue;
                                }
                                options.push(Opt { text, id: str_of(r, "id"), desc: str_of(r, "description"), row: true, index: i as u32, card });
                            }
                            if options.len() > from {
                                let title = str_of(sec, "title");
                                if !title.is_empty() {
                                    picks.push(format!("{title}:"));
                                }
                                numbered(&mut picks, options, from);
                            }
                        }
                    }
                    _ => {
                        let what = ["url", "phone_number", "copy_code"].iter().map(|k| get(k)).find(|v| !v.is_empty()).unwrap_or_default();
                        let label = get("display_text");
                        let label = if label.is_empty() { name.to_string() } else { label };
                        if !label.is_empty() {
                            links.push(if what.is_empty() { format!("- {label} (opens in the WhatsApp app only)") } else { format!("- {label}: {what}") });
                        }
                    }
                }
            }
            if !picks.is_empty() && !picks[0].starts_with("[list]") {
                out.push("[buttons]".into());
            }
            out.extend(picks);
            out.extend(links);
        }
        Some(Payload::CarouselMessage(c)) => {
            for (i, cardmsg) in c.cards.iter().enumerate() {
                out.push(format!("[card {}]", i + 1));
                interactive_into(cardmsg, Some(i as u32), out, options);
            }
        }
        _ => {}
    }
}

fn interactive(m: &wa::message::InteractiveMessage) -> (String, Choice) {
    let mut out = Vec::new();
    let mut options = Vec::new();
    interactive_into(m, None, &mut out, &mut options);
    let title = m.body.as_option().map(|b| s(&b.text).to_string()).unwrap_or_default();
    (out.join("\n"), Choice { kind: "flow".into(), title, options, ..Default::default() })
}

fn template(t: &wa::message::TemplateMessage) -> Option<(String, Choice)> {
    use wa::message::template_message::Format;
    if let Some(h) = t.hydrated_template.as_option() {
        return Some(hydrated(h));
    }
    match &t.format {
        Some(Format::HydratedFourRowTemplate(h)) => Some(hydrated(h)),
        Some(Format::InteractiveMessageTemplate(m)) => Some(interactive(m)),
        _ => None,
    }
}

/// Text (with numbered options) and the choice of a message that offers options;
/// None for every other message. The choice has no options when there is nothing to pick
/// (only links, say): the text still tells what was sent.
pub fn offer(base: &wa::Message) -> Option<(String, Choice)> {
    let found = if let Some(p) = poll_of(base) {
        Some(poll(p))
    } else if let Some(b) = base.buttons_message.as_option() {
        Some(buttons(b))
    } else if let Some(l) = base.list_message.as_option() {
        Some(list(l))
    } else if let Some(m) = base.interactive_message.as_option() {
        Some(interactive(m))
    } else if let Some(t) = base.template_message.as_option() {
        template(t)
    } else {
        base.highly_structured_message.as_option().and_then(|h| h.hydrated_hsm.as_option()).and_then(template)
    };
    found.filter(|(text, _)| !text.trim().is_empty())
}


/// A picture, video or document shown above the text of a buttons, template or
/// interactive message.
pub enum HeaderMedia<'a> {
    Image(&'a wa::message::ImageMessage),
    Video(&'a wa::message::VideoMessage),
    Document(&'a wa::message::DocumentMessage),
}

impl HeaderMedia<'_> {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Image(_) => "image",
            Self::Video(_) => "video",
            Self::Document(_) => "document",
        }
    }
}

fn interactive_media<'a>(m: &'a wa::message::InteractiveMessage, card: Option<u32>, out: &mut Vec<(Option<u32>, HeaderMedia<'a>)>) {
    use wa::message::interactive_message::header::Media;
    use wa::message::interactive_message::InteractiveMessage as Payload;
    match m.header.as_option().and_then(|h| h.media.as_ref()) {
        Some(Media::ImageMessage(x)) => out.push((card, HeaderMedia::Image(x))),
        Some(Media::VideoMessage(x)) => out.push((card, HeaderMedia::Video(x))),
        Some(Media::DocumentMessage(x)) => out.push((card, HeaderMedia::Document(x))),
        _ => {}
    }
    if let Some(Payload::CarouselMessage(c)) = &m.interactive_message {
        for (i, cardmsg) in c.cards.iter().enumerate() {
            interactive_media(cardmsg, Some(i as u32), out);
        }
    }
}

fn template_media<'a>(t: &'a wa::message::TemplateMessage, out: &mut Vec<(Option<u32>, HeaderMedia<'a>)>) {
    use wa::message::template_message::hydrated_four_row_template::Title;
    use wa::message::template_message::Format;
    let hydrated = t.hydrated_template.as_option().or(match &t.format {
        Some(Format::HydratedFourRowTemplate(h)) => Some(&**h),
        _ => None,
    });
    if let Some(h) = hydrated {
        match &h.title {
            Some(Title::ImageMessage(x)) => out.push((None, HeaderMedia::Image(x))),
            Some(Title::VideoMessage(x)) => out.push((None, HeaderMedia::Video(x))),
            Some(Title::DocumentMessage(x)) => out.push((None, HeaderMedia::Document(x))),
            _ => {}
        }
    } else if let Some(Format::InteractiveMessageTemplate(m)) = &t.format {
        interactive_media(m, None, out);
    }
}

/// The header media of a message that offers options, in the order shown; with the
/// carousel card (from 0) each one sits on.
pub fn header_media(base: &wa::Message) -> Vec<(Option<u32>, HeaderMedia<'_>)> {
    use wa::message::buttons_message::Header;
    let mut out = Vec::new();
    if let Some(b) = base.buttons_message.as_option() {
        match &b.header {
            Some(Header::ImageMessage(x)) => out.push((None, HeaderMedia::Image(x))),
            Some(Header::VideoMessage(x)) => out.push((None, HeaderMedia::Video(x))),
            Some(Header::DocumentMessage(x)) => out.push((None, HeaderMedia::Document(x))),
            _ => {}
        }
    } else if let Some(m) = base.interactive_message.as_option() {
        interactive_media(m, None, &mut out);
    } else if let Some(t) = base.template_message.as_option() {
        template_media(t, &mut out);
    } else if let Some(t) = base.highly_structured_message.as_option().and_then(|h| h.hydrated_hsm.as_option()) {
        template_media(t, &mut out);
    }
    out
}

/// `[chose] …` for a message that answers buttons or a list (someone pressed one).
pub fn answer_text(base: &wa::Message) -> Option<String> {
    use wa::message::buttons_response_message::Response;
    let picked = if let Some(b) = base.buttons_response_message.as_option() {
        match &b.response {
            Some(Response::SelectedDisplayText(t)) => t.trim().to_string(),
            None => s(&b.selected_button_id).to_string(),
        }
    } else if let Some(l) = base.list_response_message.as_option() {
        s(&l.title).to_string()
    } else if let Some(t) = base.template_button_reply_message.as_option() {
        let text = s(&t.selected_display_text);
        if text.is_empty() { s(&t.selected_id) } else { text }.to_string()
    } else if let Some(r) = base.interactive_response_message.as_option() {
        r.body.as_option().map(|b| s(&b.text).to_string()).unwrap_or_default()
    } else {
        return None;
    };
    (!picked.is_empty()).then(|| format!("[chose] {picked}"))
}

/// The context (reply, forward) of a message that offers or answers options.
pub fn context_info(base: &wa::Message) -> Option<&wa::ContextInfo> {
    macro_rules! first {
        ($($f:ident),+) => {{
            None$(.or_else(|| base.$f.as_option().and_then(|m| m.context_info.as_option())))+
        }};
    }
    first!(
        buttons_message,
        list_message,
        interactive_message,
        template_message,
        buttons_response_message,
        list_response_message,
        template_button_reply_message,
        interactive_response_message
    )
    .or_else(|| poll_of(base).and_then(|p| p.context_info.as_option()))
}

/// The indexes of `picks` (option numbers as shown, or option texts) among `options`.
pub fn pick(options: &[Opt], picks: &[String]) -> Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for p in picks {
        let p = p.trim().trim_end_matches('.');
        let found = match p.parse::<usize>() {
            Ok(n) if n >= 1 && n <= options.len() => Some(n - 1),
            Ok(n) => return Err(format!("no option {n}: this message has options 1 to {}", options.len())),
            Err(_) => options.iter().position(|o| o.text.trim().eq_ignore_ascii_case(p) || o.text.trim().to_lowercase() == p.to_lowercase()),
        };
        let Some(i) = found else {
            let all: Vec<String> = options.iter().enumerate().map(|(i, o)| format!("{}. {}", i + 1, o.text)).collect();
            return Err(format!("no option \"{p}\". The options are: {}", all.join("; ")));
        };
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// The message that answers `choice` (not a poll) with its option `i`, as a reply to
/// message `id`.
pub fn answer(choice: &Choice, i: usize, id: &str, group: bool) -> wa::Message {
    let o = &choice.options[i];
    let ctx = buffa::MessageField::some(wa::ContextInfo {
        stanza_id: Some(id.to_string()),
        participant: (group && !choice.from_jid.is_empty()).then(|| choice.from_jid.clone()),
        ..Default::default()
    });
    let some = |v: &str| Some(v.to_string());
    match choice.kind.as_str() {
        "buttons" => wa::Message {
            buttons_response_message: buffa::MessageField::some(wa::message::ButtonsResponseMessage {
                selected_button_id: some(&o.id),
                context_info: ctx,
                r#type: Some(wa::message::buttons_response_message::Type::DISPLAY_TEXT.into()),
                response: Some(wa::message::buttons_response_message::Response::SelectedDisplayText(o.text.clone())),
                ..Default::default()
            }),
            ..Default::default()
        },
        "list" => wa::Message {
            list_response_message: buffa::MessageField::some(wa::message::ListResponseMessage {
                title: some(&o.text),
                list_type: Some(wa::message::list_response_message::ListType::SINGLE_SELECT.into()),
                single_select_reply: buffa::MessageField::some(wa::message::list_response_message::SingleSelectReply {
                    selected_row_id: some(&o.id),
                    ..Default::default()
                }),
                context_info: ctx,
                description: (!o.desc.is_empty()).then(|| o.desc.clone()),
                ..Default::default()
            }),
            ..Default::default()
        },
        "flow" if o.row => wa::Message {
            interactive_response_message: buffa::MessageField::some(wa::message::InteractiveResponseMessage {
                body: buffa::MessageField::some(wa::message::interactive_response_message::Body {
                    text: some(&o.text),
                    ..Default::default()
                }),
                context_info: ctx,
                interactive_response_message: Some(
                    wa::message::interactive_response_message::InteractiveResponseMessage::NativeFlowResponseMessage(Box::new(
                        wa::message::interactive_response_message::NativeFlowResponseMessage {
                            name: some("single_select"),
                            params_json: Some(serde_json::json!({ "id": o.id }).to_string()),
                            version: Some(1),
                            ..Default::default()
                        },
                    )),
                ),
                ..Default::default()
            }),
            ..Default::default()
        },
        // template quick replies; the WhatsApp apps answer native-flow quick_reply buttons the same way
        _ => wa::Message {
            template_button_reply_message: buffa::MessageField::some(wa::message::TemplateButtonReplyMessage {
                selected_id: some(&o.id),
                selected_display_text: some(&o.text),
                context_info: ctx,
                selected_index: Some(o.index),
                selected_carousel_card_index: o.card,
                ..Default::default()
            }),
            ..Default::default()
        },
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2).filter_map(|i| u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()).collect()
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

fn path() -> PathBuf {
    crate::client::Wa::wa_dir().join("choices.json")
}

fn store() -> &'static Mutex<HashMap<String, Choice>> {
    static S: OnceLock<Mutex<HashMap<String, Choice>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(bia_mcp::statefile::read_or_default(&path())))
}

/// Remember the choice of message `id` in chat `key`.
pub fn remember(key: &str, id: &str, choice: Choice) {
    if choice.options.is_empty() {
        return;
    }
    let mut m = store().lock().unwrap();
    m.insert(format!("{key}|{id}"), choice);
    if m.len() > KEEP {
        let mut by_age: Vec<(i64, String)> = m.iter().map(|(k, c)| (c.ts, k.clone())).collect();
        by_age.sort();
        let extra = by_age.len() - KEEP;
        for (_, k) in by_age.into_iter().take(extra) {
            m.remove(&k);
        }
    }
    let _ = bia_mcp::statefile::write_json(&path(), &*m);
}

pub fn get(key: &str, id: &str) -> Option<Choice> {
    store().lock().unwrap().get(&format!("{key}|{id}")).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll_msg(count: u32) -> wa::Message {
        wa::Message {
            poll_creation_message_v3: buffa::MessageField::some(wa::message::PollCreationMessage {
                name: Some("Where to eat?".into()),
                options: ["Pizza", "Sushi"]
                    .iter()
                    .map(|n| wa::message::poll_creation_message::Option { option_name: Some(n.to_string()), ..Default::default() })
                    .collect(),
                selectable_options_count: Some(count),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn poll_lists_its_options() {
        let (text, c) = offer(&poll_msg(1)).unwrap();
        assert_eq!(text, "[poll] Where to eat? (pick one)\n1. Pizza\n2. Sushi");
        assert_eq!((c.kind.as_str(), c.max, c.options.len()), ("poll", 1, 2));
        assert!(offer(&poll_msg(0)).unwrap().0.starts_with("[poll] Where to eat? (pick any)"));
    }

    #[test]
    fn buttons_and_their_answer() {
        let b = |id: &str, text: &str| wa::message::buttons_message::Button {
            button_id: Some(id.into()),
            button_text: buffa::MessageField::some(wa::message::buttons_message::button::ButtonText {
                display_text: Some(text.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let m = wa::Message {
            buttons_message: buffa::MessageField::some(wa::message::ButtonsMessage {
                content_text: Some("Confirm the order?".into()),
                footer_text: Some("Shop".into()),
                buttons: vec![b("y", "Yes"), b("n", "No")],
                ..Default::default()
            }),
            ..Default::default()
        };
        let (text, mut c) = offer(&m).unwrap();
        assert_eq!(text, "Confirm the order?\nShop\n[buttons]\n1. Yes\n2. No");
        c.from_jid = "123@s.whatsapp.net".into();
        assert_eq!(pick(&c.options, &["2".into()]).unwrap(), vec![1]);
        assert_eq!(pick(&c.options, &["yes".into()]).unwrap(), vec![0]);
        assert!(pick(&c.options, &["3".into()]).is_err());
        assert!(pick(&c.options, &["maybe".into()]).is_err());
        let a = answer(&c, 1, "ABC", false);
        let r = a.buttons_response_message.as_option().unwrap();
        assert_eq!(r.selected_button_id.as_deref(), Some("n"));
        assert_eq!(r.context_info.as_option().unwrap().stanza_id.as_deref(), Some("ABC"));
        assert_eq!(r.context_info.as_option().unwrap().participant, None);
        assert_eq!(answer_text(&a).as_deref(), Some("[chose] No"));
    }

    #[test]
    fn list_rows_are_numbered_across_sections() {
        let row = |id: &str, t: &str, d: &str| wa::message::list_message::Row {
            title: Some(t.into()),
            description: Some(d.into()),
            row_id: Some(id.into()),
            ..Default::default()
        };
        let m = wa::Message {
            list_message: buffa::MessageField::some(wa::message::ListMessage {
                title: Some("Menu".into()),
                button_text: Some("Open".into()),
                sections: vec![
                    wa::message::list_message::Section { title: Some("Food".into()), rows: vec![row("p", "Pizza", "hot")], ..Default::default() },
                    wa::message::list_message::Section { title: Some("Drinks".into()), rows: vec![row("t", "Tea", "")], ..Default::default() },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let (text, c) = offer(&m).unwrap();
        assert_eq!(text, "Menu\n[list] Open\nFood:\n1. Pizza — hot\nDrinks:\n2. Tea");
        let a = answer(&c, 1, "ID", true);
        let r = a.list_response_message.as_option().unwrap();
        assert_eq!(r.single_select_reply.as_option().unwrap().selected_row_id.as_deref(), Some("t"));
        assert_eq!(answer_text(&a).as_deref(), Some("[chose] Tea"));
    }

    #[test]
    fn native_flow_buttons_links_and_rows() {
        use wa::message::interactive_message as im;
        let btn = |name: &str, params: serde_json::Value| im::native_flow_message::NativeFlowButton {
            name: Some(name.into()),
            button_params_json: Some(params.to_string()),
            ..Default::default()
        };
        let m = wa::Message {
            interactive_message: buffa::MessageField::some(wa::message::InteractiveMessage {
                body: buffa::MessageField::some(im::Body { text: Some("How can we help?".into()), ..Default::default() }),
                interactive_message: Some(im::InteractiveMessage::NativeFlowMessage(Box::new(im::NativeFlowMessage {
                    buttons: vec![
                        btn("quick_reply", serde_json::json!({"display_text": "Track order", "id": "track"})),
                        btn("cta_url", serde_json::json!({"display_text": "Site", "url": "https://example.com"})),
                        btn("single_select", serde_json::json!({"title": "More", "sections": [{"title": "Help", "rows": [{"title": "Refund", "id": "refund", "description": "money back"}]}]})),
                    ],
                    ..Default::default()
                }))),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (text, c) = offer(&m).unwrap();
        assert_eq!(
            text,
            "How can we help?\n[buttons]\n1. Track order\n[list] More\nHelp:\n2. Refund — money back\n- Site: https://example.com"
        );
        let quick = answer(&c, 0, "ID", false);
        assert_eq!(quick.template_button_reply_message.as_option().unwrap().selected_id.as_deref(), Some("track"));
        let row = answer(&c, 1, "ID", false);
        assert_eq!(answer_text(&row).as_deref(), Some("[chose] Refund"));
        assert!(row.interactive_response_message.is_set());
    }

    #[test]
    fn header_pictures_are_found() {
        use wa::message::interactive_message as im;
        let img = || Box::new(wa::message::ImageMessage { mimetype: Some("image/jpeg".into()), ..Default::default() });
        let card = || wa::message::InteractiveMessage {
            header: buffa::MessageField::some(im::Header { media: Some(im::header::Media::ImageMessage(img())), ..Default::default() }),
            ..Default::default()
        };
        let carousel = wa::Message {
            interactive_message: buffa::MessageField::some(wa::message::InteractiveMessage {
                interactive_message: Some(im::InteractiveMessage::CarouselMessage(Box::new(im::CarouselMessage {
                    cards: vec![card(), card()],
                    ..Default::default()
                }))),
                ..Default::default()
            }),
            ..Default::default()
        };
        let found = header_media(&carousel);
        assert_eq!(found.iter().map(|(c, m)| (*c, m.kind())).collect::<Vec<_>>(), vec![(Some(0), "image"), (Some(1), "image")]);
        let buttons = wa::Message {
            buttons_message: buffa::MessageField::some(wa::message::ButtonsMessage {
                header: Some(wa::message::buttons_message::Header::ImageMessage(img())),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(header_media(&buttons).len(), 1);
        assert!(header_media(&poll_msg(1)).is_empty());
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(unhex(&hex(&[0, 1, 0xab, 0xff])), vec![0, 1, 0xab, 0xff]);
    }
}
