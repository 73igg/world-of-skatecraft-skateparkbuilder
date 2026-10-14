//! Opt-in replay lifecycle checks use only the checkout's unattended test identity.
use super::{Action, Replay};
use crate::skate::{Host, SkateDrive};
use bevy::prelude::*;
#[derive(Default)]
struct State {
    phase: u8,
    at: f32,
    parked: Vec3,
    export_at: f32,
    export_framed: bool,
    original_window: f64,
    ability_checked: bool,
    original_board_size: f32,
}
fn run(
    time: Res<Time>,
    mut replay: ResMut<Replay>,
    host: Res<Host>,
    drive: Res<SkateDrive>,
    contact: Res<crate::skate::rig::Contact>,
    pose: Res<crate::skate::SkatePose>,
    player: Res<crate::player::Player>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    units: Query<
        (
            &crate::net::ObjectStore,
            &benilla_world::rig_anim::RigPose,
            &Transform,
            &GlobalTransform,
        ),
        With<crate::net::Embodied>,
    >,
    globals: Query<&GlobalTransform>,
    net: Res<crate::net::NetCommands>,
    abilities: Res<crate::ui_action::PlayerActions>,
    mut state: Local<State>,
    mut plan: Local<crate::skate::rig::Plan>,
    sizes: Res<crate::skate::board_size::BoardSizes>,
    mut script: Option<NonSendMut<benilla_ui::script::UiScript>>,
) {
    if std::env::var("WOW_REPLAY_PROBE").as_deref() != Ok("1")
        || std::env::var("WOW_UNATTENDED").as_deref() != Ok("1")
        || !std::env::var("WOW_USER").is_ok_and(|u| u.to_ascii_uppercase().starts_with("PROBEPARK"))
    {
        return;
    }
    let Some(script) = script.as_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    if now < state.at {
        return;
    }
    match state.phase {
        0 if drive.active => {
            let Ok((store, _, _, _)) = units.single() else {
                return;
            };
            let race = store.0.unit_race();
            state.original_board_size = sizes.scale(race);
            if contact.board_scale != state.original_board_size
                || !pose.active
                || pose.bones.is_empty()
            {
                return;
            }
            if let Some(input) = &host.input {
                input.inject(default());
            }
            if std::env::var("WOW_RETARGET_PROBE").as_deref() == Ok("1") {
                assert_eq!(contact.board_scale, state.original_board_size);
                info!(
                    "board size probe: initial_saved_scale={} race={race:?}",
                    state.original_board_size
                );
                assert!(
                    script.run_slash_command("boardsize", "1.25"),
                    "board size slash command missing"
                );
                state.at = now + 1.;
                state.phase = 13;
            } else {
                state.at = now + 5.;
                state.phase = 1;
            }
        }
        13 => {
            let race = units
                .single()
                .expect("test character missing")
                .0
                 .0
                .unit_race();
            assert_eq!(sizes.scale(race), 1.25);
            assert_eq!(
                contact.board_scale, 1.25,
                "riding board size did not change"
            );
            let saved: crate::skate::board_size::BoardSizes = serde_json::from_slice(
                &std::fs::read(
                    crate::local_state::board_sizes_path().expect("board size path missing"),
                )
                .expect("board size was not saved"),
            )
            .expect("board size preferences malformed");
            assert_eq!(saved.scale(race), 1.25);
            assert!(script.run_slash_command("boardsize", "NaN"));
            state.at = now + 1.;
            state.phase = 14;
        }
        14 => {
            let race = units
                .single()
                .expect("test character missing")
                .0
                 .0
                .unit_race();
            assert_eq!(sizes.scale(race), 1.25, "invalid command changed the board");
            assert!(script.run_slash_command("boardsize", "reset"));
            state.at = now + 1.;
            state.phase = 15;
        }
        15 => {
            let race = units
                .single()
                .expect("test character missing")
                .0
                 .0
                .unit_race();
            assert_eq!(sizes.scale(race), 1.);
            assert_eq!(
                contact.board_scale, 1.,
                "reset did not restore default deck"
            );
            assert!(script.run_slash_command("boardsize", &state.original_board_size.to_string()));
            state.at = now + 1.;
            state.phase = 16;
        }
        16 => {
            assert_eq!(contact.board_scale, state.original_board_size);
            info!("board size probe: PASS slash command, riding scale, saved setting, invalid input and reset");
            state.at = now + 1.;
            state.phase = 1;
        }
        1 => {
            if let Some(input) = &host.input {
                input.inject(skate_host::bridge::Controls {
                    buttons: 0x1000,
                    ..default()
                });
            }
            state.at = now + 3.;
            state.phase = 2;
        }
        2 => {
            if let Some(input) = &host.input {
                input.inject(default());
            }
            if std::env::var("WOW_RETARGET_PROBE").as_deref() == Ok("1") {
                let (store, rig, unit_t, unit_g) =
                    units.single().expect("test character rig missing");
                let race = store.0.unit_race();
                info!(
                    "retarget probe: board_scale={:.4} race={race:?}",
                    contact.board_scale
                );
                assert_eq!(
                    contact.board_scale,
                    sizes.scale(race),
                    "race board preference did not apply"
                );
                let frame_g = globals
                    .get(rig.joints_root)
                    .expect("test model frame missing");
                let world = Mat4::from(
                    unit_t.compute_affine() * (unit_g.affine().inverse() * frame_g.affine()),
                );
                let plan = plan.for_rig(rig);
                for name in ["LEFTFOOT", "RIGHTFOOT"] {
                    let i = plan
                        .fits
                        .iter()
                        .position(|f| f.is_some_and(|f| f.0 == name))
                        .expect("test foot mapping missing");
                    let actual = (world * Mat4::from(rig.model[i]))
                        .transform_vector3(Vec3::Y)
                        .normalize();
                    let expected = (pose.bone(name).expect("test source foot missing")
                        * crate::skate::rig::reference_bind(name)
                            .expect("test foot bind missing")
                            .inverse())
                    .transform_vector3(Vec3::Y)
                    .normalize();
                    let degrees = actual.dot(expected).clamp(-1., 1.).acos().to_degrees();
                    info!("retarget probe: on_board_sole={name} extra_tilt_deg={degrees:.4}");
                    assert!(
                        actual.distance(expected) < 0.01,
                        "retarget added foot tilt: {name} {degrees} degrees"
                    );
                }
            }
            state.parked = player.pos;
            if std::env::var("WOW_REPLAY_PROBE_ABILITY").as_deref() == Ok("0") {
                replay.actions.push(Action::Toggle);
            } else {
                assert!(
                    abilities
                        .spells
                        .contains(&benilla_formats::skatecraft::SPELL_REPLAY),
                    "Replay ability was not learned automatically"
                );
                net.0
                    .send(crate::net::ClientCommand::CastSpell {
                        spell_id: benilla_formats::skatecraft::SPELL_REPLAY,
                        target: None,
                    })
                    .expect("Replay ability command could not be sent");
                state.ability_checked = true;
            }
            state.at = now + 1.;
            state.phase = 3;
        }
        3 => {
            assert!(replay.active, "replay did not open");
            assert!(!drive.active, "native simulation still drives replay");
            assert_eq!(
                contact.board_scale, state.original_board_size,
                "replay changed the deck size"
            );
            state.parked = player.pos;
            let first = replay.bounds().0;
            replay.actions.push(Action::Seek(first));
            state.original_window = replay.window;
            replay.actions.push(Action::Window(-15.));
            state.at = now + 1.;
            state.phase = 4;
        }
        4 => {
            let expected = (state.original_window - 15.)
                .clamp(super::timeline::MIN_WINDOW, super::timeline::WINDOW);
            assert_eq!(
                replay.window, expected,
                "recording window button did not apply"
            );
            let path = crate::local_state::replay_settings_path()
                .expect("replay preferences path missing");
            let preferences: super::Preferences = serde_json::from_slice(
                &std::fs::read(path).expect("replay preferences were not saved"),
            )
            .expect("replay preferences were malformed");
            assert_eq!(preferences.window_seconds, expected);
            let restore_window = state.original_window - replay.window;
            replay.actions.push(Action::Window(restore_window));
            info!(
                "replay probe: recording-window change persisted ability_checked={}",
                state.ability_checked
            );
            assert!(
                player.pos.distance(state.parked) < 0.01,
                "replay changed live server position"
            );
            let distance = replay.actor_position().distance(player.pos);
            info!("replay probe: historical_distance={distance:.3} live_position_unchanged=true frames={}",replay.frames.len());
            replay.actions.push(Action::Keyframe(false));
            replay.actions.push(Action::Save);
            state.at = now + 2.;
            state.phase = 5;
        }
        5 => {
            if replay.io.is_some() {
                return;
            }
            assert!(!super::files::list().is_empty(), "saved clip missing");
            replay.actions.push(Action::Library);
            replay.actions.push(Action::Load(0));
            state.at = now + 1.;
            state.phase = 6;
        }
        6 => {
            if replay.io.is_some() {
                return;
            }
            assert_eq!(
                replay.status, "Replay loaded",
                "saved replay did not load: {}",
                replay.status
            );
            assert!(!replay.visuals.is_empty());
            if std::env::var("WOW_REPLAY_PROBE_EXPORT").as_deref() == Ok("1") {
                let end = replay.bounds().1;
                replay.in_point = (end - 0.5).max(replay.bounds().0);
                replay.out_point = end;
                replay.path_enabled = false;
                replay.mode = super::camera::Mode::Recorded;
                replay.actions.push(Action::Export);
                state.export_at = now;
                state.phase = 7;
                state.at = now + 2.;
            } else {
                replay.actions.push(Action::Toggle);
                state.at = now + 2.;
                state.phase = 8;
            }
        }
        7 => {
            if replay.exporting.is_some() {
                if !state.export_framed {
                    let local = replay
                        .camera
                        .compute_affine()
                        .inverse()
                        .transform_point3(replay.target());
                    let half_y = -local.z * (replay.fov * 0.5).tan();
                    let window = windows.single().expect("test window missing");
                    let aspect =
                        window.physical_width() as f32 / window.physical_height().max(1) as f32;
                    assert!(
                        local.z < 0. && local.y.abs() < half_y && local.x.abs() < half_y * aspect,
                        "export camera missed its subject: {local:?}"
                    );
                    info!("replay probe: export subject in frame camera_local={local:?}");
                    state.export_framed = true;
                }
                assert!(now - state.export_at < 90., "video export never completed");
                return;
            }
            assert!(
                replay.status.starts_with("Exported"),
                "video export failed: {}",
                replay.status
            );
            info!("replay probe: video PASS {}", replay.status);
            replay.actions.push(Action::Toggle);
            state.at = now + 2.;
            state.phase = 8;
        }
        8 => {
            assert!(!replay.active, "replay did not close");
            assert!(drive.active, "skate mode did not resume");
            assert!(drive.offboard, "replay did not return holding the board");
            assert_eq!(
                contact.board_scale, state.original_board_size,
                "carried deck size changed"
            );
            info!("replay probe: PASS playback isolated, saved and reloaded poses, exit and offboard native resume");
            if std::env::var("WOW_RETARGET_PROBE").as_deref() == Ok("1") {
                state.phase = 9;
            } else {
                state.phase = 12;
            }
        }
        9 => {
            let Ok((store, rig, _, _)) = units.single() else {
                assert!(now - state.at < 10., "test character rig unavailable");
                return;
            };
            let model_id = store
                .0
                .unit_displayid()
                .expect("test character display missing");
            let plan = plan.for_rig(rig);
            for (i, &parent) in rig.parents.iter().enumerate() {
                if parent < 0 {
                    continue;
                }
                let bind = rig.bind_translations()[i].length();
                if bind < 0.01 {
                    continue;
                }
                let posed = rig.locals[i].translation.length();
                // Sheath riders are attachment sockets, outside the body chain.
                if plan.fits[i].is_some() {
                    assert!(
                        (bind - posed).abs() < 0.002,
                        "display {} stretched key bone {}: {} -> {}",
                        model_id,
                        i,
                        bind,
                        posed
                    );
                }
            }
            info!(
                "retarget probe: PASS display={} race={:?} bones={} natural_key_bone_lengths=true",
                model_id,
                store.0.unit_race(),
                rig.locals.len()
            );
            state.phase = 12;
        }
        _ => {}
    }
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, run.before(super::controls).after(super::context));
}
