//! A bounded recording of completed native poses, separate from both live movement and playback.
use benilla_protocol::park::Object;
use bevy::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(super) const WINDOW: f64 = 90.;
pub(super) const MIN_WINDOW: f64 = 15.;
const MEMORY_LIMIT: usize = 192 * 1024 * 1024;
pub(super) const MAX_FRAMES: usize = 21_602;

#[derive(Clone)]
pub(super) struct Scene {
    pub map: u32,
    pub instance: u32,
    pub origin: Vec3,
    pub park: Arc<[Object]>,
    pub signature: Vec<u32>,
    pub actor_scale: Vec3,
    pub actor_height: f32,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            map: 0,
            instance: 0,
            origin: Vec3::ZERO,
            park: Arc::default(),
            signature: Vec::new(),
            actor_scale: Vec3::ONE,
            actor_height: 1.7,
        }
    }
}

#[derive(Clone)]
pub(super) struct Sample {
    pub at: f64,
    pub frame: super::super::Frame,
    pub scene: Arc<Scene>,
    pub cut: bool,
}

#[derive(Clone)]
pub(super) struct Visual {
    pub at: f64,
    pub transform: Transform,
    pub locals: Arc<[Transform]>,
    pub cut: bool,
}

pub(super) struct History {
    start: Instant,
    epoch: u64,
    scene: Arc<Scene>,
    frames: VecDeque<Arc<Sample>>,
    bytes: usize,
    scenes: HashMap<usize, (usize, usize)>,
    paused: bool,
    paused_at: Option<Instant>,
    window: f64,
    cameras: VecDeque<super::camera::Keyframe>,
    visuals: VecDeque<Arc<Visual>>,
}

impl History {
    fn cut_after(&self, after: Option<f64>, at: f64) -> bool {
        after.is_none_or(|after| at - after > 0.25)
            || self
                .frames
                .iter()
                .rev()
                .take_while(|f| after.is_none_or(|after| f.at > after))
                .any(|f| f.at <= at && f.cut)
    }
}

impl Default for History {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            epoch: 0,
            scene: Arc::default(),
            frames: VecDeque::new(),
            bytes: 0,
            scenes: HashMap::new(),
            paused: false,
            paused_at: None,
            window: WINDOW,
            cameras: VecDeque::new(),
            visuals: VecDeque::new(),
        }
    }
}

