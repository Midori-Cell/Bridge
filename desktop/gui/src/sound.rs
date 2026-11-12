//! Bridge's own notification sounds. They are not files: each sound is computed when it is played (a few short
//! tones with a soft start and a decay) and sent to the default audio output. Nothing of the system's sound theme
//! is used, so it sounds the same on Windows, macOS and Linux.

use std::f32::consts::TAU;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundKind {
    /// Two bright notes going up.
    #[default]
    Chime,
    /// A short, round drop.
    Pop,
    /// A small bell that rings out.
    Bell,
    /// Two soft wooden notes.
    Marimba,
    /// One high, short tone.
    Ping,
}

impl SoundKind {
    pub const ALL: [SoundKind; 5] = [SoundKind::Chime, SoundKind::Pop, SoundKind::Bell, SoundKind::Marimba, SoundKind::Ping];

    pub fn label(self) -> &'static str {
        match self {
            SoundKind::Chime => "Chime",
            SoundKind::Pop => "Pop",
            SoundKind::Bell => "Bell",
            SoundKind::Marimba => "Marimba",
            SoundKind::Ping => "Ping",
        }
    }
}

/// A note: frequency, start (s), loudness, decay time (s), and partials as (frequency ratio, loudness, decay factor).
struct Note {
    freq: f32,
    start: f32,
    amp: f32,
    decay: f32,
    partials: &'static [(f32, f32, f32)],
}

const PURE: &[(f32, f32, f32)] = &[(1.0, 1.0, 1.0), (2.0, 0.18, 0.5)];
const BELL: &[(f32, f32, f32)] = &[(1.0, 1.0, 1.0), (2.0, 0.45, 0.7), (2.76, 0.3, 0.5), (5.4, 0.14, 0.3), (8.93, 0.06, 0.2)];
const WOOD: &[(f32, f32, f32)] = &[(1.0, 1.0, 1.0), (3.93, 0.35, 0.18), (10.0, 0.08, 0.06)];

/// The samples of a sound (one channel, -1..1) at `rate` samples per second.
pub fn render(kind: SoundKind, rate: f32, volume: f32) -> Vec<f32> {
    let (notes, length): (Vec<Note>, f32) = match kind {
        SoundKind::Chime => (
            vec![
                Note { freq: 880.0, start: 0.0, amp: 0.8, decay: 0.22, partials: PURE },
                Note { freq: 1318.5, start: 0.11, amp: 1.0, decay: 0.32, partials: PURE },
            ],
            0.9,
        ),
        SoundKind::Pop => (vec![], 0.16),
        SoundKind::Bell => (vec![Note { freq: 1046.5, start: 0.0, amp: 1.0, decay: 0.55, partials: BELL }], 1.6),
        SoundKind::Marimba => (
            vec![
                Note { freq: 523.25, start: 0.0, amp: 0.9, decay: 0.16, partials: WOOD },
                Note { freq: 659.25, start: 0.09, amp: 1.0, decay: 0.2, partials: WOOD },
            ],
            0.7,
        ),
        SoundKind::Ping => (vec![Note { freq: 1760.0, start: 0.0, amp: 1.0, decay: 0.11, partials: PURE }], 0.5),
    };
    let n = (length * rate) as usize;
    let mut out = vec![0.0f32; n];
    if kind == SoundKind::Pop {
        // a drop: a tone that slides up quickly and dies away
        let mut phase = 0.0f32;
        for (i, s) in out.iter_mut().enumerate() {
            let t = i as f32 / rate;
            let freq = 380.0 + 520.0 * (1.0 - (-t / 0.025).exp());
            phase += TAU * freq / rate;
            *s = phase.sin() * attack(t, 0.003) * (-t / 0.035).exp();
        }
    }
    for note in &notes {
        let first = (note.start * rate) as usize;
        for (i, s) in out.iter_mut().enumerate().skip(first) {
            let t = (i - first) as f32 / rate;
            let mut v = 0.0;
            for &(ratio, amp, decay) in note.partials {
                v += amp * (TAU * note.freq * ratio * t).sin() * (-t / (note.decay * decay)).exp();
            }
            *s += note.amp * v * attack(t, 0.004);
        }
    }
    // the loudest point at 0.8 × volume, and a short fade at the end so it never clicks
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    let gain = 0.8 * volume.clamp(0.0, 1.0) / peak;
    let fade = (0.01 * rate) as usize;
    for (i, s) in out.iter_mut().enumerate() {
        let left = n - i;
        let end = if left < fade { left as f32 / fade as f32 } else { 1.0 };
        *s *= gain * end;
    }
    out
}

