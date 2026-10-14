//! Read-only checks against every installed vanilla player skeleton; no game data is committed.
use super::*;
use benilla_assets::{coords::wow_to_bevy, ModelJoint, ModelSkeleton};

fn skeleton(raw: &benilla_formats::Skeleton) -> ModelSkeleton {
    let pivots: Vec<_> = raw.bones.iter().map(|b| wow_to_bevy(b.pivot)).collect();
    ModelSkeleton {
        joints: raw
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let parent = usize::try_from(b.parent).ok().filter(|&p| p < i);
                ModelJoint {
                    parent: b.parent,
                    local_translation: pivots[i] - parent.map_or(Vec3::ZERO, |p| pivots[p]),
                    billboard: b.billboard,
                    parent_arm: b.parent_arm,
                }
            })
            .collect(),
        key_bones: raw.bones.iter().map(|b| b.key_bone).collect(),
        ..default()
    }
}

#[test]
fn all_installed_races_keep_natural_limb_lengths_in_crouched_poses() {
    let data = benilla_formats::wow_data_or_skip!();
    let chain = benilla_formats::Chain::open(&data).unwrap();
    if reference().is_none() {
        eprintln!("SKIP: local Skate rig reference is unavailable");
        return;
    }
    for race in [
        "Human", "Orc", "Dwarf", "NightElf", "Scourge", "Tauren", "Gnome", "Troll",
    ] {
        for sex in ["Male", "Female"] {
            let path = format!("Character\\{race}\\{sex}\\{race}{sex}.m2");
            let bytes = chain.read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let raw = benilla_formats::parse_m2_skeleton(&bytes).unwrap();
            let skeleton = skeleton(&raw);
            let rig = RigPose::new(Entity::PLACEHOLDER, &skeleton);
            let mut plan = Plan::default();
            let plan = plan.for_rig(&rig);
            let index = |name: &str| {
                plan.fits
                    .iter()
                    .position(|f| f.is_some_and(|f| f.0 == name))
            };
            let hip = index("HIPS").unwrap();
            for model_scale in [0.5, 1., 1.75] {
                let units = 1. / METERS_PER_YARD / model_scale;
                let legs: Vec<_> = [
                    ["LEFTUPLEG", "LEFTLEG", "LEFTFOOT"],
                    ["RIGHTUPLEG", "RIGHTLEG", "RIGHTFOOT"],
                ]
                .into_iter()
                .map(|names| {
                    let [thigh, knee, foot] = names.map(|name| {
                        index(name).unwrap_or_else(|| panic!("{race} {sex}: missing {name}"))
                    });
                    let a = reference_bind(names[0]).unwrap().w_axis.truncate();
                    let b = reference_bind(names[1]).unwrap().w_axis.truncate();
                    let c = reference_bind(names[2]).unwrap().w_axis.truncate();
                    solve::Leg {
                        thigh,
                        knee,
                        foot,
                        source_length: (a.distance(b) + b.distance(c)) * units,
                        source_offset: Vec3::Y * c.y * units,
                        target_sole: plan.pivots[foot].y.max(0.),
                    }
                })
                .collect();
                for crouch in [0., 0.2, 0.45] {
                    let mut goals: Vec<_> = plan
                        .fits
                        .iter()
                        .enumerate()
                        .map(|(i, fit)| {
                            fit.map(|(_, fit)| {
                                Transform::from_matrix(
                                    Mat4::from_scale(Vec3::splat(units))
                                        * fit
                                        * Mat4::from_translation(plan.pivots[i] * METERS_PER_YARD),
                                )
                            })
                        })
                        .collect();
                    goals[hip].as_mut().unwrap().translation.y -= crouch * units;
                    let result =
                        solve::solve(&plan.parents, &plan.binds, &rig.locals, &goals, hip, &legs)
                            .unwrap();
                    let models = solve::compose(&plan.parents, &result);
                    let mean = legs
                        .iter()
                        .map(|leg| goals[leg.foot].unwrap().translation)
                        .sum::<Vec3>()
                        / legs.len() as f32;
                    let ratio = legs
                        .iter()
                        .map(|leg| {
                            (plan.binds[leg.knee].length() + plan.binds[leg.foot].length())
                                / leg.source_length
                        })
                        .sum::<f32>()
                        / legs.len() as f32;
                    for leg in &legs {
                        let goal = goals[leg.foot].unwrap();
                        let delta = goal.translation - mean;
                        let height = Vec3::Y * delta.y;
                        let expected = mean
                            + (delta - height) * ratio.clamp(0.01, 1.)
                            + height
                            + goal.rotation * Vec3::Y * leg.target_sole
                            - leg.source_offset;
                        let error = models[leg.foot].translation.distance(expected);
                        assert!(error<0.003,"{race} {sex}: foot contact error {error} at scale {model_scale} crouch {crouch}");
                    }

                    for (i, fit) in plan.fits.iter().enumerate() {
                        if fit.is_some() && plan.parents[i] >= 0 {
                            assert!(
                                result[i].translation.distance(plan.binds[i]) < 1e-4,
                                "{race} {sex}: changed a mapped body's bind offset at {i}"
                            );
                            assert!(
                                result[i].scale.distance(Vec3::ONE) < 1e-5,
                                "{race} {sex}: stretched mapped bone scale at {i}"
                            );
                        }
                    }

                    for leg in &legs {
                        for joint in [leg.thigh, leg.knee, leg.foot] {
                            let expected = plan.binds[joint].length();
                            assert!(
                                (result[joint].translation.length() - expected).abs() < 1e-4,
                                "{race} {sex}: stretched bone {joint}"
                            );
                        }
                        assert!(
                            (models[leg.thigh]
                                .translation
                                .distance(models[leg.knee].translation)
                                - plan.binds[leg.knee].length())
                            .abs()
                                < 1e-4,
                            "{race} {sex}: upper leg changed length"
                        );
                        assert!(
                            (models[leg.knee]
                                .translation
                                .distance(models[leg.foot].translation)
                                - plan.binds[leg.foot].length())
                            .abs()
                                < 1e-4,
                            "{race} {sex}: lower leg changed length"
                        );
                    }
                }
            }
            eprintln!("retarget audit: {race} {sex}: both legs retain bind lengths at 3 sizes and 3 crouch depths");
        }
    }
}

