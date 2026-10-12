//! Real sampler/transport exercise, restricted to explicitly unattended owned test characters.
use super::State;
use crate::net::{ClientCommand, NetCommands};
use crate::skate::{Host, SkateDrive};
use bevy::prelude::*;
use skate_host::bridge::Controls;
use std::time::Instant;

#[derive(Default)]
struct Probe {
    phase: u8,
    next: Option<Instant>,
    revision: u64,
    saved: Vec3,
    facing: f32,
}
fn enabled() -> bool {
    std::env::var("WOW_SKATE_FLAG_PROBE").as_deref() == Ok("1")
        && std::env::var("WOW_UNATTENDED").as_deref() == Ok("1")
        && std::env::var("WOW_USER").is_ok_and(|u| u.starts_with("PROBEPARK"))
}
#[allow(clippy::too_many_arguments)]
fn run(
    host: Res<Host>,
    drive: Res<SkateDrive>,
    state: Res<State>,
    player: Res<crate::player::Player>,
    screen: Res<crate::loading_screen::LoadingScreen>,
    net: Res<NetCommands>,
    mut commands: Commands,
    mut probe: Local<Probe>,
) {
    if !enabled() || !player.active || screen.covering() && probe.phase == 0 {
        return;
    }
    if probe.next.is_some_and(|at| Instant::now() < at) {
        return;
    }
    let input = host.input.as_ref();
    let control = |buttons, left| {
        if let Some(input) = input {
            input.inject(Controls {
                buttons,
                left,
                ..Default::default()
            });
        }
    };
    let mut delay = 0.5;
    match probe.phase {
        0 => {
            let _ = net.0.send(ClientCommand::CastSpell {
                spell_id: 60001,
                target: None,
            });
            delay = 2.;
        }
        1 => {
            if !drive.active || !state.capable || input.is_none() {
                return;
            }
            control(0, [0, 0]);
        }
        2 => {
            control(0x0020, [0, 0]);
            delay = 0.9;
        }
        3 => {
            control(0, [0, 0]);
        }
        4 => {
            let Some(flag) = state.saved else {
                return;
            };
            assert!(state.pending.is_none());
            probe.revision = flag.revision;
            probe.saved = benilla_assets::coords::wow_to_bevy(flag.position);
            probe.facing = flag.facing;
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(
                    "benilla-config/skate-flag/flag-placed.png",
                ));
            let _ = net.0.send(ClientCommand::CancelAura { spell_id: 60001 });
            delay = 1.;
        }
        5 => {
            assert!(!drive.active);
            control(0x0020, [0, 0]);
            delay = 0.9;
        }
        6 => {
            control(0, [0, 0]);
        }
        7 => {
            assert_eq!(state.saved.unwrap().revision, probe.revision);
            assert!(state.pending.is_none() && state.returning.is_none());
            let _ = net.0.send(ClientCommand::CastSpell {
                spell_id: 60001,
                target: None,
            });
            delay = 2.;
        }
        8 => {
            if !drive.active {
                return;
            }
            control(0x8000, [0, 0]);
            delay = 0.15;
        }
        9 => {
            control(0, [0, 0]);
            delay = 1.;
        }
        10 => {
            assert!(drive.offboard);
            control(0, [0, 22000]);
            delay = 3.;
        }
        11 => {
            control(0, [0, 0]);
        }
        12 => {
            assert!(
                drive.pos.distance(probe.saved) > 2.,
                "walked away from flag"
            );
            control(0x0020, [0, 0]);
            delay = 0.15;
        }
        13 => {
            control(0, [0, 0]);
        }
        14 => {
            assert!(!screen.covering(), "same-zone return must not cover scene");
            if state.pending.is_some() || !drive.active {
                return;
            }
            assert!(drive.offboard, "return holds board");
            assert!(
                drive.pos.distance(probe.saved) < 0.4,
                "returned to saved point"
            );
            let angle = (drive.facing - probe.facing + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            assert!(angle.abs() < 0.03, "saved facing restored: {angle}");
            delay = 2.;
        }
        15 => {
            assert!(drive.offboard);
            assert!(
                drive.pos.distance(probe.saved) < 0.4,
                "no roll after return"
            );
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(
                    "benilla-config/skate-flag/flag-returned.png",
                ));
            info!("SKATE_FLAG_RENDER_PASS: hold/tap, private flag, WoW gating, persistence across mode switch, same-zone return, facing, held board, no drift");
        }
        16 => {
            let _ = net.0.send(ClientCommand::Chat {
                kind: crate::net::ChatKind::Say,
                target: None,
                language: None,
                text: ".go xyz -9444 -2237 72 0".into(),
            });
            delay = 5.;
        }
        17 => {
            if screen.covering() || !drive.active {
                return;
            }
            assert!(
                drive.pos.distance(probe.saved) > 1500.,
                "far destination reached"
            );
            delay = 5.;
        }
        18 => {
            control(0x0020, [0, 0]);
            delay = 0.15;
        }
        19 => {
            control(0, [0, 0]);
        }
        20 => {
            assert!(
                !screen.covering(),
                "far same-zone return must not cover scene"
            );
            if state.pending.is_some() || !drive.active {
                return;
            }
            assert!(drive.offboard);
            assert!(drive.pos.distance(probe.saved) < 0.5);
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(
                    "benilla-config/skate-flag/flag-returned-far.png",
                ));
            info!("SKATE_FLAG_FAR_RENDER_PASS: return over 1500 yards without loading cover; destination scene/collision restored and board held");
        }
        21 => {
            control(0x8000, [0, 0]);
            delay = 0.15;
        }
        22 => {
            control(0, [0, 0]);
            delay = 2.;
        }
        23 => {
            assert!(!drive.offboard, "Triangle remounts after flag return");
            info!("SKATE_FLAG_REMOUNT_PASS");
        }
        _ => return,
    }
    info!("skate flag probe: phase={}", probe.phase);
    probe.phase += 1;
    probe.next = Some(Instant::now() + std::time::Duration::from_secs_f32(delay));
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        run.in_set(benilla_world::schedule::WorldStage::Present),
    );
}
