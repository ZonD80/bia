//! Transcribe a received voice note the way the MCPs do (auto-learning a new
//! language), then wait for the background voice install.
//!   note-debug <file> [preferred-lang]
use bia_speech::registry;

fn main() {
    let path = std::env::args().nth(1).expect("file");
    // optional 2nd argument: the sender's preferred language (starting point)
    let hint = std::env::args().nth(2);
    let t = std::time::Instant::now();
    let heard = bia_speech::transcribe_note_from(std::path::Path::new(&path), hint.as_deref()).unwrap();
    println!("[{} ms] {}", t.elapsed().as_millis(), heard.text());
    println!("notice: {:?}", heard.notice);
    if let Some(l) = heard.new_lang {
        for _ in 0..120 {
            let reg = registry::current();
            if reg.voice.contains_key(&l) {
                println!("voice for {l} installed after {} s: {}", t.elapsed().as_secs(), reg.voice[&l]);
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        let reg = registry::current();
        println!("enabled={:?} voice={:?} asr_file={}", reg.enabled, reg.voice.get(&l), reg.asr_file);
    }
}
