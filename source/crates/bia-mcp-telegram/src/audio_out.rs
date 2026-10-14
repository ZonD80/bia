//! Real-time outbound call audio. One paced OS thread per call sends exactly one
//! 10 ms mono 48 kHz frame per tick straight into ntgcalls: queued TTS PCM when
//! there is some, silence otherwise. Nothing on the async update loop can stall
//! or burst the stream.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tgcalls::{FrameData, FrameSink, StreamDevice, VideoRotation};

pub const RATE: u32 = 48_000;
/// 10 ms of mono s16le @ 48 kHz.
const FRAME_SAMPLES: usize = 480;
const FRAME: Duration = Duration::from_millis(10);
/// Falling further behind than this resets the clock instead of bursting
/// frames into WebRTC (the receiver's jitter buffer would drop them).
const MAX_LAG: Duration = Duration::from_millis(40);

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
    pub fn start(sink: FrameSink, chat_id: i64, on_done: impl Fn(u64, u64) + Send + 'static) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let q = Arc::new(Mutex::new(Queue::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let q = q.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name(format!("call-audio-{chat_id}"))
                .spawn(move || pace(sink, chat_id, q, stop, move |gen| on_done(id, gen)))
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
    sink: FrameSink,
    chat_id: i64,
    q: Arc<Mutex<Queue>>,
    stop: Arc<AtomicBool>,
    on_done: impl Fn(u64),
) {
    let fd = FrameData {
        absolute_capture_timestamp_ms: 0,
        rotation: VideoRotation::VideoRotation0,
        width: 0,
        height: 0,
    };
    let mut frame = [0u8; FRAME_SAMPLES * 2];
    let mut next = Instant::now();
    let mut errors = 0u64;
    let mut stalls = 0u64;
    log(format!("pacer started chat={chat_id}"));
    while !stop.load(Ordering::Relaxed) {
        next += FRAME;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > MAX_LAG {
            stalls += 1;
            if stalls <= 5 || stalls % 100 == 0 {
                log(format!(
                    "pacer stall chat={chat_id} lag={}ms (#{stalls}); clock reset",
                    (now - next).as_millis()
                ));
            }
            next = now;
        }

        let done = {
            let mut q = q.lock().unwrap();
            let n = q.pcm.len().min(FRAME_SAMPLES);
            for (i, s) in q.pcm.drain(..n).enumerate() {
                frame[i * 2..i * 2 + 2].copy_from_slice(&s.to_le_bytes());
            }
            frame[n * 2..].fill(0);
            if q.playing && !q.producing && q.pcm.is_empty() {
                q.playing = false;
                Some(q.gen)
            } else {
                None
            }
        };

        if let Err(e) = sink.send(StreamDevice::Microphone, &frame, &fd) {
            errors += 1;
            if errors <= 5 || errors % 500 == 0 {
                log(format!("pacer send chat={chat_id} (#{errors}): {e}"));
            }
        }
        if let Some(gen) = done {
            on_done(gen);
        }
    }
    log(format!("pacer stopped chat={chat_id} stalls={stalls} errors={errors}"));
}

fn log(_msg: String) {}

/// Resample mono f32 [-1, 1] to 48 kHz s16 (Catmull-Rom). TTS hands us whole
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
    let out_len = (samples.len() as f64 / step).floor() as usize;
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
        assert_eq!(to_call_pcm(&vec![0.0; 22_050], 22_050).len(), 48_000);
        assert_eq!(to_call_pcm(&vec![0.0; 24_000], 24_000).len(), 48_000);
        assert_eq!(to_call_pcm(&vec![0.5; 480], 48_000), vec![16383; 480]);
    }

    #[test]
    fn resample_keeps_dc() {
        let out = to_call_pcm(&vec![0.25; 1000], 16_000);
        assert!(out.iter().all(|&s| (s - 8191).abs() <= 1));
    }
}
