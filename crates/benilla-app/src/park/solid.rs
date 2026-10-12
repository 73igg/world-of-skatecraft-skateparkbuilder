//! Closed-mesh union in assembly-local double precision; interior faces are not skate contacts.
use bevy::math::DVec3;
use bevy::prelude::Vec3;

const EPS: f64 = 0.00001;
const LIMIT: usize = 200_000;
const DEPTH: usize = 256;
#[derive(Clone)]
struct Polygon {
    vertices: Vec<DVec3>,
    normal: DVec3,
    w: f64,
}
impl Polygon {
    fn new(vertices: Vec<DVec3>) -> Option<Self> {
        let a = *vertices.first()?;
        let normal = (1..vertices.len().saturating_sub(1)).find_map(|i| {
            let n = (vertices[i] - a).cross(vertices[i + 1] - a);
            (n.length_squared() > 1e-20).then(|| n.normalize())
        })?;
        Some(Self {
            w: normal.dot(a),
            normal,
            vertices,
        })
    }
    fn flip(&mut self) {
        self.vertices.reverse();
        self.normal = -self.normal;
        self.w = -self.w;
    }
}
#[derive(Clone, Copy)]
struct Plane {
    normal: DVec3,
    w: f64,
}
impl Plane {
    fn side(self, p: DVec3) -> u8 {
        let d = self.normal.dot(p) - self.w;
        if d < -EPS {
            2
        } else if d > EPS {
            1
        } else {
            0
        }
    }
    fn split(
        self,
        polygon: Polygon,
        coplanar_front: &mut Vec<Polygon>,
        coplanar_back: &mut Vec<Polygon>,
        front: &mut Vec<Polygon>,
        back: &mut Vec<Polygon>,
    ) {
        let sides: Vec<u8> = polygon.vertices.iter().map(|&v| self.side(v)).collect();
        let side = sides.iter().fold(0, |all, s| all | s);
        match side {
            0 => {
                if self.normal.dot(polygon.normal) > 0. {
                    coplanar_front.push(polygon)
                } else {
                    coplanar_back.push(polygon)
                }
            }
            1 => front.push(polygon),
            2 => back.push(polygon),
            _ => {
                let mut f = Vec::new();
                let mut b = Vec::new();
                for i in 0..polygon.vertices.len() {
                    let j = (i + 1) % polygon.vertices.len();
                    let a = polygon.vertices[i];
                    let next = polygon.vertices[j];
                    if sides[i] != 2 {
                        f.push(a);
                    }
                    if sides[i] != 1 {
                        b.push(a);
                    }
                    if sides[i] | sides[j] == 3 {
                        let t = (self.w - self.normal.dot(a)) / self.normal.dot(next - a);
                        let point = a + (next - a) * t.clamp(0., 1.);
                        f.push(point);
                        b.push(point);
                    }
                }
                if let Some(p) = Polygon::new(f) {
                    front.push(p);
                }
                if let Some(p) = Polygon::new(b) {
                    back.push(p);
                }
            }
        }
    }
}
#[derive(Default)]
struct Node {
    plane: Option<Plane>,
    polygons: Vec<Polygon>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
}
impl Node {
    fn contains(&self, point: DVec3) -> bool {
        let Some(plane) = self.plane else {
            return false;
        };
        let front = || self.front.as_ref().is_some_and(|node| node.contains(point));
        let back = || self.back.as_ref().is_none_or(|node| node.contains(point));
        match plane.side(point) {
            1 => front(),
            2 => back(),
            _ => front() && back(),
        }
    }
    fn build(&mut self, polygons: Vec<Polygon>, depth: usize) -> Result<(), &'static str> {
        if polygons.is_empty() {
            return Ok(());
        }
        if depth > DEPTH || polygons.len() > LIMIT {
            return Err("Fused surface complexity limit reached");
        }
        if self.plane.is_none() {
            // A balanced split avoids a linear tree when joining many repeated pieces.
            self.plane = [0, polygons.len() / 2, polygons.len() - 1]
                .into_iter()
                .map(|i| {
                    let p = &polygons[i];
                    let plane = Plane {
                        normal: p.normal,
                        w: p.w,
                    };
                    let mut front = 0usize;
                    let mut back = 0usize;
                    let mut split = 0;
                    for p in &polygons {
                        match p.vertices.iter().fold(0, |all, &v| all | plane.side(v)) {
                            1 => front += 1,
                            2 => back += 1,
                            3 => split += 1,
                            _ => {}
                        }
                    }
                    (split * 3 + front.abs_diff(back), plane)
                })
                .min_by_key(|(score, _)| *score)
                .map(|(_, plane)| plane);
        }
        let plane = self.plane.unwrap();
        let mut cf = Vec::new();
        let mut cb = Vec::new();
        let mut f = Vec::new();
        let mut b = Vec::new();
        for p in polygons {
            plane.split(p, &mut cf, &mut cb, &mut f, &mut b);
        }
        self.polygons.extend(cf);
        self.polygons.extend(cb);
        if !f.is_empty() {
            self.front
                .get_or_insert_with(Default::default)
                .build(f, depth + 1)?;
        }
        if !b.is_empty() {
            self.back
                .get_or_insert_with(Default::default)
                .build(b, depth + 1)?;
        }
        Ok(())
    }
    fn all(&self, out: &mut Vec<Polygon>) -> Result<(), &'static str> {
        out.extend(self.polygons.iter().cloned());
        if out.len() > LIMIT {
            return Err("Fused surface complexity limit reached");
        }
        if let Some(node) = &self.front {
            node.all(out)?;
        }
        if let Some(node) = &self.back {
            node.all(out)?;
        }
        Ok(())
    }
    fn invert(&mut self) {
        for polygon in &mut self.polygons {
            polygon.flip();
        }
        if let Some(plane) = &mut self.plane {
            plane.normal = -plane.normal;
            plane.w = -plane.w;
        }
        if let Some(node) = &mut self.front {
            node.invert();
        }
        if let Some(node) = &mut self.back {
            node.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }
    fn clip(&self, polygons: Vec<Polygon>, depth: usize) -> Result<Vec<Polygon>, &'static str> {
        if depth > DEPTH || polygons.len() > LIMIT {
            return Err("Fused surface complexity limit reached");
        }
        let Some(plane) = self.plane else {
            return Ok(polygons);
        };
        let mut cf = Vec::new();
        let mut cb = Vec::new();
        let mut f = Vec::new();
        let mut b = Vec::new();
        for p in polygons {
            plane.split(p, &mut cf, &mut cb, &mut f, &mut b);
        }
        f.extend(cf);
        b.extend(cb);
        if let Some(node) = &self.front {
            f = node.clip(f, depth + 1)?;
        }
        b = if let Some(node) = &self.back {
            node.clip(b, depth + 1)?
        } else {
            Vec::new()
        };
        f.extend(b);
        if f.len() > LIMIT {
            return Err("Fused surface complexity limit reached");
        }
        Ok(f)
    }
    fn clip_to(&mut self, other: &Node) -> Result<(), &'static str> {
        self.polygons = other.clip(std::mem::take(&mut self.polygons), 0)?;
        if let Some(node) = &mut self.front {
            node.clip_to(other)?;
        }
        if let Some(node) = &mut self.back {
            node.clip_to(other)?;
        }
        Ok(())
    }
}
pub(super) fn exterior_rails(
    triangles: &[[Vec3; 3]],
    rails: Vec<Vec<Vec3>>,
) -> Result<Vec<Vec<Vec3>>, &'static str> {
    if rails.is_empty() {
        return Ok(rails);
    }
    let mut tree = Node::default();
    tree.build(polygons(triangles), 0)?;
    let mut result = Vec::new();
    for rail in rails {
        for pair in rail.windows(2) {
            let a = pair[0].as_dvec3();
            let b = pair[1].as_dvec3();
            let delta = b - a;
            let mut cuts = vec![0., 1.];
            for t in triangles {
                let p = t[0].as_dvec3();
                let e = t[1].as_dvec3() - p;
                let f = t[2].as_dvec3() - p;
                let cross = delta.cross(f);
                let determinant = e.dot(cross);
                if determinant.abs() < 1e-14 {
                    continue;
                }
                let offset = a - p;
                let u = offset.dot(cross) / determinant;
                let q = offset.cross(e);
                let v = delta.dot(q) / determinant;
                let along = f.dot(q) / determinant;
                if u >= -EPS && v >= -EPS && u + v <= 1. + EPS && along > EPS && along < 1. - EPS {
                    cuts.push(along);
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup_by(|a, b| (*a - *b).abs() < EPS);
            let mut visible = Vec::new();
            for interval in cuts.windows(2) {
                if !tree.contains(a + delta * (interval[0] + interval[1]) * 0.5) {
                    let start = (a + delta * interval[0]).as_vec3();
                    let end = (a + delta * interval[1]).as_vec3();
                    if visible.is_empty() {
                        visible.push(start);
                    }
                    visible.push(end);
                } else if !visible.is_empty() {
                    result.push(std::mem::take(&mut visible));
                }
            }
            if !visible.is_empty() {
                result.push(visible);
            }
        }
    }
    Ok(result)
}
fn polygons(triangles: &[[Vec3; 3]]) -> Vec<Polygon> {
    triangles
        .iter()
        .filter_map(|t| Polygon::new(t.iter().map(|p| p.as_dvec3()).collect()))
        .collect()
}
pub(super) fn union(a: Vec<[Vec3; 3]>, b: Vec<[Vec3; 3]>) -> Result<Vec<[Vec3; 3]>, &'static str> {
    if a.is_empty() {
        return Ok(b);
    }
    if b.is_empty() {
        return Ok(a);
    }
    let bounds = |tris: &[[Vec3; 3]]| {
        tris.iter().flatten().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), &p| (min.min(p), max.max(p)),
        )
    };
    let (amin, amax) = bounds(&a);
    let (bmin, bmax) = bounds(&b);
    if (amin.cmpgt(bmax + Vec3::splat(EPS as f32)) | bmin.cmpgt(amax + Vec3::splat(EPS as f32)))
        .any()
    {
        let mut all = a;
        all.extend(b);
        return Ok(all);
    }
    let mut left = Node::default();
    let mut right = Node::default();
    left.build(polygons(&a), 0)?;
    right.build(polygons(&b), 0)?;
    left.clip_to(&right)?;
    right.clip_to(&left)?;
    right.invert();
    right.clip_to(&left)?;
    right.invert();
    let mut extra = Vec::new();
    right.all(&mut extra)?;
    left.build(extra, 0)?;
    let mut result = Vec::new();
    left.all(&mut result)?;
    let mut triangles = Vec::new();
    for polygon in result {
        for i in 1..polygon.vertices.len() - 1 {
            let t = [
                polygon.vertices[0].as_vec3(),
                polygon.vertices[i].as_vec3(),
                polygon.vertices[i + 1].as_vec3(),
            ];
            if (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-12 {
                triangles.push(t);
            }
        }
    }
    if triangles.len() > LIMIT {
        return Err("Fused surface complexity limit reached");
    }
    Ok(triangles)
}

#[cfg(test)]
mod tests {
    use super::super::{geometry, math};
    use super::*;
    use benilla_protocol::park::{Model, Pose};
    #[test]
    fn overlapping_boxes_have_union_volume_without_internal_faces() {
        let a = geometry::build(Model::Box).triangles;
        let matrix = math::transform(Pose {
            position: [2., 0., 0.],
            ..Pose::default()
        })
        .to_matrix();
        let b = geometry::build(Model::Box)
            .triangles
            .into_iter()
            .map(|t| t.map(|p| matrix.transform_point3(p)))
            .collect();
        let union = union(a, b).unwrap();
        let volume = union
            .iter()
            .map(|t| t[0].dot(t[1].cross(t[2])) / 6.)
            .sum::<f32>()
            .abs();
        assert!((volume - 36.).abs() < 0.001);
        assert!(union.iter().all(|t| {
            let centre = (t[0] + t[1] + t[2]) / 3.;
            !((centre.x.abs() < 0.001 || (centre.x - 2.).abs() < 0.001)
                && centre.y > 0.001
                && centre.y < 1.499
                && centre.z.abs() < 1.999)
        }));
    }
}
