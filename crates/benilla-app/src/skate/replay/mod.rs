//! Client-side replay presentation. Recorded poses never enter movement, scoring or park RPCs.
mod audio;
mod camera;
mod export;
mod files;
#[cfg(feature = "dev")]
mod probe;
mod timeline;
mod ui;

use super::{Host, SkateDrive, SkatePose};
use benilla_world::{rig_anim::RigPose, schedule::WorldStage, view::WorldCamera};
use bevy::prelude::*;
use camera::{Keyframe, Mode};
use std::path::PathBuf;
use std::sync::Arc;
pub(crate) use timeline::Recording;
use timeline::{Sample, Scene};

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ReplayDrawSet;

#[derive(Clone)]
enum Action {
    Toggle,
    Play,
    Step(i32),
    Seek(f64),
    Camera,
    CameraPath,
    KeyInterpolation,
    FlySpeed(f32),
    FlyResponse,
    In,
    Out,
    ResetTrim,
    SelectKey(i32),
    Rate(f32),
    Window(f64),
    KeyRate(f32),
    Fov(f32),
    Keyframe(bool),
    DeleteKeyframe,
    Save,
    Library,
    Load(usize),
    Export,
}

#[derive(Resource)]
pub(crate) struct Replay {
    pub(crate) active: bool,
    pub(crate) releasing: bool,
    playing: bool,
    cursor: f64,
    in_point: f64,
    out_point: f64,
    rate: f32,
    window: f64,
    fov: f32,
    mode: Mode,
    camera: Transform,
    fly: camera::FlySettings,
    fly_velocity: Vec3,
    keys: Vec<Keyframe>,
    status: String,
    library_open: bool,
    library: Vec<PathBuf>,
    actions: Vec<Action>,
    frames: Vec<Arc<Sample>>,
    recorded_cameras: Vec<Keyframe>,
    visuals: Vec<Arc<timeline::Visual>>,
    pub(crate) visual_pose: Option<(Transform, Arc<[Transform]>)>,
    pub(crate) pose: SkatePose,
    previous_buttons: u16,
    alive: bool,
    saved_camera: Option<(Transform, f32)>,
    saved_visual: Option<(Entity, Transform)>,
    follow_offset: Vec3,
    signature: Vec<u32>,
    preview_keys: bool,
    path_enabled: bool,
    io: Option<crossbeam_channel::Receiver<DiskResult>>,
    exporting: Option<export::Session>,
}
type DiskResult = Result<(Option<files::Clip>, String), String>;

#[derive(serde::Serialize, serde::Deserialize)]
struct Preferences {
    window_seconds: f64,
    #[serde(default)]
    fly: camera::FlySettings,
}

fn load_preferences(mut replay: ResMut<Replay>, host: Res<Host>) {
    let preferences = crate::local_state::replay_settings_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok());
    if let Some(preferences) = preferences {
        replay.fly = preferences.fly.sanitize();
        if preferences.window_seconds.is_finite() {
            replay.window = preferences
                .window_seconds
                .clamp(timeline::MIN_WINDOW, timeline::WINDOW);
        }
    }
    host.recording.set_window(replay.window);
}

fn save_preferences(replay: &Replay) -> Result<(), String> {
    let Some(path) = crate::local_state::replay_settings_path() else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(&Preferences {
        window_seconds: replay.window,
        fly: replay.fly,
    })
    .map_err(|e| e.to_string())?;
    crate::local_state::write_atomic_bytes(&path, &bytes).map_err(|e| e.to_string())
}

fn on_ability(
    In(event): In<benilla_protocol::SessionEvent>,
    me: Res<crate::net::SelfGuid>,
    mut replay: ResMut<Replay>,
) {
    if let benilla_protocol::SessionEvent::Chat(chat) = event {
        if chat.is_addon() && Some(chat.sender_guid) == me.0 && chat.text == "SKATECRAFT\tR\t1" {
            replay.actions.push(Action::Toggle);
        }
    }
}

impl Default for Replay {
    fn default() -> Self {
        Self {
            active: false,
            releasing: false,
            playing: false,
            cursor: 0.,
            in_point: 0.,
            out_point: 0.,
            rate: 1.,
            window: timeline::WINDOW,
            fov: 60_f32.to_radians(),
            mode: Mode::Recorded,
            camera: Transform::IDENTITY,
            fly: default(),
            fly_velocity: Vec3::ZERO,
            keys: Vec::new(),
            status: String::new(),
            library_open: false,
            library: Vec::new(),
            actions: Vec::new(),
            frames: Vec::new(),
            recorded_cameras: Vec::new(),
            visuals: Vec::new(),
            visual_pose: None,
            pose: SkatePose::default(),
            previous_buttons: 0,
            alive: false,
            saved_camera: None,
            saved_visual: None,
            follow_offset: Vec3::ZERO,
            signature: Vec::new(),
            preview_keys: false,
            path_enabled: true,
            io: None,
            exporting: None,
        }
    }
}

