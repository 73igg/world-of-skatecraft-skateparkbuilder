//! Retarget rotations on the target's own bone lengths; contact corrections never stretch a limb.
use bevy::prelude::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct Leg {
    pub thigh: usize,
    pub knee: usize,
    pub foot: usize,
    pub source_length: f32,
    pub source_offset: Vec3,
    pub target_sole: f32,
}

fn parent(parents: &[i16], i: usize) -> Option<usize> {
    usize::try_from(parents[i]).ok().filter(|&p| p < i)
}

pub(super) fn compose(parents: &[i16], locals: &[Transform]) -> Vec<Transform> {
    let mut models: Vec<Transform> = Vec::with_capacity(locals.len());
    for (i, local) in locals.iter().enumerate() {
        let model = parent(parents, i).map_or(*local, |p| models[p] * *local);
        models.push(model);
    }
    models
}

fn rotate_joint(parents: &[i16], locals: &mut [Transform], i: usize, rotation: Quat) {
    let models = compose(parents, locals);
    locals[i].rotation = parent(parents, i)
        .map_or(rotation, |p| models[p].rotation.inverse() * rotation)
        .normalize();
}

/// The knee pole follows the animated bend plane, with a stable fallback for a straight leg.
fn knee_target(hip: Vec3, knee: Vec3, foot: Vec3, target: Vec3) -> Option<(Vec3, Vec3)> {
    let upper = hip.distance(knee);
    let lower = knee.distance(foot);
    let direction = (target - hip).try_normalize()?;
    if upper < 1e-5 || lower < 1e-5 {
        return None;
    }
    let minimum = (upper - lower).abs() + 1e-5;
    let maximum = (upper + lower - 1e-5).max(minimum);
    let distance = hip.distance(target).clamp(minimum, maximum);
    let along = (upper * upper - lower * lower + distance * distance) / (2. * distance);
    let height = (upper * upper - along * along).max(0.).sqrt();
    let pole = knee - hip - direction * (knee - hip).dot(direction);
    let pole = pole.try_normalize().unwrap_or_else(|| {
        let fallback = if direction.dot(Vec3::Z).abs() < 0.95 {
            Vec3::Z
        } else {
            Vec3::X
        };
        (fallback - direction * fallback.dot(direction)).normalize()
    });
    Some((
        hip + direction * along + pole * height,
        hip + direction * distance,
    ))
}

pub(super) fn plant(parents: &[i16], locals: &mut [Transform], leg: Leg, target: Vec3) {
    let models = compose(parents, locals);
    let (h, k, f) = (models[leg.thigh], models[leg.knee], models[leg.foot]);
    let Some((knee, foot)) = knee_target(h.translation, k.translation, f.translation, target)
    else {
        return;
    };
    let Some(from) = (k.translation - h.translation).try_normalize() else {
        return;
    };
    let Some(to) = (knee - h.translation).try_normalize() else {
        return;
    };
    rotate_joint(
        parents,
        locals,
        leg.thigh,
        Quat::from_rotation_arc(from, to) * h.rotation,
    );
    let models = compose(parents, locals);
    let k = models[leg.knee];
    let Some(from) = (models[leg.foot].translation - k.translation).try_normalize() else {
        return;
    };
    let Some(to) = (foot - k.translation).try_normalize() else {
        return;
    };
    rotate_joint(
        parents,
        locals,
        leg.knee,
        Quat::from_rotation_arc(from, to) * k.rotation,
    );
    rotate_joint(parents, locals, leg.foot, f.rotation);
}

