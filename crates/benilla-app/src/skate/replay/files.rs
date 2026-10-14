//! Versioned local replay data; no meshes, textures, audio, accounts or install paths are embedded.
use super::{
    camera::Keyframe,
    timeline::{Sample, Scene, Visual, MAX_FRAMES, WINDOW},
};
use benilla_protocol::park::Object;
use bevy::prelude::*;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const VERSION: u32 = 1;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_DECODED: u64 = 256 * 1024 * 1024;
pub(super) const EXTENSION: &str = "skate-replay";

#[derive(Serialize, Deserialize)]
struct SavedScene {
    map: u32,
    instance: u32,
    origin: [f32; 3],
    park: Vec<Object>,
    signature: Vec<u32>,
    actor_scale: [f32; 3],
    actor_height: f32,
}
#[derive(Serialize, Deserialize)]
struct SavedFrame {
    at: f64,
    scene: usize,
    cut: bool,
    root: [f32; 16],
    bones: Vec<[f32; 16]>,
    camera: Option<([f32; 3], [f32; 9])>,
    velocity: [f32; 3],
    tick: u64,
    state: String,
    line_points: f32,
    line_trick: String,
}
#[derive(Serialize, Deserialize)]
struct SavedVisual {
    at: f64,
    transform: benilla_protocol::park::Pose,
    locals: Vec<benilla_protocol::park::Pose>,
    cut: bool,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    names: Vec<String>,
    scenes: Vec<SavedScene>,
    frames: Vec<SavedFrame>,
    keys: Vec<Keyframe>,
    cameras: Vec<Keyframe>,
    visuals: Vec<SavedVisual>,
    in_point: f64,
    out_point: f64,
    rate: f32,
    camera_settings: super::camera::Settings,
}

pub(super) struct Clip {
    pub frames: Vec<Arc<Sample>>,
    pub keys: Vec<Keyframe>,
    pub cameras: Vec<Keyframe>,
    pub visuals: Vec<Arc<Visual>>,
    pub in_point: f64,
    pub out_point: f64,
    pub rate: f32,
    pub camera_settings: super::camera::Settings,
}

pub(super) fn list() -> Vec<PathBuf> {
    let Some(dir) = crate::local_state::replays_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXTENSION))
        .collect();
    paths.sort();
    paths.reverse();
    paths.truncate(100);
    paths
}

pub(super) fn save(path: &Path, clip: &Clip) -> Result<(), String> {
    let mut indices = HashMap::new();
    let mut scenes = Vec::new();
    let mut frames = Vec::with_capacity(clip.frames.len());
    let names = clip
        .frames
        .first()
        .ok_or("The replay is empty")?
        .frame
        .names
        .to_vec();
    for sample in &clip.frames {
        let pointer = Arc::as_ptr(&sample.scene) as usize;
        let scene = *indices.entry(pointer).or_insert_with(|| {
            let index = scenes.len();
            let s = &sample.scene;
            scenes.push(SavedScene {
                map: s.map,
                instance: s.instance,
                origin: s.origin.to_array(),
                park: s.park.to_vec(),
                signature: s.signature.clone(),
                actor_scale: s.actor_scale.to_array(),
                actor_height: s.actor_height,
            });
            index
        });
        let f = &sample.frame;
        frames.push(SavedFrame {
            at: sample.at,
            scene,
            cut: sample.cut,
            root: f.root.to_cols_array(),
            bones: f.bones.iter().map(|b| b.to_cols_array()).collect(),
            camera: f.camera.map(|(p, b)| (p.to_array(), b.to_cols_array())),
            velocity: f.velocity.to_array(),
            tick: f.tick,
            state: f.state.clone(),
            line_points: f.line_points,
            line_trick: f.line_trick.clone(),
        });
    }
    let saved = Saved {
        version: VERSION,
        names,
        scenes,
        frames,
        keys: clip.keys.clone(),
        cameras: clip.cameras.clone(),
        visuals: clip
            .visuals
            .iter()
            .map(|v| SavedVisual {
                at: v.at,
                transform: saved_transform(v.transform),
                locals: v.locals.iter().copied().map(saved_transform).collect(),
                cut: v.cut,
            })
            .collect(),
        in_point: clip.in_point,
        out_point: clip.out_point,
        rate: clip.rate,
        camera_settings: clip.camera_settings.clone(),
    };
    validate(&saved)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    serde_json::to_writer(
        Limited {
            writer: &mut encoder,
            remaining: MAX_DECODED,
        },
        &saved,
    )
    .map_err(|e| e.to_string())?;
    let data = encoder.finish().map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_FILE {
        return Err("Replay exceeds the supported file size".into());
    }
    crate::local_state::write_atomic_bytes(path, &data).map_err(|e| e.to_string())
}

