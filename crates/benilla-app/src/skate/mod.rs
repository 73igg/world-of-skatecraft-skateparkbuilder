//! Skate 3 mode: `J` drops our character onto a board driven by the Skate 3 engine (`skate-host`,
//! as in the IW4L skate mode). The engine runs on a worker thread over the world's collision
//! gathered around us; while it rides, its pose is our position, facing and camera (read by
//! `player::controller`), its skeleton is retargeted onto our character's ([`rig`]) and the board
//! is drawn under it ([`board`]). The controller is an Xbox-style pad, read by the engine itself.

mod board;
mod export;
pub(crate) mod flag;
mod input;
mod input_buffer;
mod latest;
mod profession;
mod rails;
mod rig;
mod score;
mod sound;
mod work;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use benilla_world::collision::WorldCollision;
use benilla_world::schedule::WorldStage;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use skate_host::bridge::{InputFrame, PreparedCollision, Session};

use crate::net::NetHandlerApp;

/// The engine's unit is the meter; ours is the yard.
const METERS_PER_YARD: f32 = 0.9144;
/// Half extents (yards) of the collision box gathered around the skater.
const GATHER_HALF: Vec3 = Vec3::new(70.0, 45.0, 70.0);
/// How far (yards) the skater rolls from the box centre before it is rebuilt around them.
const RECENTRE: f32 = 30.0;
/// Seconds between rebuilds while riding, so terrain streamed in since becomes solid.
const REFRESH_SECS: f32 = 5.0;
/// Below this speed (yd/s) the board counts as standing still on the wire.
const MOVING_SPEED: f32 = 0.5;

type Tri = [[f32; 3]; 3];

/// Our world (Bevy yards) to the engine's: meters from the session's `origin`, same axes (both
/// are right-handed, +Y up), which keeps the engine's floats small anywhere on a continent.
pub(crate) fn to_skate(p: Vec3, origin: Vec3) -> Vec3 {
    (p - origin) * METERS_PER_YARD
}

/// Inverse of [`to_skate`].
pub(crate) fn from_skate(p: Vec3, origin: Vec3) -> Vec3 {
    p / METERS_PER_YARD + origin
}

/// The skate engine's assets, converted from Skate 3 (`board.json`, `rig.json`, the banks):
/// `WOW_SKATE_ASSETS`, else a `skate-data/assets` beside or above the working directory.
pub(crate) fn assets_root() -> Option<PathBuf> {
    static ROOT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::var_os("WOW_SKATE_ASSETS")
            .map(PathBuf::from)
            .or_else(|| {
                ["skate-data/assets", "../skate-data/assets"]
                    .into_iter()
                    .map(PathBuf::from)
                    .find(|p| p.join("rig.json").is_file())
            })
    })
    .clone()
}

/// What the controller reads while we ride: it drives the body and the wire from this instead of
/// the mover.
#[derive(Resource, Default)]
pub(crate) struct SkateDrive {
    pub(crate) active: bool,
    /// Feet, Bevy world.
    pub(crate) pos: Vec3,
    /// Facing (Bevy yaw): the direction of travel, else the board's heading.
    pub(crate) yaw: f32,
    /// Actual skater facing, independent of the board's direction of travel.
    pub(crate) facing: f32,
    #[cfg(feature = "dev")]
    pub(crate) offboard: bool,
    /// Horizontal speed, yd/s.
    pub(crate) speed: f32,
    /// The engine's own follow camera, Bevy world.
    pub(crate) camera: Option<Transform>,
    pub(crate) camera_distance: f32,
    pub(crate) camera_height: f32,
    /// Ride with the engine's camera rather than our follow camera (`K` toggles).
    pub(crate) engine_camera: bool,
}

impl SkateDrive {
    pub(crate) fn moving(&self) -> bool {
        self.speed > MOVING_SPEED
    }
}

/// The skater's latest solved skeleton, for the retarget and the board: bones relative to the
/// skater's root, the root in engine space (meters from `origin`).
#[derive(Resource, Default)]
pub(crate) struct SkatePose {
    pub(crate) active: bool,
    pub(crate) origin: Vec3,
    pub(crate) root: Mat4,
    pub(crate) bones: Vec<Mat4>,
    pub(crate) names: Vec<String>,
}

