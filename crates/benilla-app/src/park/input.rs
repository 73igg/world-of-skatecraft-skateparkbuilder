use super::{
    geometry,
    math::{self, Tool},
    scene,
    state::{Change, HistoryAction, Key, ParkEditor, ParkWorld, Pending},
};
use crate::net::{NetCommands, SelfGuid};
use benilla_protocol::park::{Definition, Message, Model, Pose};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

#[derive(Clone)]
pub(super) enum Action {
    Create(Model),
    CreateAsset(u64),
    Catalog,
    Tool(Tool),
    Place,
    Duplicate,
    Delete,
    Undo,
    Redo,
    Exit,
    AlignHeight,
    Fuse,
}
#[derive(Resource, Default)]
pub(super) struct Actions(pub Vec<Action>);
pub(super) struct Drag {
    start: Pose,
    cursor: Vec2,
    button: bool,
    numeric: String,
    members: Vec<(Key, Definition)>,
    draft: bool,
}

pub(super) fn send(net: &NetCommands, message: Message) -> bool {
    match message.encode() {
        Ok(text) => net
            .0
            .send(crate::net::ClientCommand::AddonMessage {
                distribution: benilla_ui::script::AddonDistribution::Party,
                text,
            })
            .is_ok(),
        Err(error) => {
            warn!("park: invalid outgoing transform: {error}");
            false
        }
    }
}
fn request(
    editor: &mut ParkEditor,
    world: &ParkWorld,
    net: &NetCommands,
    owner: u64,
    action: HistoryAction,
    time: f32,
) -> bool {
    if editor.pending.is_some() || !world.ready {
        editor.status = "Waiting for the server".into();
        return false;
    }
    let changes = match &action {
        HistoryAction::Push(c) => c.clone(),
        HistoryAction::Undo(i) | HistoryAction::Redo(i) => editor.history[*i].clone(),
    };
    let undo = matches!(action, HistoryAction::Undo(_));
    editor.next_request = editor.next_request.wrapping_add(1).max(1);
    let nonce = editor.next_request;
    let mut edits = Vec::new();
    let mut drops = Vec::new();
    let mut bindings = Vec::new();
    let mut drop_count = 0u16;
    for (index, change) in changes.iter().enumerate() {
        let target = if undo { &change.before } else { &change.after };
        let current = change.id.and_then(|id| world.objects.get(&(owner, id)));
        if let Some(target) = target {
            if target.validate().is_err() {
                editor.status = "Transform exceeds the supported size or coordinate range".into();
                return false;
            }
            if let Err(error) = geometry::definition(target) {
                editor.status = error.into();
                return false;
            }
            let slot = bindings.len() as u16;
            bindings.push(index);
            edits.push(Message::EditObject {
                request: nonce,
                slot,
                id: current.map_or(0, |o| o.id),
                revision: current.map_or(0, |o| o.revision),
                model: target.model,
                asset: target.asset,
                pose: target.pose,
                parts: target.parts.len() as u16,
            });
            for (part_index, part) in target.parts.iter().enumerate() {
                edits.push(Message::EditPart {
                    request: nonce,
                    slot,
                    index: part_index as u16,
                    part: part.clone(),
                });
            }
        } else if let Some(current) = current {
            drops.push(Message::EditDrop {
                request: nonce,
                slot: drop_count,
                id: current.id,
                revision: current.revision,
            });
            drop_count += 1;
        }
    }
    if bindings.is_empty() && drop_count == 0 {
        editor.status = "No changes to save".into();
        return false;
    }
    let mut messages = vec![Message::EditBegin {
        request: nonce,
        edits: bindings.len() as u16,
        drops: drop_count,
    }];
    messages.extend(edits);
    messages.extend(drops);
    messages.push(Message::EditCommit { request: nonce });
    for message in messages {
        if !send(net, message) {
            editor.status = "Connection unavailable".into();
            return false;
        }
    }
    editor.pending = Some(Pending {
        request: nonce,
        action,
        sent_at: time,
        bindings,
        assigned: std::collections::HashMap::new(),
    });
    editor.status = "Saving selection…".into();
    true
}
fn commit(editor: &mut ParkEditor, world: &ParkWorld, net: &NetCommands, owner: u64, time: f32) {
    let changes = if !editor.drafts.is_empty() {
        editor
            .drafts
            .iter()
            .map(|definition| Change {
                id: None,
                before: None,
                after: Some(definition.clone()),
            })
            .collect()
    } else {
        editor
            .selection
            .iter()
            .filter_map(|key| {
                let old = world.objects.get(key)?;
                let next = editor.previews.get(key)?;
                (old.definition() != *next).then(|| Change {
                    id: Some(old.id),
                    before: Some(old.definition()),
                    after: Some(next.clone()),
                })
            })
            .collect::<Vec<_>>()
    };
    if changes.is_empty() {
        editor.clear_preview();
        return;
    }
    if request(
        editor,
        world,
        net,
        owner,
        HistoryAction::Push(changes),
        time,
    ) {
        editor.drag = None;
    }
}
fn cancel(editor: &mut ParkEditor) {
    editor.clear_preview();
    editor.status = "Edit cancelled".into();
}
fn begin_drag(editor: &mut ParkEditor, world: &ParkWorld, cursor: Vec2, button: bool) {
    if editor.pending.is_some() {
        return;
    }
    let Some(start) = editor.target_pose(world) else {
        return;
    };
    let draft = !editor.drafts.is_empty();
    let members = if draft {
        editor
            .drafts
            .iter()
            .enumerate()
            .map(|(index, d)| ((0, index as u32), d.clone()))
            .collect::<Vec<_>>()
    } else {
        editor
            .selection
            .iter()
            .filter_map(|key| {
                world
                    .objects
                    .get(key)
                    .map(|object| (*key, object.definition()))
            })
            .collect()
    };
    editor.preview_sources = members.clone();
    editor.preview_root = Some((start, start));
    editor.drag = Some(Drag {
        start,
        cursor,
        button,
        numeric: String::new(),
        members,
        draft,
    });
}
pub(super) fn reset_camera_drag(editor: &mut ParkEditor) {
    if let Some(drag) = editor.drag.take() {
        if drag.draft {
            editor.drafts = drag.members.into_iter().map(|(_, d)| d).collect();
            editor.preview = editor.drafts.first().map(|d| (d.model, d.pose));
            editor.preview_parts = editor
                .drafts
                .first()
                .map_or_else(Vec::new, |d| d.parts.clone());
        } else {
            editor.preview = None;
            editor.preview_parts.clear();
        }
        editor.previews.clear();
        editor.preview_sources.clear();
        editor.preview_root = None;
    }
}
fn selected_definitions(editor: &ParkEditor, world: &ParkWorld) -> Vec<Definition> {
    editor
        .selection
        .iter()
        .filter_map(|key| {
            editor.previews.get(key).cloned().or_else(|| {
                world
                    .objects
                    .get(key)
                    .map(benilla_protocol::park::Object::definition)
            })
        })
        .collect()
}