fn validate(saved: &Saved) -> Result<(), String> {
    let bad = || "Invalid or incompatible replay file".to_string();
    if saved.version != VERSION
        || saved.frames.len() < 2
        || saved.frames.len() > MAX_FRAMES
        || saved.names.is_empty()
        || saved.names.len() > 512
        || saved.scenes.is_empty()
        || saved.scenes.len() > 1024
        || saved.keys.len() > 512
        || saved.cameras.len() > MAX_FRAMES
        || saved
            .names
            .iter()
            .any(|n| n.is_empty() || n.len() > 96 || n.chars().any(char::is_control))
    {
        return Err(bad());
    }
    saved.camera_settings.pose.validate().map_err(|_| bad())?;
    if !saved.camera_settings.fov.is_finite()
        || !(5_f32.to_radians()..=150_f32.to_radians()).contains(&saved.camera_settings.fov)
        || saved
            .camera_settings
            .follow_offset
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 100_000.)
    {
        return Err(bad());
    }
    let (first, last) = (saved.frames[0].at, saved.frames.last().unwrap().at);
    if !first.is_finite()
        || first < 0.
        || !last.is_finite()
        || last - first > WINDOW + 1.
        || !saved.in_point.is_finite()
        || !saved.out_point.is_finite()
        || saved.in_point < first
        || saved.out_point > last
        || saved.in_point >= saved.out_point
        || !saved.rate.is_finite()
        || !(0.1..=4.).contains(&saved.rate)
    {
        return Err(bad());
    }
    for scene in &saved.scenes {
        if scene
            .origin
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 100_000.)
            || scene.signature.len() > 4096
            || scene.park.len() > benilla_protocol::park::MAX_OBJECTS_PER_MAP
            || scene
                .actor_scale
                .iter()
                .any(|s| !s.is_finite() || !(0.001..=100.).contains(s))
            || !scene.actor_height.is_finite()
            || !(0.01..=100.).contains(&scene.actor_height)
        {
            return Err(bad());
        }
        let mut parts = 0;
        for object in &scene.park {
            object.definition().validate().map_err(|_| bad())?;
            object.pose.validate().map_err(|_| bad())?;
            if object.map != scene.map || object.instance != scene.instance {
                return Err(bad());
            }
            parts += object.parts.len().max(1);
        }
        if parts > benilla_protocol::park::MAX_PARTS_PER_MAP {
            return Err(bad());
        }
    }
    if saved
        .names
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != saved.names.len()
    {
        return Err(bad());
    }
    for scene in &saved.scenes {
        if scene.map != saved.scenes[0].map
            || scene.instance != saved.scenes[0].instance
            || scene.signature != saved.scenes[0].signature
        {
            return Err(bad());
        }
    }
    let mut previous = f64::NEG_INFINITY;
    for frame in &saved.frames {
        if !frame.at.is_finite()
            || frame.at <= previous
            || frame.scene >= saved.scenes.len()
            || frame.bones.len() != saved.names.len()
            || frame.state.len() > 128
            || frame.line_trick.len() > 256
            || frame
                .root
                .iter()
                .chain(frame.bones.iter().flatten())
                .chain(frame.velocity.iter())
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
            || !frame.line_points.is_finite()
        {
            return Err(bad());
        }
        let valid_matrix = |columns: &[f32; 16]| {
            let mut matrix = Mat4::from_cols_array(columns);
            matrix.w_axis.w = 1.;
            let determinant = Mat3::from_mat4(matrix).determinant();
            if !determinant.is_finite() || determinant.abs() < 1e-8 {
                return false;
            }
            let transform = Transform::from_matrix(matrix);
            transform.rotation.is_finite()
                && transform.translation.is_finite()
                && transform.scale.is_finite()
                && transform.scale.abs().min_element() > 1e-5
                && (matrix.x_axis.w.abs() + matrix.y_axis.w.abs() + matrix.z_axis.w.abs()) < 1e-4
        };
        if !valid_matrix(&frame.root) || frame.bones.iter().any(|b| !valid_matrix(b)) {
            return Err(bad());
        }
        if let Some((p, b)) = frame.camera {
            if p.iter()
                .chain(b.iter())
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
            {
                return Err(bad());
            }
            let basis = Mat3::from_cols_array(&b);
            if basis.y_axis.length_squared() < 1e-6
                || basis.z_axis.length_squared() < 1e-6
                || basis.y_axis.cross(basis.z_axis).length_squared() < 1e-6
            {
                return Err(bad());
            }
        }
        previous = frame.at;
    }
    for keys in [&saved.keys, &saved.cameras] {
        previous = f64::NEG_INFINITY;
        for key in keys {
            key.pose.validate().map_err(|_| bad())?;
            if !key.at.is_finite()
                || key.at < first
                || key.at > last
                || key.at <= previous
                || !key.fov.is_finite()
                || !(5_f32.to_radians()..=150_f32.to_radians()).contains(&key.fov)
                || !key.rate.is_finite()
                || !(0.1..=4.).contains(&key.rate)
            {
                return Err(bad());
            }
            previous = key.at;
        }
    }
    if saved.visuals.len() > MAX_FRAMES {
        return Err(bad());
    }
    previous = f64::NEG_INFINITY;
    for visual in &saved.visuals {
        if !visual.at.is_finite()
            || visual.at < first
            || visual.at > last
            || visual.at <= previous
            || visual.locals.is_empty()
            || visual.locals.len() > 512
            || visual.locals.len() != saved.visuals[0].locals.len()
        {
            return Err(bad());
        }
        visual.transform.validate().map_err(|_| bad())?;
        for local in &visual.locals {
            let norm: f32 = local.rotation.iter().map(|v| v * v).sum();
            if local
                .position
                .iter()
                .chain(&local.rotation)
                .chain(&local.scale)
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
                || !(0.99..=1.01).contains(&norm)
            {
                return Err(bad());
            }
        }
        previous = visual.at;
    }
    Ok(())
}