impl SkatePose {
    /// A bone in engine space.
    pub(crate) fn bone(&self, name: &str) -> Option<Mat4> {
        let i = self.names.iter().position(|n| n == name)?;
        Some(self.root * *self.bones.get(i)?)
    }
}

struct CollisionInput {
    world: Vec<Tri>,
    props: Vec<Tri>,
    rails: Vec<Vec<[f32; 3]>>,
    revision: u64,
}
impl CollisionInput {
    fn bake(self) -> (Vec<Tri>, Vec<Vec<[f32; 3]>>) {
        let mut rails = find_rails(&self.world);
        rails.extend(self.rails);
        let props = crate::park::clean_skate_contacts(&self.world, self.props);
        let mut triangles = self.world;
        triangles.extend(props);
        (triangles, rails)
    }
}

enum Job {
    /// Start riding at `spawn` (engine space) on fresh collision; a new session the first time.
    Activate {
        epoch: u64,
        collision: CollisionInput,
        spawn: Vec3,
        heading: f32,
        aspect: f32,
        boosts: profession::Boosts,
        offboard: bool,
    },
    /// Collision gathered around the skater, built off the simulation and swapped in.
    Rebuild(u64, CollisionInput),
    Step {
        epoch: u64,
        aspect: f32,
        boosts: profession::Boosts,
    },
    Suspend,
}

enum Reply {
    Activated(u64, Frame),
    Pose(u64, Frame),
    Error(String),
}

/// One engine pose, out of the engine's own math types (its Bevy is not ours).
struct Frame {
    published_at: Instant,
    root: Mat4,
    bones: Vec<Mat4>,
    names: Vec<String>,
    camera: Option<(Vec3, Mat3)>,
    velocity: Vec3,
    tick: u64,
    state: String,
    /// Landed lines so far: count, summed points, the last one's trick.
    landed_lines: u32,
    landed_points: f64,
    landed_trick: String,
    /// The line in progress: running score and latest trick.
    line_points: f32,
    line_trick: String,
}

fn frame(p: skate_host::bridge::Pose) -> Frame {
    let m = |c: [f32; 16]| Mat4::from_cols_array(&c);
    Frame {
        published_at: Instant::now(),
        root: m(p.root.to_cols_array()),
        bones: p.bones.iter().map(|b| m(b.to_cols_array())).collect(),
        names: p.names,
        camera: p.camera.map(|(position, basis, _)| {
            (
                Vec3::from_array(position.to_array()),
                Mat3::from_cols_array(&basis.to_cols_array()),
            )
        }),
        velocity: Vec3::from_array(p.velocity.to_array()),
        tick: p.tick,
        state: p.state,
        landed_lines: p.landed_lines,
        landed_points: p.landed_points,
        landed_trick: p.landed_trick,
        line_points: p.line_points,
        line_trick: p.line_trick,
    }
}

#[derive(Resource, Default)]
struct Host {
    send: Option<work::Sender>,
    receive: Option<work::Replies>,
    input: Option<Arc<input::Sampler>>,
    epoch: u64,
    /// `J` was pressed and the worker is placing the skater.
    activating: bool,
    /// The session's engine-space origin, Bevy world.
    origin: Vec3,
    /// Where the collision was last gathered around, and when.
    centre: Vec3,
    gathered_at: f32,
    geometry_revision: u64,
    park_revision: u64,
    logged_tick: u64,
    /// The `WOW_SKATE_AUTOSTART` hook: when we became free to ride, and whether it has fired.
    ready_since: Option<f32>,
    autostarted: bool,
    /// Whether the last poll found a pad, so connects and losses are logged once.
    pad_seen: bool,
    /// The engine's landed-line count and points when last reported.
    landed: Option<(u32, f64)>,
    /// We cancelled Summon Skateboard and its aura has not gone yet: do not ride on it.
    dismissing: bool,
    /// Rocket Boost's aura was on us last frame.
    rocket_on: bool,
}

fn autostart_after() -> Option<f32> {
    static AFTER: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *AFTER.get_or_init(|| std::env::var("WOW_SKATE_AUTOSTART").ok()?.parse().ok())
}

