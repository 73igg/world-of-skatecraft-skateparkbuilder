//! Offline skate sound reconstruction from recorded state changes and the user's local sound bank.
use super::timeline::{pair, Sample};
use crate::skate::sound::{Cue, Phase, SkateAudio};
use bevy::prelude::*;
use kira::{sound::static_sound::StaticSoundData, Frame};
use std::{
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
};
const RATE: u32 = 48_000;
const BLOCK: usize = 800;
struct Voice {
    data: StaticSoundData,
    at: f64,
    gain: f32,
}
pub(super) struct Audio {
    file: std::fs::File,
    samples: u32,
    frames: Vec<Arc<Sample>>,
    state: SkateAudio,
    previous: Option<f64>,
    loops: [Option<StaticSoundData>; 4],
    loop_at: [f64; 4],
    shots: [Option<StaticSoundData>; 5],
    voices: Vec<Voice>,
    level: [f32; 4],
}
fn read(data: &StaticSoundData, at: f64, looping: bool) -> Frame {
    if data.frames.is_empty() {
        return Frame::ZERO;
    }
    let at = if looping {
        at.rem_euclid(data.frames.len() as f64)
    } else {
        at
    };
    if at < 0. || at >= data.frames.len() as f64 {
        return Frame::ZERO;
    }
    let i = at as usize;
    let a = data.frames[i];
    let b = data.frames.get(i + 1).copied().unwrap_or(if looping {
        data.frames[0]
    } else {
        Frame::ZERO
    });
    a * (1. - at.fract() as f32) + b * at.fract() as f32
}
impl Audio {
    pub fn new(path: &Path, frames: Vec<Arc<Sample>>) -> Result<Self, String> {
        let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        file.write_all(&[0; 44]).map_err(|e| e.to_string())?;
        let root = crate::skate::sound::audio_root();
        let load = |name: &str| {
            root.as_ref()
                .and_then(|r| StaticSoundData::from_file(r.join(format!("{name}.wav"))).ok())
        };
        Ok(Self {
            file,
            samples: 0,
            frames,
            state: default(),
            previous: None,
            loops: ["roll_concrete", "grind_trucks", "grind_board", "wind"].map(load),
            loop_at: [0.; 4],
            shots: ["pop_1", "land_1", "land_hard_1", "bail_1", "skid_1"].map(load),
            voices: Vec::new(),
            level: [0.; 4],
        })
    }
    pub fn block(&mut self, cursor: f64) -> Result<(), String> {
        let discontinuity = self.previous.is_none_or(|p| cursor < p || cursor - p > 0.2);
        if discontinuity {
            self.state.reset();
            self.voices.clear();
        }
        let previous = self.previous.unwrap_or(cursor);
        for frame in self
            .frames
            .iter()
            .filter(|f| f.at > previous && f.at <= cursor)
        {
            self.state.observe(&frame.frame.state, frame.frame.velocity);
        }
        let Some((frame, _, _)) = pair(&self.frames, cursor) else {
            return Err("Empty replay audio".into());
        };
        if discontinuity {
            self.state.observe(&frame.frame.state, frame.frame.velocity);
            self.state.cues.clear();
        }
        for cue in self.state.cues.drain(..) {
            let (i, gain) = match cue {
                Cue::Pop => (0, 0.9),
                Cue::Land { hard: false } => (1, 0.8),
                Cue::Land { hard: true } => (2, 1.),
                Cue::Bail => (3, 1.),
            };
            if let Some(data) = &self.shots[i] {
                self.voices.push(Voice {
                    data: data.clone(),
                    at: 0.,
                    gain,
                });
            }
        }
        let speed = frame.frame.velocity.xz().length();
        let roll = if matches!(self.state.phase, Phase::Ground | Phase::Slide) {
            (speed / 6.).clamp(0., 1.) * 0.9
        } else {
            0.
        };
        let grind = |board| {
            if self.state.phase == Phase::Grind(board) {
                (0.4 + speed / 8.).min(1.)
            } else {
                0.
            }
        };
        let levels = [
            roll,
            grind(false),
            grind(true),
            ((speed - 4.) / 10.).clamp(0., 0.6),
        ];
        let rates = [
            0.7 + (speed / 12.).min(0.8),
            0.85 + (speed / 20.).min(0.4),
            0.85 + (speed / 20.).min(0.4),
            1.,
        ];
        let playback = self
            .previous
            .map_or(1., |p| ((cursor - p) * 60.).clamp(0.1, 4.));
        if self.state.phase == Phase::Slide && self.samples % 5600 < BLOCK as u32 {
            if let Some(data) = &self.shots[4] {
                self.voices.push(Voice {
                    data: data.clone(),
                    at: 0.,
                    gain: (speed / 8.).clamp(0.3, 0.8),
                });
            }
        }
        let mut bytes = Vec::with_capacity(BLOCK * 4);
        for _ in 0..BLOCK {
            let mut mixed = Frame::ZERO;
            for (i, level) in levels.iter().enumerate() {
                self.level[i] += (*level - self.level[i]) * 0.002;
                if let Some(data) = &self.loops[i] {
                    mixed += read(data, self.loop_at[i], true) * self.level[i];
                    self.loop_at[i] += f64::from(data.sample_rate) / f64::from(RATE)
                        * f64::from(rates[i])
                        * playback;
                }
            }
            for voice in &mut self.voices {
                mixed += read(&voice.data, voice.at, false) * voice.gain;
                voice.at += f64::from(voice.data.sample_rate) / f64::from(RATE) * playback;
            }
            for channel in [mixed.left, mixed.right] {
                bytes.extend_from_slice(&((channel.clamp(-1., 1.) * 32767.) as i16).to_le_bytes());
            }
        }
        self.voices.retain(|v| v.at < v.data.frames.len() as f64);
        self.file.write_all(&bytes).map_err(|e| e.to_string())?;
        self.samples += BLOCK as u32;
        self.previous = Some(cursor);
        Ok(())
    }
    pub fn finish(mut self) -> Result<(), String> {
        let size = self.samples * 4;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        let mut header = Vec::new();
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&(size + 36).to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes());
        header.extend_from_slice(&RATE.to_le_bytes());
        header.extend_from_slice(&(RATE * 4).to_le_bytes());
        header.extend_from_slice(&4u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&size.to_le_bytes());
        self.file
            .write_all(&header)
            .and_then(|()| self.file.sync_all())
            .map_err(|e| e.to_string())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sample_interpolation_wraps_without_out_of_bounds() {
        let data = StaticSoundData {
            sample_rate: 48000,
            frames: vec![Frame::from_mono(0.), Frame::from_mono(1.)].into(),
            settings: default(),
            slice: None,
        };
        assert!((read(&data, 1.5, true).left - 0.5).abs() < 1e-6);
        assert_eq!(read(&data, 2., false), Frame::ZERO);
    }
}