pub(super) fn load(path: &Path) -> Result<Clip, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE {
        return Err("Replay file is too large".into());
    }
    let mut bytes = Vec::new();
    GzDecoder::new(file)
        .take(MAX_DECODED + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_DECODED {
        return Err("Expanded replay file is too large".into());
    }
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    validate(&saved)?;
    let names: Arc<[String]> = saved.names.into();
    let scenes: Vec<_> = saved
        .scenes
        .into_iter()
        .map(|s| {
            Arc::new(Scene {
                map: s.map,
                instance: s.instance,
                origin: Vec3::from_array(s.origin),
                park: s.park.into(),
                signature: s.signature,
                actor_scale: Vec3::from_array(s.actor_scale),
                actor_height: s.actor_height,
            })
        })
        .collect();
    let frames = saved
        .frames
        .into_iter()
        .map(|s| {
            Arc::new(Sample {
                at: s.at,
                cut: s.cut,
                scene: scenes[s.scene].clone(),
                frame: super::super::Frame {
                    published_at: Instant::now(),
                    root: Mat4::from_cols_array(&s.root),
                    bones: s
                        .bones
                        .into_iter()
                        .map(|b| Mat4::from_cols_array(&b))
                        .collect(),
                    names: names.clone(),
                    camera: s
                        .camera
                        .map(|(p, b)| (Vec3::from_array(p), Mat3::from_cols_array(&b))),
                    velocity: Vec3::from_array(s.velocity),
                    tick: s.tick,
                    state: s.state,
                    line_points: s.line_points,
                    line_trick: s.line_trick,
                    landed_lines: 0,
                    landed_points: 0.,
                    landed_trick: String::new(),
                },
            })
        })
        .collect();
    Ok(Clip {
        frames,
        keys: saved.keys,
        cameras: saved.cameras,
        visuals: saved
            .visuals
            .into_iter()
            .map(|v| {
                Arc::new(Visual {
                    at: v.at,
                    cut: v.cut,
                    transform: loaded_transform(v.transform),
                    locals: v.locals.into_iter().map(loaded_transform).collect(),
                })
            })
            .collect(),
        in_point: saved.in_point,
        out_point: saved.out_point,
        rate: saved.rate,
        camera_settings: saved.camera_settings,
    })
}

