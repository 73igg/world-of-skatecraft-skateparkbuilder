//! Load and decode private model packages off the input/render threads.
use super::{geometry, scene, state::ParkWorld};
use benilla_assets::WorldAssets;
use benilla_park_assets::{Entry, Package};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) struct Batch {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}
pub(super) struct Model {
    pub package: Arc<Package>,
    pub batches: Vec<Batch>,
    pub collider: avian3d::prelude::Collider,
}
struct Pixels {
    key: String,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    repeat_u: bool,
    repeat_v: bool,
}
enum Loaded {
    Index(Vec<Entry>),
    Object(
        u64,
        Result<(Package, Vec<Pixels>, avian3d::prelude::Collider), String>,
    ),
}
struct Upload {
    id: u64,
    package: Arc<Package>,
    textures: VecDeque<Pixels>,
    batches: Vec<Batch>,
    collider: avian3d::prelude::Collider,
}
#[derive(Resource, Default)]
pub(super) struct Library {
    pub entries: Vec<Entry>,
    pub models: HashMap<u64, Model>,
    pub errors: HashMap<u64, String>,
    pub generation: u64,
    pub indexed: bool,
    upload: Option<Upload>,
    requested: HashSet<u64>,
    jobs: Option<crossbeam_channel::Sender<u64>>,
    incoming: Option<crossbeam_channel::Receiver<Loaded>>,
    textures: HashMap<String, Handle<Image>>,
    recent: VecDeque<u64>,
    pub affine_meshes: HashMap<(u64, usize, [u32; 12]), Handle<Mesh>>,
}
impl Library {
    fn prune(&mut self, protected: &HashSet<u64>) {
        // Keep placed and edited objects resident; bound only unused browsing history.
        let estimate = |model: &Model| {
            model
                .package
                .batches
                .iter()
                .map(|b| b.positions.len() * 96 + b.indices.len() * 8)
                .sum::<usize>()
                + model.package.collision.len() * 160
        };
        let mut bytes = self.models.values().map(estimate).sum::<usize>();
        let mut unused = self
            .models
            .keys()
            .filter(|id| !protected.contains(id))
            .count();
        let mut removed = HashSet::new();
        for &id in &self.recent {
            if unused <= 8 && bytes <= 384 * 1024 * 1024 {
                break;
            }
            if protected.contains(&id) {
                continue;
            }
            if let Some(model) = self.models.remove(&id) {
                bytes = bytes.saturating_sub(estimate(&model));
                unused -= 1;
                self.requested.remove(&id);
                geometry::forget_asset(id);
                removed.insert(id);
            }
        }
        if removed.is_empty() {
            return;
        }
        self.recent.retain(|id| !removed.contains(id));
        self.affine_meshes
            .retain(|(id, _, _), _| !removed.contains(id));
        let mut used = self
            .models
            .values()
            .flat_map(|m| m.package.batches.iter().filter_map(texture_key))
            .collect::<HashSet<_>>();
        if let Some(upload) = &self.upload {
            used.extend(upload.package.batches.iter().filter_map(texture_key));
        }
        self.textures.retain(|key, _| used.contains(key));
        self.generation += 1;
    }
    pub fn request(&mut self, id: u64) {
        if id > 6 && self.models.contains_key(&id) {
            self.recent.retain(|&old| old != id);
            self.recent.push_back(id);
            return;
        }
        if id <= 6 || self.requested.contains(&id) || self.models.contains_key(&id) {
            return;
        }
        if let Some(jobs) = &self.jobs {
            if jobs.try_send(id).is_ok() {
                self.requested.insert(id);
            }
        }
    }
    pub fn retry(&mut self, id: u64) {
        self.errors.remove(&id);
        self.requested.remove(&id);
        self.request(id);
    }
    pub fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
    pub fn ready(&self, id: u64) -> bool {
        id <= 6 || self.models.contains_key(&id)
    }
    pub fn mesh(
        &mut self,
        id: u64,
        batch: usize,
        matrix: Mat4,
        meshes: &mut Assets<Mesh>,
    ) -> Option<Handle<Mesh>> {
        let model = self.models.get(&id)?;
        if matrix == Mat4::IDENTITY {
            return Some(model.batches.get(batch)?.mesh.clone());
        }
        let a = matrix.to_cols_array();
        let key = (
            id,
            batch,
            [
                a[0].to_bits(),
                a[1].to_bits(),
                a[2].to_bits(),
                a[4].to_bits(),
                a[5].to_bits(),
                a[6].to_bits(),
                a[8].to_bits(),
                a[9].to_bits(),
                a[10].to_bits(),
                a[12].to_bits(),
                a[13].to_bits(),
                a[14].to_bits(),
            ],
        );
        if let Some(handle) = self.affine_meshes.get(&key) {
            return Some(handle.clone());
        }
        let handle = meshes.add(batch_mesh(&model.package.batches[batch], matrix));
        if self.affine_meshes.len() > 2048 {
            self.affine_meshes.clear();
        }
        self.affine_meshes.insert(key, handle.clone());
        Some(handle)
    }
}
fn builtins() -> Vec<Entry> {
    benilla_protocol::park::Model::ALL
        .into_iter()
        .zip([
            [4., 1.5, 4.],
            [4., 2., 6.],
            [0.16, 1.09, 6.],
            [6., 1., 6.],
            [6., 4., 4.],
            [4., 2., 4.],
        ])
        .map(|(m, size)| Entry {
            id: m as u64,
            game: "Builder".into(),
            name: m.name().into(),
            category: "Practice pieces".into(),
            path: format!("builtin/{}", m.name()),
            aliases: Vec::new(),
            package: None,
            bounds: Some([
                [-size[0] * 0.5, 0., -size[2] * 0.5],
                [size[0] * 0.5, size[1], size[2] * 0.5],
            ]),
        })
        .collect()
}
pub(super) fn tick(
    assets: Option<Res<WorldAssets>>,
    world: Res<ParkWorld>,
    editor: Res<super::state::ParkEditor>,
    browser: Res<super::catalog_ui::Browser>,
    mut library: ResMut<Library>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    if library.jobs.is_none() {
        let Some(assets) = assets else {
            return;
        };
        let Some(root) = crate::local_state::park_assets_dir() else {
            return;
        };
        let chain = assets.chain.lock().unwrap().clone();
        let (jobs, requests) = crossbeam_channel::bounded(256);
        let (publish, incoming) = crossbeam_channel::bounded(1);
        library.entries = builtins();
        library.jobs = Some(jobs);
        library.incoming = Some(incoming);
        std::thread::Builder::new()
            .name("park-assets".into())
            .spawn(move || {
                let mut chain = chain;
                let aliases = std::fs::read(root.join("wow-names.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<BTreeMap<String, Vec<String>>>(&b).ok())
                    .unwrap_or_default();
                let mut entries = benilla_park_assets::index_wow(&mut chain, &aliases)
                    .unwrap_or_else(|error| {
                        warn!("park catalog: WoW index failed: {error}");
                        Vec::new()
                    });
                if let Ok(bytes) = std::fs::read(root.join("skate-catalog.json")) {
                    #[derive(serde::Deserialize)]
                    struct Index {
                        version: u32,
                        entries: Vec<Entry>,
                    }
                    match serde_json::from_slice::<Index>(&bytes) {
                        Ok(index) if index.version == 3 => entries.extend(index.entries),
                        _ => warn!("park catalog: unsupported Skate object index"),
                    }
                }
                let by_id = entries
                    .iter()
                    .cloned()
                    .map(|e| (e.id, e))
                    .collect::<HashMap<_, _>>();
                if by_id.len() != entries.len() {
                    warn!("park catalog: duplicate asset identity");
                    return;
                }
                if publish.send(Loaded::Index(entries)).is_err() {
                    return;
                }
                while let Ok(id) = requests.recv() {
                    let result = by_id
                        .get(&id)
                        .ok_or_else(|| {
                            anyhow::anyhow!("Object is missing from this installation's catalog")
                        })
                        .and_then(|entry| load(&root, &mut chain, entry));
                    if publish
                        .send(Loaded::Object(id, result.map_err(|e| format!("{e:#}"))))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("park asset worker");
    }
    let mut protected = HashSet::new();
    for object in world.objects.values() {
        if object.model == benilla_protocol::park::Model::Asset {
            protected.insert(object.asset);
            library.request(object.asset);
        }
        for p in &object.parts {
            if p.model == benilla_protocol::park::Model::Asset {
                protected.insert(p.asset);
                library.request(p.asset);
            }
        }
    }
    for draft in &editor.drafts {
        if draft.model == benilla_protocol::park::Model::Asset {
            protected.insert(draft.asset);
        }
        for part in &draft.parts {
            if part.model == benilla_protocol::park::Model::Asset {
                protected.insert(part.asset);
            }
        }
    }
    if browser.open {
        if let Some(id) = browser.preview {
            protected.insert(id);
        }
    }
    let events = if library.upload.is_some() {
        Vec::new()
    } else {
        library
            .incoming
            .as_ref()
            .map(|r| r.try_iter().take(1).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for event in events {
        match event {
            Loaded::Index(entries) => {
                info!("park catalog: {} WoW/Skate objects indexed", entries.len());
                library.entries.extend(entries);
                library.indexed = true;
                library.generation += 1;
            }
            Loaded::Object(id, Err(error)) => {
                warn!("park catalog: {id:016x}: {error}");
                library.errors.insert(id, error);
                library.generation += 1;
            }
            Loaded::Object(id, Ok((package, textures, collider))) => {
                library.upload = Some(Upload {
                    id,
                    package: Arc::new(package),
                    textures: textures.into(),
                    batches: Vec::new(),
                    collider,
                });
            }
        }
    }
    if let Some(mut upload) = library.upload.take() {
        for _ in 0..2 {
            let Some(t) = upload.textures.pop_front() else {
                break;
            };
            library.textures.entry(t.key).or_insert_with(|| {
                let mut image = Image::new(
                    Extent3d {
                        width: t.width,
                        height: t.height,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    t.pixels,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::RENDER_WORLD,
                );
                image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: if t.repeat_u {
                        ImageAddressMode::Repeat
                    } else {
                        ImageAddressMode::ClampToEdge
                    },
                    address_mode_v: if t.repeat_v {
                        ImageAddressMode::Repeat
                    } else {
                        ImageAddressMode::ClampToEdge
                    },
                    ..ImageSamplerDescriptor::linear()
                });
                images.add(image)
            });
        }
        if upload.textures.is_empty() {
            for _ in 0..8 {
                let Some(batch) = upload.package.batches.get(upload.batches.len()) else {
                    break;
                };
                let texture = texture_key(batch)
                    .and_then(|key| library.textures.get(&key))
                    .cloned();
                upload.batches.push(Batch {
                    mesh: meshes.add(batch_mesh(batch, Mat4::IDENTITY)),
                    material: materials.add(StandardMaterial {
                        base_color_texture: texture,
                        unlit: true,
                        alpha_mode: match batch.alpha {
                            1 => AlphaMode::Mask(batch.cutoff),
                            2 => AlphaMode::Blend,
                            3 => AlphaMode::Add,
                            _ => AlphaMode::Opaque,
                        },
                        cull_mode: if batch.two_sided {
                            None
                        } else {
                            Some(bevy::render::render_resource::Face::Back)
                        },
                        ..default()
                    }),
                });
            }
        }
        if upload.batches.len() == upload.package.batches.len() {
            library.recent.push_back(upload.id);
            library.models.insert(
                upload.id,
                Model {
                    package: upload.package,
                    batches: upload.batches,
                    collider: upload.collider,
                },
            );
            library.generation += 1;
        } else {
            library.upload = Some(upload);
        }
    }
    library.prune(&protected);
}
fn load(
    root: &Path,
    chain: &mut benilla_formats::Chain,
    entry: &Entry,
) -> anyhow::Result<(Package, Vec<Pixels>, avian3d::prelude::Collider)> {
    let mut package = if entry.game == "WoW" {
        benilla_park_assets::load_wow(chain, entry)?
    } else {
        let relative = entry
            .package
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Object model package is missing"))?;
        Package::read(&inside(root, relative)?, entry.id)?
    };
    let mut textures = Vec::new();
    let mut used = HashSet::new();
    for b in &package.batches {
        if let Some(key) = &b.texture {
            let identity = texture_key(b).unwrap();
            if used.insert(identity.clone()) {
                let (width, height, pixels) = if let Some(path) = key.strip_prefix("mpq://") {
                    benilla_formats::read_texture_rgba(chain, path)?
                } else {
                    let img = image::open(inside(root, key)?)?.into_rgba8();
                    (img.width(), img.height(), img.into_raw())
                };
                textures.push(Pixels {
                    key: identity,
                    repeat_u: b.repeat_u,
                    repeat_v: b.repeat_v,
                    width,
                    height,
                    pixels,
                });
            }
        }
    }
    if package.rails.is_empty() {
        let native = package
            .collision
            .iter()
            .map(|t| t.map(|p| p.map(|v| v * 0.9144)))
            .collect::<Vec<_>>();
        package.rails = crate::skate::find_rails(&native)
            .into_iter()
            .map(|r| r.into_iter().map(|p| p.map(|v| v / 0.9144)).collect())
            .collect();
    }
    geometry::register_asset(entry.id, &package);
    let collider = geometry::definition(&benilla_protocol::park::Definition::asset(
        entry.id,
        default(),
    ))
    .map_err(|e| anyhow::anyhow!(e))?
    .collider();
    Ok((package, textures, collider))
}
fn inside(root: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        anyhow::bail!("Object cache path leaves its catalog directory");
    }
    let resolved = root.join(path).canonicalize()?;
    if !resolved.starts_with(root.canonicalize()?) {
        anyhow::bail!("Object cache symlink leaves its catalog directory");
    }
    Ok(resolved)
}
fn batch_mesh(batch: &benilla_park_assets::Batch, matrix: Mat4) -> Mesh {
    let normal = Mat3::from_mat4(matrix).inverse().transpose();
    let positions = batch
        .positions
        .iter()
        .map(|p| matrix.transform_point3(Vec3::from_array(*p)).to_array())
        .collect::<Vec<_>>();
    let normals = batch
        .normals
        .iter()
        .map(|n| {
            (normal * Vec3::from_array(*n))
                .normalize_or_zero()
                .to_array()
        })
        .collect::<Vec<_>>();
    let colors = normals
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let shade = 0.65
                + 0.35
                    * Vec3::from_array(*n)
                        .dot(Vec3::new(-0.3, 0.8, 0.5).normalize())
                        .max(0.);
            let c = batch.colors.get(i).copied().unwrap_or([1.; 4]);
            [
                c[0].max(0.35) * shade,
                c[1].max(0.35) * shade,
                c[2].max(0.35) * shade,
                c[3],
            ]
        })
        .collect::<Vec<_>>();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, batch.uvs.clone())
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(batch.indices.clone()))
}
pub(super) fn render_parts(
    definition: &benilla_protocol::park::Definition,
    library: &mut Library,
    catalog: &scene::Catalog,
    meshes: &mut Assets<Mesh>,
) -> Option<Vec<(Handle<Mesh>, Handle<StandardMaterial>)>> {
    use benilla_protocol::park::Model;
    let parts = if definition.model == Model::Compound {
        definition.parts.clone()
    } else {
        vec![super::math::part(
            definition.model,
            definition.asset,
            Mat4::IDENTITY,
        )]
    };
    let mut draw = Vec::new();
    for p in parts {
        let matrix = super::math::part_matrix(&p);
        if p.model == Model::Asset {
            let count = library.models.get(&p.asset)?.batches.len();
            for i in 0..count {
                let material = library.models.get(&p.asset)?.batches[i].material.clone();
                draw.push((library.mesh(p.asset, i, matrix, meshes)?, material));
            }
        } else {
            let piece = &catalog.pieces[p.model as usize - 1];
            let mut geo = geometry::build(p.model);
            for t in &mut geo.triangles {
                for v in t {
                    *v = matrix.transform_point3(*v);
                }
            }
            draw.push((
                if matrix == Mat4::IDENTITY {
                    piece.mesh.clone()
                } else {
                    meshes.add(geo.mesh())
                },
                piece.material.clone(),
            ));
        }
    }
    Some(draw)
}

fn texture_key(batch: &benilla_park_assets::Batch) -> Option<String> {
    batch
        .texture
        .as_ref()
        .map(|path| format!("{}|{}{}", path, batch.repeat_u, batch.repeat_v))
}