impl Replay {
    pub(crate) fn blocks_input(&self) -> bool {
        self.active || self.releasing
    }
    fn bounds(&self) -> (f64, f64) {
        self.frames
            .first()
            .zip(self.frames.last())
            .map_or((0., 0.), |(a, b)| (a.at, b.at))
    }
    fn scene(&self) -> Option<&Arc<Scene>> {
        timeline::pair(&self.frames, self.cursor).map(|(a, _, _)| &a.scene)
    }
    pub(crate) fn park_objects(&self) -> Option<Arc<[benilla_protocol::park::Object]>> {
        self.scene().filter(|_| self.active).map(|s| s.park.clone())
    }
    pub(crate) fn actor_position(&self) -> Vec3 {
        super::from_skate(self.pose.root.w_axis.truncate(), self.pose.origin)
    }
    fn target(&self) -> Vec3 {
        self.actor_position() + Vec3::Y * self.scene().map_or(0.9, |s| s.actor_height * 0.55)
    }
    pub(in crate::skate) fn current_frame(&self) -> Option<&super::Frame> {
        timeline::pair(&self.frames, self.cursor).map(|(a, _, _)| &a.frame)
    }
    pub(in crate::skate) fn playback_rate(&self) -> f32 {
        if self.path_enabled {
            camera::sample(&self.keys, self.cursor).map_or(self.rate, |s| s.2)
        } else {
            self.rate
        }
    }
    pub(in crate::skate) fn cursor_time(&self) -> f64 {
        self.cursor
    }
    pub(in crate::skate) fn is_playing(&self) -> bool {
        self.playing && self.exporting.is_none()
    }
    fn seek(&mut self, at: f64) {
        self.fly_velocity = Vec3::ZERO;
        self.playing = false;
        self.cursor = at.clamp(self.in_point, self.out_point);
        self.preview_keys = true;
    }
    fn clip(&self) -> files::Clip {
        files::Clip {
            frames: self.frames.clone(),
            keys: self.keys.clone(),
            cameras: self.recorded_cameras.clone(),
            visuals: self.visuals.clone(),
            in_point: self.in_point,
            out_point: self.out_point,
            rate: self.rate,
            camera_settings: camera::Settings {
                mode: self.mode,
                pose: camera::pose(self.camera),
                fov: self.fov,
                use_keys: self.path_enabled,
                follow_offset: self.follow_offset.to_array(),
            },
        }
    }
    fn apply_clip(&mut self, clip: files::Clip, scope: (u32, u32)) -> Result<(), String> {
        let first = clip.frames.first().ok_or("The replay is empty")?;
        if (first.scene.map, first.scene.instance) != scope {
            return Err("Visit the recorded map and instance before loading this replay".into());
        }
        if first.scene.signature != self.signature {
            return Err(
                "This replay requires the character model, outfit and size it was recorded with"
                    .into(),
            );
        }
        self.mode = clip.camera_settings.mode;
        self.fly_velocity = Vec3::ZERO;
        self.camera = camera::transform(clip.camera_settings.pose);
        self.fov = clip.camera_settings.fov;
        self.path_enabled = clip.camera_settings.use_keys;
        self.follow_offset = Vec3::from_array(clip.camera_settings.follow_offset);
        self.frames = clip.frames;
        self.keys = clip.keys;
        self.recorded_cameras = clip.cameras;
        self.visuals = clip.visuals;
        self.in_point = clip.in_point;
        self.out_point = clip.out_point;
        self.rate = clip.rate;
        self.cursor = self.in_point;
        self.playing = false;
        self.library_open = false;
        self.status = "Replay loaded".into();
        Ok(())
    }
    fn sample_pose(&mut self) {
        self.visual_pose = timeline::visual_pair(&self.visuals, self.cursor).map(|(a, b, t)| {
            (
                timeline::blend_transform(a.transform, b.transform, t),
                a.locals
                    .iter()
                    .zip(b.locals.iter())
                    .map(|(&a, &b)| timeline::blend_transform(a, b, t))
                    .collect(),
            )
        });
        let Some((a, b, t)) = timeline::pair(&self.frames, self.cursor) else {
            return;
        };
        let mut root_a = a.frame.root;
        root_a.w_axis.w = 1.;
        let mut root_b = b.frame.root;
        root_b.w_axis.w = 1.;
        self.pose = SkatePose {
            active: true,
            state: a.frame.state.clone(),
            origin: a.scene.origin,
            root: timeline::blend_matrix(root_a, root_b, t),
            names: a.frame.names.clone(),
            bones: a
                .frame
                .bones
                .iter()
                .zip(b.frame.bones.iter())
                .map(|(&a, &b)| timeline::blend_matrix(a, b, t))
                .collect(),
        };
        if self.mode == Mode::Recorded {
            if let Some((transform, fov, _)) =
                camera::sample_recorded(&self.recorded_cameras, self.cursor)
            {
                self.camera = transform;
                self.fov = fov;
            } else if let Some((position, basis)) = a.frame.camera {
                self.camera =
                    Transform::from_translation(super::from_skate(position, a.scene.origin))
                        .looking_to(basis.z_axis, basis.y_axis);
            }
        }
        if self.mode == Mode::Follow {
            self.camera = Transform::from_translation(self.target() + self.follow_offset)
                .looking_at(self.target(), Vec3::Y);
        }
        if self.mode == Mode::Tripod {
            self.camera = self.camera.looking_at(self.target(), Vec3::Y);
        }
        if self.path_enabled && (self.playing || self.exporting.is_some() || self.preview_keys) {
            if let Some((transform, fov, _)) = camera::sample(&self.keys, self.cursor) {
                self.camera = transform;
                self.fov = fov;
            }
        }
    }
    pub(super) fn exit(&mut self, host: &mut Host) {
        self.fly_velocity = Vec3::ZERO;
        self.active = false;
        self.playing = false;
        self.releasing = true;
        self.pose.active = false;
        self.exporting = None;
        self.library_open = false;
        host.recording.resume();
        host.resume_offboard = true;
        if let Some(input) = &host.input {
            input.reset();
            input.block_gameplay(true);
        }
    }
}

