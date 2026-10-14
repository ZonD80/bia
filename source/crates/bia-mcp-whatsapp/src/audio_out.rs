//! Real-time outbound call audio. One paced OS thread per call sends exactly one
//! 60 ms mono 16 kHz frame (960 samples, the only size the WhatsApp engine
//! accepts) into the call's microphone channel per tick: queued TTS PCM when
//! there is some, silence otherwise. Nothing on the async runtime can stall or
//! burst the stream.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use whatsapp_rust::async_channel::Sender;

pub const RATE: u32 = 16_000;
/// 60 ms of mono s16 @ 16 kHz.
pub const FRAME_SAMPLES: usize = 960;
const FRAME: Duration = Duration::from_millis(60);
/// Falling further behind than this resets the clock instead of bursting
/// frames into the engine (the receiver's jitter buffer would drop them).
const MAX_LAG: Duration = Duration::from_millis(180);

#[derive(Default)]
struct Queue {
    /// Utterance currently allowed to feed the queue.
    gen: u64,
    pcm: VecDeque<i16>,
    /// TTS for `gen` is still generating.
    producing: bool,
    /// `gen` has audio queued or pending; cleared when it drains.
    playing: bool,
    /// Pacer stopped (call ended); feeds must stop generating.
    closed: bool,
}

/// Appends audio for one utterance. Stale once a newer utterance begins.
pub struct Feed {
    q: Arc<Mutex<Queue>>,
    gen: u64,
}

impl Feed {
    /// Queue samples; false means the utterance was replaced or the call
    /// ended — stop generating.
    pub fn push(&self, samples: &[i16]) -> bool {
        let mut q = self.q.lock().unwrap();
        if q.closed || q.gen != self.gen {
            return false;
        }
        q.pcm.extend(samples);
        true
    }

    pub fn is_live(&self) -> bool {
        let q = self.q.lock().unwrap();
        !q.closed && q.gen == self.gen
    }

    #[cfg(test)]
    pub fn detached() -> Self {
        Self {
            q: Arc::new(Mutex::new(Queue::default())),
            gen: 0,
        }
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.q.lock().unwrap().pcm.len()
    }

    /// No more audio for this utterance; the pacer reports done once drained.
    pub fn finish(self) {
        let mut q = self.q.lock().unwrap();
        if q.gen == self.gen {
            q.producing = false;
        }
    }
}

pub struct Outbound {
    id: u64,
    q: Arc<Mutex<Queue>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Outbound {
    /// Spawn the pacer. `on_done(id, gen)` runs on the pacer thread when
    /// utterance `gen` has fully played out; `id` is unique per pacer so stale
    /// events from an earlier call can be ignored.
    pub fn start(
        sink: Sender<Vec<i16>>,
        chat: &str,
        on_done: impl Fn(u64, u64) + Send + 'static,
    ) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let q = Arc::new(Mutex::new(Queue::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let q = q.clone();
            let stop = stop.clone();
            let name: String = chat
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .take(24)
                .collect();
            std::thread::Builder::new()
                .name(format!("call-audio-{name}"))
                .spawn(move || pace(sink, q, stop, move |gen| on_done(id, gen)))
                .expect("spawn call audio thread")
        };
        Self {
            id,
            q,
            stop,
            thread: Some(thread),
        }
    }

    /// Start a new utterance, dropping whatever is still queued (the agent
    /// interrupting itself). Incoming call audio never calls this.
    pub fn begin(&self, gen: u64) -> Feed {
        let mut q = self.q.lock().unwrap();
        q.gen = gen;
        q.pcm.clear();
        q.producing = true;
        q.playing = true;
        Feed {
            q: self.q.clone(),
            gen,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for Outbound {
    fn drop(&mut self) {
        self.q.lock().unwrap().closed = true;
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn pace(
    sink: Sender<Vec<i16>>,
    q: Arc<Mutex<Queue>>,
    stop: Arc<AtomicBool>,
    on_done: impl Fn(u64),
) {
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        next += FRAME;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > MAX_LAG {
            next = now;
        }

        let mut frame: Vec<i16> = Vec::with_capacity(FRAME_SAMPLES);
        let done = {
            let mut q = q.lock().unwrap();
            let n = q.pcm.len().min(FRAME_SAMPLES);
            frame.extend(q.pcm.drain(..n));
            frame.resize(FRAME_SAMPLES, 0);
            if q.playing && !q.producing && q.pcm.is_empty() {
                q.playing = false;
                Some(q.gen)
            } else {
                None
            }
        };

        // Loss tolerant: a full channel (engine not attached yet, relay stall)
        // just drops this frame; a closed channel means the call is over and
        // the thread keeps ticking until the owner drops the Outbound.
        let _ = sink.try_send(frame);
        if let Some(gen) = done {
            on_done(gen);
        }
    }
}

/// Resample mono f32 [-1, 1] to 16 kHz s16 (Catmull-Rom). TTS hands us whole
/// sentences, so per-chunk resampling has no audible seams.
pub fn to_call_pcm(samples: &[f32], from_hz: u32) -> Vec<i16> {
    let conv = |x: f32| (x.clamp(-1.0, 1.0) * 32767.0) as i16;
    if samples.is_empty() || from_hz == 0 {
        return Vec::new();
    }
    if from_hz == RATE {
        return samples.iter().copied().map(conv).collect();
    }
    let at = |i: isize| samples[i.clamp(0, samples.len() as isize - 1) as usize];
    let step = from_hz as f64 / RATE as f64;
    let out_len = (samples.len() as u64 * RATE as u64 / from_hz as u64) as usize;
    let mut out = Vec::with_capacity(out_len);
    for k in 0..out_len {
        let pos = k as f64 * step;
        let i = pos.floor() as isize;
        let t = (pos - i as f64) as f32;
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let v = p1
            + 0.5
                * t
                * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)));
        out.push(conv(v));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_lengths() {
        assert_eq!(to_call_pcm(&vec![0.0; 22_050], 22_050).len(), 16_000);
        assert_eq!(to_call_pcm(&vec![0.0; 24_000], 24_000).len(), 16_000);
        assert_eq!(to_call_pcm(&vec![0.5; 480], 16_000), vec![16383; 480]);
    }

    #[test]
    fn resample_keeps_dc() {
        let out = to_call_pcm(&vec![0.25; 1000], 48_000);
        assert!(out.iter().all(|&s| (s - 8191).abs() <= 1));
    }
}
