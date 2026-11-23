//! Clawd's optional robot voice. Windows' built-in speech makes the words, a small effect makes them
//! metallic, and a mixer plays many at once. Off by default, and nothing leaves the PC.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{Interface, Result, GUID, HSTRING, PSTR};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::Speech::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::SHCreateMemStream;

const RATE: u32 = 22_050;
const RING_HZ: f32 = 92.0;
const MAX_WORDS: usize = 120;
const CHUNK: usize = 882;           // 40 ms of sound per buffer
const BUFFERS: usize = 3;
const MAX_VOICES: usize = 48;       // past this a crowd already sounds like a crowd
const MAX_CACHE: usize = 256;
const MAX_WAITING: usize = 6;       // lines waiting to be turned into speech
const CLOSE_AFTER: Duration = Duration::from_millis(1500);
/// SAPI's id for "this stream is described by a WAVEFORMATEX".
const WAVE_FORMAT_EX: GUID = GUID::from_u128(0xC31ADBAE_527F_4FF5_A230_F62BB61FF70C);

type Clip = Arc<Vec<i16>>;

/// What's worth reading aloud: no decoration, and nothing at all for a line like "@_@".
pub fn speakable(text: &str) -> Option<String> {
    let clean: String = text.chars().take(MAX_WORDS).map(|c| if c.is_alphanumeric() || " .,!?'-".contains(c) { c } else { ' ' }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    clean.chars().any(char::is_alphanumeric).then_some(clean)
}

/// A light metallic buzz plus a very short echo. The words stay clear, and quiet endings stay audible.
fn robotize(samples: &[i16]) -> Vec<i16> {
    let delay = (RATE as f32 * 0.006) as usize;
    let tail = (RATE as f32 * 0.12) as usize;      // room for the echo to ring out
    let mut wet = vec![0f32; samples.len() + tail];
    for i in 0..wet.len() {
        let dry = samples.get(i).copied().unwrap_or(0) as f32;
        let tone = (i as f32 / RATE as f32 * RING_HZ * std::f32::consts::TAU).sin();
        let echo = if i >= delay { wet[i - delay] * 0.38 } else { 0.0 };
        wet[i] = dry * (0.72 + 0.28 * tone) + echo;
    }
    let peak = wet.iter().fold(1.0f32, |m, v| m.max(v.abs()));
    let gain = 0.85 * i16::MAX as f32 / peak;
    let (n, fade_in, fade_out) = (wet.len(), (RATE / 200) as usize, (RATE / 40) as usize);
    wet.iter().enumerate().map(|(i, v)| {
        let edge = (i as f32 / fade_in.max(1) as f32).min((n - 1 - i) as f32 / fade_out.max(1) as f32).min(1.0);
        (v * gain * edge) as i16
    }).collect()
}

struct Playing { clip: Clip, pos: f64, rate: f64, gain: f32 }

/// Adds every voice together. The level eases down as the crowd grows, so it's loud but never harsh.
fn mix(voices: &mut Vec<Playing>, out: &mut [i16]) {
    let master = (voices.len().max(1) as f32).powf(-0.4);
    for s in out.iter_mut() {
        let mut sum = 0f32;
        for v in voices.iter_mut() {
            let i = v.pos as usize;
            if i + 1 >= v.clip.len() { continue; }
            let t = (v.pos - i as f64) as f32;
            sum += (v.clip[i] as f32 * (1.0 - t) + v.clip[i + 1] as f32 * t) * v.gain;
            v.pos += v.rate;
        }
        *s = ((sum / 32768.0 * master).tanh() * 32767.0) as i16;
    }
    voices.retain(|v| (v.pos as usize) + 1 < v.clip.len());
}

unsafe fn render(voice: &ISpVoice, text: &str) -> Result<Vec<i16>> {
    let memory = SHCreateMemStream(None).ok_or_else(|| windows::core::Error::from(E_FAIL))?;
    let stream: ISpStream = CoCreateInstance(&SpStream, None, CLSCTX_ALL)?;
    let format = WAVEFORMATEX { wFormatTag: 1, nChannels: 1, nSamplesPerSec: RATE, nAvgBytesPerSec: RATE * 2, nBlockAlign: 2, wBitsPerSample: 16, cbSize: 0 };
    stream.SetBaseStream(&memory, &WAVE_FORMAT_EX, &format)?;
    voice.SetOutput(&stream.cast::<windows::core::IUnknown>()?, true)?;
    voice.Speak(&HSTRING::from(text), (SPF_IS_NOT_XML.0 | SPF_PURGEBEFORESPEAK.0) as u32, None)?;
    let mut size = 0u64;
    memory.Seek(0, STREAM_SEEK_END, Some(&mut size))?;
    memory.Seek(0, STREAM_SEEK_SET, None)?;
    let mut bytes = vec![0u8; size as usize];
    let mut got = 0u32;
    memory.Read(bytes.as_mut_ptr().cast(), bytes.len() as u32, Some(&mut got)).ok()?;
    bytes.truncate(got as usize);
    Ok(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
}

/// The speakers, open only while something is being said.
struct Speakers { out: HWAVEOUT, heads: Box<[WAVEHDR; BUFFERS]>, data: Vec<Vec<i16>>, queued: [bool; BUFFERS] }

impl Speakers {
    unsafe fn open() -> Option<Box<Self>> {
        let format = WAVEFORMATEX { wFormatTag: 1, nChannels: 1, nSamplesPerSec: RATE, nAvgBytesPerSec: RATE * 2, nBlockAlign: 2, wBitsPerSample: 16, cbSize: 0 };
        let mut out = HWAVEOUT::default();
        if waveOutOpen(Some(&mut out), WAVE_MAPPER, &format, 0, 0, CALLBACK_NULL) != 0 { return None; }
        let mut me = Box::new(Speakers { out, heads: Box::new(std::mem::zeroed()), data: vec![vec![0i16; CHUNK]; BUFFERS], queued: [false; BUFFERS] });
        for i in 0..BUFFERS {
            me.heads[i].lpData = PSTR(me.data[i].as_mut_ptr().cast());
            me.heads[i].dwBufferLength = (CHUNK * 2) as u32;
            waveOutPrepareHeader(out, &mut me.heads[i], std::mem::size_of::<WAVEHDR>() as u32);
        }
        Some(me)
    }

    /// Refills whichever buffers have finished playing.
    unsafe fn pump(&mut self, voices: &mut Vec<Playing>) {
        for i in 0..BUFFERS {
            let flags = std::ptr::read_volatile(std::ptr::addr_of!(self.heads[i].dwFlags));
            if self.queued[i] && flags & 1 == 0 { continue; }       // 1 is WHDR_DONE
            self.queued[i] = false;
            if voices.is_empty() { continue; }
            mix(voices, &mut self.data[i]);
            std::ptr::write_volatile(std::ptr::addr_of_mut!(self.heads[i].dwFlags), flags & !1);
            if waveOutWrite(self.out, &mut self.heads[i], std::mem::size_of::<WAVEHDR>() as u32) == 0 { self.queued[i] = true; }
        }
    }

    fn playing(&self) -> bool {
        (0..BUFFERS).any(|i| self.queued[i] && unsafe { std::ptr::read_volatile(std::ptr::addr_of!(self.heads[i].dwFlags)) } & 1 == 0)
    }
}

impl Drop for Speakers {
    fn drop(&mut self) {
        unsafe {
            waveOutReset(self.out);
            for i in 0..BUFFERS { waveOutUnprepareHeader(self.out, &mut self.heads[i], std::mem::size_of::<WAVEHDR>() as u32); }
            waveOutClose(self.out);
        }
    }
}

enum Cmd { Play { clip: Clip, rate: f64, gain: f32, alone: bool }, Stop }

fn mixer(rx: Receiver<Cmd>) {
    let mut voices: Vec<Playing> = Vec::new();
    let mut speakers: Option<Box<Speakers>> = None;
    let mut quiet_since = Instant::now();
    loop {
        // With nothing to play, this thread simply sleeps until the next line.
        let idle = voices.is_empty() && !speakers.as_ref().is_some_and(|s| s.playing());
        let first = if idle && speakers.is_none() {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(Duration::from_millis(5))
        };
        let mut next = match first {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        while let Some(cmd) = next.take() {
            match cmd {
                Cmd::Play { clip, rate, gain, alone } => {
                    if voices.len() < MAX_VOICES && !(alone && (!voices.is_empty() || speakers.as_ref().is_some_and(|s| s.playing()))) {
                        voices.push(Playing { clip, pos: 0.0, rate, gain });
                    }
                }
                Cmd::Stop => {
                    voices.clear();
                    speakers = None;
                }
            }
            next = rx.try_recv().ok();
        }
        if !voices.is_empty() {
            quiet_since = Instant::now();
            if speakers.is_none() { speakers = unsafe { Speakers::open() }; }
            if speakers.is_none() { voices.clear(); }       // no audio device right now
        }
        if let Some(s) = &mut speakers {
            unsafe { s.pump(&mut voices); }
            if voices.is_empty() && !s.playing() && quiet_since.elapsed() > CLOSE_AFTER { speakers = None; }
        }
    }
}

struct Job { text: String, rate: f64, gain: f32, alone: bool, round: u64 }

pub struct Voice { jobs: Sender<Job>, mix: Sender<Cmd>, cache: Arc<Mutex<HashMap<String, Clip>>>, waiting: Arc<AtomicUsize>, round: Arc<AtomicU64> }

impl Voice {
    pub fn new() -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (mix_tx, mix_rx) = channel::<Cmd>();
        let cache: Arc<Mutex<HashMap<String, Clip>>> = Arc::default();
        let waiting = Arc::new(AtomicUsize::new(0));
        let round = Arc::new(AtomicU64::new(0));       // goes up on stop, so lines still being prepared are dropped
        std::thread::spawn(move || mixer(mix_rx));
        let (to_mix, store, count, current) = (mix_tx.clone(), cache.clone(), waiting.clone(), round.clone());
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let engine = CoCreateInstance::<_, ISpVoice>(&SpVoice, None, CLSCTX_ALL).ok();
            if let Some(v) = &engine { let _ = v.SetRate(-1); }
            for job in job_rx {
                let stale = |j: &Job| j.round != current.load(Ordering::SeqCst);
                if stale(&job) {
                    count.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let round = job.round;
                if let Some(Ok(samples)) = engine.as_ref().map(|v| render(v, &job.text)) {
                    if !samples.is_empty() {
                        let clip: Clip = Arc::new(robotize(&samples));
                        if let Ok(mut c) = store.lock() {
                            if c.len() >= MAX_CACHE { c.clear(); }
                            c.insert(job.text, clip.clone());
                        }
                        if round == current.load(Ordering::SeqCst) { let _ = to_mix.send(Cmd::Play { clip, rate: job.rate, gain: job.gain, alone: job.alone }); }
                    }
                }
                count.fetch_sub(1, Ordering::SeqCst);
            }
        });
        Voice { jobs, mix: mix_tx, cache, waiting, round }
    }

    /// `pitch` gives each buddy its own voice. With `overlap` off, lines said over another are skipped.
    pub fn say(&self, text: &str, volume: f64, overlap: bool, pitch: f64) {
        let Some(line) = speakable(text) else { return };
        let (gain, alone) = (volume.clamp(0.0, 1.0) as f32, !overlap);
        if let Some(clip) = self.cache.lock().ok().and_then(|c| c.get(&line).cloned()) {
            let _ = self.mix.send(Cmd::Play { clip, rate: pitch, gain, alone });
            return;
        }
        if self.waiting.fetch_add(1, Ordering::SeqCst) >= MAX_WAITING {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        if self.jobs.send(Job { text: line, rate: pitch, gain, alone, round: self.round.load(Ordering::SeqCst) }).is_err() { self.waiting.fetch_sub(1, Ordering::SeqCst); }
    }

    /// Silence right now, including anything still being prepared.
    pub fn stop(&self) {
        self.round.fetch_add(1, Ordering::SeqCst);
        let _ = self.mix.send(Cmd::Stop);
    }
}

/// A steady, different pitch for each buddy.
pub fn pitch_for(id: u32) -> f64 { 0.84 + (id as f64 * 0.618_034).fract() * 0.36 }

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(level: i16, n: usize, gain: f32) -> Playing { Playing { clip: Arc::new(vec![level; n]), pos: 0.0, rate: 1.0, gain } }

    #[test]
    fn only_real_words_are_read_aloud() {
        assert_eq!(speakable("  *yawn*  hi!").as_deref(), Some("yawn hi!"));
        assert_eq!(speakable("@_@"), None);
        assert_eq!(speakable("<3"), Some("3".into()));
    }

    #[test]
    fn robot_effect_keeps_every_word_and_lets_the_end_ring_out() {
        let input = vec![6_000i16; 2000];
        let out = robotize(&input);
        assert!(out.len() > input.len());
        assert!(out[1990].unsigned_abs() > 3000, "the last word is still at full level");
        assert!(out[out.len() - 1].unsigned_abs() < 50, "and it ends without a click");
    }

    #[test]
    fn voices_overlap_and_a_crowd_never_clips() {
        let mut one = vec![voice(8000, 100, 1.0)];
        let mut two = vec![voice(8000, 100, 1.0), voice(8000, 100, 1.0)];
        let (mut a, mut b) = ([0i16; 50], [0i16; 50]);
        mix(&mut one, &mut a);
        mix(&mut two, &mut b);
        assert!(b[10] > a[10], "two together are louder than one");
        let mut crowd: Vec<Playing> = (0..MAX_VOICES).map(|_| voice(i16::MAX, 100, 1.0)).collect();
        let mut c = [0i16; 50];
        mix(&mut crowd, &mut c);
        assert!(c.iter().all(|&s| s > 0), "no wrap-around distortion");
    }

    #[test]
    fn finished_voices_leave_and_volume_zero_is_silent() {
        let mut v = vec![voice(9000, 10, 0.0)];
        let mut out = [1i16; 40];
        mix(&mut v, &mut out);
        assert!(v.is_empty());
        assert!(out.iter().all(|&s| s == 0));
    }

    #[test]
    fn buddies_get_different_steady_pitches() {
        assert_eq!(pitch_for(3), pitch_for(3));
        assert_ne!(pitch_for(3), pitch_for(4));
        assert!((0..500).all(|i| (0.84..=1.2).contains(&pitch_for(i))));
    }
}
