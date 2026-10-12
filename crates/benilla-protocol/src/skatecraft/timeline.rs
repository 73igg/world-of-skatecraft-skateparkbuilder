//! Bounded remote presentation clock, separate from the local physics clock.

use std::collections::VecDeque;

use super::{Snapshot, DISCONTINUITY, MAX_BONES};

const CAPACITY: usize = 32;
const MIN_DELAY: f64 = 0.075;
const MAX_DELAY: f64 = 0.200;
const STALE_AFTER: f64 = 0.750;
const MAX_EXTRAPOLATION: f64 = 0.100;
const RETIRED_GENERATIONS: usize = 8;

pub struct FramePair<'a> {
    pub before: &'a Snapshot,
    pub after: &'a Snapshot,
    pub alpha: f32,
    /// Only root motion may be extrapolated; bones hold their last complete solved pose.
    pub extrapolation_seconds: f32,
}

/// One visible owner. The caller separately validates GUID, map, visibility, and rig assets.
pub struct PoseBuffer {
    generation: Option<u64>,
    retired: VecDeque<u64>,
    bones: usize,
    active: bool,
    frames: VecDeque<Snapshot>,
    offset: Option<f64>,
    last_receive: Option<f64>,
    jitter: f64,
    delay: f64,
    playhead: Option<f64>,
}

impl Default for PoseBuffer {
    fn default() -> Self {
        Self {
            generation: None,
            retired: VecDeque::with_capacity(RETIRED_GENERATIONS),
            bones: 0,
            active: false,
            frames: VecDeque::with_capacity(CAPACITY),
            offset: None,
            last_receive: None,
            jitter: 0.,
            delay: 0.100,
            playhead: None,
        }
    }
}

impl PoseBuffer {
    /// Called only after a compatible dictionary from the authenticated server is accepted.
    /// Repeated definitions do not rewind a live session, and retired sessions cannot return.
    pub fn begin(&mut self, generation: u64, bones: usize) -> bool {
        if generation == 0
            || bones == 0
            || bones > MAX_BONES
            || self.generation == Some(generation)
            || self.retired.contains(&generation)
        {
            return false;
        }
        if let Some(previous) = self.generation {
            if self.retired.len() == RETIRED_GENERATIONS {
                self.retired.pop_front();
            }
            self.retired.push_back(previous);
        }
        self.generation = Some(generation);
        self.bones = bones;
        self.active = true;
        self.reset_clock();
        true
    }

    fn reset_clock(&mut self) {
        self.frames.clear();
        self.offset = None;
        self.last_receive = None;
        self.playhead = None;
        self.jitter = 0.;
        self.delay = 0.100;
    }

    pub fn stop(&mut self, generation: u64) -> bool {
        if self.generation != Some(generation) {
            return false;
        }
        self.active = false;
        self.reset_clock();
        true
    }

    /// Arrival time is a monotonic local clock in seconds, recorded at network receipt.
    pub fn push(&mut self, frame: Snapshot, received: f64) -> bool {
        if !self.active
            || Some(frame.generation) != self.generation
            || frame.bones.len() != self.bones
            || !received.is_finite()
            || received < 0.
            || frame.source_ms > (1u64 << 53)
            || self
                .last_receive
                .is_some_and(|previous| received < previous)
        {
            return false;
        }
        if let Some(previous) = self.frames.back() {
            let forward = frame.sequence.wrapping_sub(previous.sequence);
            if forward == 0 || forward >= 0x8000_0000 || frame.source_ms <= previous.source_ms {
                return false;
            }
        }
        if frame.flags & DISCONTINUITY != 0 {
            self.reset_clock();
        }
        let source = frame.source_ms as f64 / 1000.;
        let transit = received - source;
        let offset = self.offset.map_or(transit, |offset| offset.min(transit));
        self.offset = Some(offset);
        self.jitter = self.jitter * 0.9 + (transit - offset).max(0.) * 0.1;
        self.delay = (0.100 + 2. * self.jitter).clamp(MIN_DELAY, MAX_DELAY);
        self.last_receive = Some(received);
        if self.frames.len() == CAPACITY {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
        true
    }

    pub fn sample(&mut self, now: f64) -> Option<FramePair<'_>> {
        if !self.active
            || !now.is_finite()
            || self.frames.is_empty()
            || now - self.last_receive? > STALE_AFTER
            || now < self.last_receive?
        {
            return None;
        }
        let desired = now - self.offset? - self.delay;
        let playhead = self
            .playhead
            .map_or(desired, |previous| previous.max(desired));
        self.playhead = Some(playhead);
        while self.frames.len() > 2 && self.frames[1].source_ms as f64 / 1000. <= playhead {
            self.frames.pop_front();
        }
        let before = self.frames.front()?;
        let after = self.frames.get(1).unwrap_or(before);
        let start = before.source_ms as f64 / 1000.;
        let end = after.source_ms as f64 / 1000.;
        if playhead >= end {
            Some(FramePair {
                before: after,
                after,
                alpha: 0.,
                extrapolation_seconds: (playhead - end).clamp(0., MAX_EXTRAPOLATION) as f32,
            })
        } else {
            Some(FramePair {
                before,
                after,
                alpha: if end > start {
                    ((playhead - start) / (end - start)).clamp(0., 1.) as f32
                } else {
                    0.
                },
                extrapolation_seconds: 0.,
            })
        }
    }

