//! Weld nearby authored boundary edges and split T-junctions before native edge inference.
use super::geometry::Geometry;
use bevy::prelude::*;
use std::collections::HashMap;

const WELD: f32 = 0.01 / 0.9144; // One centimetre, expressed in editor yards.
const EPS: f32 = 0.00001;
fn key(point: Vec3) -> [i64; 3] {
    point
        .to_array()
        .map(|v| (f64::from(v) / f64::from(EPS)).round() as i64)
}
fn boundaries(geometry: &Geometry) -> Vec<(Vec3, Vec3)> {
    type EdgeKey = ([i64; 3], [i64; 3]);
    type EdgeFaces = (Vec3, Vec3, Vec<Vec3>);
    let mut edges: HashMap<EdgeKey, EdgeFaces> = HashMap::new();
    for t in &geometry.triangles {
        let normal = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        for i in 0..3 {
            let mut a = t[i];
            let mut b = t[(i + 1) % 3];
            let mut ka = key(a);
            let mut kb = key(b);
            if ka > kb {
                std::mem::swap(&mut a, &mut b);
                std::mem::swap(&mut ka, &mut kb);
            }
            edges
                .entry((ka, kb))
                .or_insert_with(|| (a, b, Vec::new()))
                .2
                .push(normal);
        }
    }
    let mut edges = edges
        .into_iter()
        .filter_map(|(k, (a, b, n))| {
            ((b - a).length() > WELD * 4.
                && (n.len() == 1 || n.iter().any(|normal| normal.dot(n[0]) < 0.99)))
            .then_some((k, a, b))
        })
        .collect::<Vec<_>>();
    edges.sort_by_key(|(key, _, _)| *key);
    edges.into_iter().map(|(_, a, b)| (a, b)).collect()
}
pub(super) fn weld(pieces: &mut [Geometry]) {
    let mut anchors = Vec::<(Vec3, Vec3)>::new();
    for geometry in pieces {
        let (min, max) = geometry.triangles.iter().flatten().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), &p| (min.min(p), max.max(p)),
        );
        let mut moves = HashMap::<[i64; 3], Vec3>::new();
        if (max - min).min_element() > WELD * 2. {
            for (a, b) in boundaries(geometry) {
                let direction = (b - a).normalize();
                let mut best = None;
                let mut distance = WELD;
                for &(start, end) in &anchors {
                    let axis = (end - start).normalize();
                    if direction.dot(axis).abs() < 0.999 {
                        continue;
                    }
                    let length = (end - start).length();
                    let ta = (a - start).dot(axis);
                    let tb = (b - start).dot(axis);
                    // Endpoint-only contact must not flatten a neighbouring curve segment.
                    if ta.max(tb).min(length) - ta.min(tb).max(0.) <= WELD * 0.25 {
                        continue;
                    }
                    let pa = start + axis * ta;
                    let pb = start + axis * tb;
                    let error = a.distance(pa).max(b.distance(pb));
                    if error < distance {
                        distance = error;
                        best = Some((pa, pb));
                    }
                }
                if let Some((pa, pb)) = best {
                    moves.entry(key(a)).or_insert(pa);
                    moves.entry(key(b)).or_insert(pb);
                }
            }
        }
        for p in geometry.triangles.iter_mut().flatten() {
            if let Some(&next) = moves.get(&key(*p)) {
                *p = next;
            }
        }
        for p in geometry.rails.iter_mut().flatten() {
            if let Some(&next) = moves.get(&key(*p)) {
                *p = next;
            }
        }
        geometry
            .triangles
            .retain(|t| (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-12);
        anchors.extend(boundaries(geometry));
    }
}

