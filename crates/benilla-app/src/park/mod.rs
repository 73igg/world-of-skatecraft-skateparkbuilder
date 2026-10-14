//! World of Skatecraft's session-owned park editor; objects live on the authenticated server.
#[cfg(feature = "dev")]
mod catalog_probe;
mod catalog_ui;
mod collision;
mod geometry;
mod input;
mod library;
mod math;
#[cfg(feature = "dev")]
mod probe;
mod replay;
pub(crate) use replay::ReplaySceneReady;
mod scene;
mod seams;
mod solid;
mod state;
mod ui;

pub(super) fn diagnostic_active() -> bool {
    #[cfg(feature = "dev")]
    {
        probe::enabled() || catalog_probe::enabled()
    }
    #[cfg(not(feature = "dev"))]
    {
        false
    }
}
pub(super) fn diagnostic_cursor() -> Option<Vec2> {
    #[cfg(feature = "dev")]
    {
        probe::cursor()
    }
    #[cfg(not(feature = "dev"))]
    {
        None
    }
}

use crate::net::{NetCommands, NetHandlerApp, SelfGuid};
use benilla_protocol::{park::Message, SessionEvent, SessionEventKind};
use benilla_world::schedule::WorldStage;
use bevy::prelude::*;
pub(crate) use collision::clean_skate_contacts;
pub(crate) use scene::{authored_collision, ParkScene};
pub(crate) use state::{ParkEditor, ParkWorld};

fn on_message(
    In(event): In<SessionEvent>,
    self_guid: Res<SelfGuid>,
    net: Res<NetCommands>,
    mut world: ResMut<ParkWorld>,
    mut editor: ResMut<ParkEditor>,
) {
    let SessionEvent::Chat(chat) = event else {
        return;
    };
    if !chat.is_addon()
        || Some(chat.sender_guid) != self_guid.0
        || !chat.text.starts_with(benilla_protocol::park::PREFIX)
    {
        return;
    }
    let Ok(message) = Message::decode(&chat.text) else {
        warn!("park: incompatible or malformed server message");
        return;
    };
    match message {
        Message::Available => {
            world.capable = true;
            input::send(&net, Message::Hello);
        }
        Message::Toggle => {
            editor.toggle_requested = true;
        }
        Message::Rejected { request, reason } => {
            if editor
                .pending
                .as_ref()
                .is_some_and(|pending| pending.request == request)
            {
                editor.pending = None;
                editor.clear_preview();
                editor.status = match reason {
                    2 => "Object changed; refreshing it",
                    3 => "Object no longer exists",
                    4 => "Session or map object limit reached",
                    5 => "You can only edit your own objects",
                    _ => "Server rejected that transform",
                }
                .into();
                input::send(&net, Message::Sync);
            }
        }
        _ => {
            let old_scope = (world.map, world.instance);
            if let Err(error) = world.receive(&message) {
                editor.status = error.into();
                input::send(&net, Message::Sync);
                return;
            }
            if let Message::End { .. } = message {
                if old_scope != (world.map, world.instance) {
                    editor.history.clear();
                    editor.cursor = 0;
                    editor.selected = None;
                    editor.selection.clear();
                    editor.clear_preview();
                }
                info!(
                    "park: snapshot map={} instance={} objects={}",
                    world.map,
                    world.instance,
                    world.objects.len()
                );
            }
            editor.acknowledged(&message, self_guid.0.unwrap_or(0));
            editor.prune(&world);
            if editor
                .selected
                .is_some_and(|key| !world.objects.contains_key(&key))
                && editor.pending.is_none()
            {
                editor.selected = None;
                editor.preview = None;
                editor.drag = None;
            }
        }
    }
}
fn leave(
    In(event): In<SessionEvent>,
    self_guid: Res<SelfGuid>,
    mut world: ResMut<ParkWorld>,
    mut editor: ResMut<ParkEditor>,
) {
    let own = match event {
        SessionEvent::Disconnected { .. }
        | SessionEvent::LoggedOut
        | SessionEvent::TransferPending { .. }
        | SessionEvent::Worldport { .. } => true,
        SessionEvent::Teleport { guid, .. } => Some(guid) == self_guid.0,
        _ => false,
    };
    if !own {
        return;
    }
    // Travel discards this client's view; the server keeps objects until the owning session ends.
    let ended = matches!(
        event,
        SessionEvent::Disconnected { .. } | SessionEvent::LoggedOut
    );
    world.clear();
    if ended {
        world.capable = false;
    }
    editor.active = false;
    editor.saved_camera = None;
    editor.toggle_requested = false;
    editor.selected = None;
    editor.selection.clear();
    editor.clear_preview();
    editor.pending = None;
    editor.history.clear();
    editor.cursor = 0;
}
fn camera_override(
    editor: Res<ParkEditor>,
    mut camera: Query<&mut Transform, With<benilla_world::view::WorldCamera>>,
) {
    if editor.active {
        for mut transform in &mut camera {
            *transform = editor.camera;
        }
    }
}

pub(crate) fn plugin(app: &mut App) {
    replay::plugin(app);
    app.init_resource::<ParkEditor>()
        .init_resource::<ParkWorld>()
        .init_resource::<ParkScene>()
        .init_resource::<input::Actions>()
        .init_resource::<library::Library>()
        .net_handler(SessionEventKind::Chat, on_message)
        .net_handler(SessionEventKind::Disconnected, leave)
        .net_handler(SessionEventKind::LoggedOut, leave)
        .net_handler(SessionEventKind::TransferPending, leave)
        .net_handler(SessionEventKind::Worldport, leave)
        .net_handler(SessionEventKind::Teleport, leave)
        .add_systems(Startup, (scene::startup, ui::startup))
        .add_systems(Update, library::tick.before(scene::sync))
        .add_systems(
            Update,
            (ui::buttons, input::update)
                .chain()
                .after(WorldStage::Net)
                .before(crate::ui_script::UiInput),
        )
        .add_systems(
            Update,
            scene::sync
                .in_set(WorldStage::Input)
                .before(crate::skate::SkateInputSet)
                .before(crate::player::PlayerControlSet),
        )
        .add_systems(
            Update,
            (camera_override, scene::draw, ui::draw)
                .chain()
                .in_set(WorldStage::Input)
                .after(crate::player::PlayerControlSet),
        );
    catalog_ui::plugin(app);
    #[cfg(feature = "dev")]
    catalog_probe::plugin(app);
    #[cfg(feature = "dev")]
    app.add_systems(
        Update,
        probe::run.after(WorldStage::Net).before(input::update),
    );
}
