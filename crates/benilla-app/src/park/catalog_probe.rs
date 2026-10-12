//! Owned, unattended catalog regression; synthetic input never moves the OS cursor.
use super::{
    catalog_ui::Browser,
    input::{Action, Actions},
    library::Library,
    state::{ParkEditor, ParkWorld},
};
use benilla_protocol::park::Model;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy_egui::{egui, EguiInput, EguiPreUpdateSet};
pub(super) fn enabled() -> bool {
    std::env::var("WOW_PARK_CATALOG_PROBE").as_deref() == Ok("1")
        && std::env::var("WOW_UNATTENDED").as_deref() == Ok("1")
}
#[derive(Resource, Default)]
struct Events(Vec<egui::Event>);
#[derive(Default)]
struct Probe {
    phase: usize,
    next: f32,
    release: Option<KeyCode>,
    wow: u64,
    skate: u64,
    wow_object: Option<super::state::Key>,
    skate_object: Option<super::state::Key>,
    click: egui::Pos2,
}
pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Events>()
        .add_systems(
            PreUpdate,
            inject
                .after(EguiPreUpdateSet::ProcessInput)
                .before(EguiPreUpdateSet::BeginPass),
        )
        .add_systems(
            Update,
            run.after(super::library::tick)
                .after(benilla_world::schedule::WorldStage::Net)
                .before(super::catalog_ui::capture)
                .before(super::input::update),
        );
}
fn inject(
    mut events: ResMut<Events>,
    mut inputs: Query<&mut EguiInput, With<super::catalog_ui::CatalogCamera>>,
) {
    if !enabled() {
        return;
    }
    if let Ok(mut input) = inputs.single_mut() {
        input.0.events.append(&mut events.0);
    }
}
fn text(events: &mut Events, value: &str, replace: bool) {
    if replace {
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..default()
        };
        for pressed in [true, false] {
            events.0.push(egui::Event::Key {
                key: egui::Key::A,
                physical_key: Some(egui::Key::A),
                pressed,
                repeat: false,
                modifiers,
            });
        }
    }
    events.0.push(egui::Event::Text(value.into()));
}
fn click(events: &mut Events, point: egui::Pos2, pressed: bool) {
    events.0.push(egui::Event::PointerMoved(point));
    events.0.push(egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: default(),
    });
}
#[allow(clippy::too_many_arguments)]
fn run(
    time: Res<Time>,
    screen: Res<crate::loading_screen::LoadingScreen>,
    world: Res<ParkWorld>,
    guid: Res<crate::net::SelfGuid>,
    player: Res<crate::player::Player>,
    mut editor: ResMut<ParkEditor>,
    mut browser: ResMut<Browser>,
    mut library: ResMut<Library>,
    mut actions: ResMut<Actions>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut keyboard: MessageWriter<KeyboardInput>,
    windows: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut events: ResMut<Events>,
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
        || !library.indexed
        || editor.pending.is_some()
        || time.elapsed_secs() < probe.next
    {
        return;
    }
    let mut key = None;
    match probe.phase {
        0 => editor.toggle_requested = true,
        1 => actions.0.push(Action::Catalog),
        2 => {
            assert!(browser.open);
            text(&mut events, "QUARTER", false);
        }
        3 => {
            assert_eq!(browser.query.text, "QUARTER", "real catalog text entry");
            assert!(browser
                .results
                .iter()
                .any(|&i| library.entries[i].game == "Skate 3"));
            info!("PARK_CATALOG_SEARCH_PASS: typed case-insensitive substring search through the actual text field");
            browser.query.game = Some("WoW".into());
            text(&mut events, "mailbox", true);
        }
        4 => {
            assert_eq!(browser.query.text, "mailbox");
            assert!(!browser.results.is_empty());
            assert!(browser
                .results
                .iter()
                .all(|&i| library.entries[i].game == "WoW"));
            probe.wow = library.entries[browser.results[0]].id;
            browser.selected = Some(probe.wow);
            browser.preview = Some(probe.wow);
            library.request(probe.wow);
            info!("PARK_CATALOG_FILTER_PASS: WoW game filter and replacement of search text");
        }
        5 => {
            if !library.ready(probe.wow) {
                assert!(
                    !library.errors.contains_key(&probe.wow),
                    "WoW preview error: {:?}",
                    library.errors.get(&probe.wow)
                );
                return;
            }
            let Some(use_rect) = browser.use_rect else {
                return;
            };
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(
                    crate::local_state::screenshots_dir()
                        .unwrap()
                        .join("catalog-wow.png"),
                ));
            probe.click = use_rect.center();
            click(&mut events, probe.click, true);
            info!("PARK_CATALOG_WOW_PREVIEW_PASS: complete textured model and preview viewport");
        }
        6 => click(&mut events, probe.click, false),
        7 => {
            assert!(!browser.open);
            assert_eq!(editor.drafts[0].asset, probe.wow);
            actions.0.push(Action::Place);
        }
        8 => {
            probe.wow_object = editor.selected;
            assert_eq!(world.objects[&editor.selected.unwrap()].asset, probe.wow);
            actions.0.push(Action::Catalog);
        }
        9 => {
            browser.query.game = Some("Skate 3".into());
            text(&mut events, "quarter", true);
        }
        10 => {
            assert_eq!(browser.query.text, "quarter");
            assert!(browser
                .results
                .iter()
                .all(|&i| library.entries[i].game == "Skate 3"));
            probe.skate = library.entries[browser.results[0]].id;
            browser.selected = Some(probe.skate);
            browser.preview = Some(probe.skate);
            library.request(probe.skate);
        }
        11 => {
            if !library.ready(probe.skate) {
                assert!(
                    !library.errors.contains_key(&probe.skate),
                    "Skate preview error: {:?}",
                    library.errors.get(&probe.skate)
                );
                return;
            }
            let Some(use_rect) = browser.use_rect else {
                return;
            };
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(
                    crate::local_state::screenshots_dir()
                        .unwrap()
                        .join("catalog-skate.png"),
                ));
            probe.click = use_rect.center();
            click(&mut events, probe.click, true);
            info!("PARK_CATALOG_SKATE_PREVIEW_PASS: native park-object geometry, textures and framed preview");
        }
        12 => click(&mut events, probe.click, false),
        13 => {
            assert!(!browser.open);
            assert_eq!(editor.drafts[0].asset, probe.skate);
            let mut pose = editor.drafts[0].pose;
            pose.position[0] += 12.;
            editor.drafts[0].pose = pose;
            editor.preview = Some((Model::Asset, pose));
            actions.0.push(Action::Place);
        }
        14 => {
            probe.skate_object = editor.selected;
            assert_eq!(world.objects[&editor.selected.unwrap()].asset, probe.skate);
            key = Some((KeyCode::KeyG, "g"));
        }
        15 => key = Some((KeyCode::KeyZ, "z")),
        16 => key = Some((KeyCode::Digit1, "1")),
        17 => key = Some((KeyCode::Enter, "")),
        18 => key = Some((KeyCode::KeyR, "r")),
        19 => key = Some((KeyCode::KeyZ, "z")),
        20 => key = Some((KeyCode::Digit4, "45")),
        21 => key = Some((KeyCode::Enter, "")),
        22 => key = Some((KeyCode::KeyS, "s")),
        23 => key = Some((KeyCode::KeyX, "x")),
        24 => key = Some((KeyCode::Digit1, "1.5")),
        25 => key = Some((KeyCode::Enter, "")),
        26 => {
            let object = &world.objects[&editor.selected.unwrap()];
            assert_eq!(object.asset, probe.skate);
            assert!((object.pose.scale[0] - 1.5).abs() < 0.00001);
            assert!(
                Quat::from_array(object.pose.rotation)
                    .angle_between(Quat::from_rotation_y(45_f32.to_radians()))
                    .abs()
                    < 0.001
            );
            editor.select(probe.wow_object, false);
            editor.select(probe.skate_object, true);
            actions.0.push(Action::AlignHeight);
            info!("PARK_CATALOG_TRANSFORM_PASS: imported object movement, rotation and nonuniform resize preserve its asset");
        }
        27 => {
            assert_eq!(
                world.objects[&probe.wow_object.unwrap()].pose.position[1],
                world.objects[&probe.skate_object.unwrap()].pose.position[1]
            );
            actions.0.push(Action::Fuse);
        }
        28 => {
            let object = &world.objects[&editor.selected.unwrap()];
            assert_eq!(object.model, Model::Compound);
            assert_eq!(object.parts.len(), 2);
            assert!(object.parts.iter().any(|p| p.asset == probe.wow));
            assert!(object.parts.iter().any(|p| p.asset == probe.skate));
            let center = Vec3::from_array(object.pose.position);
            editor.camera = Transform::from_translation(center + Vec3::new(12., 8., 20.))
                .looking_at(center + Vec3::Y, Vec3::Y);
            actions.0.push(Action::Undo);
            info!("PARK_CATALOG_FUSE_PASS: mixed WoW/Skate assembly, shared height and preserved affine model references");
        }
        29 => {
            assert_eq!(editor.selection.len(), 2);
            assert!(editor
                .selection
                .iter()
                .all(|k| world.objects[k].model == Model::Asset));
            actions.0.push(Action::Redo);
        }
        30 => {
            assert_eq!(world.objects[&editor.selected.unwrap()].parts.len(), 2);
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(
                    crate::local_state::screenshots_dir()
                        .unwrap()
                        .join("catalog-fused.png"),
                ));
            info!("PARK_CATALOG_UNDO_PASS: undo restores both imported objects and redo preserves IDs/material references");
        }
        31 => actions.0.push(Action::Exit),
        32 => {
            assert!(!editor.active);
            assert_eq!(
                world
                    .objects
                    .keys()
                    .filter(|key| Some(key.0) == guid.0)
                    .count(),
                1
            );
            info!("PARK_CATALOG_PROBE_PASS: search, filtering, both game previews, UI placement, transforms, alignment, mixed fusion, undo/redo and session persistence");
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
            window: windows.single().unwrap(),
        });
    }
    probe.phase += 1;
    probe.next = time.elapsed_secs() + 0.7;
}
