//! Small menu sounds, synthesised at start-up so the app ships no audio files. Off unless the user turns them on.
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

const RATE: u32 = 22_050;

#[derive(Clone, Copy)]
pub enum Cue { Open, Tick, Pick }

/// A 16-bit mono WAV of (frequency, seconds) notes, each faded in and out so it doesn't click.
fn wav(notes: &[(f32, f32)]) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for &(freq, secs) in notes {
        let n = (secs * RATE as f32) as usize;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let k = i as f32 / n as f32;
            let envelope = (k * 12.0).min(1.0) * (1.0 - k).powi(2);
            samples.push(((t * freq * std::f32::consts::TAU).sin() * envelope * 0.16 * i16::MAX as f32) as i16);
        }
    }
    let bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for s in samples { out.extend_from_slice(&s.to_le_bytes()); }
    out
}

pub fn play(cue: Cue) {
    static CUES: OnceLock<[Vec<u8>; 3]> = OnceLock::new();
    let all = CUES.get_or_init(|| [wav(&[(520.0, 0.05), (780.0, 0.08)]), wav(&[(1200.0, 0.022)]), wav(&[(660.0, 0.05), (990.0, 0.1)])]);
    let data = &all[cue as usize];
    // The buffers live for the whole run, which SND_ASYNC with SND_MEMORY requires.
    unsafe { let _ = PlaySoundW(PCWSTR(data.as_ptr() as *const u16), HMODULE::default(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_matches_its_samples() {
        let w = wav(&[(440.0, 0.1)]);
        assert_eq!(&w[..4], b"RIFF");
        let data = u32::from_le_bytes([w[40], w[41], w[42], w[43]]) as usize;
        assert_eq!(w.len(), 44 + data);
        assert!(w[44..].iter().any(|&b| b != 0));
    }
}