fn restore_visual(
    mut replay: ResMut<Replay>,
    mut units: Query<&mut Transform, With<crate::net::Embodied>>,
) {
    if let Some((entity, transform)) = replay.saved_visual.take() {
        if let Ok(mut current) = units.get_mut(entity) {
            *current = transform;
        }
    }
}

type ContextKey = (u64, (u32, u32), Entity, Vec<u32>);
#[allow(clippy::type_complexity)]
fn context(
    host: Res<Host>,
    park: Res<crate::park::ParkWorld>,
    mut replay: ResMut<Replay>,
    units: Query<
        (
            Entity,
            &Transform,
            &RigPose,
            Option<&crate::net::ObjectStore>,
        ),
        With<crate::net::Embodied>,
    >,
    model_frames: Query<&Transform>,
    mut previous: Local<Option<ContextKey>>,
) {
    let Ok((entity, unit, rig, store)) = units.single() else {
        replay.alive = false;
        return;
    };
    replay.alive = store.is_none_or(|store| !store.0.is_dead_or_ghost());
    let mut signature: Vec<u32> = rig.key_bones.iter().map(|&v| v as u16 as u32).collect();
    signature.extend(rig.parents.iter().map(|&v| v as u16 as u32));
    signature.extend(
        rig.bind_translations()
            .iter()
            .flat_map(|v| v.to_array().map(f32::to_bits)),
    );
    signature.extend(unit.scale.to_array().map(f32::to_bits));
    let frame_scale = if rig.joints_root == entity {
        Vec3::ONE
    } else {
        model_frames
            .get(rig.joints_root)
            .map_or(Vec3::ONE, |t| t.scale)
    };
    signature.extend(frame_scale.to_array().map(f32::to_bits));
    if let Some(store) = store {
        let fields = &store.0;
        signature.extend([
            fields.unit_displayid().unwrap_or_default() as u32,
            fields.player_bytes().unwrap_or_default(),
            fields.player_bytes_2().unwrap_or_default() & 255,
            fields.player_flags() & 0xc00,
        ]);
        for slot in 0..19 {
            signature.push(fields.player_visible_item_entry(slot).unwrap_or_default());
            for enchant in 0..2 {
                signature.push(
                    fields
                        .player_visible_item_enchant(slot, enchant)
                        .unwrap_or_default(),
                );
            }
        }
    }
    replay.signature = signature.clone();
    let key = (park.generation, park.scope(), rig.joints_root, signature);
    if previous.as_ref() == Some(&key) {
        return;
    }
    *previous = Some(key);
    let mut pivots = vec![Vec3::ZERO; rig.parents.len()];
    for i in 0..pivots.len() {
        let parent = usize::try_from(rig.parents[i]).ok().filter(|&p| p < i);
        pivots[i] = rig.bind_translations()[i] + parent.map_or(Vec3::ZERO, |p| pivots[p]);
    }
    let scale = (unit.scale.y * frame_scale.y).abs();
    let height = rig
        .key_bones
        .iter()
        .position(|&k| k == 6)
        .map_or(1.7, |i| pivots[i].y * scale)
        .clamp(0.25, 100.);
    let scope = park.scope();
    host.recording.context(
        host.epoch,
        Scene {
            map: scope.0,
            instance: scope.1,
            origin: host.origin,
            park: park.replay_objects().into(),
            signature: replay.signature.clone(),
            actor_scale: unit.scale,
            actor_height: height,
        },
    );
}

