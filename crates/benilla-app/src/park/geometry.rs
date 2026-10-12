//! Small authored park pieces: the same geometry drives picking, walking and skating.
use benilla_protocol::park::{Definition, Model};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

#[derive(Clone, Default)]
pub(super) struct Geometry {
    pub triangles: Vec<[Vec3; 3]>,
    pub rails: Vec<Vec<Vec3>>,
    pub picking: Vec<[Vec3; 3]>,
}
static IMPORTED: OnceLock<RwLock<HashMap<u64, Arc<Geometry>>>> = OnceLock::new();
pub(super) fn forget_asset(id: u64) {
    if let Some(assets) = IMPORTED.get() {
        assets.write().unwrap().remove(&id);
    }
}
pub(super) fn register_asset(id: u64, package: &benilla_park_assets::Package) {
    let triangles = package
        .collision
        .iter()
        .map(|t| t.map(Vec3::from_array))
        .collect();
    let picking = package
        .batches
        .iter()
        .flat_map(|b| {
            b.indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| [t[0], t[1], t[2]].map(|i| Vec3::from_array(b.positions[i as usize])))
        })
        .collect();
    let rails = package
        .rails
        .iter()
        .map(|r| r.iter().copied().map(Vec3::from_array).collect())
        .collect();
    IMPORTED
        .get_or_init(RwLock::default)
        .write()
        .unwrap()
        .insert(
            id,
            Arc::new(Geometry {
                triangles,
                rails,
                picking,
            }),
        );
}
impl Geometry {
    pub(super) fn pick_triangles(&self) -> &[[Vec3; 3]] {
        if self.picking.is_empty() {
            &self.triangles
        } else {
            &self.picking
        }
    }
}
fn source(model: Model, asset: u64) -> Result<Arc<Geometry>, &'static str> {
    if model == Model::Asset {
        IMPORTED
            .get_or_init(RwLock::default)
            .read()
            .unwrap()
            .get(&asset)
            .cloned()
            .ok_or("Object model is still loading or is missing from this installation")
    } else {
        Ok(Arc::new(build(model)))
    }
}
impl Geometry {
    fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        self.triangles.push([a, b, c]);
    }
    fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
        self.triangle(a, b, c);
        self.triangle(a, c, d);
    }
    fn block(&mut self, min: Vec3, max: Vec3) {
        let v = [
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(max.x, min.y, min.z),
            Vec3::new(max.x, max.y, min.z),
            Vec3::new(min.x, max.y, min.z),
            Vec3::new(min.x, min.y, max.z),
            Vec3::new(max.x, min.y, max.z),
            Vec3::new(max.x, max.y, max.z),
            Vec3::new(min.x, max.y, max.z),
        ];
        for [a, b, c, d] in [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 4, 7, 3],
            [1, 2, 6, 5],
            [0, 1, 5, 4],
            [3, 7, 6, 2],
        ] {
            self.quad(v[a], v[b], v[c], v[d]);
        }
    }
    pub(super) fn mesh(&self) -> Mesh {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv = Vec::new();
        let mut colors = Vec::new();
        for triangle in &self.triangles {
            let normal = (triangle[1] - triangle[0])
                .cross(triangle[2] - triangle[0])
                .normalize_or_zero();
            let shade = 0.55 + 0.45 * normal.dot(Vec3::new(-0.3, 0.8, 0.5).normalize()).max(0.);
            for point in triangle {
                positions.push(point.to_array());
                normals.push(normal.to_array());
                uv.push([point.x, point.z]);
                colors.push([shade, shade, shade, 1.]);
            }
        }
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    }
    pub(super) fn collider(&self) -> avian3d::prelude::Collider {
        let verts = self.triangles.iter().flatten().copied().collect();
        let indices = (0..self.triangles.len() as u32)
            .map(|i| [i * 3, i * 3 + 1, i * 3 + 2])
            .collect();
        avian3d::prelude::Collider::trimesh(verts, indices)
    }
}

