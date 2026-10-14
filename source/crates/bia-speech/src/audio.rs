//! Audio plumbing: decode any file to 16 kHz mono (ffmpeg), resample call audio, encode
//! voice notes as OGG/Opus, and the waveform previews Telegram and WhatsApp show.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

pub const RATE: u32 = 16_000;

pub fn i16_to_f32(pcm: &[i16]) -> Vec<f32> {
    pcm.iter().map(|&s| f32::from(s) / 32768.0).collect()
}

pub fn le_bytes_to_i16(pcm: &[u8]) -> Vec<i16> {
    pcm.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

/// Resample mono audio. 48 kHz → 16 kHz (call audio) averages each group of three,
/// which is enough low-pass for speech; other ratios interpolate linearly.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    if from == to * 3 {
        return input.chunks(3).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    }
    let ratio = from as f64 / to as f64;
    let n = ((input.len() as f64) / ratio).floor() as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos as usize;
            let frac = (pos - j as f64) as f32;
            let a = input[j];
            let b = *input.get(j + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

pub fn has_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Decode any audio file ffmpeg understands (ogg/opus voice notes, mp3, m4a, wav…)
/// to 16 kHz mono f32.
pub fn decode_file(path: &Path) -> Result<Vec<f32>, String> {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-ac", "1", "-ar", "16000", "-f", "f32le", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg: {e} (is ffmpeg installed?)"))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg could not decode {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out
        .stdout
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Encode mono samples as an OGG/Opus voice note (48 kHz, speech-tuned).
pub fn encode_voice_note(samples: &[f32], rate: u32, out: &Path) -> Result<(), String> {
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-y", "-v", "error", "-f", "f32le", "-ar"])
        .arg(rate.to_string())
        .args(["-ac", "1", "-i", "-", "-c:a", "libopus", "-b:a", "32k", "-application", "voip", "-ar", "48000", "-ac", "1"])
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut stdin = child.stdin.take().ok_or("ffmpeg stdin")?;
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = writer.join();
    if !status.success() {
        return Err(format!("ffmpeg opus encode failed: {}", err.trim()));
    }
    Ok(())
}

/// Peak level of `n` equal slices, 0.0..=1.0 (normalised to the loudest slice).
fn levels(samples: &[f32], n: usize) -> Vec<f32> {
    if samples.is_empty() || n == 0 {
        return vec![0.0; n];
    }
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let a = i * samples.len() / n;
            let b = ((i + 1) * samples.len() / n).max(a + 1).min(samples.len());
            samples[a..b].iter().fold(0.0f32, |m, s| m.max(s.abs()))
        })
        .collect();
    let max = v.iter().cloned().fold(0.0f32, f32::max);
    if max > 0.0 {
        for x in &mut v {
            *x /= max;
        }
    }
    v
}

/// Telegram voice waveform: 100 values of 5 bits, packed little-endian.
pub fn telegram_waveform(samples: &[f32]) -> Vec<u8> {
    let vals: Vec<u8> = levels(samples, 100).iter().map(|l| (l * 31.0).round() as u8).collect();
    let mut out = vec![0u8; (vals.len() * 5).div_ceil(8)];
    for (i, v) in vals.iter().enumerate() {
        let bit = i * 5;
        let (byte, shift) = (bit / 8, bit % 8);
        let word = u16::from(*v & 31) << shift;
        out[byte] |= (word & 0xff) as u8;
        if byte + 1 < out.len() {
            out[byte + 1] |= (word >> 8) as u8;
        }
    }
    out
}

/// WhatsApp voice waveform: 64 bytes, 0..=100.
pub fn whatsapp_waveform(samples: &[f32]) -> Vec<u8> {
    levels(samples, 64).iter().map(|l| (l * 100.0).round() as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_48k_to_16k() {
        let x: Vec<f32> = (0..4800).map(|i| (i as f32 / 10.0).sin()).collect();
        assert_eq!(resample(&x, 48_000, 16_000).len(), 1600);
        assert_eq!(resample(&x, 24_000, 16_000).len(), 3200);
        assert_eq!(resample(&x, 16_000, 16_000).len(), 4800);
    }

    #[test]
    fn waveforms() {
        let x: Vec<f32> = (0..16000).map(|i| (i as f32 / 16000.0) * if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let tg = telegram_waveform(&x);
        assert_eq!(tg.len(), 63);
        // last value is loudest (31): bits 495..500 all set
        let last = (u16::from(tg[61]) | (u16::from(tg[62]) << 8)) >> (495 % 8);
        assert_eq!(last & 31, 31);
        let wa = whatsapp_waveform(&x);
        assert_eq!(wa.len(), 64);
        assert_eq!(*wa.last().unwrap(), 100);
        assert!(wa[0] < 5);
    }
}