#[allow(clippy::type_complexity)]
fn controls(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    mut replay: ResMut<Replay>,
    mut host: ResMut<Host>,
    mut drive: ResMut<SkateDrive>,
    mut pose: ResMut<SkatePose>,
    mut audio: ResMut<super::sound::SkateAudio>,
    player: Res<crate::player::Player>,
    park: Res<crate::park::ParkEditor>,
    world: Res<crate::park::ParkWorld>,
    ui: Res<crate::ui_script::UiKeyboardCapture>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<WorldCamera>>,
) {
    let controls = host
        .input
        .as_ref()
        .and_then(|i| i.ui_controls())
        .unwrap_or_default();
    let pressed = controls.buttons & !replay.previous_buttons;
    replay.previous_buttons = controls.buttons;
    let chord = controls.buttons & 0x00c0 == 0x00c0 && pressed & 0x00c0 != 0;
    if (keys.just_pressed(KeyCode::F6) && (!ui.typing || replay.active)) || chord {
        replay.actions.push(Action::Toggle);
    }
    if replay.releasing && !replay.active {
        let neutral = controls.buttons == 0
            && controls.triggers.iter().all(|&v| v < 13)
            && controls
                .left
                .iter()
                .chain(&controls.right)
                .all(|&v| i32::from(v).abs() < 6000)
            && !mouse.pressed(MouseButton::Right)
            && !mouse.pressed(MouseButton::Left)
            && ![
                KeyCode::KeyW,
                KeyCode::KeyA,
                KeyCode::KeyS,
                KeyCode::KeyD,
                KeyCode::Space,
            ]
            .iter()
            .any(|&k| keys.pressed(k));
        if neutral {
            replay.releasing = false;
            if let Some(input) = &host.input {
                input.block_gameplay(false);
            }
        }
    }
    if let Some(io) = &replay.io {
        if let Ok(result) = io.try_recv() {
            replay.io = None;
            match result {
                Ok((Some(clip), _)) if replay.active => {
                    if let Err(error) = replay.apply_clip(clip, world.scope()) {
                        replay.status = error;
                    }
                }
                Ok((None, message)) => {
                    replay.status = message;
                    replay.library = files::list();
                }
                Err(error) => replay.status = error,
                _ => {}
            }
        }
    }
    if replay.active {
        if !player.may_skate()
            || !replay.alive
            || park.active
            || replay
                .frames
                .first()
                .is_some_and(|f| f.scene.signature != replay.signature)
        {
            replay.exit(&mut host);
            return;
        }
        if keys.just_pressed(KeyCode::Escape) || pressed & 0x2000 != 0 {
            replay.actions.push(Action::Toggle);
        }
        if replay.exporting.is_none() {
            if keys.just_pressed(KeyCode::Space) || pressed & 0x1000 != 0 {
                replay.actions.push(Action::Play);
            }
            if keys.just_pressed(KeyCode::KeyC) || pressed & 0x8000 != 0 {
                replay.actions.push(Action::Camera);
            }
            if keys.just_pressed(KeyCode::Comma) || pressed & 0x0004 != 0 {
                replay.actions.push(Action::Step(-1));
            }
            if keys.just_pressed(KeyCode::Period) || pressed & 0x0008 != 0 {
                replay.actions.push(Action::Step(1));
            }
            if keys.just_pressed(KeyCode::BracketLeft) {
                replay.actions.push(Action::In);
            }
            if keys.just_pressed(KeyCode::BracketRight) {
                replay.actions.push(Action::Out);
            }
            if keys.just_pressed(KeyCode::KeyT) || pressed & 0x4000 != 0 {
                replay
                    .actions
                    .push(Action::Keyframe(keys.pressed(KeyCode::ShiftLeft)));
            }
            if keys.just_pressed(KeyCode::PageUp) {
                replay.actions.push(Action::SelectKey(-1));
            }
            if keys.just_pressed(KeyCode::PageDown) {
                replay.actions.push(Action::SelectKey(1));
            }
            if keys.just_pressed(KeyCode::Home) {
                let at = replay.in_point;
                replay.actions.push(Action::Seek(at));
            }
            if keys.just_pressed(KeyCode::End) {
                let at = replay.out_point;
                replay.actions.push(Action::Seek(at));
            }
            if keys.just_pressed(KeyCode::Delete) {
                replay.actions.push(Action::DeleteKeyframe);
            }
            let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
            if ctrl && keys.just_pressed(KeyCode::KeyS) {
                replay.actions.push(Action::Save);
            }
            if ctrl && keys.just_pressed(KeyCode::KeyO) {
                replay.actions.push(Action::Library);
            }
            if ctrl && keys.just_pressed(KeyCode::KeyE) {
                replay.actions.push(Action::Export);
            }
            let scrub = f32::from(controls.triggers[1]) / 255.
                - f32::from(controls.triggers[0]) / 255.
                + f32::from(keys.pressed(KeyCode::ArrowRight))
                - f32::from(keys.pressed(KeyCode::ArrowLeft));
            if scrub.abs() > 0.05 {
                let at = replay.cursor + f64::from(scrub * time.delta_secs().min(0.1) * 4.);
                replay.seek(at);
            } else if replay.playing {
                let rate = replay.playback_rate();
                replay.cursor = (replay.cursor + f64::from(time.delta_secs().min(0.1) * rate))
                    .min(replay.out_point);
                if replay.cursor >= replay.out_point {
                    replay.playing = false;
                }
            }
        }
    }
    for action in std::mem::take(&mut replay.actions) {
        if replay.exporting.is_some() && !matches!(action, Action::Toggle) {
            continue;
        }
        match action {
            Action::Toggle => {
                if replay.active {
                    replay.exit(&mut host);
                    continue;
                }
                if !drive.active
                    || !player.may_skate()
                    || !replay.alive
                    || park.active
                    || replay.signature.is_empty()
                {
                    replay.status = "Replay is available while in Skate mode".into();
                    continue;
                }
                let (frames, recorded_cameras, visuals) = host.recording.freeze();
                if frames.len() < 2 {
                    host.recording.resume();
                    replay.status = "Skate for a moment to record a replay".into();
                    continue;
                }
                super::stop(&mut host, &mut drive, &mut pose);
                audio.reset();
                replay.frames = frames;
                replay.recorded_cameras = recorded_cameras;
                replay.visuals = visuals;
                replay.keys.clear();
                let (start, end) = replay.bounds();
                replay.in_point = start;
                replay.out_point = end;
                replay.cursor = end;
                replay.active = true;
                replay.playing = false;
                replay.mode = Mode::Recorded;
                replay.preview_keys = false;
                replay.path_enabled = true;
                replay.rate = 1.;
                replay.status = "Replay ready".into();
                if let Ok((camera, projection)) = cameras.single_mut() {
                    let fov = match &*projection {
                        Projection::Perspective(p) => p.fov,
                        _ => 1.,
                    };
                    replay.saved_camera = Some((*camera, fov));
                    replay.camera = *camera;
                    replay.fov = fov;
                }
                if let Some(input) = &host.input {
                    input.block_gameplay(true);
                }
            }
            Action::Play if replay.active => {
                replay.fly_velocity = Vec3::ZERO;
                if replay.cursor >= replay.out_point {
                    replay.cursor = replay.in_point;
                }
                replay.playing = !replay.playing;
            }
            Action::Seek(at) if replay.active => replay.seek(at),
            Action::Step(direction) if replay.active => {
                let next = if direction > 0 {
                    replay.frames.iter().find(|f| f.at > replay.cursor + 1e-6)
                } else {
                    replay
                        .frames
                        .iter()
                        .rev()
                        .find(|f| f.at < replay.cursor - 1e-6)
                };
                if let Some(next) = next {
                    let at = next.at;
                    replay.seek(at);
                }
            }
            Action::Camera if replay.active => {
                replay.fly_velocity = Vec3::ZERO;
                replay.preview_keys = false;
                replay.path_enabled = false;
                replay.mode = replay.mode.next();
                replay.follow_offset = replay.camera.translation - replay.target();
            }
            Action::CameraPath if replay.active => {
                replay.fly_velocity = Vec3::ZERO;
                replay.path_enabled = !replay.path_enabled;
                replay.preview_keys = true;
                replay.status = if replay.path_enabled {
                    "Camera keys enabled"
                } else {
                    "Camera keys disabled"
                }
                .into();
            }
            Action::In if replay.active => {
                if replay.cursor < replay.out_point {
                    replay.in_point = replay.cursor;
                }
            }
            Action::Out if replay.active => {
                if replay.cursor > replay.in_point {
                    replay.out_point = replay.cursor;
                }
            }
            Action::ResetTrim if replay.active => {
                let (start, end) = replay.bounds();
                replay.in_point = start;
                replay.out_point = end;
            }
            Action::SelectKey(direction) if replay.active => {
                let at = if direction > 0 {
                    replay.keys.iter().find(|k| k.at > replay.cursor + 1e-6)
                } else {
                    replay
                        .keys
                        .iter()
                        .rev()
                        .find(|k| k.at < replay.cursor - 1e-6)
                }
                .map(|k| k.at);
                if let Some(at) = at {
                    replay.path_enabled = true;
                    replay.seek(at);
                }
            }
            Action::Rate(delta) if replay.active => {
                let next = (replay.rate + delta).clamp(0.1, 4.);
                let factor = next / replay.rate;
                for key in &mut replay.keys {
                    key.rate = (key.rate * factor).clamp(0.1, 4.);
                }
                replay.rate = next
            }
            Action::Window(delta) if replay.active => {
                let next = (replay.window + delta).clamp(timeline::MIN_WINDOW, timeline::WINDOW);
                if next == replay.window {
                    continue;
                }
                replay.window = next;
                host.recording.set_window(replay.window);
                let (start, end) = replay.bounds();
                replay.in_point = (end - replay.window).max(start);
                replay.out_point = end;
                let at = replay.in_point;
                replay.seek(at);
                replay.status = format!(
                    "Recording window: {:.0}s. Longer windows use footage still available; discarded footage cannot be recovered.",
                    replay.window
                );
                if let Err(error) = save_preferences(&replay) {
                    replay.status = format!("Window changed, but could not be remembered: {error}");
                }
            }
            Action::KeyRate(delta) if replay.active => {
                let at = replay.cursor;
                if let Some(key) = replay.keys.iter_mut().find(|k| (k.at - at).abs() < 0.05) {
                    key.rate = (key.rate + delta).clamp(0.1, 4.);
                    replay.status = format!("Selected key speed: {:.2}x", key.rate);
                } else {
                    replay.status = "Seek to a camera key before adjusting its speed".into();
                }
            }
            Action::KeyInterpolation if replay.active => {
                let at = replay.cursor;
                if let Some(key) = replay.keys.iter_mut().find(|k| (k.at - at).abs() < 0.05) {
                    key.interpolation = key.interpolation.next();
                    replay.status =
                        format!("Outgoing camera motion: {}", key.interpolation.label());
                } else {
                    replay.status = "Seek to a camera key before changing its motion".into();
                }
            }
            Action::FlySpeed(factor) if replay.active => {
                replay.fly.speed = (replay.fly.speed * factor).clamp(1., 64.);
                if let Err(error) = save_preferences(&replay) {
                    replay.status =
                        format!("Camera speed changed, but could not be remembered: {error}");
                }
            }
            Action::FlyResponse if replay.active => {
                replay.fly.response = replay.fly.response.next();
                replay.fly_velocity = Vec3::ZERO;
                if let Err(error) = save_preferences(&replay) {
                    replay.status =
                        format!("Camera response changed, but could not be remembered: {error}");
                }
            }
            Action::Fov(delta) if replay.active => {
                replay.playing = false;
                if replay.mode == Mode::Recorded {
                    replay.mode = Mode::Free;
                }
                replay.preview_keys = false;
                replay.fov = (replay.fov + delta).clamp(5_f32.to_radians(), 150_f32.to_radians());
            }
            Action::Keyframe(cut) if replay.active => {
                replay.fly_velocity = Vec3::ZERO;
                replay.sample_pose();
                replay.path_enabled = true;
                let at = replay.cursor;
                let interpolation = replay
                    .keys
                    .iter()
                    .find(|k| (k.at - at).abs() <= 0.015)
                    .map_or(camera::Interpolation::Smooth, |k| k.interpolation);
                replay.keys.retain(|k| (k.at - at).abs() > 0.015);
                if replay.keys.len() < 512 {
                    let key = Keyframe {
                        at,
                        pose: camera::pose(replay.camera),
                        fov: replay.fov,
                        rate: replay.rate,
                        cut,
                        interpolation,
                    };
                    replay.keys.push(key);
                    replay.keys.sort_by(|a, b| a.at.total_cmp(&b.at));
                    replay.status = "Camera keyframe saved".into();
                }
            }
            Action::DeleteKeyframe if replay.active => {
                let at = replay.cursor;
                replay.keys.retain(|k| (k.at - at).abs() > 0.05);
            }
            Action::Save if replay.active && replay.io.is_none() => {
                if let Some(dir) = crate::local_state::replays_dir() {
                    let path = dir.join(format!(
                        "Skate-{}-{}.{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_micros(),
                        replay.frames.len(),
                        files::EXTENSION
                    ));
                    let clip = replay.clip();
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    replay.io = Some(rx);
                    replay.status = "Saving replay…".into();
                    std::thread::spawn(move || {
                        let result = files::save(&path, &clip).map(|()| {
                            (
                                None,
                                format!("Saved {}", path.file_name().unwrap().to_string_lossy()),
                            )
                        });
                        let _ = tx.send(result);
                    });
                }
            }
            Action::Library if replay.active => {
                replay.library = files::list();
                replay.library_open = !replay.library_open;
            }
            Action::Load(index) if replay.active && replay.io.is_none() => {
                if let Some(path) = replay.library.get(index).cloned() {
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    replay.io = Some(rx);
                    replay.status = "Loading replay…".into();
                    std::thread::spawn(move || {
                        let _ = tx.send(files::load(&path).map(|c| (Some(c), String::new())));
                    });
                }
            }
            Action::Export if replay.active => export::begin(&mut replay),
            _ => {}
        }
    }
    if !replay.active {
        return;
    }
    replay.sample_pose();
    replay.preview_keys = false;
    if replay.mode == Mode::Free
        && replay.exporting.is_none()
        && (!replay.playing || !replay.path_enabled || replay.keys.is_empty())
    {
        let axis = |v: i16| {
            let v = f32::from(v) / 32768.;
            v.signum() * ((v.abs() - 0.18) / 0.82).clamp(0., 1.)
        };
        let (mut yaw, mut pitch, roll) = replay.camera.rotation.to_euler(EulerRot::YXZ);
        let dt = time.delta_secs().min(0.1);
        yaw -= axis(controls.right[0]) * dt * 2.;
        pitch += axis(controls.right[1]) * dt * 2.;
        if mouse.pressed(MouseButton::Right) {
            yaw -= motion.delta.x * 0.003;
            pitch -= motion.delta.y * 0.003;
        }
        pitch = pitch.clamp(-1.5, 1.5);
        replay.camera.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll);
        let keyboard = !keys.pressed(KeyCode::ControlLeft) && !keys.pressed(KeyCode::ControlRight);
        let x = axis(controls.left[0]) + f32::from(keyboard && keys.pressed(KeyCode::KeyD))
            - f32::from(keyboard && keys.pressed(KeyCode::KeyA));
        let z = axis(controls.left[1]) + f32::from(keyboard && keys.pressed(KeyCode::KeyW))
            - f32::from(keyboard && keys.pressed(KeyCode::KeyS));
        let y =
            f32::from(controls.buttons & 0x0200 != 0 || (keyboard && keys.pressed(KeyCode::KeyE)))
                - f32::from(
                    controls.buttons & 0x0100 != 0 || (keyboard && keys.pressed(KeyCode::KeyQ)),
                );
        let fast = controls.buttons & 0x0040 != 0
            || keys.pressed(KeyCode::ShiftLeft)
            || keys.pressed(KeyCode::ShiftRight);
        let direction = Quat::from_rotation_y(yaw) * Vec3::new(x, 0., -z) + Vec3::Y * y;
        let fly = replay.fly;
        let movement = fly.movement(&mut replay.fly_velocity, direction, fast, dt);
        replay.camera.translation += movement;
    } else {
        replay.fly_velocity = Vec3::ZERO;
    }
}

