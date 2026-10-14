//! Audio level helper for call diagnostics. Speech (VAD, STT, TTS, languages) lives
//! in the bia-speech crate.

pub fn pcm16_rms(pcm: &[u8]) -> u32 {
    if pcm.len() < 2 {
        return 0;
    }
    let mut sum = 0u64;
    let mut n = 0u64;
    for chunk in pcm.chunks_exact(2) {
        let s = i16::from_le_bytes([chunk[0], chunk[1]]) as i32;
        sum += (s * s) as u64;
        n += 1;
    }
    if n == 0 {
        return 0;
    }
    ((sum / n) as f64).sqrt() as u32
}