struct Points {
    min: Vec3,
    max: Vec3,
    leaf: Vec<Vec3>,
    children: Option<(Box<Points>, Box<Points>)>,
}
impl Points {
    fn build(mut points: Vec<Vec3>) -> Self {
        let (min, max) = points.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), &p| (min.min(p), max.max(p)),
        );
        if points.len() <= 24 {
            return Self {
                min,
                max,
                leaf: points,
                children: None,
            };
        }
        let size = max - min;
        let axis = if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.z {
            1
        } else {
            2
        };
        points.sort_by(|a, b| {
            a[axis]
                .total_cmp(&b[axis])
                .then(a.x.total_cmp(&b.x))
                .then(a.y.total_cmp(&b.y))
                .then(a.z.total_cmp(&b.z))
        });
        let right = points.split_off(points.len() / 2);
        Self {
            min,
            max,
            leaf: Vec::new(),
            children: Some((Box::new(Self::build(points)), Box::new(Self::build(right)))),
        }
    }
    fn query(&self, min: Vec3, max: Vec3, out: &mut Vec<Vec3>) {
        if (self.min.cmpgt(max) | self.max.cmplt(min)).any() {
            return;
        }
        if let Some((left, right)) = &self.children {
            left.query(min, max, out);
            right.query(min, max, out);
        } else {
            out.extend(
                self.leaf
                    .iter()
                    .copied()
                    .filter(|p| !(p.cmplt(min) | p.cmpgt(max)).any()),
            );
        }
    }
}
pub(super) fn stitch(triangles: Vec<[Vec3; 3]>) -> Result<Vec<[Vec3; 3]>, &'static str> {
    if triangles.is_empty() {
        return Ok(triangles);
    }
    let mut canonical = HashMap::new();
    for &point in triangles.iter().flatten() {
        canonical.entry(key(point)).or_insert(point);
    }
    let points = Points::build(canonical.values().copied().collect());
    let mut result = Vec::new();
    for triangle in triangles {
        let triangle = triangle.map(|p| canonical[&key(p)]);
        if (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .length_squared()
            <= 1e-12
        {
            continue;
        }
        let mut polygon = Vec::new();
        let mut extra = false;
        for i in 0..3 {
            let a = triangle[i];
            let b = triangle[(i + 1) % 3];
            let delta = b - a;
            let square = delta.length_squared();
            let mut candidates = Vec::new();
            points.query(
                a.min(b) - Vec3::splat(EPS),
                a.max(b) + Vec3::splat(EPS),
                &mut candidates,
            );
            let mut edge = vec![(0., a)];
            for p in candidates {
                let t = (p - a).dot(delta) / square;
                if t > EPS && t < 1. - EPS && p.distance_squared(a + delta * t) < EPS * EPS {
                    edge.push((t, p));
                    extra = true;
                }
            }
            edge.sort_by(|a, b| a.0.total_cmp(&b.0));
            edge.dedup_by(|a, b| key(a.1) == key(b.1));
            polygon.extend(edge.into_iter().map(|(_, p)| p));
        }
        if extra {
            let centre = (triangle[0] + triangle[1] + triangle[2]) / 3.;
            for i in 0..polygon.len() {
                let t = [centre, polygon[i], polygon[(i + 1) % polygon.len()]];
                if (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-12 {
                    result.push(t);
                }
            }
        } else {
            result.push(triangle);
        }
        if result.len() > 600_000 {
            return Err("Fused surface complexity limit reached");
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use benilla_protocol::park::Model;
    #[test]
    fn weld_closes_a_small_parallel_join_without_moving_far_edges() {
        let a = super::super::geometry::build(Model::Box);
        let mut b = a.clone();
        for point in b.triangles.iter_mut().flatten() {
            point.x += 4.004;
        }
        let mut pieces = [a, b];
        weld(&mut pieces);
        assert!(pieces[1]
            .triangles
            .iter()
            .flatten()
            .any(|p| (p.x - 2.).abs() < EPS));
        assert!(pieces[1]
            .triangles
            .iter()
            .flatten()
            .any(|p| (p.x - 6.004).abs() < EPS));
    }
    #[test]
    fn a_t_junction_is_split_into_matching_contact_edges() {
        let a = Vec3::ZERO;
        let b = Vec3::new(2., 0., 0.);
        let c = Vec3::new(0., 0., 2.);
        let mid = Vec3::new(1., 0., 0.);
        let triangles = stitch(vec![[a, c, b], [mid, b, Vec3::new(1., 0., -1.)]]).unwrap();
        assert!(triangles.iter().any(|t| t.contains(&a) && t.contains(&mid)));
        assert!(triangles.iter().any(|t| t.contains(&mid) && t.contains(&b)));
        assert!(!triangles.iter().any(|t| t.contains(&a) && t.contains(&b)));
    }
    #[test]
    fn touching_endpoints_do_not_flatten_the_next_curve_segment() {
        let mut left = super::super::geometry::build(Model::QuarterPipe);
        let mut right = left.clone();
        let rotation = Quat::from_rotation_y(std::f32::consts::PI);
        for point in left.triangles.iter_mut().flatten() {
            *point = rotation * *point + Vec3::new(0., 0., -2.);
        }
        for point in right.triangles.iter_mut().flatten() {
            point.z += 2.;
        }
        let height = 4. * (1. - (std::f32::consts::FRAC_PI_2 / 32.).cos());
        let mut pieces = [left, right];
        weld(&mut pieces);
        assert!(pieces[1]
            .triangles
            .iter()
            .flatten()
            .any(|p| (p.y - height).abs() < EPS));
    }
}