/// Grind rails over engine-space triangles: [`rails::find`] runs in inches, z up.
pub(crate) fn find_rails(tris: &[Tri]) -> Vec<Vec<[f32; 3]>> {
    const INCH: f32 = 0.0254;
    let z_up: Vec<[Vec3; 3]> = tris
        .iter()
        .map(|t| t.map(|p| Vec3::new(p[0], -p[2], p[1]) / INCH))
        .collect();
    let (found, census) = rails::find(&z_up);
    info!(
        "skate rails: {} walkable edges, {} lips, {} runs, {} rails",
        census.candidates, census.lips, census.runs, census.rails
    );
    found
        .into_iter()
        .map(|rail| {
            rail.into_iter()
                .map(|v| [v.x * INCH, v.z * INCH, -v.y * INCH])
                .collect()
        })
        .collect()
}

fn spawn_worker(
    root: PathBuf,
) -> std::io::Result<(work::Sender, work::Replies, Arc<input::Sampler>)> {
    let (send, jobs) = work::channel();
    let (publish, replies) = work::replies();
    let input = Arc::new(input::Sampler::spawn()?);
    let worker_input = input.clone();
    std::thread::Builder::new()
        .name("skate".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            if let Err(e) = run_worker(&root, jobs, &publish, &worker_input) {
                publish.send(Reply::Error(e));
            }
        })?;
    Ok((send, replies, input))
}

