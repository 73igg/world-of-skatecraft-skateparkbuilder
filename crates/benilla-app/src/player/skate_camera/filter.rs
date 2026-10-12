//! Camera distance and damping are applied before the final world sweep, in yards.
use bevy::prelude::*;

const SKIN: f32 = 0.025;
#[derive(Clone, Copy)]
pub(super) struct Goal {
    pub root: Vec3,
    pub camera: Transform,
    pub height: f32,
    pub distance: f32,
    pub speed: f32,
    pub radius: f32,
    pub shake: Vec3,
}
pub(super) struct Seat {
    pub transform: Transform,
    pub pivot: Vec3,
    pub clipped: bool,
}
#[derive(Resource, Default)]
pub(super) struct Follow {
    seeded: bool,
    root: Vec3,
    offset: Vec3,
    focus: Vec3,
    offset_stage: Vec3,
    focus_stage: Vec3,
    orientation: Quat,
    height: f32,
    arm: f32,
}
impl Follow {
    pub(super) fn reset(&mut self) {
        self.seeded = false;
    }
    pub(super) fn seat(
        &mut self,
        dt: f32,
        goal: Goal,
        mut sweep: impl FnMut(Vec3, Vec3, f32) -> Option<f32>,
    ) -> Seat {
        let reset = !self.seeded
            || dt > 0.5
            || goal.root.distance(self.root) > goal.speed * dt.max(0.) * 3. + 5.;
        let dt = dt.clamp(0., 0.1);
        let height = goal.height.clamp(0.5, 2.5).max(goal.radius + 0.15);
        self.height = if reset {
            height
        } else {
            self.height + (height - self.height) * response(dt, 0.4)
        }
        .max(goal.radius + 0.15);
        let pivot = goal.root + Vec3::Y * self.height;
        let forward = goal.camera.rotation * Vec3::NEG_Z;
        let reach = (pivot - goal.camera.translation).dot(forward).max(0.1);
        let focus = goal.camera.translation + forward * reach - goal.root;
        let native = goal.camera.translation - goal.root;
        // Extending the horizontal follow arm keeps Low/High height intact. Animated hips
        // must not become a lever that magnifies each push's bone motion by (distance - 1).
        let offset = Vec3::new(native.x * goal.distance, native.y, native.z * goal.distance);
        let fast = ((goal.speed - 12.) / 32.).clamp(0., 1.);
        let tau = 0.009 + fast * 0.071;
        if reset {
            self.offset = offset;
            self.focus = focus;
            self.offset_stage = offset;
            self.focus_stage = focus;
            self.orientation = goal.camera.rotation;
        } else {
            damp(&mut self.offset_stage, &mut self.offset, offset, dt, tau);
            damp(&mut self.focus_stage, &mut self.focus, focus, dt, tau);
            self.orientation = self
                .orientation
                .slerp(goal.camera.rotation, response(dt, 0.015 + fast * 0.065))
                .normalize();
        }
        // Carry world translation directly with the skater; damping absolute position would
        // create a growing follow lag at boosted speed. Collide after damping and game shake.
        let wish = goal.root + self.offset + goal.shake;
        let boom = wish - pivot;
        let length = boom.length();
        let hit = sweep(pivot, boom, goal.radius);
        let open = if length > 1e-6 {
            hit.map_or(1., |d| (d - SKIN).clamp(0., length) / length)
        } else {
            1.
        };
        self.arm = if reset || open < self.arm {
            open
        } else {
            self.arm + (open - self.arm) * response(dt, 0.18)
        };
        self.arm = self.arm.min(open);
        let eye = pivot + boom * self.arm;
        let focus = goal.root + self.focus;
        let mut transform = Transform::from_translation(eye);
        transform.rotation = self.orientation;
        if eye.distance_squared(focus) > 1e-8 {
            transform = transform.looking_at(focus, self.orientation * Vec3::Y);
        }
        self.seeded = true;
        self.root = goal.root;
        Seat {
            transform,
            pivot,
            clipped: hit.is_some(),
        }
    }
}
fn response(dt: f32, tau: f32) -> f32 {
    1. - (-dt / tau).exp()
}

