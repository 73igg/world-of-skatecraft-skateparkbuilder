use super::{asset_id, Batch, Entry, Package};
use anyhow::{bail, Context, Result};
use benilla_formats::{Chain, CollisionMesh, ModelBlend, RenderSubmesh};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::collections::{BTreeMap, BTreeSet};

fn canonical(path: &str) -> String {
    let path = path.replace('\\', "/").to_lowercase();
    if path.ends_with(".mdx") || path.ends_with(".mdl") {
        format!("{}.m2", &path[..path.len() - 4])
    } else {
        path
    }
}
pub fn label(value: &str) -> String {
    let mut text = String::new();
    let mut previous = None;
    for c in value.chars() {
        if c == '_' || c == '-' {
            text.push(' ');
        } else {
            if c.is_uppercase() && previous.is_some_and(|p: char| p.is_lowercase()) {
                text.push(' ');
            }
            text.push(c);
        }
        previous = Some(c);
    }
    text.split_whitespace()
        .map(|s| {
            let mut chars = s.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn grouped_wmo(path: &str) -> bool {
    let stem = path.strip_suffix(".wmo").unwrap_or(path);
    stem.rsplit_once('_')
        .is_some_and(|(_, tail)| tail.len() == 3 && tail.bytes().all(|c| c.is_ascii_digit()))
}
fn bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}
fn bounds(points: impl IntoIterator<Item = Vec3>) -> Option<[[f32; 3]; 2]> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for p in points {
        min = min.min(p);
        max = max.max(p);
    }
    if !min.is_finite() || !max.is_finite() {
        return None;
    }
    let o = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
    Some([(min - o).to_array(), (max - o).to_array()])
}
pub fn index_wow(chain: &mut Chain, aliases: &BTreeMap<String, Vec<String>>) -> Result<Vec<Entry>> {
    let mut paths = BTreeSet::new();
    let mut resolved_aliases = aliases.clone();
    if let Ok(catalog) = benilla_formats::load_gameobject_catalog(chain) {
        for (display, path) in catalog.iter() {
            let path = canonical(path);
            if let Some(names) = aliases.get(&display.to_string()) {
                resolved_aliases
                    .entry(path.clone())
                    .or_default()
                    .extend(names.iter().cloned());
            }
            if chain.contains(&path) {
                paths.insert(path);
            }
        }
    }
    for item in chain.list()? {
        let path = canonical(&item.name);
        if path.starts_with("world/")
            && (path.ends_with(".m2") || path.ends_with(".wmo") && !grouped_wmo(&path))
        {
            paths.insert(path);
        }
    }
    let mut entries = Vec::new();
    for path in paths {
        if grouped_wmo(&path) {
            continue;
        }
        let model = path.rsplit('/').next().unwrap_or(&path);
        let stem = model.rsplit_once('.').map_or(model, |v| v.0);
        let category = path
            .rsplit_once('/')
            .and_then(|(p, _)| p.rsplit('/').next())
            .map(label)
            .unwrap_or_else(|| "Objects".into());
        let mut names = resolved_aliases.get(&path).cloned().unwrap_or_default();
        names.sort();
        names.dedup();
        let name = names.first().cloned().unwrap_or_else(|| label(stem));
        let metadata = chain.read_file(&path).ok().and_then(|bytes| {
            if path.ends_with(".wmo") {
                let root = benilla_formats::parse_wmo_root(&bytes).ok()?;
                bounds(
                    root.group_infos()
                        .iter()
                        .flat_map(|b| [bevy(b.bbox_min), bevy(b.bbox_max)]),
                )
            } else {
                let b = benilla_formats::parse_m2_bounds(&bytes).ok()?;
                bounds([bevy(b.bbox_min), bevy(b.bbox_max)])
            }
        });
        // Particle-only records and invisible collision helpers have no placeable model.
        if metadata.is_some_and(|b| b[0] == b[1]) {
            continue;
        }
        entries.push(Entry {
            id: asset_id(1, &path),
            game: "WoW".into(),
            name,
            category,
            path,
            aliases: names,
            package: None,
            bounds: metadata,
        });
    }
    Ok(entries)
}
fn batch(sub: RenderSubmesh, transform: Mat4) -> Batch {
    let normal = Mat3::from_mat4(transform).inverse().transpose();
    let positions = sub
        .positions
        .into_iter()
        .map(|p| transform.transform_point3(bevy(p)).to_array())
        .collect::<Vec<_>>();
    let mut normals = sub
        .normals
        .into_iter()
        .map(|p| (normal * bevy(p)).normalize_or_zero().to_array())
        .collect::<Vec<_>>();
    if normals.len() != positions.len() {
        normals = vec![[0.; 3]; positions.len()];
        for face in sub.indices.as_chunks::<3>().0.iter() {
            let [a, b, c] = *face;
            let face = (Vec3::from_array(positions[b as usize])
                - Vec3::from_array(positions[a as usize]))
            .cross(
                Vec3::from_array(positions[c as usize]) - Vec3::from_array(positions[a as usize]),
            );
            for i in [a, b, c] {
                normals[i as usize] = (Vec3::from_array(normals[i as usize]) + face).to_array();
            }
        }
        for n in &mut normals {
            *n = Vec3::from_array(*n).normalize_or_zero().to_array();
        }
    }
    let uvs = if sub.uvs.len() == positions.len() {
        sub.uvs
    } else {
        vec![[0.; 2]; positions.len()]
    };
    let alpha = match sub.blend {
        ModelBlend::AlphaTest => 1,
        ModelBlend::Blend => 2,
        _ => 0,
    };
    Batch {
        positions,
        normals,
        uvs,
        colors: sub.vertex_colors,
        indices: sub.indices,
        texture: sub.texture.map(|p| format!("mpq://{}", canonical(&p))),
        alpha,
        cutoff: 0.5,
        two_sided: sub.two_sided,
        repeat_u: sub.wrap_x,
        repeat_v: sub.wrap_y,
    }
}
fn hull(mesh: CollisionMesh, transform: Mat4) -> Vec<[[f32; 3]; 3]> {
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| {
            let a = mesh.positions.get(t[0] as usize)?;
            let b = mesh.positions.get(t[1] as usize)?;
            let c = mesh.positions.get(t[2] as usize)?;
            Some([a, b, c].map(|p| transform.transform_point3(bevy(*p)).to_array()))
        })
        .collect()
}
fn append(chain: &mut Chain, path: &str, transform: Mat4, pack: &mut Package) -> Result<()> {
    let render = benilla_formats::load_object_model(chain, path)
        .with_context(|| format!("Loading {path}"))?;
    let collision = if path.ends_with(".wmo") {
        benilla_formats::load_wmo_collision_tris(chain, path)?
    } else {
        benilla_formats::load_m2_collision_hull(chain, path)?
    };
    let mut shapes = hull(collision, transform);
    let first = pack.batches.len();
    pack.batches.extend(
        render
            .into_iter()
            .filter(|s| !s.positions.is_empty() && !s.indices.is_empty())
            .map(|s| batch(s, transform)),
    );
    if shapes.is_empty() {
        for b in &pack.batches[first..] {
            shapes.extend(b.indices.as_chunks::<3>().0.iter().map(|t| {
                [
                    b.positions[t[0] as usize],
                    b.positions[t[1] as usize],
                    b.positions[t[2] as usize],
                ]
            }));
        }
    }
    pack.collision.extend(shapes);
    Ok(())
}
pub fn load_wow(chain: &mut Chain, entry: &Entry) -> Result<Package> {
    if entry.game != "WoW" || asset_id(1, &entry.path) != entry.id {
        bail!("WoW catalog identity mismatch");
    }
    let mut pack = Package {
        version: 3,
        id: entry.id,
        batches: Vec::new(),
        collision: Vec::new(),
        rails: Vec::new(),
        bounds: [[0.; 3]; 2],
    };
    if entry.path.ends_with(".wmo") {
        let root = benilla_formats::parse_wmo_root(&chain.read_file(&entry.path)?)?;
        let stem = entry.path.trim_end_matches(".wmo");
        for group in 0..root.group_count() {
            if !chain.contains(&format!("{stem}_{group:03}.wmo")) {
                bail!("Building is missing group {group}");
            }
        }
        append(chain, &entry.path, Mat4::IDENTITY, &mut pack)?;
        if let Some(set) = root.doodad_sets().first() {
            let basis = Mat3::from_cols(Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y);
            for d in root
                .doodads()
                .iter()
                .skip(set.start as usize)
                .take(set.count as usize)
            {
                if d.model.is_empty() {
                    continue;
                }
                let q = Quat::from_mat3(
                    &(basis
                        * Mat3::from_quat(Quat::from_array(d.orientation).normalize())
                        * basis.transpose()),
                );
                append(
                    chain,
                    &canonical(&d.model),
                    Mat4::from_scale_rotation_translation(
                        Vec3::splat(d.scale),
                        q,
                        bevy(d.position),
                    ),
                    &mut pack,
                )?;
            }
        }
    } else {
        append(chain, &entry.path, Mat4::IDENTITY, &mut pack)?;
    }
    pack.batch_static_materials();
    pack.ground_origin()?;
    Ok(pack)
}
