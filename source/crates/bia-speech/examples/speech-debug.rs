//! Print VAD segments, detected language and text per segment for an audio file.
//!   speech-debug <file> [min_silence]
use bia_speech::{audio, models, registry};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = std::path::Path::new(&args[1]);
    let min_sil: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.25);
    let reg = registry::current();
    let mut x = vec![0.0f32; 16000];
    x.extend(audio::decode_file(path).unwrap());
    if std::env::var("DENOISE").is_ok() {
        x = models::denoiser(&reg).unwrap().lock().unwrap().run(&x).unwrap();
    }
    println!("{:.2}s audio", x.len() as f32 / 16000.0);
    {
        let mut vad = models::Vad::new(&reg, min_sil, 20.0).unwrap();
        let mut was = false;
        for (i, c) in x.chunks(512).enumerate() {
            let segs = vad.accept(c);
            let now = vad.speaking();
            if now != was {
                println!("  t={:.2}s detected={now}", (i * 512) as f32 / 16000.0);
                was = now;
            }
            for s in segs {
                println!("  t={:.2}s segment start={:.2}s dur={:.2}s", (i * 512) as f32 / 16000.0, s.start as f32 / 16000.0, s.samples.len() as f32 / 16000.0);
            }
        }
        for s in vad.flush() {
            println!("  flush segment start={:.2}s dur={:.2}s", s.start as f32 / 16000.0, s.samples.len() as f32 / 16000.0);
        }
    }
    let segs = models::segments(&reg, &x, min_sil).unwrap();
    let slid = models::slid(&reg).unwrap();
    let rec = models::recognizer(&reg, &reg.asr_file).unwrap();
    println!("  whole file: {}", rec.lock().unwrap().decode(&x, "").unwrap_or_default());
    for s in segs {
        let d = s.samples.len() as f32 / 16000.0;
        let l = slid.lock().unwrap().detect(&s.samples).unwrap_or_default();
        let t = rec.lock().unwrap().decode(&s.samples, &l).unwrap_or_default();
        println!("  start={:.2}s dur={d:.2}s lid={l} text={t}", s.start as f32 / 16000.0);
    }
}