/// Native simulation deadlines, independently of render-frame work and controller polling.
fn run_worker(
    root: &Path,
    jobs: work::Receiver,
    publish: &work::Publisher,
    input: &input::Sampler,
) -> Result<(), String> {
    let mut session: Option<Session> = None;
    let (built_send, built) = latest::channel::<(u64, u64, Result<PreparedCollision, String>)>();
    let mut built_send = Some(built_send);
    let mut builds: Option<latest::Sender<(u64, CollisionInput)>> = None;
    let mut collision_revision = 0;
    let mut epoch = 0;
    let mut active = false;
    let mut next_tick = Instant::now();
    let mut input_generation = 0;
    let mut boosts = profession::Boosts::default();
    let mut rocket_pending = false;
    let mut boost_state = profession::BoostState::default();
    let mut last_report = Instant::now();
    let mut max_age = Duration::ZERO;
    let mut max_tick = Duration::ZERO;
    let mut ticks = 0u64;
    let mut missed = 0u64;
    loop {
        let timeout = if active {
            next_tick.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(3600)
        };
        match jobs.recv_timeout(timeout) {
            work::Received::Closed => break,
            work::Received::Timeout => {}
            work::Received::Job(job) => match job {
                Job::Activate {
                    epoch: e,
                    collision,
                    spawn,
                    heading,
                    aspect,
                    boosts: initial_boosts,
                    offboard,
                } => {
                    epoch = e;
                    let started = Instant::now();
                    collision_revision = collision.revision;
                    let (triangles, rails) = collision.bake();
                    let s = match session.as_mut() {
                        Some(s) => {
                            let prepared = s.collision_builder().build(triangles, rails)?;
                            s.install_collision(prepared)?;
                            s
                        }
                        None => {
                            let mut s =
                                Session::new(root, triangles, rails, spawn.to_array(), heading)?;
                            s.set_respawn_in_place(true);
                            let builder = s.collision_builder();
                            let (tx, rx) = latest::channel::<(u64, CollisionInput)>();
                            let completed = built_send.take().expect("one collision builder");
                            std::thread::Builder::new()
                                .name("skate-collision".into())
                                .stack_size(32 * 1024 * 1024)
                                .spawn(move || {
                                    while let Some((e, collision)) = rx.recv() {
                                        let revision = collision.revision;
                                        let (tris, rails) = collision.bake();
                                        completed.send((e, revision, builder.build(tris, rails)));
                                    }
                                })
                                .map_err(|e| e.to_string())?;
                            builds = Some(tx);
                            session.insert(s)
                        }
                    };
                    s.set_aspect_ratio(aspect);
                    boosts = initial_boosts;
                    boosts.rocket = false;
                    rocket_pending = false;
                    boost_state = profession::BoostState::default();
                    profession::apply(s, &boosts, &mut boost_state);
                    s.set_camera_type(boosts.camera_preset.engine_type());
                    let p = if offboard {
                        s.activate_offboard(spawn.to_array(), heading)?
                    } else {
                        s.activate(spawn.to_array(), heading)?
                    };
                    input.reset();
                    active = true;
                    next_tick = Instant::now() + Duration::from_secs_f32(s.period());
                    info!(
                        "skate: on the board in {} ms",
                        started.elapsed().as_millis()
                    );
                    publish.send(Reply::Activated(epoch, frame(p)));
                }
                Job::Rebuild(e, collision) if e == epoch && active => {
                    collision_revision = collision.revision;
                    if let Some(tx) = &builds {
                        tx.send((e, collision));
                    }
                }
                Job::Rebuild(..) => {}
                Job::Suspend => {
                    active = false;
                    rocket_pending = false;
                    input.reset();
                    if let Some(s) = session.as_mut() {
                        s.suspend_input();
                    }
                }
                Job::Step {
                    epoch: e,
                    aspect,
                    boosts: settings,
                } if e == epoch && active => {
                    boosts = settings;
                    rocket_pending |= boosts.rocket;
                    boosts.rocket = false;
                    if let Some(s) = session.as_mut() {
                        s.set_aspect_ratio(aspect);
                    }
                }
                Job::Step { .. } => {}
            },
        }
        let Some(s) = session.as_mut().filter(|_| active) else {
            continue;
        };
        if let Some((built_epoch, revision, prepared)) = built.take() {
            match prepared {
                Ok(p) if built_epoch == epoch && revision == collision_revision => {
                    let started = Instant::now();
                    s.install_collision(p)?;
                    let elapsed = started.elapsed();
                    if elapsed > Duration::from_millis(25) {
                        warn!(
                            "skate input: collision installation took {} ms",
                            elapsed.as_millis()
                        );
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("skate collision: {e}"),
            }
        }
        let now = Instant::now();
        if now < next_tick {
            continue;
        }
        let period = Duration::from_secs_f32(s.period());
        if now.saturating_duration_since(next_tick) > period * 2 {
            missed += (now.saturating_duration_since(next_tick).as_secs_f64()
                / period.as_secs_f64()) as u64;
            next_tick = now;
        }
        let (sample, generation, discarded) = input.take(now);
        if generation != input_generation {
            s.suspend_input();
            input_generation = generation;
        }
        let input_frame = sample.map_or_else(InputFrame::neutral, |sample| {
            max_age = max_age.max(now.saturating_duration_since(sample.at));
            sample.value
        });
        s.collect(input_frame, period.as_secs_f32());
        s.set_camera_type(boosts.camera_preset.engine_type());
        if rocket_pending {
            profession::rocket(s);
            rocket_pending = false;
        }
        profession::apply(s, &boosts, &mut boost_state);
        let started = Instant::now();
        s.advance()?;
        profession::apply(s, &boosts, &mut boost_state);
        max_tick = max_tick.max(started.elapsed());
        ticks += 1;
        next_tick += Duration::from_secs_f32(s.period());
        let p = s.pose();
        if !p.root.is_finite() || p.bones.iter().any(|b| !b.is_finite()) {
            return Err("the engine published a non-finite pose".into());
        }
        publish.send(Reply::Pose(epoch, frame(p)));
        if last_report.elapsed() >= Duration::from_secs(10) {
            info!(
                "skate input: ticks={} max_sample_age_ms={:.2} max_tick_ms={:.2} missed_deadlines={} discarded_samples={}",
                ticks,
                max_age.as_secs_f64() * 1000.0,
                max_tick.as_secs_f64() * 1000.0,
                missed,
                discarded
            );
            max_age = Duration::ZERO;
            max_tick = Duration::ZERO;
            ticks = 0;
            missed = 0;
            last_report = Instant::now();
        }
    }
    Ok(())
}

/// Every collision triangle in the box around `centre`, engine space; zero-area faces are left
/// out, as their NaN normals poison the board's physics.
fn gather(
    collide: &WorldCollision,
    centre: Vec3,
    origin: Vec3,
    scene: &crate::park::ParkScene,
) -> Vec<Tri> {
    collide
        .faces_near_body(centre, GATHER_HALF, usize::MAX)
        .into_iter()
        .filter(|face| !scene.entities.contains(&face.entity))
        .filter_map(|face| {
            let t = face.verts().map(|v| to_skate(v, origin));
            let solid = t.iter().all(|v| v.is_finite())
                && (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-6;
            solid.then(|| t.map(|v| v.to_array()))
        })
        .collect()
}

fn stop(host: &mut Host, drive: &mut SkateDrive, pose: &mut SkatePose) {
    host.epoch = host.epoch.wrapping_add(1);
    host.activating = false;
    if let Some(send) = &host.send {
        let _ = send.send(Job::Suspend);
    }
    if drive.active {
        info!("skate: off the board");
    }
    drive.active = false;
    drive.camera = None;
    pose.active = false;
}

#[allow(clippy::too_many_arguments)]
fn leave_world(
    In(event): In<benilla_protocol::SessionEvent>,
    self_guid: Res<crate::net::SelfGuid>,
    mut host: ResMut<Host>,
    mut drive: ResMut<SkateDrive>,
    mut pose: ResMut<SkatePose>,
    mut audio: ResMut<sound::SkateAudio>,
    mut score: ResMut<score::SkateScore>,
    mut boards: Query<&mut Visibility, With<board::BoardPart>>,
) {
    let leaving = match event {
        benilla_protocol::SessionEvent::Disconnected { .. }
        | benilla_protocol::SessionEvent::LoggedOut
        | benilla_protocol::SessionEvent::TransferPending { .. }
        | benilla_protocol::SessionEvent::Worldport { .. } => true,
        benilla_protocol::SessionEvent::Teleport { guid, .. } => Some(guid) == self_guid.0,
        _ => false,
    };
    if !leaving {
        return;
    }
    stop(&mut host, &mut drive, &mut pose);
    host.landed = None;
    host.rocket_on = false;
    host.ready_since = None;
    audio.reset();
    score.clear_live();
    for mut visibility in &mut boards {
        *visibility = Visibility::Hidden;
    }
}

fn present(
    frame: Frame,
    origin: Vec3,
    drive: &mut SkateDrive,
    pose: &mut SkatePose,
    camera_distance: f32,
) {
    drive.pos = from_skate(frame.root.w_axis.truncate(), origin);
    #[cfg(feature = "dev")]
    {
        drive.offboard = frame.state.starts_with("Biped");
    }
    let flat = Vec3::new(frame.velocity.x, 0.0, frame.velocity.z);
    let facing = frame.root.z_axis.truncate();
    if facing.x * facing.x + facing.z * facing.z > 1e-6 {
        drive.facing = f32::atan2(-facing.x, -facing.z);
    }
    drive.speed = flat.length() / METERS_PER_YARD;
    let heading = if drive.moving() {
        flat
    } else {
        let z = frame.root.z_axis.truncate();
        Vec3::new(z.x, 0.0, z.z)
    };
    if heading.length_squared() > 1e-6 {
        // Bevy yaw `θ` faces `(−sin θ, 0, −cos θ)`.
        drive.yaw = f32::atan2(-heading.x, -heading.z);
    }
    // Publish the native eye unchanged. The final camera pass applies distance and collision
    // together; scaling a checked eye around animated hips introduces clipping and push wobble.
    let hips = frame
        .names
        .iter()
        .position(|n| n == "HIPS")
        .and_then(|i| frame.bones.get(i))
        .map(|b| frame.root.transform_point3(b.w_axis.truncate()));
    drive.camera = frame.camera.and_then(|(position, basis)| {
        let at = from_skate(position, origin);
        let look = basis.z_axis.try_normalize()?;
        let up = basis.y_axis.try_normalize()?;
        Some(Transform::from_translation(at).looking_to(look, up))
    });
    drive.camera_distance = camera_distance;
    drive.camera_height = hips.map_or(1., |h| {
        (h - frame.root.w_axis.truncate()).y / METERS_PER_YARD
    });
    pose.origin = origin;
    // The engine leaves the root's last row unset (its teleport writes `w = 0`).
    pose.root = Mat4::from_cols(
        frame.root.x_axis,
        frame.root.y_axis,
        frame.root.z_axis,
        frame.root.w_axis.truncate().extend(1.0),
    );
    pose.bones = frame.bones;
    pose.names = frame.names;
}

/// Decodes the animation banks while the world loads, so the first `J` only places the skater.
fn preload() {
    let Some(root) = assets_root() else {
        info!("skate: no skate-data/assets (set WOW_SKATE_ASSETS); J is off");
        return;
    };
    let _ = std::thread::Builder::new()
        .name("skate-preload".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            if let Err(e) = export::ensure(&root) {
                warn!("skate: cannot derive rig.json/board.json: {e}");
            }
            let start = std::time::Instant::now();
            match Session::preload(&root) {
                Ok(()) => info!(
                    "skate: animation banks preloaded in {} ms",
                    start.elapsed().as_millis()
                ),
                Err(e) => warn!("skate preload: {e}"),
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    ui: Res<crate::ui_script::UiKeyboardCapture>,
    player: Res<crate::player::Player>,
    collide: WorldCollision,
    windows: Query<&Window, With<PrimaryWindow>>,
    body: Query<Entity, With<crate::net::Embodied>>,
    mut sheath: MessageWriter<crate::creature_anim::SheathRequest>,
    mut host: ResMut<Host>,
    mut drive: ResMut<SkateDrive>,
    mut pose: ResMut<SkatePose>,
    mut audio: ResMut<sound::SkateAudio>,
    profession: profession::Profession,
    mut score: ResMut<score::SkateScore>,
    park: (
        Res<crate::park::ParkEditor>,
        Res<crate::park::ParkWorld>,
        Res<crate::park::ParkScene>,
    ),
    mut flags: ResMut<flag::State>,
) {
    if park.0.active {
        if drive.active || host.activating {
            stop(&mut host, &mut drive, &mut pose);
            audio.reset();
        }
        return;
    }
    let aspect = windows
        .single()
        .map(|w| w.width() / w.height().max(1.0))
        .unwrap_or(16.0 / 9.0);
    // On a Skatecraft server the Summon Skateboard aura is the ride.
    let server_board = profession.server_board();
    let auras = profession.auras();
    let riding = drive.active || host.activating;
    if riding && !player.may_skate() {
        let returning = flags.returning.is_some();
        if returning {
            info!("skate flag: native activation held until destination is ready");
        }
        stop(&mut host, &mut drive, &mut pose);
        // A flag's loading hold is part of its return, so keep the mode alive for the retry.
        if server_board && auras.board && !returning {
            profession.dismiss();
            host.dismissing = true;
        }
    }
    if keys.just_pressed(KeyCode::KeyK) && !ui.typing && drive.active {
        drive.engine_camera = !drive.engine_camera;
    }
    // Test hook: `WOW_SKATE_AUTOSTART=<seconds>` presses `J` once, that long after we may ride.
    let autostart = autostart_after().is_some_and(|after| {
        if !player.may_skate() || host.autostarted {
            host.ready_since = None;
            return false;
        }
        let since = *host.ready_since.get_or_insert(time.elapsed_secs());
        let due = time.elapsed_secs() - since >= after;
        host.autostarted |= due;
        due
    });
    if (keys.just_pressed(KeyCode::KeyJ) && !ui.typing) || autostart {
        if server_board {
            if drive.active || host.activating || auras.board {
                profession.dismiss();
                host.dismissing = true;
                stop(&mut host, &mut drive, &mut pose);
            } else if player.may_skate() {
                profession.summon();
            }
        } else if drive.active || host.activating {
            stop(&mut host, &mut drive, &mut pose);
        } else if player.may_skate() && flags.may_resume(player.pos) {
            start(
                &time,
                &collide,
                &player,
                aspect,
                profession::Boosts::from_auras(
                    auras,
                    profession.speed(),
                    profession.camera_preset(),
                ),
                (&park.1, &park.2),
                &mut host,
                flags.offboard(),
            );
        }
    }
    if server_board {
        if !auras.board {
            host.dismissing = false;
            if drive.active || host.activating {
                stop(&mut host, &mut drive, &mut pose);
            }
        } else if !host.dismissing
            && !drive.active
            && !host.activating
            && player.may_skate()
            && flags.may_resume(player.pos)
        {
            start(
                &time,
                &collide,
                &player,
                aspect,
                profession::Boosts::from_auras(
                    auras,
                    profession.speed(),
                    profession.camera_preset(),
                ),
                (&park.1, &park.2),
                &mut host,
                flags.offboard(),
            );
        }
    }
    let rocket = auras.rocket && !host.rocket_on;
    host.rocket_on = auras.rocket;

    let replies = host
        .receive
        .as_ref()
        .map_or_else(Vec::new, work::Replies::take);
    for reply in replies {
        match reply {
            Reply::Activated(epoch, f) if epoch == host.epoch && host.activating => {
                if flags.offboard() {
                    info!("skate flag: returned holding board state={}", f.state);
                    flags.resumed();
                }
                host.activating = false;
                drive.active = true;
                // `WOW_SKATE_CAM=engine` starts on the engine's camera.
                drive.engine_camera |= std::env::var("WOW_SKATE_CAM").is_ok_and(|v| v == "engine");
                // Weapons stow on the back: in hand they follow the skater's wrists, whose roll
                // swings a sword or shield about.
                if let Ok(entity) = body.single() {
                    sheath.write(crate::creature_anim::SheathRequest {
                        entity,
                        state: 0,
                        ceremony: false,
                    });
                }
                pose.active = true;
                host.landed = Some((f.landed_lines, f.landed_points));
                audio.reset();
                audio.observe(&f.state, f.velocity);
                present(
                    f,
                    host.origin,
                    &mut drive,
                    &mut pose,
                    profession.camera_distance(),
                );
            }
            Reply::Pose(epoch, f) if epoch == host.epoch && drive.active => {
                if f.tick / 120 != host.logged_tick / 120 {
                    info!(
                        "skate tick={} speed={:.2} yd/s state={} pose_age_ms={:.2}",
                        f.tick,
                        f.velocity.length() / METERS_PER_YARD,
                        f.state,
                        f.published_at.elapsed().as_secs_f64() * 1000.0
                    );
                    host.logged_tick = f.tick;
                    if std::env::var_os("WOW_SKATE_DEBUG").is_some() {
                        let at = |n: &str| {
                            f.names
                                .iter()
                                .position(|x| x == n)
                                .map(|i| f.bones[i].w_axis.truncate())
                        };
                        info!(
                            "skate debug: root {:?} hips {:?} lfoot {:?} rfoot {:?} board {:?} tf {:?} tb {:?} wfl {:?} wbr {:?} ltoe_rep {:?}",
                            f.root.w_axis.truncate(),
                            at("HIPS"),
                            at("LEFTFOOT"),
                            at("RIGHTFOOT"),
                            at("SKATEBOARD_ROOT"),
                            at("TRUCK_FRONT"),
                            at("TRUCK_BACK"),
                            at("LEFT_WHEELFRONT"),
                            at("RIGHT_WHEELBACK"),
                            at("LEFTTOEBASE_REPARENTED"),
                        );
                    }
                }
                let mut landed = None;
                if let Some((lines, points)) = host.landed {
                    if f.landed_lines != lines {
                        landed = Some((f.landed_points - points) as f32);
                        host.landed = Some((f.landed_lines, f.landed_points));
                    }
                }
                let banked =
                    score.observe(time.elapsed_secs(), f.line_points, &f.line_trick, landed);
                if let Some(line) = banked.filter(|_| server_board) {
                    profession.landed(f64::from(line), &score::trick_label(&f.landed_trick));
                }
                audio.observe(&f.state, f.velocity);
                present(
                    f,
                    host.origin,
                    &mut drive,
                    &mut pose,
                    profession.camera_distance(),
                );
            }
            Reply::Error(e) => {
                warn!("skate stopped: {e}");
                stop(&mut host, &mut drive, &mut pose);
                host.send = None;
                host.receive = None;
                host.input = None;
                return;
            }
            _ => {}
        }
    }
    if !drive.active {
        score.clear_live();
        return;
    }

    let now = time.elapsed_secs();
    let rolled = drive.pos - host.centre;
    if Vec3::new(rolled.x, 0.0, rolled.z).length() > RECENTRE
        || rolled.y.abs() > GATHER_HALF.y * 0.5
        || now - host.gathered_at > REFRESH_SECS
        || host.park_revision != park.1.generation
    {
        host.geometry_revision = host.geometry_revision.wrapping_add(1);
        host.park_revision = park.1.generation;
        let triangles = gather(&collide, drive.pos, host.origin, &park.2);
        let (props, rails) =
            crate::park::authored_collision(&park.1, drive.pos, GATHER_HALF, host.origin);
        let collision = CollisionInput {
            world: triangles,
            props,
            rails,
            revision: host.geometry_revision,
        };
        host.centre = drive.pos;
        host.gathered_at = now;
        if let Some(send) = &host.send {
            let _ = send.send(Job::Rebuild(host.epoch, collision));
        }
    }
    let (pad, edges) = host
        .input
        .as_ref()
        .map_or((false, Vec::new()), |input| input.edges(Instant::now()));
    if server_board {
        for edge in edges {
            if edge.held & profession::PAD_LB == 0 {
                profession.dpad(edge.pressed);
            }
        }
    }
    if pad != host.pad_seen {
        host.pad_seen = pad;
        match pad {
            true => info!("skate: controller connected"),
            false => warn!("skate: no controller found; the board needs an Xbox-style pad"),
        }
    }
    if let Some(send) = &host.send {
        let job = Job::Step {
            epoch: host.epoch,
            aspect,
            boosts: profession::Boosts {
                rocket,
                ..profession::Boosts::from_auras(
                    auras,
                    profession.speed(),
                    profession.camera_preset(),
                )
            },
        };
        if send.send(job).is_err() {
            stop(&mut host, &mut drive, &mut pose);
        }
    }
}

fn start(
    time: &Time,
    collide: &WorldCollision,
    player: &crate::player::Player,
    aspect: f32,
    boosts: profession::Boosts,
    park: (&crate::park::ParkWorld, &crate::park::ParkScene),
    host: &mut Host,
    offboard: bool,
) {
    if host.send.is_none() {
        let Some(root) = assets_root() else {
            warn!("skate: no skate-data/assets found (set WOW_SKATE_ASSETS)");
            return;
        };
        match spawn_worker(root) {
            Ok((send, receive, input)) => {
                host.send = Some(send);
                host.receive = Some(receive);
                host.input = Some(input);
            }
            Err(e) => {
                warn!("skate worker: {e}");
                return;
            }
        }
    }
    host.epoch = host.epoch.wrapping_add(1);
    host.activating = true;
    host.origin = player.pos;
    host.centre = player.pos;
    host.gathered_at = time.elapsed_secs();
    host.geometry_revision = host.geometry_revision.wrapping_add(1);
    host.park_revision = park.0.generation;
    let triangles = gather(collide, player.pos, host.origin, park.1);
    let (props, rails) =
        crate::park::authored_collision(park.0, player.pos, GATHER_HALF, host.origin);
    info!(
        "skate: {} collision triangles around {:?}",
        triangles.len(),
        player.pos
    );
    // The engine's heading `h` faces `(sin h, 0, cos h)`; our yaw `θ` faces `(−sin θ, 0, −cos θ)`.
    let heading = player.face_yaw() + std::f32::consts::PI;
    if let Some(send) = &host.send {
        let _ = send.send(Job::Activate {
            epoch: host.epoch,
            collision: CollisionInput {
                world: triangles,
                props,
                rails,
                revision: host.geometry_revision,
            },
            spawn: to_skate(player.pos + Vec3::Y * 0.1, host.origin),
            heading,
            aspect,
            boosts,
            offboard,
        });
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SkateInputSet;

pub(crate) fn plugin(app: &mut App) {
    flag::plugin(app);
    app.init_resource::<Host>()
        .init_resource::<SkateDrive>()
        .init_resource::<SkatePose>()
        .init_resource::<board::Board>()
        .init_resource::<sound::SkateAudio>()
        .init_resource::<score::SkateScore>()
        .init_resource::<profession::SpeedSetting>()
        .init_resource::<profession::CameraSetting>()
        .net_handler(
            benilla_protocol::SessionEventKind::Chat,
            profession::on_setting,
        )
        .net_handler(
            benilla_protocol::SessionEventKind::Disconnected,
            profession::reset_settings,
        )
        .net_handler(
            benilla_protocol::SessionEventKind::Disconnected,
            leave_world,
        )
        .net_handler(benilla_protocol::SessionEventKind::LoggedOut, leave_world)
        .net_handler(
            benilla_protocol::SessionEventKind::TransferPending,
            leave_world,
        )
        .net_handler(benilla_protocol::SessionEventKind::Worldport, leave_world)
        .net_handler(benilla_protocol::SessionEventKind::Teleport, leave_world)
        .net_handler(
            benilla_protocol::SessionEventKind::XpGain,
            score::on_xp_gain,
        )
        .add_systems(Startup, (preload, score::spawn))
        .add_systems(
            Update,
            (update, board::update, sound::update, score::draw)
                .chain()
                .in_set(SkateInputSet)
                .in_set(WorldStage::Input)
                .before(crate::player::PlayerControlSet)
                .in_set(crate::char_select::InWorldGated),
        )
        .add_systems(
            PostUpdate,
            // The last writer of our pose: after the body twist, which would turn the retargeted
            // spine and head again, the global sequences and the terrain conform.
            rig::retarget
                .in_set(benilla_world::rig_anim::PosePost)
                .after(crate::creature_anim::apply_body_twist)
                .after(benilla_world::rig_anim::apply_global_sequences)
                .after(crate::entities::conform_units),
        );
}