fn saved_transform(transform: Transform) -> benilla_protocol::park::Pose {
    benilla_protocol::park::Pose {
        position: transform.translation.to_array(),
        rotation: transform.rotation.normalize().to_array(),
        scale: transform.scale.to_array(),
    }
}
fn loaded_transform(pose: benilla_protocol::park::Pose) -> Transform {
    Transform {
        translation: Vec3::from_array(pose.position),
        rotation: Quat::from_array(pose.rotation).normalize(),
        scale: Vec3::from_array(pose.scale),
    }
}

struct Limited<W> {
    writer: W,
    remaining: u64,
}
impl<W: Write> Write for Limited<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.len() as u64 > self.remaining {
            return Err(std::io::Error::other(
                "Expanded replay exceeds the supported file size",
            ));
        }
        let written = self.writer.write(buffer)?;
        self.remaining -= written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(in crate::skate::replay) fn frame(tick: u64) -> super::super::super::Frame {
        super::super::super::Frame {
            published_at: Instant::now(),
            root: Mat4::IDENTITY,
            bones: vec![Mat4::IDENTITY].into(),
            names: vec!["HIPS".into()].into(),
            camera: None,
            velocity: Vec3::ZERO,
            tick,
            state: "PhysicsGround".into(),
            line_points: 0.,
            line_trick: String::new(),
            landed_lines: 0,
            landed_points: 0.,
            landed_trick: String::new(),
        }
    }
    fn clip() -> Clip {
        let scene = Arc::new(Scene::default());
        Clip {
            frames: (0..3)
                .map(|i| {
                    Arc::new(Sample {
                        at: i as f64 * 0.1,
                        frame: frame(i),
                        scene: scene.clone(),
                        cut: i == 0,
                    })
                })
                .collect(),
            keys: Vec::new(),
            cameras: Vec::new(),
            visuals: vec![Arc::new(Visual {
                at: 0.,
                transform: Transform::IDENTITY,
                locals: vec![Transform::IDENTITY].into(),
                cut: true,
            })],
            in_point: 0.,
            out_point: 0.2,
            rate: 0.5,
            camera_settings: default(),
        }
    }
    #[test]
    fn clips_roundtrip_poses_trim_speed_and_scene_sharing() {
        let path = std::env::temp_dir().join(format!(
            "benilla-replay-{}-{}.skate-replay",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        save(&path, &clip()).unwrap();
        let loaded = load(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(loaded.frames.len(), 3);
        assert_eq!(loaded.rate, 0.5);
        assert_eq!(loaded.out_point, 0.2);
        assert!(Arc::ptr_eq(
            &loaded.frames[0].scene,
            &loaded.frames[1].scene
        ));
        assert_eq!(loaded.visuals[0].locals[0], Transform::IDENTITY);
        assert_eq!(loaded.frames[2].frame.root, Mat4::IDENTITY);
    }
    #[test]
    fn invalid_transforms_and_oversized_output_are_rejected() {
        let mut data = clip();
        let sample = Arc::make_mut(&mut data.frames[1]);
        sample.frame.root = Mat4::ZERO;
        let path = std::env::temp_dir().join("benilla-invalid-replay.skate-replay");
        assert!(save(&path, &data).is_err());
        assert!(!path.exists());
        let mut output = Vec::new();
        let mut writer = Limited {
            writer: &mut output,
            remaining: 2,
        };
        assert!(writer.write_all(&[1, 2, 3]).is_err());
        assert!(output.is_empty());
    }
}