pub(super) fn solve(
    parents: &[i16],
    binds: &[Vec3],
    animated: &[Transform],
    goals: &[Option<Transform>],
    hips: usize,
    legs: &[Leg],
) -> Option<Vec<Transform>> {
    if parents.len() != binds.len()
        || parents.len() != animated.len()
        || parents.len() != goals.len()
    {
        return None;
    }
    let hip_goal = goals.get(hips).copied().flatten()?;
    let mut locals = animated.to_vec();
    let mut models: Vec<Transform> = Vec::with_capacity(locals.len());
    for i in 0..locals.len() {
        let p = parent(parents, i);
        if let Some(goal) = goals[i] {
            locals[i].translation = binds[i];
            locals[i].scale = Vec3::ONE;
            locals[i].rotation = p
                .map_or(goal.rotation, |p| {
                    models[p].rotation.inverse() * goal.rotation
                })
                .normalize();
        } else if p.is_none() {
            locals[i].rotation = hip_goal.rotation;
        }
        models.push(p.map_or(locals[i], |p| models[p] * locals[i]));
    }
    let mut source_feet = Vec3::ZERO;
    let mut target_feet = Vec3::ZERO;
    let mut ratio = 0.;
    let mut count = 0.;
    for leg in legs {
        let Some(foot) = goals.get(leg.foot).copied().flatten() else {
            continue;
        };
        let length = models[leg.thigh]
            .translation
            .distance(models[leg.knee].translation)
            + models[leg.knee]
                .translation
                .distance(models[leg.foot].translation);
        if leg.source_length > 1e-5 && length > 1e-5 {
            source_feet += foot.translation;
            target_feet +=
                foot.translation + foot.rotation * Vec3::Y * leg.target_sole - leg.source_offset;
            ratio += length / leg.source_length;
            count += 1.;
        }
    }
    let desired_hips = if count > 0. {
        source_feet /= count;
        target_feet /= count;
        ratio /= count;
        target_feet + (hip_goal.translation - source_feet) * ratio
    } else {
        hip_goal.translation
    };
    let shift = desired_hips - models[hips].translation;
    for (i, local) in locals.iter_mut().enumerate() {
        if parent(parents, i).is_none() {
            local.translation += shift;
        }
    }
    if count > 0. {
        // Smaller bodies use a narrower stance on the same physical board.
        let stance = ratio.clamp(0.01, 1.);
        let normal = legs
            .iter()
            .map(|leg| leg.source_offset.normalize_or(Vec3::Y))
            .sum::<Vec3>()
            .normalize_or(Vec3::Y);
        let contacts: Vec<_> = legs
            .iter()
            .filter_map(|&leg| {
                let goal = goals[leg.foot]?;
                Some((
                    leg,
                    source_feet
                        + {
                            let delta = goal.translation - source_feet;
                            let height = normal * delta.dot(normal);
                            (delta - height) * stance + height
                        }
                        + goal.rotation * Vec3::Y * leg.target_sole
                        - leg.source_offset,
                ))
            })
            .collect();
        // A target's thigh sockets can sit higher than the human pelvis. Move the whole pelvis
        // within the legs' shared reach interval before planting; otherwise the feet hover.
        let models = compose(parents, &locals);
        let up = (desired_hips - target_feet).normalize_or(Vec3::Y);
        let mut intervals = vec![(f32::NEG_INFINITY, f32::INFINITY)];
        for &(leg, foot) in &contacts {
            let upper = models[leg.thigh]
                .translation
                .distance(models[leg.knee].translation);
            let lower = models[leg.knee]
                .translation
                .distance(models[leg.foot].translation);
            let v = models[leg.thigh].translation - foot;
            let along = v.dot(up);
            let perpendicular = v - up * along;
            let reach = (upper + lower - 1e-5).max(0.);
            let square = reach * reach - perpendicular.length_squared();
            if square >= 0. {
                let reach = square.sqrt();
                intervals = intervals
                    .into_iter()
                    .filter_map(|(a, b)| {
                        let a = a.max(-along - reach);
                        let b = b.min(-along + reach);
                        (a <= b).then_some((a, b))
                    })
                    .collect();
                // Unequal segments cannot fold all the way to the thigh socket. Exclude that
                // inner sphere as well, choosing the nearest shared reachable pelvis height.
                let inner = (upper - lower).abs() + 1e-5;
                let square = inner * inner - perpendicular.length_squared();
                if square > 0. {
                    let inner = square.sqrt();
                    let (lo, hi) = (-along - inner, -along + inner);
                    intervals = intervals
                        .into_iter()
                        .flat_map(|(a, b)| {
                            [
                                (a <= lo).then_some((a, b.min(lo))),
                                (b >= hi).then_some((a.max(hi), b)),
                            ]
                            .into_iter()
                            .flatten()
                        })
                        .collect();
                }
            }
        }
        if let Some(height) = intervals
            .into_iter()
            .map(|(a, b)| 0_f32.clamp(a, b))
            .min_by(|a, b| a.abs().total_cmp(&b.abs()).then_with(|| b.total_cmp(a)))
        {
            let correction = up * height;
            for (i, local) in locals.iter_mut().enumerate() {
                if parent(parents, i).is_none() {
                    local.translation += correction;
                }
            }
        }
        for (leg, foot) in contacts {
            plant(parents, &mut locals, leg, foot);
        }
    }
    locals
        .iter()
        .all(|t| t.translation.is_finite() && t.rotation.is_finite() && t.scale.is_finite())
        .then_some(locals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreachable_contact_keeps_both_leg_lengths() {
        let (knee, foot) =
            knee_target(Vec3::ZERO, Vec3::NEG_Y, Vec3::NEG_Y * 2., Vec3::Y * -20.).unwrap();
        assert!((knee.length() - 1.).abs() < 1e-4);
        assert!((knee.distance(foot) - 1.).abs() < 1e-4);
        assert!(foot.length() < 2.001);
    }

    #[test]
    fn very_small_and_large_bodies_keep_their_proportions() {
        for scale in [0.35, 0.6, 1., 1.7, 2.5] {
            let parents = [-1, 0, 1, 2];
            let binds = [
                Vec3::Y * scale,
                Vec3::X * 0.1 * scale,
                Vec3::NEG_Y * 0.45 * scale,
                Vec3::NEG_Y * 0.45 * scale,
            ];
            let animated = binds.map(Transform::from_translation);
            let goals = [
                Some(Transform::from_xyz(0., 1., 0.)),
                Some(Transform::from_xyz(0.1, 1., 0.)),
                Some(Transform::from_xyz(0.1, 0.55, 0.)),
                Some(Transform::from_xyz(0.1, 0.1, 0.)),
            ];
            let legs = [Leg {
                thigh: 1,
                knee: 2,
                foot: 3,
                source_length: 0.9,
                source_offset: Vec3::Y * 0.1,
                target_sole: 0.1 * scale,
            }];
            let result = solve(&parents, &binds, &animated, &goals, 0, &legs).unwrap();
            for i in 1..4 {
                assert!((result[i].translation.length() - binds[i].length()).abs() < 1e-5);
            }
            let models = compose(&parents, &result);
            assert!(
                (models[1].translation.distance(models[2].translation) - 0.45 * scale).abs() < 1e-4
            );
            assert!(
                (models[2].translation.distance(models[3].translation) - 0.45 * scale).abs() < 1e-4
            );
        }
    }

    #[test]
    fn a_degenerate_leg_falls_back_without_nan() {
        assert!(knee_target(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO).is_none());
    }
    #[test]
    fn high_thigh_sockets_lower_the_pelvis_instead_of_hovering_feet() {
        let parents = [-1, 0, 1, 2];
        let binds = [
            Vec3::Y,
            Vec3::new(0.1, 0.3, 0.),
            Vec3::NEG_Y * 0.45,
            Vec3::NEG_Y * 0.45,
        ];
        let animated = binds.map(Transform::from_translation);
        let goals = [
            Some(Transform::from_xyz(0., 1., 0.)),
            Some(Transform::from_xyz(0.1, 1., 0.)),
            Some(Transform::from_xyz(0.1, 0.55, 0.)),
            Some(Transform::from_xyz(0.1, 0.1, 0.)),
        ];
        let leg = Leg {
            thigh: 1,
            knee: 2,
            foot: 3,
            source_length: 0.9,
            source_offset: Vec3::Y * 0.1,
            target_sole: 0.1,
        };
        let locals = solve(&parents, &binds, &animated, &goals, 0, &[leg]).unwrap();
        let models = compose(&parents, &locals);
        assert!(models[3].translation.distance(Vec3::new(0.1, 0.1, 0.)) < 1e-4);
        assert!(models[0].translation.y < 0.71);
        assert!((models[1].translation.distance(models[2].translation) - 0.45).abs() < 1e-4);
    }

    #[test]
    fn unequal_leg_segments_keep_contact_during_a_deep_crouch() {
        let parents = [-1, 0, 1, 2];
        let binds = [
            Vec3::Y,
            Vec3::X * 0.1,
            Vec3::NEG_Y * 0.25,
            Vec3::NEG_Y * 0.75,
        ];
        let animated = binds.map(Transform::from_translation);
        let goals = [
            Some(Transform::from_xyz(0., 0.25, 0.)),
            Some(Transform::from_xyz(0.1, 0.25, 0.)),
            Some(Transform::from_xyz(0.1, 0., 0.)),
            Some(Transform::from_xyz(0.1, 0.1, 0.)),
        ];
        let leg = Leg {
            thigh: 1,
            knee: 2,
            foot: 3,
            source_length: 1.,
            source_offset: Vec3::Y * 0.1,
            target_sole: 0.1,
        };
        let result = solve(&parents, &binds, &animated, &goals, 0, &[leg]).unwrap();
        let models = compose(&parents, &result);
        assert!(models[3].translation.distance(Vec3::new(0.1, 0.1, 0.)) < 1e-4);
        for i in 1..4 {
            assert!(result[i].translation.distance(binds[i]) < 1e-5);
        }
    }
}
