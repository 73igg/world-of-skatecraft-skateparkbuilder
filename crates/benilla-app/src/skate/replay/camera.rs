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
fn sample_impl(keys: &[Keyframe], at: f64, smooth: bool) -> Option<(Transform, f32, f32)> {
    let first = keys.first()?;
    if at <= first.at {
        return Some((transform(first.pose), first.fov, first.rate));
    }
    let upper = keys.partition_point(|k| k.at <= at);
    let a = &keys[upper.saturating_sub(1)];
    let b = keys.get(upper).unwrap_or(a);
    let t = if b.at <= a.at || b.cut {
        0.
    } else {
        ((at - a.at) / (b.at - a.at)).clamp(0., 1.) as f32
    };
    // Easing stays inside the keyframe endpoints, so interpolation cannot overshoot into terrain.
    let ease = if smooth { t * t * (3. - 2. * t) } else { t };
    Some((
        blend_transform(transform(a.pose), transform(b.pose), ease),
        a.fov + (b.fov - a.fov) * ease,
        a.rate + (b.rate - a.rate) * t,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cuts_and_endpoints_are_exact() {
        let key = |at, x, cut| Keyframe {
            at,
            pose: pose(Transform::from_xyz(x, 0., 0.)),
            fov: 1.,
            rate: 1.,
            cut,
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
            },
            Keyframe {
                at: 1.,
                pose: pose(Transform::from_xyz(3., 2., 1.)),
                fov: 0.5,
                rate: 2.,
                cut: false,
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
