use super::State;
use benilla_assets::coords::wow_to_bevy;
use benilla_world::world_map::CurrentMap;
use bevy::prelude::*;

#[derive(Component)]
struct LocalFlag;
fn show(
    mut commands: Commands,
    state: Res<State>,
    displays: Res<super::super::visibility::SkateVisibility>,
    drive: Res<super::super::SkateDrive>,
    map: Option<Res<CurrentMap>>,
    park: Res<crate::park::ParkWorld>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut markers: Query<(Entity, &mut Transform, &mut Visibility), With<LocalFlag>>,
) {
    let visible = state.saved.filter(|f| {
        displays.show_flag
            && drive.active
            && map.as_ref().is_some_and(|m| m.0 == f.map)
            && f.instance == park.scope().1
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skate::{visibility::SkateVisibility, SkateDrive};

    #[test]
    fn hiding_the_marker_preserves_the_return_point_and_restores_the_same_entity() {
        let flag = benilla_protocol::skate_flag::Flag {
            revision: 1,
            map: 0,
            instance: 0,
            zone: 12,
            position: [1., 2., 3.],
            facing: 0.5,
        };
        let mut app = App::new();
        app.init_resource::<State>()
            .insert_resource(SkateDrive {
                active: true,
                ..default()
            })
            .insert_resource(CurrentMap(0))
            .init_resource::<crate::park::ParkWorld>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .insert_resource(SkateVisibility {
                show_score: true,
                show_flag: false,
            })
            .add_systems(Update, show);
        app.world_mut().resource_mut::<State>().saved = Some(flag);
        app.update();
        let mut markers = app
            .world_mut()
            .query_filtered::<(Entity, &Visibility), With<LocalFlag>>();
        assert_eq!(markers.iter(app.world()).count(), 0);
        app.world_mut().resource_mut::<SkateVisibility>().show_flag = true;
        app.update();
        let entity = markers.single(app.world()).unwrap().0;
        assert_eq!(markers.single(app.world()).unwrap().1, &Visibility::Visible);
        app.world_mut().resource_mut::<SkateVisibility>().show_flag = false;
        app.update();
        assert_eq!(markers.single(app.world()).unwrap().1, &Visibility::Hidden);
        assert_eq!(app.world().resource::<State>().saved, Some(flag));
        assert!(app.world().resource::<State>().may_resume(Vec3::ZERO));
        app.world_mut().resource_mut::<SkateVisibility>().show_flag = true;
        app.update();
        assert_eq!(markers.single(app.world()).unwrap().0, entity);
        assert_eq!(markers.single(app.world()).unwrap().1, &Visibility::Visible);
        app.world_mut().resource_mut::<SkateDrive>().active = false;
        app.update();
        assert_eq!(markers.single(app.world()).unwrap().1, &Visibility::Hidden);
    }
}