fn triangle_hit(ray: Ray3d, triangle: [Vec3; 3]) -> Option<f32> {
    let edge1 = triangle[1] - triangle[0];
    let edge2 = triangle[2] - triangle[0];
    let p = ray.direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() < 1e-7 {
        return None;
    }
    let inverse = determinant.recip();
    let origin = ray.origin - triangle[0];
    let u = origin.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = origin.cross(edge1);
    let v = ray.direction.dot(q) * inverse;
    if v < 0. || u + v > 1. {
        return None;
    }
    let distance = edge2.dot(q) * inverse;
    (distance >= 0.).then_some(distance)
}
fn pick(ray: Ray3d, world: &ParkWorld, owner: u64, _catalog: &scene::Catalog) -> Option<Key> {
    world
        .objects
        .iter()
        .filter(|(key, _)| key.0 == owner)
        .filter_map(|(key, object)| {
            let matrix = math::transform(object.pose).to_matrix();
            let geometry = geometry::definition(&object.definition()).ok()?;
            geometry
                .pick_triangles()
                .iter()
                .filter_map(|t| triangle_hit(ray, t.map(|p| matrix.transform_point3(p))))
                .min_by(f32::total_cmp)
                .map(|distance| (*key, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(key, _)| key)
}
fn distance_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let length = d.length_squared();
    if length < 1e-4 {
        return point.distance(a);
    }
    point.distance(a + d * ((point - a).dot(d) / length).clamp(0., 1.))
}
fn gizmo_axis(
    editor: &ParkEditor,
    world: &ParkWorld,
    cursor: Vec2,
    size: Vec2,
    projection: &Projection,
) -> Option<usize> {
    let pose = editor.target_pose(world)?;
    let pivot = Vec3::from_array(pose.position);
    let length = scene::handle_size(pivot, editor.camera.translation);
    let from = math::screen_point(pivot, size, editor.camera, projection)?;
    let mut best = (None, 12.);
    for i in 0..3 {
        let axis = if editor.local || editor.tool == Tool::Scale {
            Quat::from_array(pose.rotation) * math::AXES[i]
        } else {
            math::AXES[i]
        };
        let distance = if editor.tool == Tool::Rotate {
            let a = axis.any_orthonormal_vector();
            let b = axis.cross(a);
            let mut nearest = f32::INFINITY;
            for n in 0..64 {
                let point = |n: usize| {
                    let angle = n as f32 / 64. * std::f32::consts::TAU;
                    pivot + (a * angle.cos() + b * angle.sin()) * length * 0.7
                };
                if let (Some(a), Some(b)) = (
                    math::screen_point(point(n), size, editor.camera, projection),
                    math::screen_point(point(n + 1), size, editor.camera, projection),
                ) {
                    nearest = nearest.min(distance_to_segment(cursor, a, b));
                }
            }
            nearest
        } else if let Some(to) =
            math::screen_point(pivot + axis * length, size, editor.camera, projection)
        {
            if to.distance(from) < 16. {
                continue;
            }
            distance_to_segment(cursor, from + (to - from) * 0.25, to)
        } else {
            continue;
        };
        if distance < best.1 {
            best = (Some(i), distance);
        }
    }
    best.0
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut keyboard: MessageReader<KeyboardInput>,
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut camera: Query<(&mut Transform, &Projection), With<benilla_world::view::WorldCamera>>,
    mut world: ResMut<ParkWorld>,
    catalog_browser: (Res<scene::Catalog>, Res<super::catalog_ui::Browser>),
    self_guid: Res<SelfGuid>,
    net: Res<NetCommands>,
    ui: Res<crate::ui_script::UiKeyboardCapture>,
    player_world: (
        Res<crate::player::Player>,
        benilla_world::collision::WorldCollision,
    ),
    mut editor: ResMut<ParkEditor>,
    mut actions: ResMut<Actions>,
) {
    let (catalog, browser) = catalog_browser;
    let (player, collision) = player_world;
    let Ok((window, mut cursor_options)) = windows.single_mut() else {
        return;
    };
    let Ok((mut cam, projection)) = camera.single_mut() else {
        return;
    };
    let owner = self_guid.0.unwrap_or(0);
    let size = Vec2::new(window.width(), window.height());
    let cursor = super::diagnostic_cursor()
        .or_else(|| window.cursor_position())
        .unwrap_or(size * 0.5);
    let over_panel = cursor.x < 270.;
    if world.capable && !world.ready && player.active && time.elapsed_secs() - world.last_sync > 0.5
    {
        world.last_sync = time.elapsed_secs();
        send(&net, Message::Sync);
    }
    if keys.just_pressed(KeyCode::F8) && (editor.active || !ui.typing) {
        editor.toggle_requested = true;
    }
    if editor.toggle_requested {
        editor.toggle_requested = false;
        if editor.active {
            editor.active = false;
            cancel(&mut editor);
        } else if player.active && owner != 0 {
            editor.active = true;
            editor.saved_camera = Some(*cam);
            editor.camera = *cam;
            editor.status = if world.ready {
                "Choose an object, or click one of your objects"
            } else {
                "The server needs the Park Builder update"
            }
            .into();
        }
    }
    if !editor.active {
        keyboard.clear();
        actions.0.clear();
        if let Some(saved) = editor.saved_camera.take() {
            *cam = saved;
            cursor_options.visible = true;
            cursor_options.grab_mode = CursorGrabMode::None;
        }
        return;
    }
    if browser.open {
        keyboard.clear();
        *cam = editor.camera;
        return;
    }
    if !player.active || owner == 0 {
        editor.active = false;
        editor.saved_camera = None;
        cancel(&mut editor);
        keyboard.clear();
        return;
    }
    cursor_options.visible = true;
    cursor_options.grab_mode = CursorGrabMode::None;
    if !window.focused && !super::diagnostic_active() {
        if editor.drag.is_some() {
            cancel(&mut editor);
        }
        keyboard.clear();
        return;
    }
    if let Some(pending) = &editor.pending {
        if time.elapsed_secs() - pending.sent_at > 5. {
            editor.status = "Waiting for the server; the preview is not confirmed yet".into();
        }
    }
    let fine = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let navigating = buttons.pressed(MouseButton::Right);
    if navigating {
        if buttons.just_pressed(MouseButton::Right) {
            reset_camera_drag(&mut editor);
        }
        if !super::diagnostic_active() {
            cursor_options.visible = false;
            cursor_options.grab_mode = CursorGrabMode::Locked;
        }
        let (yaw, pitch, _) = editor.camera.rotation.to_euler(EulerRot::YXZ);
        editor.camera.rotation = Quat::from_euler(
            EulerRot::YXZ,
            yaw - motion.delta.x * 0.003,
            (pitch - motion.delta.y * 0.003).clamp(-1.5, 1.5),
            0.,
        );
        let mut movement = Vec3::ZERO;
        if keys.pressed(KeyCode::KeyW) {
            movement.z -= 1.;
        }
        if keys.pressed(KeyCode::KeyS) {
            movement.z += 1.;
        }
        if keys.pressed(KeyCode::KeyA) {
            movement.x -= 1.;
        }
        if keys.pressed(KeyCode::KeyD) {
            movement.x += 1.;
        }
        if keys.pressed(KeyCode::KeyQ) {
            movement.y -= 1.;
        }
        if keys.pressed(KeyCode::KeyE) {
            movement.y += 1.;
        }
        let rotation = editor.camera.rotation;
        editor.camera.translation += rotation
            * movement.normalize_or_zero()
            * time.delta_secs().min(0.1)
            * if fine { 60. } else { 15. };
        keyboard.clear();
        actions.0.clear();
        *cam = editor.camera;
        return;
    } else if !over_panel && editor.drag.is_none() {
        let forward = editor.camera.rotation * Vec3::NEG_Z;
        editor.camera.translation += forward * scroll.delta.y.clamp(-10., 10.) * 1.5;
    }
    if !navigating {
        if keys.just_pressed(KeyCode::Escape) {
            if editor.preview.is_some() || editor.drag.is_some() {
                cancel(&mut editor);
            } else {
                actions.0.push(Action::Exit);
            }
        }
        if control && keys.just_pressed(KeyCode::KeyZ) {
            actions
                .0
                .push(if fine { Action::Redo } else { Action::Undo });
        }
        if control && keys.just_pressed(KeyCode::KeyY) {
            actions.0.push(Action::Redo);
        }
        if control && keys.just_pressed(KeyCode::KeyD) {
            actions.0.push(Action::Duplicate);
        }
        if keys.just_pressed(KeyCode::Delete) {
            actions.0.push(Action::Delete);
        }
        for (key, model) in [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
        ]
        .into_iter()
        .zip(Model::ALL)
        {
            if editor.drag.is_none() && keys.just_pressed(key) {
                actions.0.push(Action::Create(model));
            }
        }
        for (key, tool) in [
            (KeyCode::KeyG, Tool::Move),
            (KeyCode::KeyR, Tool::Rotate),
            (KeyCode::KeyS, Tool::Scale),
        ] {
            if keys.just_pressed(key) && !control {
                editor.tool = tool;
                editor.axis = None;
                begin_drag(&mut editor, &world, cursor, false);
            }
        }
        if editor.drag.is_some() {
            for (i, key) in [KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ]
                .into_iter()
                .enumerate()
            {
                if keys.just_pressed(key) && !control {
                    editor.local = editor.axis == Some(i) && !editor.local;
                    editor.axis = Some(i);
                }
            }
        }
    }
    for action in std::mem::take(&mut actions.0) {
        match action {
            Action::Exit => {
                editor.active = false;
                cancel(&mut editor);
                if let Some(saved) = editor.saved_camera.take() {
                    *cam = saved;
                }
                keyboard.clear();
                return;
            }
            Action::Tool(tool) => {
                reset_camera_drag(&mut editor);
                editor.tool = tool;
                editor.axis = None;
                editor.drag = None;
            }
            Action::Create(model) => {
                if editor.pending.is_none() && world.ready {
                    editor.clear_preview();
                    editor.select(None, false);
                    editor.axis = None;
                    let forward = editor.camera.rotation * Vec3::NEG_Z;
                    let mut position = editor.camera.translation + forward * 8.;
                    position.y = player.pos.y;
                    let toe = position
                        + Vec3::new(
                            0.,
                            0.,
                            match model {
                                Model::Ramp => -3.,
                                Model::QuarterPipe => -2.,
                                _ => 0.,
                            },
                        );
                    let ray_origin = toe + Vec3::Y * 2.;
                    if let Some(hit) = collision.ray_body(ray_origin, Dir3::NEG_Y, 2048.) {
                        position.y = ray_origin.y - hit.distance;
                    }
                    editor.preview = Some((
                        model,
                        Pose {
                            position: position.to_array(),
                            ..default()
                        },
                    ));
                    let pose = editor.preview.unwrap().1;
                    editor.drafts = vec![Definition::primitive(model, pose)];
                    editor.status =
                        "New object: G move · R rotate · S size · Enter place · Esc cancel".into();
                }
            }
            Action::CreateAsset(asset) => {
                if editor.pending.is_none() && world.ready {
                    editor.clear_preview();
                    editor.select(None, false);
                    editor.axis = None;
                    let mut position =
                        editor.camera.translation + editor.camera.rotation * Vec3::NEG_Z * 8.;
                    position.y = player.pos.y;
                    let origin = position + Vec3::Y * 2.;
                    if let Some(hit) = collision.ray_body(origin, Dir3::NEG_Y, 2048.) {
                        position.y = origin.y - hit.distance;
                    }
                    let pose = Pose {
                        position: position.to_array(),
                        ..default()
                    };
                    editor.preview = Some((Model::Asset, pose));
                    editor.drafts = vec![Definition::asset(asset, pose)];
                    editor.status =
                        "New object: G move · R rotate · S size · Enter place · Esc cancel".into();
                }
            }
            Action::Catalog => {}
            Action::Place => commit(&mut editor, &world, &net, owner, time.elapsed_secs()),
            Action::Duplicate => {
                if editor.pending.is_none() {
                    let definitions = selected_definitions(&editor, &world);
                    if !definitions.is_empty() {
                        editor.clear_preview();
                        editor.select(None, false);
                        editor.preview = definitions.first().map(|d| (d.model, d.pose));
                        editor.preview_parts = definitions[0].parts.clone();
                        editor.drafts = definitions;
                        editor.status = "Duplicate: position it, then Enter to place".into();
                    }
                }
            }
            Action::Delete => {
                let changes = editor
                    .selection
                    .iter()
                    .filter_map(|key| world.objects.get(key))
                    .map(|object| Change {
                        id: Some(object.id),
                        before: Some(object.definition()),
                        after: None,
                    })
                    .collect::<Vec<_>>();
                if !changes.is_empty() {
                    request(
                        &mut editor,
                        &world,
                        &net,
                        owner,
                        HistoryAction::Push(changes),
                        time.elapsed_secs(),
                    );
                } else {
                    cancel(&mut editor);
                }
            }
            Action::Undo => {
                if editor.cursor > 0 {
                    let index = editor.cursor - 1;
                    request(
                        &mut editor,
                        &world,
                        &net,
                        owner,
                        HistoryAction::Undo(index),
                        time.elapsed_secs(),
                    );
                }
            }
            Action::Redo => {
                if editor.cursor < editor.history.len() {
                    let index = editor.cursor;
                    request(
                        &mut editor,
                        &world,
                        &net,
                        owner,
                        HistoryAction::Redo(index),
                        time.elapsed_secs(),
                    );
                }
            }
            Action::AlignHeight => {
                if editor.selection.len() < 2 {
                    editor.status = "Ctrl-click at least two objects to align".into();
                    continue;
                }
                let reference = editor.selected.and_then(|key| {
                    editor
                        .previews
                        .get(&key)
                        .map(|d| d.pose)
                        .or_else(|| world.objects.get(&key).map(|o| o.pose))
                });
                if let Some(reference) = reference {
                    let changes = editor
                        .selection
                        .iter()
                        .filter_map(|key| {
                            let object = world.objects.get(key)?;
                            let mut next = editor
                                .previews
                                .get(key)
                                .cloned()
                                .unwrap_or_else(|| object.definition());
                            next.pose.position[1] = reference.position[1];
                            (object.definition() != next).then(|| Change {
                                id: Some(object.id),
                                before: Some(object.definition()),
                                after: Some(next),
                            })
                        })
                        .collect::<Vec<_>>();
                    if changes.is_empty() {
                        editor.status = "Selected objects already share the same Z height".into();
                    } else {
                        request(
                            &mut editor,
                            &world,
                            &net,
                            owner,
                            HistoryAction::Push(changes),
                            time.elapsed_secs(),
                        );
                    }
                }
            }
            Action::Fuse => {
                if editor.pending.is_some() {
                    editor.status = "Waiting for the server".into();
                    continue;
                }
                match math::fuse(&selected_definitions(&editor, &world)) {
                    Ok(fused) => {
                        let mut changes = editor
                            .selection
                            .iter()
                            .filter_map(|key| world.objects.get(key))
                            .map(|object| Change {
                                id: Some(object.id),
                                before: Some(object.definition()),
                                after: None,
                            })
                            .collect::<Vec<_>>();
                        changes.push(Change {
                            id: None,
                            before: None,
                            after: Some(fused),
                        });
                        request(
                            &mut editor,
                            &world,
                            &net,
                            owner,
                            HistoryAction::Push(changes),
                            time.elapsed_secs(),
                        );
                    }
                    Err(error) => editor.status = error.into(),
                }
            }
        }
    }
    if editor.drag.is_some() {
        if keys.just_pressed(KeyCode::Backspace) {
            editor.drag.as_mut().unwrap().numeric.pop();
        }
        for event in keyboard.read() {
            if event.state.is_pressed() {
                if let Some(text) = &event.text {
                    for ch in text
                        .chars()
                        .filter(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+'))
                    {
                        let text = &mut editor.drag.as_mut().unwrap().numeric;
                        if text.len() < 32 {
                            text.push(ch);
                        }
                    }
                }
            }
        }
        let drag = editor.drag.as_ref().unwrap();
        let start = drag.start;
        let members = drag.members.clone();
        let draft = drag.draft;
        let button_drag = drag.button;
        let numeric = drag.numeric.parse::<f32>().ok().filter(|n| n.is_finite());
        let start_cursor = drag.cursor;
        let pivot = Vec3::from_array(start.position);
        let view = editor.camera.rotation * Vec3::NEG_Z;
        let direction = editor.axis.map(|i| {
            if editor.local || editor.tool == Tool::Scale {
                Quat::from_array(start.rotation) * math::AXES[i]
            } else {
                math::AXES[i]
            }
        });
        let mut result = start;
        if let Some(value) = numeric {
            result = math::apply(
                start,
                editor.tool,
                editor.axis,
                editor.local,
                if editor.tool == Tool::Rotate {
                    value.to_radians()
                } else {
                    value
                },
                view,
            );
        } else {
            let delta = cursor - start_cursor;
            let precision = if fine { 0.1 } else { 1. };
            match editor.tool {
                Tool::Move => {
                    let normal = math::drag_plane(direction, view);
                    if let (Some(a), Some(b)) = (
                        math::screen_ray(start_cursor, size, editor.camera, projection)
                            .and_then(|r| math::ray_plane(r, pivot, normal)),
                        math::screen_ray(cursor, size, editor.camera, projection)
                            .and_then(|r| math::ray_plane(r, pivot, normal)),
                    ) {
                        let offset = (b - a) * precision;
                        let offset = direction.map_or(offset, |axis| axis * offset.dot(axis));
                        let offset = if control {
                            (offset * 4.).round() / 4.
                        } else {
                            offset
                        };
                        result.position = (pivot + offset).to_array();
                    } else if let Some(axis) = direction {
                        result = math::apply(
                            start,
                            Tool::Move,
                            editor.axis,
                            editor.local,
                            -delta.y * pivot.distance(editor.camera.translation) / size.y
                                * precision,
                            axis,
                        );
                    }
                }
                Tool::Rotate => {
                    let mut angle = (delta.x - delta.y) * 0.01 * precision;
                    if control {
                        angle = (angle / (15_f32.to_radians())).round() * 15_f32.to_radians();
                    }
                    result =
                        math::apply(start, Tool::Rotate, editor.axis, editor.local, angle, view);
                }
                Tool::Scale => {
                    let mut factor = ((delta.x - delta.y) * 0.01 * precision).exp();
                    if control {
                        factor = (factor * 10.).round() / 10.;
                    }
                    result =
                        math::apply(start, Tool::Scale, editor.axis, editor.local, factor, view);
                }
            }
        }
        if result.validate().is_ok() {
            let moved = !button_drag || numeric.is_some() || cursor.distance(start_cursor) >= 4.;
            let result = if moved { result } else { start };
            let delta =
                math::transform(result).to_matrix() * math::transform(start).to_matrix().inverse();
            let transformed = members
                .iter()
                .map(|(key, definition)| (*key, math::transformed(definition, delta)))
                .collect::<Vec<_>>();
            if transformed
                .iter()
                .all(|(_, definition)| definition.validate().is_ok())
            {
                editor.preview_root = Some((start, result));
                if draft {
                    editor.drafts = transformed
                        .into_iter()
                        .map(|(_, definition)| definition)
                        .collect();
                    editor.preview = editor.drafts.first().map(|d| (d.model, d.pose));
                    editor.preview_parts = editor
                        .drafts
                        .first()
                        .map_or_else(Vec::new, |d| d.parts.clone());
                } else {
                    editor.previews = transformed.into_iter().collect();
                }
            }
        }
        let button = editor.drag.as_ref().unwrap().button;
        if keys.just_pressed(KeyCode::Enter)
            || button && buttons.just_released(MouseButton::Left)
            || !button && buttons.just_pressed(MouseButton::Left) && !over_panel
        {
            commit(&mut editor, &world, &net, owner, time.elapsed_secs());
        }
    } else {
        keyboard.clear();
        if keys.just_pressed(KeyCode::Enter) {
            commit(&mut editor, &world, &net, owner, time.elapsed_secs());
        }
        if buttons.just_pressed(MouseButton::Left)
            && !over_panel
            && !navigating
            && editor.pending.is_none()
        {
            if control {
                if let Some(ray) = math::screen_ray(cursor, size, editor.camera, projection) {
                    editor.clear_preview();
                    let selected = pick(ray, &world, owner, &catalog);
                    editor.select(selected, true);
                }
            } else if let Some(axis) = gizmo_axis(&editor, &world, cursor, size, projection) {
                editor.axis = Some(axis);
                begin_drag(&mut editor, &world, cursor, true);
            } else if editor.preview.is_some() && editor.selected.is_none() {
                commit(&mut editor, &world, &net, owner, time.elapsed_secs());
            } else if let Some(ray) = math::screen_ray(cursor, size, editor.camera, projection) {
                let selected = pick(ray, &world, owner, &catalog);
                if selected.is_some_and(|key| editor.selection.contains(&key)) {
                    editor.selected = selected;
                } else {
                    editor.select(selected, false);
                }
                editor.clear_preview();
                editor.axis = None;
                if editor.selected.is_some() {
                    begin_drag(&mut editor, &world, cursor, true);
                }
            }
        }
    }
    *cam = editor.camera;
}