pub(super) fn build(model: Model) -> Geometry {
    let mut g = Geometry::default();
    match model {
        Model::Compound | Model::Asset => return g,
        Model::Box => g.block(Vec3::new(-2., 0., -2.), Vec3::new(2., 1.5, 2.)),
        Model::Platform => {
            g.block(Vec3::new(-3., 0., -3.), Vec3::new(3., 1., 3.));
            let corners = [
                Vec3::new(-3., 1., -3.),
                Vec3::new(3., 1., -3.),
                Vec3::new(3., 1., 3.),
                Vec3::new(-3., 1., 3.),
            ];
            for i in 0..4 {
                g.rails.push(vec![corners[i], corners[(i + 1) % 4]]);
            }
        }
        Model::Rail => {
            g.block(Vec3::new(-0.07, 0.95, -3.), Vec3::new(0.07, 1.09, 3.));
            for z in [-2., 2.] {
                g.block(
                    Vec3::new(-0.08, 0., z - 0.08),
                    Vec3::new(0.08, 0.95, z + 0.08),
                );
            }
            g.rails
                .push(vec![Vec3::new(0., 1.09, -3.), Vec3::new(0., 1.09, 3.)]);
        }
        Model::Ramp => {
            let a = Vec3::new(-2., 0., -3.);
            let b = Vec3::new(2., 0., -3.);
            let c = Vec3::new(2., 2., 3.);
            let d = Vec3::new(-2., 2., 3.);
            let e = Vec3::new(-2., 0., 3.);
            let f = Vec3::new(2., 0., 3.);
            g.quad(a, d, c, b);
            g.quad(e, f, c, d);
            g.quad(a, b, f, e);
            g.triangle(a, e, d);
            g.triangle(b, c, f);
            g.rails.push(vec![d, c]);
        }
        Model::QuarterPipe => {
            const SEGMENTS: usize = 32;
            let mut left = Vec::new();
            let mut right = Vec::new();
            for i in 0..=SEGMENTS {
                let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::FRAC_PI_2;
                let y = 4. * (1. - angle.cos());
                let z = 4. * angle.sin() - 2.;
                left.push(Vec3::new(-3., y, z));
                right.push(Vec3::new(3., y, z));
            }
            for i in 0..SEGMENTS {
                g.quad(left[i], left[i + 1], right[i + 1], right[i]);
                g.triangle(Vec3::new(-3., 0., 2.), left[i + 1], left[i]);
                g.triangle(Vec3::new(3., 0., 2.), right[i], right[i + 1]);
            }
            g.quad(
                Vec3::new(-3., 0., 2.),
                Vec3::new(3., 0., 2.),
                right[SEGMENTS],
                left[SEGMENTS],
            );
            g.quad(
                left[0],
                right[0],
                Vec3::new(3., 0., 2.),
                Vec3::new(-3., 0., 2.),
            );
            g.rails.push(vec![left[SEGMENTS], right[SEGMENTS]]);
        }
        Model::Stairs => {
            let mut profile = vec![(0., -2.)];
            for i in 0..4 {
                let z = i as f32 - 2.;
                let y = (i + 1) as f32 * 0.5;
                profile.push((y, z));
                profile.push((y, z + 1.));
            }
            profile.push((0., 2.));
            let left: Vec<Vec3> = profile.iter().map(|&(y, z)| Vec3::new(-2., y, z)).collect();
            let right: Vec<Vec3> = profile.iter().map(|&(y, z)| Vec3::new(2., y, z)).collect();
            for i in 0..profile.len() {
                let j = (i + 1) % profile.len();
                g.quad(left[i], left[j], right[j], right[i]);
            }
            // Each step's side rectangle avoids a concave polygon fan crossing outside the stairs.
            for i in 0..4 {
                let z = i as f32 - 2.;
                let y = (i + 1) as f32 * 0.5;
                g.quad(
                    Vec3::new(-2., 0., z),
                    Vec3::new(-2., 0., z + 1.),
                    Vec3::new(-2., y, z + 1.),
                    Vec3::new(-2., y, z),
                );
                g.quad(
                    Vec3::new(2., 0., z),
                    Vec3::new(2., y, z),
                    Vec3::new(2., y, z + 1.),
                    Vec3::new(2., 0., z + 1.),
                );
            }
        }
    }
    g.triangles
        .retain(|t| (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-10);
    g
}

#[derive(Default)]
struct Cache {
    values: HashMap<Vec<u32>, Arc<Geometry>>,
    order: VecDeque<Vec<u32>>,
    bytes: usize,
}
fn cache_bytes(g: &Geometry) -> usize {
    (g.triangles.len() + g.picking.len()) * std::mem::size_of::<[Vec3; 3]>()
        + g.rails
            .iter()
            .map(|r| r.len() * std::mem::size_of::<Vec3>())
            .sum::<usize>()
}
static ASSEMBLIES: OnceLock<Mutex<Cache>> = OnceLock::new();
pub(super) fn definition(definition: &Definition) -> Result<Arc<Geometry>, &'static str> {
    if definition.model != Model::Compound {
        return source(definition.model, definition.asset);
    }
    let key: Vec<u32> = definition
        .parts
        .iter()
        .flat_map(|part| {
            [
                part.model as u32,
                part.asset as u32,
                (part.asset >> 32) as u32,
            ]
            .into_iter()
            .chain(part.affine.map(f32::to_bits))
        })
        .collect();
    let cache = ASSEMBLIES.get_or_init(Mutex::default);
    if let Some(geometry) = cache.lock().unwrap().values.get(&key) {
        return Ok(geometry.clone());
    }
    let mut pieces = Vec::new();
    for part in &definition.parts {
        let source = source(part.model, part.asset)?;
        let matrix = super::math::part_matrix(part);
        let triangles = source
            .triangles
            .iter()
            .copied()
            .map(|t| t.map(|p| matrix.transform_point3(p)))
            .collect();
        let rails = source
            .rails
            .iter()
            .map(|r| {
                r.iter()
                    .copied()
                    .map(|p| matrix.transform_point3(p))
                    .collect()
            })
            .collect();
        let picking = if definition.parts.iter().any(|p| p.model == Model::Asset) {
            source
                .pick_triangles()
                .iter()
                .map(|t| t.map(|p| matrix.transform_point3(p)))
                .collect()
        } else {
            Vec::new()
        };
        pieces.push(Geometry {
            triangles,
            rails,
            picking,
        });
    }
    let imported = definition.parts.iter().any(|p| p.model == Model::Asset);
    if !imported || pieces.iter().map(|p| p.triangles.len()).sum::<usize>() < 8000 {
        super::seams::weld(&mut pieces);
    }
    let mut combined = Geometry::default();
    for piece in pieces {
        // Open meshes are surfaces, not volumes. Keep their complete collision in an assembly.
        if imported
            && (combined.triangles.len() + piece.triangles.len() > 8000
                || !closed(&piece.triangles)
                || !combined.triangles.is_empty() && !closed(&combined.triangles))
        {
            combined.triangles.extend(piece.triangles);
        } else if imported {
            let a = combined.triangles;
            let b = piece.triangles;
            combined.triangles = super::solid::union(a.clone(), b.clone()).unwrap_or_else(|_| {
                let mut surfaces = a;
                surfaces.extend(b);
                surfaces
            });
        } else {
            combined.triangles = super::solid::union(combined.triangles, piece.triangles)?;
        }
        combined.rails.extend(piece.rails);
        combined.picking.extend(piece.picking);
    }
    if combined.triangles.len() < 120000 {
        combined.triangles = super::seams::stitch(combined.triangles)?;
    }
    if combined.triangles.is_empty() {
        return Err("Objects are too small to fuse or resize at this scale");
    }
    if combined.triangles.len() < 8000 && closed(&combined.triangles) {
        combined.rails = if imported {
            super::solid::exterior_rails(&combined.triangles, combined.rails.clone())
                .unwrap_or(combined.rails)
        } else {
            super::solid::exterior_rails(&combined.triangles, combined.rails)?
        };
    }
    let geometry = Arc::new(combined);
    let mut cache = cache.lock().unwrap();
    if !cache.values.contains_key(&key) {
        let bytes = cache_bytes(&geometry);
        while !cache.values.is_empty()
            && (cache.values.len() >= 128 || cache.bytes + bytes > 128 * 1024 * 1024)
        {
            if let Some(old) = cache.order.pop_front() {
                if let Some(g) = cache.values.remove(&old) {
                    cache.bytes = cache.bytes.saturating_sub(cache_bytes(&g));
                }
            }
        }
        cache.bytes += bytes;
        cache.order.push_back(key.clone());
        cache.values.insert(key, geometry.clone());
    }
    Ok(geometry)
}
fn closed(tris: &[[Vec3; 3]]) -> bool {
    let mut edges = HashMap::<([i64; 3], [i64; 3]), (usize, i32)>::new();
    let key = |v: Vec3| {
        v.to_array()
            .map(|n| (f64::from(n) * 100000.).round() as i64)
    };
    for t in tris {
        for i in 0..3 {
            let a = key(t[i]);
            let b = key(t[(i + 1) % 3]);
            if a == b {
                continue;
            }
            let (pair, sign) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
            let e = edges.entry(pair).or_default();
            e.0 += 1;
            e.1 += sign;
        }
    }
    !edges.is_empty() && edges.values().all(|&(count, sign)| count == 2 && sign == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_piece_has_finite_nonzero_collision_and_valid_rails() {
        for model in Model::ALL {
            let g = build(model);
            assert!(!g.triangles.is_empty());
            for t in &g.triangles {
                assert!(t.iter().all(|v| v.is_finite()));
                assert!((t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-10);
            }
            for rail in &g.rails {
                assert!(rail.len() >= 2);
                assert!((rail[1] - rail[0]).length() > 0.1);
            }
            let _ = g.collider();
        }
    }
    #[test]
    fn ramp_and_quarter_pipe_faces_point_into_the_riding_space() {
        for model in [Model::Ramp, Model::QuarterPipe] {
            let g = build(model);
            let t = g.triangles[0];
            assert!((t[1] - t[0]).cross(t[2] - t[0]).y > 0.);
        }
    }
    #[test]
    fn rail_grind_path_is_on_the_bar_top() {
        let g = build(Model::Rail);
        assert_eq!(g.rails[0][0].y, 1.09);
        assert_eq!(g.rails[0][1].y, 1.09);
    }
    #[test]
    fn a_grind_rail_buried_inside_a_fused_box_is_removed() {
        let box_def = Definition::primitive(
            Model::Box,
            benilla_protocol::park::Pose {
                scale: [1., 1., 2.],
                ..default()
            },
        );
        let rail = Definition::primitive(Model::Rail, benilla_protocol::park::Pose::default());
        let fused = super::super::math::fuse(&[box_def, rail]).unwrap();
        assert!(definition(&fused).unwrap().rails.is_empty());
    }
}
