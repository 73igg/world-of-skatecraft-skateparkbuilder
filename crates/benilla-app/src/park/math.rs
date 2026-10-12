//! Transform drags are evaluated from their initial pose so movement never accumulates drift.
use benilla_protocol::park::{
    Definition, Model, Part, Pose, MAX_PARTS_PER_OBJECT, MAX_SCALE, MIN_SCALE,
};
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tool {
    #[default]
    Move,
    Rotate,
    Scale,
}
pub(super) const AXES: [Vec3; 3] = [Vec3::X, Vec3::NEG_Z, Vec3::Y]; // Blender-style Z up.
pub(super) fn transform(pose: Pose) -> Transform {
    Transform {
        translation: Vec3::from_array(pose.position),
        rotation: Quat::from_array(pose.rotation),
        scale: Vec3::from_array(pose.scale),
    }
}
pub(super) fn pose(t: Transform) -> Pose {
    Pose {
        position: t.translation.to_array(),
        rotation: t.rotation.normalize().to_array(),
        scale: t.scale.to_array(),
    }
}

pub(super) fn part_matrix(part: &Part) -> Mat4 {
    let a = part.affine;
    Mat4::from_cols(
        Vec4::new(a[0], a[1], a[2], 0.),
        Vec4::new(a[3], a[4], a[5], 0.),
        Vec4::new(a[6], a[7], a[8], 0.),
        Vec4::new(a[9], a[10], a[11], 1.),
    )
}
pub(super) fn part(model: Model, asset: u64, matrix: Mat4) -> Part {
    let x = matrix.x_axis;
    let y = matrix.y_axis;
    let z = matrix.z_axis;
    let p = matrix.w_axis;
    Part {
        model,
        asset,
        affine: [x.x, x.y, x.z, y.x, y.y, y.z, z.x, z.y, z.z, p.x, p.y, p.z],
    }
}
pub(super) fn world_parts(definition: &Definition) -> Vec<Part> {
    let root = transform(definition.pose).to_matrix();
    if definition.model == Model::Compound {
        definition
            .parts
            .iter()
            .map(|piece| part(piece.model, piece.asset, root * part_matrix(piece)))
            .collect()
    } else {
        vec![part(definition.model, definition.asset, root)]
    }
}
pub(super) fn fuse(definitions: &[Definition]) -> Result<Definition, &'static str> {
    if definitions.len() < 2 {
        return Err("Select at least two objects to fuse");
    }
    let centre = definitions
        .iter()
        .fold(Vec3::ZERO, |sum, d| sum + Vec3::from_array(d.pose.position))
        / definitions.len() as f32;
    let pose = Pose {
        position: centre.to_array(),
        ..default()
    };
    let inverse = transform(pose).to_matrix().inverse();
    let parts: Vec<Part> = definitions
        .iter()
        .flat_map(world_parts)
        .map(|piece| part(piece.model, piece.asset, inverse * part_matrix(&piece)))
        .collect();
    if parts.len() > MAX_PARTS_PER_OBJECT {
        return Err("A fused object supports up to 256 pieces");
    }
    let fused = Definition {
        model: Model::Compound,
        asset: 0,
        pose,
        parts,
    };
    fused
        .validate()
        .map_err(|_| "Fused object exceeds the supported transform range")?;
    Ok(fused)
}
pub(super) fn transformed(definition: &Definition, delta: Mat4) -> Definition {
    let matrix = delta * transform(definition.pose).to_matrix();
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    let candidate = Pose {
        position: translation.to_array(),
        rotation: rotation.normalize().to_array(),
        scale: scale.to_array(),
    };
    let rebuilt = transform(candidate).to_matrix();
    let error = matrix
        .to_cols_array()
        .into_iter()
        .zip(rebuilt.to_cols_array())
        .fold(0f32, |error, (a, b)| error.max((a - b).abs()));
    if candidate.validate().is_ok() && error < 0.0001 {
        return Definition {
            pose: candidate,
            ..definition.clone()
        };
    }
    // Bake an affine residual when a selection's axis resize creates shear.
    let pose = Pose {
        position: translation.to_array(),
        ..default()
    };
    let inverse = transform(pose).to_matrix().inverse();
    let parts = world_parts(definition)
        .into_iter()
        .map(|piece| {
            part(
                piece.model,
                piece.asset,
                inverse * delta * part_matrix(&piece),
            )
        })
        .collect();
    Definition {
        model: Model::Compound,
        asset: 0,
        pose,
        parts,
    }
}

pub(super) fn screen_ray(
    cursor: Vec2,
    size: Vec2,
    camera: Transform,
    projection: &Projection,
) -> Option<Ray3d> {
    let Projection::Perspective(lens) = projection else {
        return None;
    };
    if size.min_element() <= 0. {
        return None;
    }
    let ndc = Vec2::new(cursor.x / size.x * 2. - 1., 1. - cursor.y / size.y * 2.);
    let tangent = (lens.fov * 0.5).tan();
    let local = Vec3::new(ndc.x * tangent * size.x / size.y, ndc.y * tangent, -1.);
    Some(Ray3d::new(
        camera.translation,
        Dir3::new(camera.rotation * local).ok()?,
    ))
}
pub(super) fn screen_point(
    point: Vec3,
    size: Vec2,
    camera: Transform,
    projection: &Projection,
) -> Option<Vec2> {
    let Projection::Perspective(lens) = projection else {
        return None;
    };
    let local = camera.rotation.inverse() * (point - camera.translation);
    if local.z >= -0.001 {
        return None;
    }
    let tangent = (lens.fov * 0.5).tan();
    let ndc = Vec2::new(
        local.x / (-local.z * tangent * size.x / size.y),
        local.y / (-local.z * tangent),
    );
    Some(Vec2::new(
        (ndc.x + 1.) * size.x * 0.5,
        (1. - ndc.y) * size.y * 0.5,
    ))
}

