use super::State;
use benilla_assets::coords::wow_to_bevy;
use benilla_world::world_map::CurrentMap;
use bevy::prelude::*;

#[derive(Component)]
struct LocalFlag;
fn show(
    mut commands: Commands,
    state: Res<State>,
    drive: Res<super::super::SkateDrive>,
    map: Option<Res<CurrentMap>>,
    park: Res<crate::park::ParkWorld>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut markers: Query<(Entity, &mut Transform, &mut Visibility), With<LocalFlag>>,
) {
    let visible = state.saved.filter(|f| {
        drive.active && map.as_ref().is_some_and(|m| m.0 == f.map) && f.instance == park.scope().1
    });
    if let Ok((_, mut transform, mut visibility)) = markers.single_mut() {
        *visibility = if visible.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(flag) = visible {
            *transform = Transform::from_translation(wow_to_bevy(flag.position))
                .with_rotation(Quat::from_rotation_y(flag.facing));
        }
        return;
    }
    let Some(flag) = visible else {
        return;
    };
    let pole = meshes.add(Cuboid::new(0.025, 0.95, 0.025));
    let banner = meshes.add(Cuboid::new(0.38, 0.23, 0.015));
    let base = meshes.add(Cylinder::new(0.12, 0.035));
    let orange = materials.add(StandardMaterial {
        base_color: Color::srgb(1., 0.48, 0.06),
        unlit: true,
        ..default()
    });
    let dark = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.12, 0.12),
        unlit: true,
        ..default()
    });
    // No collider or network identity: each client draws only its own saved point.
    commands
        .spawn((
            LocalFlag,
            Name::new("Your Skate flag"),
            Transform::from_translation(wow_to_bevy(flag.position))
                .with_rotation(Quat::from_rotation_y(flag.facing)),
            Visibility::Visible,
        ))
        .with_children(|p| {
            p.spawn((
                Mesh3d(pole),
                MeshMaterial3d(dark.clone()),
                Transform::from_xyz(0.55, 0.475, 0.),
            ));
            p.spawn((
                Mesh3d(banner),
                MeshMaterial3d(orange),
                Transform::from_xyz(0.74, 0.79, 0.),
            ));
            p.spawn((
                Mesh3d(base),
                MeshMaterial3d(dark),
                Transform::from_xyz(0.55, 0.0175, 0.),
            ));
        });
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        show.in_set(benilla_world::schedule::WorldStage::Present),
    );
}
