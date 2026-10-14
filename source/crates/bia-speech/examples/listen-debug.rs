//! Feed a recording to a call `Listener` as 48 kHz / 20 ms frames (like Telegram call
//! audio) and print each utterance with its transcript and timing.
//!   listen-debug <file>...   (files are played one after another with 1.5 s gaps)
use bia_speech::{audio, Listener};
use std::time::Instant;

fn main() {
    let mut x = Vec::new();
    for f in std::env::args().skip(1) {
        x.extend(audio::decode_file(std::path::Path::new(&f)).unwrap());
        x.extend(vec![0.0f32; 24_000]);
    }
    // 16 kHz → 48 kHz (sample repeat is fine for a VAD/ASR test) as i16 frames.
    let pcm48: Vec<i16> = x
        .iter()
        .flat_map(|s| std::iter::repeat((s * 32767.0) as i16).take(3))
        .collect();
    let mut l = Listener::new(48_000).unwrap();
    let t0 = Instant::now();
    for (i, frame) in pcm48.chunks(960).enumerate() {
        if let Some(u) = l.push(frame) {
            let at = i as f32 * 0.02;
            let secs = u.secs();
            let t = Instant::now();
            let heard = u.transcribe(None, None).unwrap();
            println!(
                "utterance ends by {at:.2}s ({secs:.2}s speech), transcribed in {} ms: {}{}",
                t.elapsed().as_millis(),
                heard.text(),
                heard.new_lang.map(|l| format!("  [new language: {l}]")).unwrap_or_default()
            );
            for d in heard.dropped {
                println!("    dropped: {d}");
            }
        }
    }
    println!("total wall {} ms for {:.1}s audio", t0.elapsed().as_millis(), x.len() as f32 / 16000.0);
}