pub(super) fn ray_plane(ray: Ray3d, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denominator = ray.direction.dot(normal);
    if denominator.abs() < 1e-5 {
        return None;
    }
    let t = (point - ray.origin).dot(normal) / denominator;
    (t.is_finite() && t >= 0.).then(|| ray.origin + *ray.direction * t)
}
pub(super) fn drag_plane(axis: Option<Vec3>, view_forward: Vec3) -> Vec3 {
    axis.map_or(view_forward, |axis| {
        let side = axis.cross(view_forward);
        let normal = side.cross(axis).normalize_or_zero();
        if normal.length_squared() > 0.5 {
            normal
        } else {
            axis.any_orthonormal_vector()
        }
    })
}
pub(super) fn apply(
    start: Pose,
    tool: Tool,
    axis: Option<usize>,
    local: bool,
    amount: f32,
    view_axis: Vec3,
) -> Pose {
    let mut t = transform(start);
    let direction = axis.map_or(view_axis, |i| {
        if local || tool == Tool::Scale {
            t.rotation * AXES[i]
        } else {
            AXES[i]
        }
    });
    match tool {
        Tool::Move => t.translation += direction * amount,
        Tool::Rotate => {
            t.rotation = (Quat::from_axis_angle(direction.normalize_or_zero(), amount) * t.rotation)
                .normalize()
        }
        Tool::Scale => {
            let factor = amount.max(0.0001);
            if let Some(i) = axis {
                let slot = [0, 2, 1][i];
                t.scale[slot] = (t.scale[slot] * factor).clamp(MIN_SCALE, MAX_SCALE);
            } else {
                t.scale = (t.scale * factor).clamp(Vec3::splat(MIN_SCALE), Vec3::splat(MAX_SCALE));
            }
        }
    }
    pose(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_fusion_preserves_rotated_nonuniform_affine_geometry() {
        let a = Definition::primitive(
            Model::Box,
            Pose {
                position: [10., 2., 7.],
                rotation: Quat::from_rotation_y(0.7).to_array(),
                scale: [2., 0.5, 3.],
            },
        );
        let b = Definition::primitive(
            Model::Ramp,
            Pose {
                position: [15., 2., 7.],
                ..default()
            },
        );
        let fused = fuse(&[a.clone(), b.clone()]).unwrap();
        let moved = transformed(&fused, Mat4::from_scale(Vec3::new(2., 1., 0.5)));
        let again = fuse(&[moved.clone(), b]).unwrap();
        let expected = world_parts(&moved);
        let actual = world_parts(&again);
        for (a, b) in expected.iter().zip(actual.iter()) {
            assert!(
                part_matrix(a)
                    .transform_point3(Vec3::new(1., 2., 3.))
                    .distance(part_matrix(b).transform_point3(Vec3::new(1., 2., 3.)))
                    < 0.0001
            );
        }
        assert!(again.validate().is_ok());
    }
    #[test]
    fn transformed_local_move_and_world_move_use_distinct_axes() {
        let p = Pose {
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array(),
            ..default()
        };
        assert!(
            (Vec3::from_array(apply(p, Tool::Move, Some(0), false, 2., Vec3::X).position)
                - Vec3::new(2., 0., 0.))
            .length()
                < 1e-5
        );
        assert!(
            (Vec3::from_array(apply(p, Tool::Move, Some(0), true, 2., Vec3::X).position)
                - Vec3::new(0., 0., -2.))
            .length()
                < 1e-5
        );
    }
    #[test]
    fn scale_z_changes_height_and_never_becomes_zero_or_negative() {
        let p = apply(Pose::default(), Tool::Scale, Some(2), true, 2., Vec3::X);
        assert_eq!(p.scale, [1., 2., 1.]);
        assert!(apply(p, Tool::Scale, None, false, -5., Vec3::X)
            .validate()
            .is_ok());
    }
    #[test]
    fn rotations_preserve_position_and_normalization() {
        let p = Pose {
            position: [1., 2., 3.],
            ..default()
        };
        let rotated = apply(p, Tool::Rotate, Some(2), false, 23., Vec3::Y);
        assert_eq!(rotated.position, p.position);
        assert!(rotated.validate().is_ok());
    }
    #[test]
    fn parallel_or_behind_camera_drag_planes_do_not_jump() {
        let ray = Ray3d::new(Vec3::ZERO, Dir3::Z);
        assert!(ray_plane(ray, Vec3::NEG_Z, Vec3::Z).is_none());
        assert!(ray_plane(ray, Vec3::X, Vec3::X).is_none());
    }

    #[test]
    fn cursor_projection_matches_rendered_points_at_different_window_sizes() {
        let camera = Transform::from_xyz(3., 8., 20.).looking_at(Vec3::ZERO, Vec3::Y);
        let lens = Projection::Perspective(PerspectiveProjection::default());
        let point = Vec3::new(2., 1., 0.);
        for size in [Vec2::new(640., 360.), Vec2::new(1920., 1080.)] {
            let cursor = screen_point(point, size, camera, &lens).unwrap();
            let ray = screen_ray(cursor, size, camera, &lens).unwrap();
            let hit = ray_plane(ray, point, Vec3::Z).unwrap();
            assert!(hit.distance(point) < 1e-4);
        }
    }
}