#[allow(clippy::type_complexity)]
fn draw(
    mut replay: ResMut<Replay>,
    mut camera: Query<
        (&mut Transform, &mut Projection),
        (With<WorldCamera>, Without<crate::net::Embodied>),
    >,
    mut units: Query<(Entity, &mut Transform), (With<crate::net::Embodied>, Without<WorldCamera>)>,
    mut rig: ResMut<crate::player::CameraControl>,
    host: Res<Host>,
    drive: Res<SkateDrive>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut windows: Query<
        (&mut Window, &mut bevy::window::CursorOptions),
        With<bevy::window::PrimaryWindow>,
    >,
    mut grabbed: Local<(bool, Option<Vec2>)>,
) {
    if let Ok((mut window, mut cursor)) = windows.single_mut() {
        let want = replay.active
            && replay.mode == Mode::Free
            && mouse.pressed(MouseButton::Right)
            && replay.exporting.is_none();
        if want {
            if !grabbed.0 {
                grabbed.1 = window.cursor_position();
                grabbed.0 = true;
            }
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            cursor.visible = false;
        } else if grabbed.0 {
            cursor.grab_mode = bevy::window::CursorGrabMode::None;
            cursor.visible = true;
            if let Some(position) = grabbed.1.take() {
                window.set_cursor_position(Some(position));
            }
            grabbed.0 = false;
        }
    }
    if !replay.active {
        if let Some((transform, fov)) = replay.saved_camera.take() {
            if let Ok((mut camera, mut projection)) = camera.single_mut() {
                *camera = transform;
                if let Projection::Perspective(p) = &mut *projection {
                    p.fov = fov;
                }
            }
        }
        if drive.active {
            if let Ok((camera, Projection::Perspective(p))) = camera.single() {
                host.recording.camera(drive.tick, *camera, p.fov);
            }
        }
        return;
    }
    if let Ok((mut camera, mut projection)) = camera.single_mut() {
        *camera = replay.camera;
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = replay.fov;
        }
    }
    if let Ok((entity, mut unit)) = units.single_mut() {
        replay.saved_visual = Some((entity, *unit));
        unit.translation = replay.actor_position();
        let forward = replay.pose.root.z_axis.truncate();
        unit.rotation = Quat::from_rotation_y(f32::atan2(-forward.x, -forward.z));
        unit.scale = replay.scene().map_or(unit.scale, |s| s.actor_scale);
        if let Some((transform, _)) = &replay.visual_pose {
            *unit = *transform;
        }
    }
    rig.show_avatar();
}

