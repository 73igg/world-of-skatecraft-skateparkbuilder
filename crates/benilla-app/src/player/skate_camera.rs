//! The native Skate framing, with a stable zoom anchor and a final full-world collision pass.
mod filter;
use benilla_world::{
    collision::WorldCollision,
    model_fade::{self_model_fade_alpha, SELF_FADE_WINDOW},
    view::{ViewDistance, WorldCamera},
};
use bevy::prelude::*;
use filter::{Follow, Goal};

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Follow>().add_systems(
        Update,
        seat.in_set(benilla_world::schedule::WorldStage::Input)
            .in_set(crate::char_select::InWorldGated)
            .run_if(not(resource_exists::<crate::run_mode::CaptureMode>))
            .after(super::controller::control)
            .after(crate::camera_shake::apply_camera_shake)
            .before(super::camera::apply_self_model_fade),
    );
}
fn seat(
    time: Res<Time>,
    drive: Res<crate::skate::SkateDrive>,
    mut follow: ResMut<Follow>,
    mut rig: ResMut<super::CameraControl>,
    options: Res<super::camera_dynamics::CameraOptions>,
    view: Res<ViewDistance>,
    collide: WorldCollision,
    mut cameras: Query<(&mut Transform, &Projection), With<WorldCamera>>,
    mut trace_at: Local<f32>,
) {
    let Some(native) = drive
        .camera
        .filter(|_| drive.active && drive.engine_camera && rig.skate_seated)
    else {
        follow.reset();
        return;
    };
    let Ok((mut camera, projection)) = cameras.single_mut() else {
        follow.reset();
        return;
    };
    let near = view.nearclip;
    let radius = match projection {
        Projection::Perspective(p) => {
            let t = (p.fov * 0.5).tan();
            (near.max(p.near) * (1. + t * t * (1. + p.aspect_ratio * p.aspect_ratio)).sqrt())
                .max(0.3)
        }
        _ => 0.3,
    };
    let shake = camera.translation - native.translation;
    let output = follow.seat(
        time.delta_secs(),
        Goal {
            root: drive.pos,
            camera: native,
            height: drive.camera_height,
            distance: drive.camera_distance,
            speed: drive.speed,
            radius,
            shake,
        },
        |p, d, r| collide.cast_camera_with_radius(p, d, r, options.water_collision),
    );
    *camera = output.transform;
    rig.clipped = output.clipped;
    rig.self_fade_alpha = self_model_fade_alpha(
        camera.translation.distance(output.pivot),
        near,
        SELF_FADE_WINDOW,
    );
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *TRACE.get_or_init(|| std::env::var_os("WOW_SKATE_CAMERA_TRACE").is_some())
        && time.elapsed_secs() - *trace_at >= 0.2
    {
        *trace_at = time.elapsed_secs();
        let eye = camera.translation;
        let contacts = [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ]
        .iter()
        .filter(|d| {
            collide
                .cast_camera_with_radius(eye, **d * 0.002, radius, false)
                .is_some_and(|distance| distance < 0.001)
        })
        .count();
        let ground = collide
            .ray_body(eye, Dir3::NEG_Y, 100.)
            .map(|hit| hit.distance);
        info!(
            "skate camera: distance={:.2} speed={:.2} radius={radius:.4} eye={eye:?} pivot={:?} clipped={} contacts={contacts} ground={ground:?} fade={:.3}",
            drive.camera_distance, drive.speed, output.pivot, output.clipped, rig.self_fade_alpha,
        );
    }
}