fn frame_bytes(sample: &Sample) -> usize {
    std::mem::size_of::<Sample>()
        + sample.frame.bones.len() * 64
        + sample
            .frame
            .names
            .iter()
            .map(|n| n.len() + 24)
            .sum::<usize>()
        + sample.frame.state.len()
        + sample.frame.line_trick.len()
        + sample.frame.landed_trick.len()
}
fn visual_bytes(visual: &Visual) -> usize {
    std::mem::size_of::<Visual>() + visual.locals.len() * std::mem::size_of::<Transform>()
}
impl History {
    fn reset(&mut self) {
        self.frames.clear();
        self.cameras.clear();
        self.visuals.clear();
        self.scenes.clear();
        self.bytes = 0;
    }
    fn prune(&mut self, at: f64) {
        while self.frames.len() > 2
            && (self.frames.len() > MAX_FRAMES
                || self.frames[1].at < at - self.window
                || self.bytes > MEMORY_LIMIT)
        {
            let old = self.frames.pop_front().unwrap();
            self.bytes -= frame_bytes(&old);
            let pointer = Arc::as_ptr(&old.scene) as usize;
            if let Some((count, bytes)) = self.scenes.get_mut(&pointer) {
                *count -= 1;
                if *count == 0 {
                    self.bytes -= *bytes;
                    self.scenes.remove(&pointer);
                }
            }
            let first = self.frames.front().unwrap().at;
            while self.visuals.len() > 1 && self.visuals[1].at < first {
                self.bytes -= visual_bytes(&self.visuals.pop_front().unwrap());
            }
            while self.cameras.len() > 1 && self.cameras[1].at < first {
                self.cameras.pop_front();
            }
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Recording(Arc<Mutex<History>>);

impl Recording {
    pub(super) fn set_window(&self, seconds: f64) {
        if seconds.is_finite() {
            self.0.lock().unwrap_or_else(|p| p.into_inner()).window =
                seconds.clamp(MIN_WINDOW, WINDOW);
        }
    }

    pub(in crate::skate) fn invalidate(&self, epoch: u64) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).epoch = epoch;
    }
    pub(super) fn context(&self, epoch: u64, scene: Scene) {
        let mut history = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if history.scene.map != scene.map
            || history.scene.instance != scene.instance
            || (!history.scene.signature.is_empty() && history.scene.signature != scene.signature)
        {
            history.reset();
        }
        history.epoch = epoch;
        history.scene = Arc::new(scene);
    }

    pub(in crate::skate) fn capture(&self, epoch: u64, frame: &super::super::Frame) {
        let mut history = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if history.paused || epoch != history.epoch {
            return;
        }
        let at = frame
            .published_at
            .saturating_duration_since(history.start)
            .as_secs_f64();
        if history.frames.back().is_some_and(|p| at <= p.at) {
            return;
        }
        let cut = history.frames.back().is_none_or(|p| {
            at - p.at > 0.25
                || (!Arc::ptr_eq(&p.scene, &history.scene)
                    && (p.scene.origin != history.scene.origin || p.frame.names != frame.names))
        });
        let mut recorded = frame.clone();
        if let Some(previous) = history
            .frames
            .back()
            .filter(|p| p.frame.names == frame.names)
        {
            recorded.names = previous.frame.names.clone();
        }
        let sample = Arc::new(Sample {
            at,
            frame: recorded,
            scene: history.scene.clone(),
            cut,
        });
        history.bytes += frame_bytes(&sample);
        let pointer = Arc::as_ptr(&sample.scene) as usize;
        let scene_bytes = std::mem::size_of::<Scene>()
            + sample.scene.signature.len() * 4
            + sample
                .scene
                .park
                .iter()
                .map(|o| {
                    std::mem::size_of::<Object>()
                        + o.parts.len() * std::mem::size_of::<benilla_protocol::park::Part>()
                })
                .sum::<usize>();
        let (count, _) = history.scenes.entry(pointer).or_insert((0, scene_bytes));
        let first = *count == 0;
        *count += 1;
        if first {
            history.bytes += scene_bytes;
        }
        history.frames.push_back(sample);
        history.prune(at);
    }

    pub(super) fn freeze(
        &self,
    ) -> (
        Vec<Arc<Sample>>,
        Vec<super::camera::Keyframe>,
        Vec<Arc<Visual>>,
    ) {
        let mut history = self.0.lock().unwrap_or_else(|p| p.into_inner());
        history.paused = true;
        history.paused_at = Some(Instant::now());
        let displayed = history.visuals.back().map_or(f64::INFINITY, |v| v.at);
        let frames: Vec<_> = history
            .frames
            .iter()
            .filter(|f| f.at <= displayed)
            .cloned()
            .collect();
        let cameras = frames
            .first()
            .zip(frames.last())
            .map_or_else(Vec::new, |(first, last)| {
                let mut cameras: Vec<_> = history
                    .cameras
                    .iter()
                    .filter(|c| c.at >= first.at && c.at <= last.at)
                    .cloned()
                    .collect();
                if let Some(before) = history
                    .cameras
                    .iter()
                    .rev()
                    .find(|c| c.at < first.at)
                    .filter(|_| cameras.first().is_none_or(|c| c.at > first.at))
                {
                    let mut before = before.clone();
                    before.at = first.at;
                    cameras.insert(0, before);
                }
                cameras
            });
        let visuals = frames
            .first()
            .zip(frames.last())
            .map_or_else(Vec::new, |(first, last)| {
                let mut result: Vec<_> = history
                    .visuals
                    .iter()
                    .filter(|v| v.at >= first.at && v.at <= last.at)
                    .cloned()
                    .collect();
                if let Some(before) = history
                    .visuals
                    .iter()
                    .rev()
                    .find(|v| v.at < first.at)
                    .filter(|_| result.first().is_none_or(|v| v.at > first.at))
                {
                    let mut before = (**before).clone();
                    before.at = first.at;
                    result.insert(0, Arc::new(before));
                }
                result
            });
        (frames, cameras, visuals)
    }

    pub(in crate::skate) fn resume(&self) {
        let mut history = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(at) = history.paused_at.take() {
            history.start += at.elapsed();
        }
        history.paused = false;
    }

    pub(in crate::skate) fn clear(&self) {
        let mut history = self.0.lock().unwrap_or_else(|p| p.into_inner());
        history.reset();
        history.paused = false;
        history.paused_at = None;
    }

    pub(in crate::skate) fn activate(&self, epoch: u64, origin: Vec3, scope: (u32, u32)) {
        let mut history = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if (history.scene.map, history.scene.instance) != scope {
            history.reset();
        }
        let mut scene = (*history.scene).clone();
        scene.map = scope.0;
        scene.instance = scope.1;
        scene.origin = origin;
        history.scene = Arc::new(scene);
        history.epoch = epoch;
    }

    pub(in crate::skate) fn camera(&self, tick: u64, transform: Transform, fov: f32) {
        let mut history = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if history.paused {
            return;
        }
        let Some(frame) = history.frames.iter().rev().find(|f| f.frame.tick == tick) else {
            return;
        };
        let at = frame.at;
        // A render can skip the native pose carrying a teleport cut. Propagate every cut since
        // the preceding displayed pose, including when the same tick is seated again.
        let after = history
            .cameras
            .iter()
            .rev()
            .find(|c| c.at < at)
            .map(|c| c.at);
        let cut = history.cut_after(after, at);
        let key = super::camera::Keyframe {
            at,
            pose: super::camera::pose(transform),
            fov,
            rate: 1.,
            cut,
        };
        if history.cameras.back().is_some_and(|c| c.at == at) {
            history.cameras.pop_back();
        }
        if history.cameras.back().is_none_or(|c| c.at < at) {
            history.cameras.push_back(key);
        }
        while history.cameras.len() > 2
            && (history.cameras.len() > MAX_FRAMES || history.cameras[1].at < at - history.window)
        {
            history.cameras.pop_front();
        }
    }

    pub(in crate::skate) fn visual(&self, tick: u64, transform: Transform, locals: &[Transform]) {
        let mut history = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if history.paused {
            return;
        }
        let Some(frame) = history.frames.iter().rev().find(|f| f.frame.tick == tick) else {
            return;
        };
        let at = frame.at;
        let after = history
            .visuals
            .iter()
            .rev()
            .find(|v| v.at < at)
            .map(|v| v.at);
        let visual = Arc::new(Visual {
            at,
            cut: history.cut_after(after, at),
            transform,
            locals: locals.to_vec().into(),
        });
        if let Some(old) = history.visuals.pop_back_if(|v| v.at == visual.at) {
            history.bytes -= visual_bytes(&old);
        }
        if history.visuals.back().is_none_or(|v| v.at < visual.at) {
            history.bytes += visual_bytes(&visual);
            history.visuals.push_back(visual);
        }
        let at = history.frames.back().map_or(0., |f| f.at);
        history.prune(at);
    }
}

pub(super) fn visual_pair(visuals: &[Arc<Visual>], cursor: f64) -> Option<(&Visual, &Visual, f32)> {
    let upper = visuals.partition_point(|v| v.at <= cursor);
    let a = visuals.get(upper.saturating_sub(1))?;
    let b = visuals.get(upper).unwrap_or(a);
    if b.cut || b.at <= a.at || b.locals.len() != a.locals.len() {
        return Some((a, a, 0.));
    }
    Some((a, b, ((cursor - a.at) / (b.at - a.at)).clamp(0., 1.) as f32))
}

pub(super) fn blend_transform(a: Transform, b: Transform, alpha: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, alpha),
        rotation: a.rotation.normalize().slerp(b.rotation.normalize(), alpha),
        scale: a.scale.lerp(b.scale, alpha),
    }
}