fn capture_visual(
    host: Res<Host>,
    drive: Res<SkateDrive>,
    replay: Res<Replay>,
    units: Query<(&Transform, &RigPose), With<crate::net::Embodied>>,
) {
    if drive.active && !replay.active {
        if let Ok((transform, rig)) = units.single() {
            host.recording.visual(drive.tick, *transform, &rig.locals);
        }
    }
}

fn transition(
    In(event): In<benilla_protocol::SessionEvent>,
    me: Res<crate::net::SelfGuid>,
    mut replay: ResMut<Replay>,
    mut host: ResMut<Host>,
) {
    if matches!(&event, benilla_protocol::SessionEvent::Teleport { guid, .. } if Some(*guid) != me.0)
    {
        return;
    }
    if matches!(
        event,
        benilla_protocol::SessionEvent::Disconnected { .. }
            | benilla_protocol::SessionEvent::LoggedOut
            | benilla_protocol::SessionEvent::TransferPending { .. }
            | benilla_protocol::SessionEvent::Worldport { .. }
            | benilla_protocol::SessionEvent::Teleport { .. }
    ) {
        if replay.active {
            replay.exit(&mut host);
        }
        replay.saved_camera = None;
        // Same-map flag returns keep the preceding run. Reactivation starts a cut at its new
        // origin; a different map, instance or appearance invalidates history in the context pass.
        if matches!(
            event,
            benilla_protocol::SessionEvent::Disconnected { .. }
                | benilla_protocol::SessionEvent::LoggedOut
        ) {
            host.recording.clear();
            replay.frames.clear();
            replay.recorded_cameras.clear();
            replay.visuals.clear();
            replay.visual_pose = None;
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    use crate::net::NetHandlerApp;
    use benilla_protocol::SessionEventKind as K;
    app.init_resource::<Replay>()
        .add_systems(Startup, load_preferences)
        .add_systems(PreUpdate, restore_visual)
        .add_systems(
            Update,
            (context, ui::buttons, controls)
                .chain()
                .after(WorldStage::Net)
                .before(crate::ui_script::UiInput)
                .before(super::SkateInputSet),
        )
        .add_systems(
            Update,
            draw.in_set(WorldStage::Input)
                .in_set(ReplayDrawSet)
                .after(crate::player::SkateCameraSet)
                .after(crate::player::PlayerControlSet)
                .before(crate::player::apply_self_model_fade),
        )
        .net_handler(K::Chat, on_ability)
        .net_handler(K::Disconnected, transition)
        .net_handler(K::LoggedOut, transition)
        .net_handler(K::TransferPending, transition)
        .net_handler(K::Worldport, transition)
        .net_handler(K::Teleport, transition);
    ui::plugin(app);
    app.add_systems(
        PostUpdate,
        capture_visual
            .in_set(benilla_world::rig_anim::PosePost)
            .after(super::rig::retarget),
    );
    export::plugin(app);
    #[cfg(feature = "dev")]
    probe::plugin(app);
}
