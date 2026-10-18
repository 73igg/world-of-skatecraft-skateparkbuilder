//! Camera edits share the replay clock; changing a shot never changes the captured action.
use super::timeline::blend_transform;
use benilla_protocol::park::Pose;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Mode {
    #[default]
    Recorded,
    Free,
    Follow,
    Tripod,
}

impl Mode {
    pub fn next(self) -> Self {
        match self {
            Self::Recorded => Self::Free,
            Self::Free => Self::Follow,
            Self::Follow => Self::Tripod,
            Self::Tripod => Self::Recorded,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Recorded => "Recorded",
            Self::Free => "Free",
            Self::Follow => "Follow",
            Self::Tripod => "Tripod",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Keyframe {
    pub at: f64,
    pub pose: Pose,
    pub fov: f32,
    pub cut: bool,
    pub rate: f32,
    // Outgoing segment; older clips used an ease at every key.
    #[serde(default)]
    pub interpolation: Interpolation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Interpolation {
    Linear,
    #[default]
    Ease,
    Smooth,
}
impl Interpolation {
    pub fn next(self) -> Self {
        match self {
            Self::Smooth => Self::Linear,
            Self::Linear => Self::Ease,
            Self::Ease => Self::Smooth,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Ease => "Ease in/out",
            Self::Smooth => "Smooth continuous",
        }
    }
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(super) enum MoveResponse {
    Direct,
    #[default]
    Smooth,
    Soft,
}
impl MoveResponse {
    pub fn next(self) -> Self {
        match self {
            Self::Direct => Self::Smooth,
            Self::Smooth => Self::Soft,
            Self::Soft => Self::Direct,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direct",
            Self::Smooth => "Smooth",
            Self::Soft => "Soft",
        }
    }
    fn seconds(self) -> f32 {
        match self {
            Self::Direct => 0.,
            Self::Smooth => 0.12,
            Self::Soft => 0.3,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct FlySettings {
    pub speed: f32,
    pub response: MoveResponse,
}
impl Default for FlySettings {
    fn default() -> Self {
        Self {
            speed: 8.,
            response: MoveResponse::Smooth,
        }
    }
}
impl FlySettings {
    pub fn sanitize(mut self) -> Self {
        self.speed = if self.speed.is_finite() {
            self.speed.clamp(1., 64.)
        } else {
            Self::default().speed
        };
        self
    }
    pub fn movement(self, velocity: &mut Vec3, direction: Vec3, fast: bool, dt: f32) -> Vec3 {
        let target = direction.clamp_length_max(1.) * self.speed * if fast { 2. } else { 1. };
        let tau = self.response.seconds();
        if tau == 0. {
            *velocity = target;
            return target * dt;
        }
        // Integrate exponential velocity response exactly, including Shift changes and releases.
        let decay = (-dt / tau).exp();
        let offset = *velocity - target;
        *velocity = target + offset * decay;
        target * dt + offset * tau * (1. - decay)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Settings {
    pub mode: Mode,
    pub pose: Pose,
    pub fov: f32,
    pub use_keys: bool,
    pub follow_offset: [f32; 3],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Recorded,
            pose: pose(Transform::IDENTITY),
            fov: 60_f32.to_radians(),
            use_keys: true,
            follow_offset: [0.; 3],
        }
    }
}

pub(super) fn pose(transform: Transform) -> Pose {
    Pose {
        position: transform.translation.to_array(),
        rotation: transform.rotation.normalize().to_array(),
        scale: [1.; 3],
    }
}

pub(super) fn transform(pose: Pose) -> Transform {
    Transform::from_translation(Vec3::from_array(pose.position))
        .with_rotation(Quat::from_array(pose.rotation).normalize())
}

pub(super) fn sample(keys: &[Keyframe], at: f64) -> Option<(Transform, f32, f32)> {
    sample_impl(keys, at, true)
}
pub(super) fn sample_recorded(keys: &[Keyframe], at: f64) -> Option<(Transform, f32, f32)> {
    sample_impl(keys, at, false)
}
fn sample_impl(keys: &[Keyframe], at: f64, edited: bool) -> Option<(Transform, f32, f32)> {
    let first = keys.first()?;
    if at <= first.at {
        return Some((transform(first.pose), first.fov, first.rate));
    }
    let upper = keys.partition_point(|k| k.at <= at);
    let index = upper.saturating_sub(1);
    let a = &keys[index];
    let b = keys.get(upper).unwrap_or(a);
    if at == a.at || b.at <= a.at || b.cut {
        return Some((transform(a.pose), a.fov, a.rate));
    }
    let t = ((at - a.at) / (b.at - a.at)).clamp(0., 1.) as f32;
    let interpolation = if edited {
        a.interpolation
    } else {
        Interpolation::Linear
    };
    if interpolation == Interpolation::Smooth && !b.cut && b.at > a.at {
        let dt = (b.at - a.at) as f32;
        let position_delta = |a: &Keyframe, b: &Keyframe| {
            Vec3::from_array(b.pose.position) - Vec3::from_array(a.pose.position)
        };
        let qa = transform(a.pose).rotation;
        let qb = transform(b.pose).rotation;
        let q1 =
            Quat::from_scaled_axis(bounded_tangent(keys, index, rotation_delta) * dt / 3.) * qa;
        let q2 =
            Quat::from_scaled_axis(-bounded_tangent(keys, index + 1, rotation_delta) * dt / 3.)
                * qb;
        let q01 = qa.slerp(q1, t);
        let q12 = q1.slerp(q2, t);
        let q23 = q2.slerp(qb, t);
        return Some((
            Transform::from_translation(hermite(
                Vec3::from_array(a.pose.position),
                Vec3::from_array(b.pose.position),
                bounded_tangent(keys, index, position_delta) * dt,
                bounded_tangent(keys, index + 1, position_delta) * dt,
                t,
            ))
            .with_rotation(q01.slerp(q12, t).slerp(q12.slerp(q23, t), t).normalize()),
            hermite(
                Vec3::X * a.fov,
                Vec3::X * b.fov,
                Vec3::X * fov_tangent(keys, index) * dt,
                Vec3::X * fov_tangent(keys, index + 1) * dt,
                t,
            )
            .x
            .clamp(a.fov.min(b.fov), a.fov.max(b.fov)),
            a.rate + (b.rate - a.rate) * t,
        ));
    }
    let ease = if interpolation == Interpolation::Ease {
        t * t * (3. - 2. * t)
    } else {
        t
    };
    Some((
        blend_transform(transform(a.pose), transform(b.pose), ease),
        a.fov + (b.fov - a.fov) * ease,
        a.rate + (b.rate - a.rate) * t,
    ))
}

fn shortest(q: Quat) -> Quat {
    if q.w < 0. {
        -q
    } else {
        q
    }
}

fn hermite(a: Vec3, b: Vec3, va: Vec3, vb: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    // Relative travel keeps stationary shots exact even at large map coordinates.
    a + (b - a) * (-2. * t3 + 3. * t2) + va * (t3 - 2. * t2 + t) + vb * (t3 - t2)
}

fn tangent(keys: &[Keyframe], index: usize, delta: impl Fn(&Keyframe, &Keyframe) -> Vec3) -> Vec3 {
    let Some(previous) = index.checked_sub(1).and_then(|i| keys.get(i)) else {
        return Vec3::ZERO;
    };
    let key = &keys[index];
    let Some(next) = keys.get(index + 1) else {
        return Vec3::ZERO;
    };
    if key.cut
        || next.cut
        || previous.interpolation == Interpolation::Ease
        || key.interpolation == Interpolation::Ease
    {
        return Vec3::ZERO;
    }
    let before = (key.at - previous.at) as f32;
    let after = (next.at - key.at) as f32;
    if before < 1e-6 || after < 1e-6 {
        return Vec3::ZERO;
    }
    let left = delta(previous, key) / before;
    let right = delta(key, next) / after;
    if previous.interpolation == Interpolation::Linear {
        left
    } else if key.interpolation == Interpolation::Linear {
        right
    } else {
        (left * after + right * before) / (before + after)
    }
}

fn rotation_delta(a: &Keyframe, b: &Keyframe) -> Vec3 {
    shortest(
        Quat::from_array(b.pose.rotation).normalize()
            * Quat::from_array(a.pose.rotation).normalize().inverse(),
    )
    .to_scaled_axis()
}

fn bounded_tangent(
    keys: &[Keyframe],
    index: usize,
    delta: impl Fn(&Keyframe, &Keyframe) -> Vec3 + Copy,
) -> Vec3 {
    let velocity = tangent(keys, index, delta);
    if velocity == Vec3::ZERO {
        return velocity;
    }
    let left = delta(&keys[index - 1], &keys[index]).length()
        / (keys[index].at - keys[index - 1].at) as f32;
    let right = delta(&keys[index], &keys[index + 1]).length()
        / (keys[index + 1].at - keys[index].at) as f32;
    // Bound both handles by their adjacent travel, even when key spacing changes sharply.
    velocity.clamp_length_max(3. * left.min(right))
}

fn fov_tangent(keys: &[Keyframe], index: usize) -> f32 {
    let slope = tangent(keys, index, |a, b| Vec3::X * (b.fov - a.fov)).x;
    if slope == 0. {
        return 0.;
    }
    let left =
        (keys[index].fov - keys[index - 1].fov) / (keys[index].at - keys[index - 1].at) as f32;
    let right =
        (keys[index + 1].fov - keys[index].fov) / (keys[index + 1].at - keys[index].at) as f32;
    if left * right <= 0. {
        0.
    } else {
        slope.signum() * slope.abs().min(3. * left.abs().min(right.abs()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn smooth_key(at: f64) -> Keyframe {
        Keyframe {
            at,
            pose: pose(
                Transform::from_xyz(at as f32, 0., 0.)
                    .with_rotation(Quat::from_rotation_y(at as f32 * 0.3)),
            ),
            fov: 0.5 + at as f32 * 0.1,
            rate: 1.,
            cut: false,
            interpolation: Interpolation::Smooth,
        }
    }
    #[test]
    fn continuous_keys_share_velocity_with_unequal_spacing() {
        let keys = [smooth_key(0.), smooth_key(1.), smooth_key(3.)];
        let e = 0.001;
        let (a, fa, _) = sample(&keys, 1. - e).unwrap();
        let (b, fb, _) = sample(&keys, 1.).unwrap();
        let (c, fc, _) = sample(&keys, 1. + e).unwrap();
        let left = (b.translation - a.translation) / e as f32;
        let right = (c.translation - b.translation) / e as f32;
        assert!((left - Vec3::X).length() < 0.005);
        assert!((left - right).length() < 0.005);
        let left_turn = shortest(b.rotation * a.rotation.inverse()).to_scaled_axis() / e as f32;
        let right_turn = shortest(c.rotation * b.rotation.inverse()).to_scaled_axis() / e as f32;
        assert!((left_turn - Vec3::Y * 0.3).length() < 0.005);
        assert!((left_turn - right_turn).length() < 0.005);
        assert!(((fb - fa) / e as f32 - (fc - fb) / e as f32).abs() < 0.005);
    }
    #[test]
    fn uneven_spacing_does_not_launch_the_camera_past_a_straight_shot() {
        let mut keys = [smooth_key(0.), smooth_key(0.1), smooth_key(3.)];
        keys[1].pose.position = [1., 0., 0.];
        keys[2].pose.position = [2., 0., 0.];
        keys[1].pose.rotation = Quat::from_rotation_y(2.).to_array();
        keys[2].pose.rotation = Quat::from_rotation_y(2.1).to_array();
        for i in 0..1001 {
            let (pose, _, _) = sample(&keys, 0.1 + f64::from(i) * 2.9 / 1000.).unwrap();
            assert!((1. - 1e-6..=2. + 1e-6).contains(&pose.translation.x));
            assert!((pose.rotation.length() - 1.).abs() < 1e-6);
        }
    }
    #[test]
    fn stationary_shots_do_not_drift_at_map_coordinates() {
        let position = Vec3::new(17_000.25, 23_000.5, -12_500.75);
        let mut keys = [smooth_key(0.), smooth_key(1.), smooth_key(3.)];
        for key in &mut keys {
            key.pose.position = position.to_array();
        }
        for i in 0..1001 {
            assert_eq!(
                sample(&keys, f64::from(i) * 0.003).unwrap().0.translation,
                position
            );
        }
    }
    #[test]
    fn smooth_paths_respect_cuts_and_keep_zoom_bounded() {
        let mut keys = [
            smooth_key(0.),
            smooth_key(0.1),
            smooth_key(2.),
            smooth_key(3.),
        ];
        keys[0].fov = 2.;
        keys[1].fov = 0.1;
        keys[2].fov = 1.;
        keys[3].cut = true;
        let before = sample(&keys, 1.).unwrap();
        keys[3].pose = pose(Transform::from_xyz(1000., 1000., 1000.));
        assert_eq!(sample(&keys, 1.).unwrap(), before);
        assert_eq!(sample(&keys, 2.9).unwrap().0, transform(keys[2].pose));
        assert_eq!(sample(&keys, 3.).unwrap().0, transform(keys[3].pose));
        for i in 0..1000 {
            let at = f64::from(i) * 0.002;
            let (_, fov, _) = sample(&keys, at).unwrap();
            assert!((0.1 - 1e-6..=2. + 1e-6).contains(&fov));
        }
    }
    #[test]
    fn legacy_keys_keep_easing_and_recorded_cameras_stay_linear() {
        let mut json = serde_json::to_value(smooth_key(0.)).unwrap();
        json.as_object_mut().unwrap().remove("interpolation");
        let key: Keyframe = serde_json::from_value(json).unwrap();
        assert_eq!(key.interpolation, Interpolation::Ease);
        let keys = [key, smooth_key(1.)];
        assert!((sample(&keys, 0.25).unwrap().0.translation.x - 0.15625).abs() < 1e-6);
        assert!((sample_recorded(&keys, 0.25).unwrap().0.translation.x - 0.25).abs() < 1e-6);
    }
    #[test]
    fn fly_acceleration_boost_and_release_are_frame_rate_independent() {
        let fly = FlySettings::default();
        let run = |fps: u32| {
            let mut velocity = Vec3::ZERO;
            let mut position = Vec3::ZERO;
            for phase in 0..3 {
                for _ in 0..fps {
                    position += fly.movement(
                        &mut velocity,
                        if phase == 2 { Vec3::ZERO } else { Vec3::X },
                        phase == 1,
                        1. / fps as f32,
                    );
                }
            }
            (position, velocity)
        };
        let (expected, _) = run(30);
        for fps in [60, 144] {
            let (position, velocity) = run(fps);
            assert!((expected - position).length() < 0.001);
            assert!(velocity.length() < 0.01);
        }
        let mut velocity = Vec3::X * fly.speed;
        fly.movement(&mut velocity, Vec3::X, true, 1. / 60.);
        assert!(velocity.x > fly.speed && velocity.x < fly.speed * 2.);
        let direct = FlySettings {
            response: MoveResponse::Direct,
            ..fly
        };
        assert_eq!(
            direct.movement(&mut velocity, Vec3::ZERO, false, 1.),
            Vec3::ZERO
        );
    }
    #[test]
    fn cuts_and_endpoints_are_exact() {
        let key = |at, x, cut| Keyframe {
            at,
            pose: pose(Transform::from_xyz(x, 0., 0.)),
            fov: 1.,
            rate: 1.,
            cut,
            interpolation: Interpolation::Ease,
        };
        let keys = [key(0., 0., false), key(1., 10., true)];
        assert_eq!(sample(&keys, -1.).unwrap().0.translation.x, 0.);
        assert_eq!(sample(&keys, 0.9).unwrap().0.translation.x, 0.);
        assert_eq!(sample(&keys, 1.).unwrap().0.translation.x, 10.);
    }
    #[test]
    fn edits_do_not_overshoot_their_segment() {
        let keys = [
            Keyframe {
                at: 0.,
                pose: pose(Transform::IDENTITY),
                fov: 1.,
                rate: 0.25,
                cut: false,
                interpolation: Interpolation::Ease,
            },
            Keyframe {
                at: 1.,
                pose: pose(Transform::from_xyz(3., 2., 1.)),
                fov: 0.5,
                rate: 2.,
                cut: false,
                interpolation: Interpolation::Ease,
            },
        ];
        for i in 0..101 {
            let (t, f, rate) = sample(&keys, f64::from(i) / 100.).unwrap();
            assert!((0. ..=3.).contains(&t.translation.x));
            assert!((0.5..=1.).contains(&f));
            assert!((0.25..=2.).contains(&rate));
        }
    }
}
