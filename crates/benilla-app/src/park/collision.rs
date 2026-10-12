//! Keep solid undersides, but suppress contact edges where a park shell meets the ground.
use bevy::prelude::*;
use std::collections::HashMap;

type Tri = [[f32; 3]; 3];
const CELL: f32 = 8.;
// Engine-space metres: small placement gaps should not create an underside contact ledge.
const JOIN_GAP: f32 = 0.025;
const EMBED_DEPTH: f32 = 0.5;

struct Ground {
    triangles: Vec<[Vec3; 3]>,
    cells: HashMap<(i32, i32), Vec<usize>>,
}
impl Ground {
    fn new(world: &[Tri]) -> Self {
        let mut ground = Self {
            triangles: Vec::new(),
            cells: HashMap::new(),
        };
        for source in world {
            let t = source.map(Vec3::from_array);
            if (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero().y < 0.15 {
                continue;
            }
            let min = t[0].min(t[1]).min(t[2]);
            let max = t[0].max(t[1]).max(t[2]);
            let index = ground.triangles.len();
            ground.triangles.push(t);
            for x in (min.x / CELL).floor() as i32..=(max.x / CELL).floor() as i32 {
                for z in (min.z / CELL).floor() as i32..=(max.z / CELL).floor() as i32 {
                    ground.cells.entry((x, z)).or_default().push(index);
                }
            }
        }
        ground
    }
    fn supports(&self, point: Vec3) -> bool {
        let key = (
            (point.x / CELL).floor() as i32,
            (point.z / CELL).floor() as i32,
        );
        self.cells.get(&key).is_some_and(|candidates| {
            candidates.iter().any(|&index| {
                let [a, b, c] = self.triangles[index];
                let e = b - a;
                let f = c - a;
                let p = point - a;
                let determinant = e.x * f.z - e.z * f.x;
                if determinant.abs() < 1e-8 {
                    return false;
                }
                let u = (p.x * f.z - p.z * f.x) / determinant;
                let v = (e.x * p.z - e.z * p.x) / determinant;
                if u < -0.0001 || v < -0.0001 || u + v > 1.0001 {
                    return false;
                }
                let height = a.y + e.y * u + f.y * v;
                height >= point.y - JOIN_GAP && height <= point.y + EMBED_DEPTH
            })
        })
    }
}

pub(crate) fn clean_skate_contacts(world: &[Tri], props: Vec<Tri>) -> Vec<Tri> {
    if props.is_empty() {
        return props;
    }
    let ground = Ground::new(world);
    props
        .into_iter()
        .filter(|source| {
            let [a, b, c] = source.map(Vec3::from_array);
            let cross = (b - a).cross(c - a);
            // Boolean splits and large world coordinates may collapse a submillimetre sliver.
            if !cross.is_finite() || cross.length_squared() <= 1e-12 {
                return false;
            }
            let normal = cross.normalize();
            if normal.y >= -0.15 {
                return true;
            }
            // Coverage at corners, edge midpoints and centroid prevents deleting a raised overhang.
            ![
                a,
                b,
                c,
                (a + b) * 0.5,
                (b + c) * 0.5,
                (c + a) * 0.5,
                (a + b + c) / 3.,
            ]
            .into_iter()
            .all(|point| ground.supports(point))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn floor() -> Vec<Tri> {
        vec![
            [[-10., 0., -10.], [-10., 0., 10.], [10., 0., 10.]],
            [[-10., 0., -10.], [10., 0., 10.], [10., 0., -10.]],
        ]
    }
    #[test]
    fn flush_and_buried_shell_bottoms_do_not_become_contact_ledges() {
        for y in [0., 0.01, -0.2] {
            let bottom = [[-1., y, -1.], [1., y, -1.], [1., y, 1.]];
            assert!(clean_skate_contacts(&floor(), vec![bottom]).is_empty());
        }
    }
    #[test]
    fn floating_undersides_and_vertical_walls_keep_collision() {
        let underside = [[-1., 2., -1.], [1., 2., -1.], [1., 2., 1.]];
        let wall = [[-1., 0., -1.], [-1., 2., -1.], [1., 2., -1.]];
        assert_eq!(
            clean_skate_contacts(&floor(), vec![underside, wall]),
            vec![underside, wall]
        );
    }
    #[test]
    fn an_underside_extending_past_its_ground_support_is_preserved() {
        let underside = [[-1., 0., -1.], [11., 0., -1.], [11., 0., 1.]];
        assert_eq!(
            clean_skate_contacts(&floor(), vec![underside]),
            vec![underside]
        );
    }
}
