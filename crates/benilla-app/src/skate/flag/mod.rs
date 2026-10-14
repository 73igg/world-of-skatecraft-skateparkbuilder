//! A character's private saved return point. Storage and teleports belong to the server.
pub(crate) mod gesture;
#[cfg(feature = "dev")]
mod probe;
mod visual;
use super::{Host, SkateDrive};
use crate::net::{ClientCommand, NetCommands, NetHandlerApp, SelfGuid};
use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_protocol::{
    skate_flag::{Flag, Message},
    SessionEvent, SessionEventKind,
};
use benilla_world::{
    schedule::WorldStage,
    terrain_stream::{RetainedFocus, WorldLoadProgress},
    world_map::CurrentMap,
};
use bevy::prelude::*;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) struct ReturnTarget {
    pub flag: Flag,
    pub seamless: bool,
    pub arrived: bool,
    ready: bool,
}
#[derive(Resource, Default)]
pub(crate) struct State {
    pub(super) saved: Option<Flag>,
    capable: bool,
    sequence: u32,
    pending: Option<(u32, bool, Instant)>,
    pub(super) returning: Option<ReturnTarget>,
}
impl State {
    pub(crate) fn silent_transition(&self) -> bool {
        self.returning.is_some_and(|r| r.seamless)
    }
    pub(super) fn may_resume(&self, at: Vec3) -> bool {
        self.returning
            .is_none_or(|r| r.arrived && r.ready && at.distance(wow_to_bevy(r.flag.position)) < 0.5)
    }
    pub(super) fn offboard(&self) -> bool {
        self.returning.is_some_and(|r| r.arrived)
    }
    pub(super) fn resumed(&mut self) {
        self.returning = None;
        self.pending = None;
    }
    fn request(&mut self, returning: bool) -> u32 {
        self.sequence = self.sequence.wrapping_add(1).max(1);
        self.pending = Some((self.sequence, returning, Instant::now()));
        self.sequence
    }
}
fn send(net: &NetCommands, message: Message) {
    if let Ok(text) = message.encode() {
        let _ = net.0.send(ClientCommand::AddonMessage {
            distribution: benilla_ui::script::AddonDistribution::Party,
            text,
        });
    }
}
fn on_message(In(event): In<SessionEvent>, me: Res<SelfGuid>, mut state: ResMut<State>) {
    let SessionEvent::Chat(chat) = event else {
        return;
    };
    if !chat.is_addon()
        || Some(chat.sender_guid) != me.0
        || !chat.text.starts_with(benilla_protocol::skate_flag::PREFIX)
    {
        return;
    }
    let Ok(message) = Message::decode(&chat.text) else {
        warn!("skate flag: invalid server message");
        return;
    };
    match message {
        Message::Snapshot { request, flag } => {
            if request == 0 || state.pending.is_some_and(|p| p.0 == request) {
                state.capable = true;
                state.saved = flag;
                if request != 0 {
                    state.pending = None;
                }
                info!(
                    "skate flag: saved point restored revision={}",
                    flag.map_or(0, |f| f.revision)
                );
            }
        }
        Message::Returning {
            request,
            flag,
            seamless,
        } => {
            if state.pending.is_some_and(|p| p.0 == request && p.1) {
                state.saved = Some(flag);
                state.returning = Some(ReturnTarget {
                    flag,
                    seamless,
                    arrived: false,
                    ready: false,
                });
                info!("skate flag: return accepted request={request} seamless={seamless}");
            }
        }
        Message::Rejected { request, reason } => {
            if state.pending.is_some_and(|p| p.0 == request) {
                state.pending = None;
                state.returning = None;
            }
            warn!("skate flag: server declined request={request} reason={reason}");
        }
        _ => {}
    }
}
fn on_transition(In(event): In<SessionEvent>, me: Res<SelfGuid>, mut state: ResMut<State>) {
    if matches!(
        event,
        SessionEvent::LoggedOut | SessionEvent::Disconnected { .. }
    ) {
        // Only the live session cache is released. The database flag is never removed here.
        *state = State::default();
        return;
    }
    let Some(target) = state.returning.as_mut() else {
        return;
    };
    if matches!(event, SessionEvent::TransferAborted { .. }) {
        state.returning = None;
        state.pending = None;
        return;
    }
    let destination = match event {
        SessionEvent::Teleport { guid, position, .. } if Some(guid) == me.0 => Some(position),
        SessionEvent::Worldport {
            map_id, position, ..
        } => {
            if map_id != target.flag.map {
                state.returning = None;
                state.pending = None;
                return;
            }
            Some(position)
        }
        _ => None,
    };
    if let Some(position) = destination {
        if Vec3::from_array(position).distance(Vec3::from_array(target.flag.position)) < 0.5 {
            target.arrived = true;
        } else {
            state.returning = None;
            state.pending = None;
        }
    }
}
fn poll(
    drive: Res<SkateDrive>,
    host: Res<Host>,
    player: Res<crate::player::Player>,
    screen: Res<crate::loading_screen::LoadingScreen>,
    park: Res<crate::park::ParkEditor>,
    ui: Res<crate::ui_script::UiKeyboardCapture>,
    net: Res<NetCommands>,
    mut state: ResMut<State>,
) {
    if state
        .pending
        .is_some_and(|p| p.2.elapsed() > Duration::from_secs(10))
        && state.returning.is_none_or(|target| !target.arrived)
    {
        state.pending = None;
        state.returning = None;
        send(&net, Message::Hello);
        warn!("skate flag: request timed out; refreshing saved point");
    }
    let allowed = drive.active
        && player.may_skate()
        && !screen.covering()
        && !park.active
        && !ui.typing
        && state.pending.is_none();
    let actions = host.input.as_ref().map_or_else(Vec::new, |input| {
        input.flag_actions(allowed, Instant::now())
    });
    // Only one request may own a teleport at a time, even after a render stall or rapid taps.
    for action in actions.into_iter().take(1) {
        if !state.capable {
            send(&net, Message::Hello);
            warn!("skate flag: server support is not ready");
            continue;
        }
        let returning = action == gesture::Action::Return;
        if returning {
            let revision = state.saved.map_or(0, |flag| flag.revision);
            let request = state.request(true);
            send(&net, Message::Return { request, revision });
        } else {
            let position = bevy_to_wow(drive.pos);
            let facing = drive.facing.rem_euclid(std::f32::consts::TAU);
            // Serialize an exact movement sample before placement; the server validates against
            // that authoritative position instead of an up-to-500-ms-old heartbeat.
            let _ = net.0.send(ClientCommand::Move {
                kind: crate::net::MoveKind::Heartbeat,
                flags: if drive.moving() {
                    crate::creature_anim::move_flags::FORWARD
                } else {
                    0
                },
                pos: position,
                orientation: facing,
                pitch: 0.,
                fall_time: 0,
                jump: None,
                transport: None,
            });
            let request = state.request(false);
            send(
                &net,
                Message::Place {
                    request,
                    position,
                    facing,
                },
            );
        }
    }
}
fn prepare_destination(
    time: Res<Time>,
    mut state: ResMut<State>,
    drive: Res<SkateDrive>,
    replay: Res<super::replay::Replay>,
    mut player: ResMut<crate::player::Player>,
    profession: super::profession::Profession,
    map: Option<Res<CurrentMap>>,
    progress: Res<WorldLoadProgress>,
    mut retained: ResMut<RetainedFocus>,
) {
    retained.0 = state
        .saved
        .filter(|f| {
            map.as_ref().is_some_and(|m| m.0 == f.map)
                && (drive.active || profession.auras().board || state.returning.is_some())
        })
        .map(|f| [f.position[0], f.position[1]]);
    if replay.active {
        let position = benilla_assets::coords::bevy_to_wow(replay.actor_position());
        retained.0 = Some([position[0], position[1]]);
    }
    if let Some(target) = state.returning.as_mut().filter(|r| r.arrived) {
        let at = wow_to_bevy(target.flag.position);
        let tile = benilla_formats::world_to_tile(target.flag.position[0], target.flag.position[1]);
        target.ready = player.pos.distance(at) < 0.5
            && progress.focus_tile == Some((tile.0 as i32, tile.1 as i32))
            && progress.focus_resident
            && progress.presentable();
        if player.pos.distance(at) < 0.5 && !target.ready {
            player.settling = true;
            player.settle_deadline = time.elapsed_secs() + 5.;
        }
    }
}
pub(super) fn plugin(app: &mut App) {
    app.init_resource::<State>()
        .init_resource::<RetainedFocus>()
        .net_handler(SessionEventKind::Chat, on_message)
        .net_handler(SessionEventKind::Teleport, on_transition)
        .net_handler(SessionEventKind::Worldport, on_transition)
        .net_handler(SessionEventKind::TransferAborted, on_transition)
        .net_handler(SessionEventKind::LoggedOut, on_transition)
        .net_handler(SessionEventKind::Disconnected, on_transition)
        .add_systems(
            Update,
            (poll, prepare_destination)
                .chain()
                .in_set(WorldStage::Input)
                .after(crate::player::PlayerControlSet)
                .in_set(crate::char_select::InWorldGated),
        );
    visual::plugin(app);
    #[cfg(feature = "dev")]
    probe::plugin(app);
}
