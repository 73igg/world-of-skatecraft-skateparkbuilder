//! Deterministic editor input on an explicitly unattended test account; never moves the OS cursor.
use super::{
    input::{Action, Actions},
    math,
    state::{ParkEditor, ParkWorld},
};
use benilla_protocol::park::{Model, Pose};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::sync::{Mutex, OnceLock};

static CURSOR: Mutex<Option<Vec2>> = Mutex::new(None);
static ENABLED: OnceLock<bool> = OnceLock::new();
pub(super) fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var("WOW_PARK_PROBE").as_deref() == Ok("1")
            && std::env::var("WOW_UNATTENDED").as_deref() == Ok("1")
    })
}
pub(super) fn cursor() -> Option<Vec2> {
    *CURSOR.lock().unwrap()
}
#[derive(Default)]
pub(super) struct Probe {
    phase: usize,
    next: f32,
    release: Option<KeyCode>,
    base_x: f32,
    cursor: Vec2,
    saved_pose: Pose,
    saved_camera: Vec3,
    boxes: [Option<super::state::Key>; 2],
    fused_height: f32,
}
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    time: Res<Time>,
    screen: Res<crate::loading_screen::LoadingScreen>,
    world: Res<ParkWorld>,
    self_guid: Res<crate::net::SelfGuid>,
    player: Res<crate::player::Player>,
    mut editor: ResMut<ParkEditor>,
    mut actions: ResMut<Actions>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut keyboard: MessageWriter<KeyboardInput>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    camera: Query<&Projection, With<benilla_world::view::WorldCamera>>,
    mut commands: Commands,
    mut probe: Local<Probe>,
) {
    if !enabled() {
        return;
    }
    if let Some(key) = probe.release.take() {
        keys.release(key);
    }
    if screen.covering()
        || !world.ready
        || !player.active
        || time.elapsed_secs() < probe.next
        || editor.pending.is_some()
    {
        return;
    }
    let owner = self_guid.0.unwrap_or(0);
    let Ok((window_entity, window)) = windows.single() else {
        return;
    };
    let Ok(projection) = camera.single() else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    let mut key = None;
    match probe.phase {
        0 => {
            editor.toggle_requested = true;
            *CURSOR.lock().unwrap() = Some(size * 0.65);
        }
        1 => actions.0.push(Action::Create(Model::Ramp)),
        2 => {
            probe.base_x = editor.preview.expect("new ramp preview").1.position[0];
            key = Some((KeyCode::KeyG, "g"));
        }
        3 => key = Some((KeyCode::KeyX, "x")),
        4 => key = Some((KeyCode::Digit2, "2")),
        5 => key = Some((KeyCode::Enter, "")),
        6 => key = Some((KeyCode::KeyR, "r")),
        7 => key = Some((KeyCode::KeyZ, "z")),
        8 => key = Some((KeyCode::Digit4, "45")),
        9 => key = Some((KeyCode::Enter, "")),
        10 => key = Some((KeyCode::KeyS, "s")),
        11 => key = Some((KeyCode::KeyX, "x")),
        12 => key = Some((KeyCode::Digit2, "2")),
        13 => key = Some((KeyCode::Enter, "")),
        14 => {
            let object = world
                .objects
                .get(&editor.selected.expect("selected saved ramp"))
                .expect("saved ramp");
            assert!((object.pose.position[0] - probe.base_x - 2.).abs() < 0.002);
            assert!(Vec3::from_array(object.pose.scale).distance(Vec3::new(2., 1., 1.)) < 0.00001);
            assert!(
                (Quat::from_array(object.pose.rotation)
                    .angle_between(Quat::from_rotation_y(45_f32.to_radians())))
                .abs()
                    < 0.001
            );
            let center = math::transform(object.pose).transform_point(Vec3::Y);
            editor.camera = Transform::from_translation(center + Vec3::new(9., 6., 9.))
                .looking_at(center, Vec3::Y);
            let click_point =
                math::transform(object.pose).transform_point(Vec3::new(1.5, 0.5, -1.5));
            probe.cursor =
                math::screen_point(click_point, size, editor.camera, projection).unwrap();
            *CURSOR.lock().unwrap() = Some(probe.cursor);
            info!("PARK_INPUT_PASS: typed movement, 45-degree rotation and local-axis resize; revision={}",object.revision);
        }
        15 => buttons.press(MouseButton::Left),
        16 => {
            *CURSOR.lock().unwrap() = Some(probe.cursor + Vec2::new(30., 0.));
        }
        17 => buttons.release(MouseButton::Left),
        18 => {
            let object = world
                .objects
                .get(&editor.selected.expect("selected dragged ramp"))
                .unwrap();
            assert!(
                object.pose.scale[0] > 2.2,
                "model drag must resize the ramp: pose={:?}, axis={:?}, dragging={}",
                object.pose,
                editor.axis,
                editor.drag.is_some()
            );
            let path = crate::local_state::screenshots_dir()
                .expect("test screenshots")
                .join("park-editor.png");
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(path));
            info!("PARK_DRAG_PASS: mouse-model drag, server acknowledgment and synchronized scale");
        }
        19 => actions.0.push(Action::Undo),
        20 => {
            let object = world.objects.get(&editor.selected.unwrap()).unwrap();
            assert!(Vec3::from_array(object.pose.scale).distance(Vec3::new(2., 1., 1.)) < 0.00001);
            actions.0.push(Action::Redo);
        }
        21 => {
            let object = world.objects.get(&editor.selected.unwrap()).unwrap();
            assert!(object.pose.scale[0] > 2.2);
            actions.0.push(Action::Exit);
        }
        22 => {
            assert!(!editor.active);
            assert_eq!(world.objects.keys().filter(|key| key.0 == owner).count(), 1);
            info!("PARK_PROBE_PASS: editing, dragging, rotation, scale, undo/redo, and persistence after leaving editor");
        }
        23 => editor.toggle_requested = true,
        24 => {
            probe.saved_pose = world.objects[&editor.selected.unwrap()].pose;
            probe.saved_camera = editor.camera.translation;
            key = Some((KeyCode::KeyG, "g"));
        }
        25 => {
            buttons.press(MouseButton::Right);
            key = Some((KeyCode::KeyS, "s"));
        }
        26 => {
            buttons.release(MouseButton::Right);
            assert_eq!(
                editor.tool,
                super::math::Tool::Move,
                "camera S must not activate the resize tool"
            );
            assert!(editor.drag.is_none() && editor.pending.is_none());
            assert_eq!(
                world.objects[&editor.selected.unwrap()].pose,
                probe.saved_pose
            );
            assert!(editor.camera.translation.distance(probe.saved_camera) > 0.001);
            info!("PARK_CAMERA_PASS: right-mouse navigation preempts transform shortcuts and preserves selection");
        }
        27 => actions.0.push(Action::Create(Model::Box)),
        28 => {
            let mut pose = Pose {
                position: probe.saved_pose.position,
                ..default()
            };
            pose.position[0] -= 12.;
            pose.position[2] += 6.;
            editor.drafts[0].pose = pose;
            editor.preview = Some((Model::Box, pose));
            actions.0.push(Action::Place);
        }
        29 => probe.boxes[0] = editor.selected,
        30 => actions.0.push(Action::Create(Model::Box)),
        31 => {
            let mut pose = world.objects[&probe.boxes[0].unwrap()].pose;
            pose.position[0] += 6.;
            pose.position[1] += 0.3;
            editor.drafts[0].pose = pose;
            editor.preview = Some((Model::Box, pose));
            actions.0.push(Action::Place);
        }
        32 => {
            probe.boxes[1] = editor.selected;
            let a = world.objects[&probe.boxes[0].unwrap()].pose;
            let b = world.objects[&probe.boxes[1].unwrap()].pose;
            let centre = (Vec3::from_array(a.position) + Vec3::from_array(b.position)) * 0.5;
            editor.camera = Transform::from_translation(centre + Vec3::new(-5., 9., 18.))
                .looking_at(centre + Vec3::new(-5., 0.5, 0.), Vec3::Y);
            editor.select(None, false);
            editor.clear_preview();
            let cursor = math::screen_point(
                Vec3::from_array(a.position) + Vec3::Y * 1.5,
                size,
                editor.camera,
                projection,
            )
            .unwrap();
            assert!(cursor.x > 270.);
            *CURSOR.lock().unwrap() = Some(cursor);
            keys.press(KeyCode::ControlLeft);
        }
        33 => buttons.press(MouseButton::Left),
        34 => {
            buttons.release(MouseButton::Left);
            assert_eq!(editor.selection, vec![probe.boxes[0].unwrap()]);
            let pose = world.objects[&probe.boxes[1].unwrap()].pose;
            let cursor = math::screen_point(
                Vec3::from_array(pose.position) + Vec3::Y * 1.5,
                size,
                editor.camera,
                projection,
            )
            .unwrap();
            assert!(cursor.x > 270.);
            *CURSOR.lock().unwrap() = Some(cursor);
        }
        35 => buttons.press(MouseButton::Left),
        36 => {
            buttons.release(MouseButton::Left);
            keys.release(KeyCode::ControlLeft);
            assert_eq!(
                editor.selection,
                vec![probe.boxes[0].unwrap(), probe.boxes[1].unwrap()]
            );
            assert_eq!(editor.selected, probe.boxes[1]);
            info!("PARK_MULTISELECT_PASS: Ctrl-click selects both meshes and keeps the last as active reference");
        }
        37 => actions.0.push(Action::AlignHeight),
        38 => {
            let a = world.objects[&probe.boxes[0].unwrap()].pose;
            let b = world.objects[&probe.boxes[1].unwrap()].pose;
            assert_eq!(a.position[1], b.position[1]);
            assert_eq!(editor.selection.len(), 2);
            assert_eq!(editor.selected, probe.boxes[1]);
            info!("PARK_ALIGN_PASS: exact shared Z height and multi-selection retained after acknowledgment");
        }
        39 => actions.0.push(Action::Fuse),
        40 => {
            let object = &world.objects[&editor.selected.unwrap()];
            assert_eq!(object.model, Model::Compound);
            assert_eq!(object.parts.len(), 2);
            assert!(probe
                .boxes
                .iter()
                .all(|key| !world.objects.contains_key(&key.unwrap())));
            probe.fused_height = object.pose.position[1];
            assert_eq!(editor.selection.len(), 1);
            info!("PARK_FUSE_PASS: one server object replaces both pieces and preserves their affine geometry");
        }
        41 => key = Some((KeyCode::KeyG, "g")),
        42 => key = Some((KeyCode::KeyZ, "z")),
        43 => key = Some((KeyCode::Digit1, "1")),
        44 => key = Some((KeyCode::Enter, "")),
        45 => {
            assert!(
                (world.objects[&editor.selected.unwrap()].pose.position[1]
                    - probe.fused_height
                    - 1.)
                    .abs()
                    < 0.001
            );
            actions.0.push(Action::Undo);
        }
        46 => {
            assert!(
                (world.objects[&editor.selected.unwrap()].pose.position[1] - probe.fused_height)
                    .abs()
                    < 0.001
            );
            actions.0.push(Action::Undo);
        }
        47 => {
            assert_eq!(editor.selection.len(), 2);
            assert!(editor
                .selection
                .iter()
                .all(|key| world.objects[key].model == Model::Box));
            actions.0.push(Action::Redo);
        }
        48 => {
            assert_eq!(
                world.objects[&editor.selected.unwrap()].model,
                Model::Compound
            );
            actions.0.push(Action::Redo);
        }
        49 => {
            assert!(
                (world.objects[&editor.selected.unwrap()].pose.position[1]
                    - probe.fused_height
                    - 1.)
                    .abs()
                    < 0.001
            );
            let path = crate::local_state::screenshots_dir()
                .unwrap()
                .join("park-group-editor.png");
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(path));
            info!("PARK_GROUP_UNDO_PASS: fused movement, undo back to separate objects, and redo with remapped server IDs");
        }
        50 => actions.0.push(Action::Exit),
        51 => {
            assert!(!editor.active);
            assert_eq!(world.objects.keys().filter(|key| key.0 == owner).count(), 2);
            info!("PARK_GROUP_PROBE_PASS: camera precedence, Ctrl-selection, Z alignment, fusion, movement, undo/redo and session persistence");
        }
        _ => return,
    }
    if let Some((code, text)) = key {
        keys.press(code);
        probe.release = Some(code);
        keyboard.write(KeyboardInput {
            key_code: code,
            logical_key: Key::Character(text.into()),
            state: ButtonState::Pressed,
            text: (!text.is_empty()).then(|| text.into()),
            repeat: false,
            window: window_entity,
        });
    }
    probe.phase += 1;
    probe.next = time.elapsed_secs() + 0.5;
}
