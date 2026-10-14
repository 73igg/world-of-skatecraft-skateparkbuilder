//! Render export advances only after GPU capture; encoding stays off the input/render threads.
use super::Replay;
use bevy::camera::{visibility::RenderLayers, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

const FPS: f64 = 60.;
struct Pixels {
    width: u32,
    height: u32,
    data: Vec<u8>,
}
enum Work {
    Frame(Pixels, f64),
    Finish,
}
#[derive(Resource)]
struct Captures {
    tx: crossbeam_channel::Sender<(u64, Result<Pixels, String>)>,
    rx: crossbeam_channel::Receiver<(u64, Result<Pixels, String>)>,
}
impl Default for Captures {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(2);
        Self { tx, rx }
    }
}
#[derive(Component)]
struct ExportCamera;
#[derive(Component)]
struct Pending(u64);

pub(super) struct Session {
    id: u64,
    tx: crossbeam_channel::Sender<Work>,
    rx: crossbeam_channel::Receiver<Result<PathBuf, String>>,
    cancelled: Arc<AtomicBool>,
    awaiting: bool,
    finishing: bool,
    frames: u32,
    target: Option<Handle<Image>>,
    camera: Option<Entity>,
    pending: Option<Pixels>,
    progress: std::time::Instant,
    finish_sent: bool,
    dimensions: Option<UVec2>,
    process: Arc<Mutex<Option<std::process::Child>>>,
}
impl Session {
    pub(super) fn frame_count(&self) -> u32 {
        self.frames
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Ok(mut process) = self.process.try_lock() {
            if let Some(child) = process.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

fn executable() -> PathBuf {
    if let Some(path) = std::env::var_os("SKATECRAFT_FFMPEG") {
        return path.into();
    }
    if let Some(root) = benilla_formats::project_folder() {
        let file = root.join(if cfg!(windows) {
            ".setup/ffmpeg/bin/ffmpeg.exe"
        } else {
            ".setup/ffmpeg/bin/ffmpeg"
        });
        if file.is_file() {
            return file;
        }
    }
    "ffmpeg".into()
}
fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

pub(super) fn begin(replay: &mut Replay) {
    if replay.exporting.is_some() {
        return;
    }
    let exe = executable();
    let mut check = Command::new(&exe);
    check
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hidden(&mut check);
    if !check.status().is_ok_and(|s| s.success()) {
        replay.status="Video export needs FFmpeg. Install it or set SKATECRAFT_FFMPEG; Save clip works without it.".into();
        return;
    }
    let Some(dir) = crate::local_state::replays_dir() else {
        replay.status = "Replay storage is unavailable".into();
        return;
    };
    if let Err(error) = std::fs::create_dir_all(&dir) {
        replay.status = error.to_string();
        return;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;
    let path = dir.join(format!("Skate-{id}.mp4"));
    let (tx, rx) = crossbeam_channel::bounded(2);
    let (done, result) = crossbeam_channel::bounded(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = cancelled.clone();
    let clip = replay.clip();
    let process = Arc::new(Mutex::new(None));
    let child_process = process.clone();
    std::thread::spawn(move || {
        let result = encode(exe, path, rx, stop, clip.frames, child_process);
        let _ = done.send(result);
    });
    replay.cursor = replay.in_point;
    replay.playing = false;
    replay.exporting = Some(Session {
        id,
        tx,
        rx: result,
        cancelled,
        awaiting: false,
        finishing: false,
        frames: 0,
        target: None,
        camera: None,
        pending: None,
        progress: std::time::Instant::now(),
        finish_sent: false,
        dimensions: None,
        process,
    });
    replay.status = "Rendering video… Escape cancels.".into();
}

fn encode(
    exe: PathBuf,
    path: PathBuf,
    rx: crossbeam_channel::Receiver<Work>,
    cancelled: Arc<AtomicBool>,
    frames: Vec<Arc<super::timeline::Sample>>,
    process: Arc<Mutex<Option<std::process::Child>>>,
) -> Result<PathBuf, String> {
    let (first, first_cursor) = match rx.recv() {
        Ok(Work::Frame(frame, cursor)) => (frame, cursor),
        _ => return Err("Export cancelled".into()),
    };
    let temporary =
        path.with_file_name(format!(".{}", path.file_name().unwrap().to_string_lossy()));
    let audio_path = path.with_extension("temporary.wav");
    let muxed = path.with_file_name(format!(
        ".mux-{}",
        path.file_name().unwrap().to_string_lossy()
    ));
    let _cleanup = Cleanup(vec![temporary.clone(), audio_path.clone(), muxed.clone()]);
    let mut audio = super::audio::Audio::new(&audio_path, frames)?;
    let log =
        std::fs::File::create(path.with_extension("export.log")).map_err(|e| e.to_string())?;
    let mut command = Command::new(&exe);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgb24",
            "-video_size",
        ])
        .arg(format!("{}x{}", first.width, first.height))
        .args([
            "-framerate",
            "60",
            "-i",
            "pipe:0",
            "-an",
            "-vf",
            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-crf",
            "18",
            "-threads",
            "2",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ])
        .arg(&temporary)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(log);
    hidden(&mut command);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut pipe = child
        .stdin
        .take()
        .ok_or("Video encoder has no input pipe")?;
    *process.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
    let result = (|| {
        pipe.write_all(&first.data).map_err(|e| e.to_string())?;
        audio.block(first_cursor)?;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err("Export cancelled".into());
            }
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(Work::Frame(frame, cursor)) => {
                    if frame.width != first.width || frame.height != first.height {
                        return Err(
                            "Window size changed during export; keep its size fixed and retry"
                                .into(),
                        );
                    }
                    pipe.write_all(&frame.data).map_err(|e| e.to_string())?;
                    audio.block(cursor)?;
                }
                Ok(Work::Finish) => return Ok(()),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("Export cancelled".into()),
            }
        }
    })();
    drop(pipe);
    if result.is_err() {
        if let Some(child) = process.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
            let _ = child.kill();
        }
    }
    let status = wait_owned(&process, &cancelled)?;
    if result.is_err() || !status.success() {
        let _ = std::fs::remove_file(&temporary);
        return result.and(Err(
            "Video encoder failed; see the export log in Replays".into()
        ));
    }
    audio.finish()?;
    let log = std::fs::OpenOptions::new()
        .append(true)
        .open(path.with_extension("export.log"))
        .map_err(|e| e.to_string())?;
    let mut mux = Command::new(exe);
    mux.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&temporary)
        .arg("-i")
        .arg(&audio_path)
        .args([
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-shortest",
            "-movflags",
            "+faststart",
        ])
        .arg(&muxed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    hidden(&mut mux);
    *process.lock().unwrap_or_else(|p| p.into_inner()) =
        Some(mux.spawn().map_err(|e| e.to_string())?);
    if !wait_owned(&process, &cancelled)?.success() {
        return Err("Audio/video encoding failed; see the export log".into());
    }
    std::fs::rename(&muxed, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

fn wait_owned(
    process: &Mutex<Option<std::process::Child>>,
    cancelled: &AtomicBool,
) -> Result<std::process::ExitStatus, String> {
    loop {
        {
            let mut process = process.lock().unwrap_or_else(|p| p.into_inner());
            let child = process.as_mut().ok_or("Video encoder process is missing")?;
            if cancelled.load(Ordering::Relaxed) {
                let _ = child.kill();
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                process.take();
                return if cancelled.load(Ordering::Relaxed) {
                    Err("Export cancelled".into())
                } else {
                    Ok(status)
                };
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

struct Cleanup(Vec<PathBuf>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn captured(event: On<ScreenshotCaptured>, tags: Query<&Pending>, captures: Res<Captures>) {
    let Ok(tag) = tags.get(event.entity) else {
        return;
    };
    let result = event
        .image
        .clone()
        .try_into_dynamic()
        .map(|image| {
            let rgb = image.to_rgb8();
            Pixels {
                width: rgb.width(),
                height: rgb.height(),
                data: rgb.into_raw(),
            }
        })
        .map_err(|e| e.to_string());
    let _ = captures.tx.try_send((tag.0, result));
}

#[allow(clippy::type_complexity)]
fn pump(
    mut commands: Commands,
    mut replay: ResMut<Replay>,
    captures: Res<Captures>,
    scene: Res<crate::park::ReplaySceneReady>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    source: Query<
        (&Transform, &Projection, Option<&RenderLayers>),
        With<benilla_world::view::WorldCamera>,
    >,
    mut cameras: Query<
        (Entity, &mut Transform, &mut Projection),
        (
            With<ExportCamera>,
            Without<benilla_world::view::WorldCamera>,
        ),
    >,
) {
    if replay.exporting.is_none() {
        for (entity, _, _) in &cameras {
            commands.entity(entity).despawn();
        }
        while captures.rx.try_recv().is_ok() {}
        return;
    }
    let cursor = replay.cursor;
    let end = replay.out_point;
    let rate = replay.playback_rate();
    let Some(session) = replay.exporting.as_mut() else {
        return;
    };
    if let Ok(result) = session.rx.try_recv() {
        replay.exporting = None;
        replay.status = match result {
            Ok(path) => format!("Exported {}", path.file_name().unwrap().to_string_lossy()),
            Err(error) => error,
        };
        return;
    }
    if let (Some(dimensions), Ok(window)) = (session.dimensions, windows.single()) {
        if UVec2::new(window.physical_width(), window.physical_height()) != dimensions {
            replay.exporting = None;
            replay.status =
                "Window size changed during export; retry with a fixed window size".into();
            return;
        }
    }
    if session.progress.elapsed().as_secs() > if session.finishing { 120 } else { 30 } {
        replay.exporting = None;
        replay.status = "Export timed out; the incomplete output was cancelled".into();
        return;
    }
    if let Some(error) = &scene.error {
        replay.exporting = None;
        replay.status = error.clone();
        return;
    }
    if scene.loading {
        return;
    }
    if session.target.is_none() {
        let (Ok(window), Ok((transform, projection, layers))) = (windows.single(), source.single())
        else {
            return;
        };
        if window.physical_width() == 0 || window.physical_height() == 0 {
            return;
        }
        session.dimensions = Some(UVec2::new(
            window.physical_width(),
            window.physical_height(),
        ));
        let mut image = Image::new_target_texture(
            window.physical_width(),
            window.physical_height(),
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let target = images.add(image);
        let entity = commands
            .spawn((
                ExportCamera,
                Camera3d::default(),
                Camera {
                    order: -2,
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                *transform,
                projection.clone(),
                layers.cloned().unwrap_or_default(),
                bevy::render::view::Msaa::Off,
                bevy::render::view::Hdr,
                bevy::core_pipeline::tonemapping::Tonemapping::None,
                benilla_world::ffx_glow::FfxGlow::WORLD,
            ))
            .id();
        session.target = Some(target);
        session.camera = Some(entity);
        return;
    }
    if let (Ok((transform, projection, _)), Some(entity)) = (source.single(), session.camera) {
        if let Ok((_, mut camera, mut target_projection)) = cameras.get_mut(entity) {
            *camera = *transform;
            *target_projection = projection.clone();
        }
    }
    while let Ok((id, result)) = captures.rx.try_recv() {
        if id == session.id {
            session.awaiting = false;
            session.progress = std::time::Instant::now();
            match result {
                Ok(pixels) => session.pending = Some(pixels),
                Err(error) => {
                    replay.exporting = None;
                    replay.status = error;
                    return;
                }
            }
        }
    }
    if let Some(pixels) = session.pending.take() {
        match session.tx.try_send(Work::Frame(pixels, cursor)) {
            Ok(()) => {
                session.frames += 1;
                session.progress = std::time::Instant::now();
                if cursor + f64::from(rate) / FPS >= end - 1e-7 {
                    session.finishing = true;
                    session.progress = std::time::Instant::now();
                } else {
                    replay.cursor = (cursor + f64::from(rate) / FPS).min(end);
                    // The next Update seats every visual at this time together. Never change a
                    // pose after the actor has drawn: the board's post-pass would render a new frame.
                }
                return;
            }
            Err(crossbeam_channel::TrySendError::Full(Work::Frame(pixels, _))) => {
                session.pending = Some(pixels);
                return;
            }
            Err(_) => {
                replay.exporting = None;
                replay.status = "The video encoder stopped".into();
                return;
            }
        }
    }
    if session.finishing && !session.finish_sent {
        match session.tx.try_send(Work::Finish) {
            Ok(()) => session.finish_sent = true,
            Err(crossbeam_channel::TrySendError::Full(_)) => {}
            Err(_) => {
                replay.exporting = None;
                replay.status = "Video encoder stopped".into();
                return;
            }
        }
    }
    if !session.awaiting && !session.finishing {
        session.awaiting = true;
        session.progress = std::time::Instant::now();
        commands
            .spawn((
                Screenshot::image(session.target.as_ref().unwrap().clone()),
                Pending(session.id),
            ))
            .observe(captured);
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Captures>().add_systems(
        PostUpdate,
        pump.before(bevy::transform::TransformSystems::Propagate),
    );
}