#[test]
fn matching_hierarchies_with_different_bind_poses_recalibrate() {
    let data = benilla_formats::wow_data_or_skip!();
    let chain = benilla_formats::Chain::open(&data).unwrap();
    let raw = benilla_formats::parse_m2_skeleton(
        &chain.read("Character\\Gnome\\Male\\GnomeMale.m2").unwrap(),
    )
    .unwrap();
    let mut skeleton = skeleton(&raw);
    let mut plan = Plan::default();
    let a = RigPose::new(Entity::PLACEHOLDER, &skeleton);
    plan.for_rig(&a);
    let first = plan.pivots.clone();
    for joint in &mut skeleton.joints {
        joint.local_translation *= 0.5;
    }
    let b = RigPose::new(Entity::PLACEHOLDER, &skeleton);
    plan.for_rig(&b);
    assert!(plan
        .pivots
        .iter()
        .zip(&first)
        .all(|(a, b)| a.distance(*b * 0.5) < 1e-5));
}

#[test]
fn all_playable_feet_keep_their_resting_sole_direction() {
    let data = benilla_formats::wow_data_or_skip!();
    let chain = benilla_formats::Chain::open(&data).unwrap();
    if reference().is_none() {
        eprintln!("SKIP: local Skate rig reference is unavailable");
        return;
    }
    for race in [
        "Human", "Orc", "Dwarf", "NightElf", "Scourge", "Tauren", "Gnome", "Troll",
    ] {
        for sex in ["Male", "Female"] {
            let path = format!("Character\\{race}\\{sex}\\{race}{sex}.m2");
            let raw = benilla_formats::parse_m2_skeleton(&chain.read(&path).unwrap()).unwrap();
            let rig = RigPose::new(Entity::PLACEHOLDER, &skeleton(&raw));
            let mut plan = Plan::default();
            let plan = plan.for_rig(&rig);
            for name in ["LEFTFOOT", "RIGHTFOOT", "LEFTTOEBASE", "RIGHTTOEBASE"] {
                let fit = plan.fits.iter().flatten().find(|(bone, _)| *bone == name);
                let Some((_, fit)) = fit else {
                    // Some authored rigs have no separate nonzero toe segment.
                    assert!(name.ends_with("TOEBASE"), "{race} {sex}: missing {name}");
                    continue;
                };
                for deck in [
                    Quat::IDENTITY,
                    Quat::from_euler(EulerRot::XYZ, 0.3, 0.5, -0.2),
                ] {
                    // In the authored M2 rest pose the sole points down. Retargeting may follow
                    // an angled deck, but must not add ankle-to-toe alignment to the sole normal.
                    let posed = Mat4::from_quat(deck) * *fit;
                    let up = posed.transform_vector3(Vec3::Y).normalize();
                    assert!(
                        up.distance(deck * Vec3::Y) < 1e-4,
                        "{race} {sex}: {name} added sole tilt: {up:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn native_board_and_foot_contact_heights_are_measured() {
    let _data = benilla_formats::wow_data_or_skip!();
    let Some(root) = crate::skate::assets_root() else {
        eprintln!("SKIP: local Skate assets unavailable");
        return;
    };
    let a = [-20., 0., -20.];
    let b = [-20., 0., 20.];
    let c = [20., 0., 20.];
    let d = [20., 0., -20.];
    let mut session = skate_host::bridge::Session::new(
        &root,
        vec![[a, b, c], [a, c, d]],
        Vec::new(),
        [0., 0.2, 0.],
        0.,
    )
    .unwrap();
    session.activate([0., 0.2, 0.], 0.).unwrap();
    for _ in 0..180 {
        session.tick(default()).unwrap();
    }
    for offboard in [false, true] {
        if offboard {
            session.activate_offboard([0., 0.2, 0.], 0.).unwrap();
            for _ in 0..180 {
                session.tick(default()).unwrap();
            }
        }
        let frame = crate::skate::frame(session.pose());
        let mut root = frame.root;
        root.w_axis.w = 1.;
        for name in ["LEFTFOOT", "RIGHTFOOT", "SKATEBOARD_ROOT"] {
            let i = frame.names.iter().position(|n| n == name).unwrap();
            let global = root * frame.bones[i];
            eprintln!("native contact audit: offboard={offboard} state={} bone={name} y_m={:.4} root_y_m={:.4}",frame.state,global.w_axis.y,root.w_axis.y);
            assert!(global.is_finite());
        }
    }
}
