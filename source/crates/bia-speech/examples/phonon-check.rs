//! cargo run --release -p bia-speech --example phonon-check -- <phonon dir> [lang] [text]
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = a.first().expect("phonon dir");
    let lang = a.get(1).cloned().unwrap_or_else(|| "en".into());
    let text = a.get(2).cloned().unwrap_or_else(|| "Tomorrow in Madrid it will be sunny and mild, around eighteen degrees.".into());
    let v = bia_speech::registry::Voice { kind: "phonon".into(), langs: vec![lang], dir: dir.clone(), ..Default::default() };
    let t0 = std::time::Instant::now();
    let e = bia_speech::tts::Engine::load(&v).expect("load");
    let loaded = t0.elapsed();
    let t1 = std::time::Instant::now();
    let mut first = None;
    let n = e.generate(&text, &mut |_c, _r| { first.get_or_insert(t1.elapsed()); true }).expect("generate");
    let secs = n as f32 / e.sample_rate as f32;
    println!("rate {} load {:.2}s first {:.0}ms audio {:.2}s in {:.2}s", e.sample_rate, loaded.as_secs_f32(), first.unwrap().as_secs_f64() * 1000.0, secs, t1.elapsed().as_secs_f32());
}
