//! Streaming TTS for calls on one worker thread: bia-speech synthesizes each sentence
//! with the voice of its language (`lang` + inline <lang=xx> spans), and every chunk
//! is resampled to 48 kHz and queued on the call's pacer while the next one is made.

use crate::audio_out::{self, Feed};
use crate::client::Tg;
use std::sync::mpsc;
use std::sync::OnceLock;

enum Job {
    Warm,
    Speak { text: String, lang: String, chat_id: i64, feed: Feed },
}

fn worker() -> &'static mpsc::Sender<Job> {
    static TX: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("tts".into())
            .spawn(move || run(rx))
            .expect("spawn tts thread");
        tx
    })
}

/// Load the call models and voices in the background so the first `speak` is fast.
pub fn warm() {
    let _ = worker().send(Job::Warm);
}

/// Synthesize `text` (in `lang`) into `feed` asynchronously; returns immediately.
/// Check the voices with `bia_speech::tts::plan` first so errors reach the caller.
pub fn speak(text: String, lang: String, chat_id: i64, feed: Feed) {
    let _ = worker().send(Job::Speak { text, lang, chat_id, feed });
}

fn run(rx: mpsc::Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        let Job::Speak { text, lang, chat_id, feed } = job else {
            bia_speech::warm();
            continue;
        };
        if !feed.is_live() {
            feed.finish();
            continue;
        }
        let result = bia_speech::tts::speak(&text, &lang, &mut |chunk, rate| {
            feed.push(&audio_out::to_call_pcm(chunk, rate))
        });
        feed.finish();
        if let Err(e) = result {
            Tg::global().inbound("system", chat_id, "", &format!("[voice failed: {e}]"));
        }
    }
}