fn attack(t: f32, time: f32) -> f32 {
    (t / time).min(1.0)
}

/// Play a sound on the default output, on a thread of its own (the caller never waits). `volume` 0..100.
pub fn play(kind: SoundKind, volume: u8) {
    let _ = std::thread::Builder::new().name("bridge-sound".into()).spawn(move || {
        if play_now(kind, volume as f32 / 100.0).is_none() {
            tracing::debug!("no sound: no audio output could be opened");
        }
    });
}

fn play_now(kind: SoundKind, volume: f32) -> Option<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::SampleFormat;
    let device = cpal::default_host().default_output_device()?;
    let supported = device.default_output_config().ok()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate.0 as f32;
    let samples = render(kind, rate, volume);
    let length = Duration::from_secs_f32(samples.len() as f32 / rate);
    let channels = config.channels as usize;
    let stream = match format {
        SampleFormat::F32 => stream::<f32>(&device, &config, samples, channels),
        SampleFormat::I16 => stream::<i16>(&device, &config, samples, channels),
        SampleFormat::U16 => stream::<u16>(&device, &config, samples, channels),
        SampleFormat::I32 => stream::<i32>(&device, &config, samples, channels),
        SampleFormat::F64 => stream::<f64>(&device, &config, samples, channels),
        SampleFormat::I8 => stream::<i8>(&device, &config, samples, channels),
        SampleFormat::U8 => stream::<u8>(&device, &config, samples, channels),
        _ => None,
    }?;
    stream.play().ok()?;
    std::thread::sleep(length + Duration::from_millis(150));
    Some(())
}

fn stream<T>(device: &cpal::Device, config: &cpal::StreamConfig, samples: Vec<f32>, channels: usize) -> Option<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let mut next = 0usize;
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in out.chunks_mut(channels.max(1)) {
                    let v = samples.get(next).copied().unwrap_or(0.0);
                    next += 1;
                    for s in frame {
                        *s = T::from_sample(v);
                    }
                }
            },
            |e| tracing::debug!("sound: {e}"),
            None,
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_short_quiet_enough_and_ends_silent() {
        for kind in SoundKind::ALL {
            let s = render(kind, 48_000.0, 1.0);
            assert!(!s.is_empty() && s.len() < 48_000 * 2, "{kind:?} length {}", s.len());
            let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!((0.79..=0.801).contains(&peak), "{kind:?} peak {peak}");
            assert!(s.last().unwrap().abs() < 1e-3, "{kind:?} ends with a click");
            assert!(s.iter().all(|v| v.is_finite()));
        }
        let quiet = render(SoundKind::Chime, 48_000.0, 0.25);
        let peak = quiet.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.19..=0.21).contains(&peak));
    }
}

#[cfg(test)]
mod preview {
    /// `BRIDGE_SOUND_DIR=/some/dir cargo test -p bridge-gui write_previews -- --ignored` writes each sound as a WAV file.
    #[test]
    #[ignore]
    fn write_previews() {
        let dir = std::path::PathBuf::from(std::env::var("BRIDGE_SOUND_DIR").unwrap_or_else(|_| ".".into()));
        for kind in super::SoundKind::ALL {
            let s = super::render(kind, 48_000.0, 0.7);
            let mut b = Vec::new();
            let data = (s.len() * 2) as u32;
            b.extend_from_slice(b"RIFF");
            b.extend_from_slice(&(36 + data).to_le_bytes());
            b.extend_from_slice(b"WAVEfmt ");
            b.extend_from_slice(&16u32.to_le_bytes());
            b.extend_from_slice(&1u16.to_le_bytes());
            b.extend_from_slice(&1u16.to_le_bytes());
            b.extend_from_slice(&48_000u32.to_le_bytes());
            b.extend_from_slice(&96_000u32.to_le_bytes());
            b.extend_from_slice(&2u16.to_le_bytes());
            b.extend_from_slice(&16u16.to_le_bytes());
            b.extend_from_slice(b"data");
            b.extend_from_slice(&data.to_le_bytes());
            for v in s {
                b.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
            }
            std::fs::write(dir.join(format!("bridge-{}.wav", kind.label().to_lowercase())), b).unwrap();
        }
    }
}