pub(super) fn blend_matrix(a: Mat4, b: Mat4, alpha: f32) -> Mat4 {
    blend_transform(Transform::from_matrix(a), Transform::from_matrix(b), alpha).to_matrix()
}

pub(super) fn pair(frames: &[Arc<Sample>], cursor: f64) -> Option<(&Sample, &Sample, f32)> {
    let upper = frames.partition_point(|f| f.at <= cursor);
    let before = frames.get(upper.saturating_sub(1))?;
    let after = frames.get(upper).unwrap_or(before);
    if after.cut || before.frame.names != after.frame.names || after.at <= before.at {
        return Some((before, before, 0.));
    }
    Some((
        before,
        after,
        ((cursor - before.at) / (after.at - before.at)).clamp(0., 1.) as f32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotations_interpolate_without_shearing() {
        let a = Mat4::IDENTITY;
        let b = Mat4::from_rotation_translation(Quat::from_rotation_y(2.), Vec3::X * 10.);
        let m = blend_matrix(a, b, 0.5);
        assert!((m.w_axis.x - 5.).abs() < 1e-5);
        assert!((m.x_axis.truncate().length() - 1.).abs() < 1e-5);
        assert!(m.x_axis.truncate().dot(m.z_axis.truncate()).abs() < 1e-5);
    }
    fn capture_at(recording: &Recording, at: f64, tick: u64) {
        let mut frame = super::super::files::tests::frame(tick);
        frame.published_at =
            recording.0.lock().unwrap().start + std::time::Duration::from_secs_f64(at);
        recording.capture(1, &frame);
    }
    #[test]
    fn rolling_history_is_bounded_and_keeps_a_scrub_bracket() {
        let recording = Recording::default();
        recording.activate(1, Vec3::ZERO, (0, 0));
        for tick in 0..11001 {
            capture_at(&recording, tick as f64 * 0.01, tick);
        }
        let (frames, _, _) = recording.freeze();
        assert!(frames.last().unwrap().at - frames.first().unwrap().at <= WINDOW + 0.011);
        assert!(frames.len() > 8900);
        assert!(frames.windows(2).all(|w| w[0].at < w[1].at));
    }
    #[test]
    fn window_changes_keep_frozen_edits_and_bound_subsequent_recording() {
        let recording = Recording::default();
        recording.activate(1, Vec3::ZERO, (0, 0));
        for tick in 0..600 {
            capture_at(&recording, tick as f64 * 0.1, tick);
            recording.visual(tick, Transform::IDENTITY, &[Transform::IDENTITY]);
            recording.camera(tick, Transform::IDENTITY, 1.);
        }
        let (frozen, _, _) = recording.freeze();
        recording.set_window(15.);
        assert!(frozen.last().unwrap().at - frozen.first().unwrap().at > 59.);
        recording.resume();
        capture_at(&recording, 60., 600);
        recording.visual(600, Transform::IDENTITY, &[Transform::IDENTITY]);
        recording.camera(600, Transform::IDENTITY, 1.);
        let (shorter, cameras, visuals) = recording.freeze();
        let span = shorter.last().unwrap().at - shorter.first().unwrap().at;
        assert!((15.0..=15.11).contains(&span));
        assert!(cameras.first().unwrap().at >= shorter.first().unwrap().at);
        assert!(visuals.first().unwrap().at >= shorter.first().unwrap().at);
        recording.set_window(90.);
        recording.resume();
        capture_at(&recording, 60.1, 601);
        let (expanded, _, _) = recording.freeze();
        assert!(expanded.first().unwrap().at >= shorter.first().unwrap().at);
        recording.set_window(f64::NAN);
        assert_eq!(recording.0.lock().unwrap().window, 90.);
    }
    #[test]
    fn pause_stale_epochs_and_reactivation_make_safe_cuts() {
        let recording = Recording::default();
        recording.activate(1, Vec3::ZERO, (0, 0));
        capture_at(&recording, 1., 1);
        capture_at(&recording, 1.01, 2);
        recording.freeze();
        capture_at(&recording, 1.02, 3);
        assert_eq!(recording.0.lock().unwrap().frames.len(), 2);
        recording.resume();
        recording.activate(2, Vec3::X, (0, 0));
        capture_at(&recording, 2., 4);
        assert_eq!(recording.0.lock().unwrap().frames.len(), 2);
        let mut frame = super::super::files::tests::frame(5);
        frame.published_at = recording.0.lock().unwrap().start + std::time::Duration::from_secs(2);
        recording.capture(2, &frame);
        let (frames, _, _) = recording.freeze();
        assert!(frames.last().unwrap().cut);
        let (a, b, t) = pair(&frames, 1.5).unwrap();
        assert_eq!(a.at, b.at);
        assert_eq!(t, 0.);
        recording.activate(3, Vec3::ZERO, (1, 0));
        assert!(recording.0.lock().unwrap().frames.is_empty());
    }
    #[test]
    fn memory_pressure_evicts_whole_frames_and_old_visuals() {
        let recording = Recording::default();
        recording.activate(1, Vec3::ZERO, (0, 0));
        for tick in 0..8 {
            capture_at(&recording, tick as f64 * 0.01, tick);
            recording.visual(tick, Transform::IDENTITY, &[Transform::IDENTITY]);
        }
        {
            let mut history = recording.0.lock().unwrap();
            history.bytes = MEMORY_LIMIT + 200;
            history.prune(0.07);
            assert!(history.frames.len() < 8);
            assert!(history.bytes <= MEMORY_LIMIT);
            assert!(history.visuals.front().unwrap().at <= history.frames.front().unwrap().at);
        }
    }
    #[test]
    fn freeze_excludes_undisplayed_poses_and_does_not_duplicate_brackets() {
        let recording = Recording::default();
        recording.activate(1, Vec3::ZERO, (0, 0));
        for tick in 0..4 {
            capture_at(&recording, tick as f64 * 0.1, tick);
            if tick < 3 {
                recording.visual(tick, Transform::IDENTITY, &[Transform::IDENTITY]);
                recording.camera(tick, Transform::IDENTITY, 1.);
            }
        }
        {
            let mut history = recording.0.lock().unwrap();
            let old = history.frames.pop_front().unwrap();
            history.bytes -= frame_bytes(&old);
        }
        let (frames, cameras, visuals) = recording.freeze();
        assert_eq!(frames.last().unwrap().frame.tick, 2);
        assert_eq!(cameras.len(), 2);
        assert_eq!(visuals.len(), 2);
        assert!(cameras.windows(2).all(|w| w[0].at < w[1].at));
        assert!(visuals.windows(2).all(|w| w[0].at < w[1].at));
    }

    #[test]
    fn a_skipped_teleport_pose_still_cuts_the_displayed_camera_and_character() {
        let recording = Recording::default();
        recording.activate(1, Vec3::ZERO, (0, 0));
        capture_at(&recording, 1., 1);
        recording.visual(1, Transform::IDENTITY, &[Transform::IDENTITY]);
        recording.camera(1, Transform::IDENTITY, 1.);
        recording.activate(2, Vec3::X * 10., (0, 0));
        for (tick, at) in [(2, 1.01), (3, 1.02), (4, 1.03)] {
            let mut frame = super::super::files::tests::frame(tick);
            frame.published_at =
                recording.0.lock().unwrap().start + std::time::Duration::from_secs_f64(at);
            recording.capture(2, &frame);
        }
        let moved = Transform::from_translation(Vec3::X * 10.);
        for _ in 0..2 {
            recording.visual(4, moved, &[Transform::IDENTITY]);
            recording.camera(4, moved, 1.);
        }
        let (_, cameras, visuals) = recording.freeze();
        assert!(cameras[1].cut);
        assert!(visuals[1].cut);
        let (a, b, alpha) = visual_pair(&visuals, 1.02).unwrap();
        assert_eq!(a.at, b.at);
        assert_eq!(alpha, 0.);
        let (camera, _, _) = super::super::camera::sample_recorded(&cameras, 1.02).unwrap();
        assert_eq!(camera.translation, Vec3::ZERO);
    }
}