/// Exact integration of two equal low-pass stages: smooth acceleration without an overshoot,
/// and the same response at different frame rates. The two stages share the follow delay.
fn damp(stage: &mut Vec3, output: &mut Vec3, target: Vec3, dt: f32, tau: f32) {
    let error = *stage - target;
    let t = dt / tau;
    let decay = (-t).exp();
    *output = target + (*output - target + error * t) * decay;
    *stage = target + error * decay;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn goal(root: Vec3, eye: Vec3) -> Goal {
        Goal {
            root,
            camera: Transform::from_translation(root + eye).looking_at(root + Vec3::Y, Vec3::Y),
            height: 1.,
            distance: 4.,
            speed: 75.,
            radius: 0.3,
            shake: Vec3::ZERO,
        }
    }
    #[test]
    fn distance_preserves_low_height_and_does_not_use_animated_hip_motion() {
        let mut follow = Follow::default();
        let g = goal(Vec3::ZERO, Vec3::new(0.7, 0.6, 2.));
        let seat = follow.seat(1. / 60., g, |_, _, _| None);
        assert!((seat.transform.translation.y - 0.6).abs() < 1e-5);
        assert!((seat.transform.translation.x - 2.8).abs() < 1e-5);
        for height in [0.7, 1.3, 0.8, 1.1] {
            let seat = follow.seat(1. / 60., Goal { height, ..g }, |_, _, _| None);
            assert!((seat.transform.translation.y - 0.6).abs() < 1e-5);
        }
    }
    #[test]
    fn clipping_is_immediate_and_release_is_smooth() {
        let mut follow = Follow::default();
        let g = goal(Vec3::ZERO, Vec3::new(0., 1., 3.));
        follow.seat(1. / 60., g, |_, _, _| None);
        let clipped = follow.seat(1. / 60., g, |_, _, _| Some(2.));
        assert!(clipped.transform.translation.distance(clipped.pivot) < 2.);
        let clear = follow.seat(1. / 60., g, |_, _, _| None);
        assert!(clear.transform.translation.z > clipped.transform.translation.z);
        assert!(clear.transform.translation.z < 3.);
    }
    #[test]
    fn world_translation_has_no_speed_dependent_lag_and_teleports_reset() {
        let mut follow = Follow::default();
        let a = follow.seat(
            1. / 60.,
            goal(Vec3::ZERO, Vec3::new(0., 2., 3.)),
            |_, _, _| None,
        );
        let root = Vec3::new(0., 0., 75. / 60.);
        let b = follow.seat(1. / 60., goal(root, Vec3::new(0., 2., 3.)), |_, _, _| None);
        assert!((b.transform.translation - a.transform.translation - root).length() < 1e-5);
        let root = Vec3::new(1000., 30., 1000.);
        let c = follow.seat(1. / 60., goal(root, Vec3::new(0., 2., 3.)), |_, _, _| None);
        assert!((c.transform.translation - root - Vec3::new(0., 2., 12.)).length() < 1e-4);
    }
    #[test]
    fn safety_sweep_sees_smoothed_distance_and_last_game_shake() {
        let mut follow = Follow::default();
        let mut g = goal(Vec3::ZERO, Vec3::new(0., 0.6, 2.));
        g.shake = Vec3::new(0., -1., 0.);
        let s = follow.seat(1. / 60., g, |p, d, r| Some(((p.y - r) / -d.y) * d.length()));
        assert!(s.transform.translation.y >= g.radius);
    }

    #[test]
    fn damping_and_obstruction_release_do_not_depend_on_frame_rate() {
        let sample = |fps: usize| {
            let mut follow = Follow::default();
            let initial = goal(Vec3::ZERO, Vec3::new(0., 1., 3.));
            follow.seat(1. / fps as f32, initial, |_, _, _| Some(2.));
            let changed = goal(Vec3::ZERO, Vec3::new(2., 1.5, 3.));
            let mut last = Vec3::ZERO;
            for _ in 0..fps / 2 {
                last = follow
                    .seat(1. / fps as f32, changed, |_, _, _| None)
                    .transform
                    .translation;
            }
            last
        };
        assert!(sample(30).distance(sample(144)) < 0.002);
    }

    #[test]
    fn mode_switch_and_long_pause_discard_the_previous_follow_pose() {
        let mut follow = Follow::default();
        follow.seat(
            1. / 60.,
            goal(Vec3::ZERO, Vec3::new(0., 1., 3.)),
            |_, _, _| Some(1.),
        );
        let changed = goal(Vec3::ZERO, Vec3::new(2., 2., 2.));
        follow.reset();
        let a = follow.seat(1. / 60., changed, |_, _, _| None);
        assert!(a.transform.translation.distance(Vec3::new(8., 2., 8.)) < 1e-5);
        let b = follow.seat(1., goal(Vec3::ZERO, Vec3::new(0., 1., 3.)), |_, _, _| None);
        assert!(b.transform.translation.distance(Vec3::new(0., 1., 12.)) < 1e-5);
    }
}