    pub fn queued(&self) -> usize {
        self.frames.len()
    }
    pub fn delay_seconds(&self) -> f64 {
        self.delay
    }
}

/// Presentation events are idempotent within a session, including repeated recovery packets.
#[derive(Default)]
pub struct EventDeduper {
    generation: Option<u64>,
    last: Option<u32>,
}

impl EventDeduper {
    pub fn begin(&mut self, generation: u64) {
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.last = None;
        }
    }

    pub fn accept(&mut self, generation: u64, event: u32) -> bool {
        if self.generation != Some(generation) {
            return false;
        }
        if let Some(previous) = self.last {
            let forward = event.wrapping_sub(previous);
            if forward == 0 || forward >= 0x8000_0000 {
                return false;
            }
        }
        self.last = Some(event);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(generation: u64, sequence: u32, source_ms: u64) -> Snapshot {
        Snapshot {
            generation,
            sequence,
            source_ms,
            flags: 0,
            root: [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.],
            velocity: [0.; 3],
            facing: 0.,
            state: "PhysicsAir".into(),
            bones: vec![[0.; 12]],
        }
    }

    #[test]
    fn interpolation_uses_sender_time_without_synchronized_pc_clocks() {
        let mut buffer = PoseBuffer::default();
        assert!(buffer.begin(7, 1));
        assert!(buffer.push(frame(7, 0, 50_000), 2.));
        assert!(buffer.push(frame(7, 1, 50_050), 2.05));
        assert!(buffer.push(frame(7, 2, 50_100), 2.10));
        let pair = buffer.sample(2.125).unwrap();
        assert_eq!((pair.before.sequence, pair.after.sequence), (0, 1));
        assert!((pair.alpha - 0.5).abs() < 0.001);
    }

    #[test]
    fn history_and_jitter_delay_remain_bounded() {
        let mut buffer = PoseBuffer::default();
        buffer.begin(1, 1);
        for i in 0..100_000 {
            buffer.push(frame(1, i, u64::from(i) * 50), f64::from(i) * 0.06);
        }
        assert_eq!(buffer.queued(), CAPACITY);
        assert!(buffer.delay_seconds() <= MAX_DELAY);
    }

    #[test]
    fn duplicate_old_wrong_layout_and_wrong_session_packets_are_rejected() {
        let mut buffer = PoseBuffer::default();
        buffer.begin(1, 1);
        assert!(buffer.push(frame(1, 10, 500), 1.));
        assert!(!buffer.push(frame(1, 10, 550), 1.1));
        assert!(!buffer.push(frame(1, 9, 600), 1.2));
        assert!(!buffer.push(frame(2, 11, 650), 1.3));
        let mut wrong_layout = frame(1, 11, 650);
        wrong_layout.bones.clear();
        assert!(!buffer.push(wrong_layout, 1.4));
    }

    #[test]
    fn stop_cannot_be_undone_by_delayed_poses_or_repeated_definitions() {
        let mut buffer = PoseBuffer::default();
        buffer.begin(1, 1);
        buffer.push(frame(1, 1, 50), 1.);
        assert!(buffer.stop(1));
        assert!(!buffer.push(frame(1, 2, 100), 1.1));
        assert!(!buffer.begin(1, 1));
        assert!(buffer.begin(2, 1));
        assert!(!buffer.begin(1, 1));
        assert!(!buffer.stop(1));
    }

    #[test]
    fn sequence_wrap_still_rejects_reordered_packets() {
        let mut buffer = PoseBuffer::default();
        buffer.begin(1, 1);
        assert!(buffer.push(frame(1, u32::MAX, 50), 1.));
        assert!(buffer.push(frame(1, 0, 100), 1.1));
        assert!(!buffer.push(frame(1, u32::MAX, 150), 1.2));
    }

    #[test]
    fn outage_limits_extrapolation_and_fresh_pose_restores_presentation() {
        let mut buffer = PoseBuffer::default();
        buffer.begin(1, 1);
        buffer.push(frame(1, 1, 50), 1.);
        assert!(buffer.sample(1.4).unwrap().extrapolation_seconds <= MAX_EXTRAPOLATION as f32);
        assert!(buffer.sample(2.).is_none());
        assert!(buffer.push(frame(1, 2, 1050), 2.));
        assert!(buffer.sample(2.).is_some());
    }

    #[test]
    fn teleport_never_interpolates_through_the_old_location() {
        let mut buffer = PoseBuffer::default();
        buffer.begin(1, 1);
        buffer.push(frame(1, 1, 50), 1.);
        let mut destination = frame(1, 2, 100);
        destination.flags = DISCONTINUITY;
        destination.root[9] = 10_000.;
        assert!(buffer.push(destination, 1.1));
        let pair = buffer.sample(1.1).unwrap();
        assert_eq!(pair.before.root[9], 10_000.);
        assert_eq!(pair.after.root[9], 10_000.);
    }

    #[test]
    fn repeated_events_fire_once_and_new_sessions_start_fresh() {
        let mut deduper = EventDeduper::default();
        deduper.begin(1);
        assert!(deduper.accept(1, 12));
        assert!(!deduper.accept(1, 12));
        assert!(!deduper.accept(2, 13));
        deduper.begin(2);
        assert!(deduper.accept(2, 0));
    }
}
