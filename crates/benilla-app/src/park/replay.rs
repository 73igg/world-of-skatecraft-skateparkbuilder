//! Historical park visuals are independent of authoritative objects and their RPCs.
use super::{geometry, library, math, scene};
use benilla_protocol::park::{Model, Object};
use bevy::prelude::*;
use std::sync::Arc;
#[derive(Resource, Default)]
pub(crate) struct ReplaySceneReady {
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
}
#[derive(Resource, Default)]
struct View {
    objects: Option<Arc<[Object]>>,
    root: Option<Entity>,
    pending: Vec<Object>,
    hidden: Vec<(Entity, Visibility)>,
}
#[allow(clippy::too_many_arguments)]
fn draw(
    mut commands: Commands,
    replay: Res<crate::skate::replay::Replay>,
    mut view: ResMut<View>,
    mut ready: ResMut<ReplaySceneReady>,
    catalog: Res<scene::Catalog>,
    mut library: ResMut<library::Library>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut live: Query<(Entity, &mut Visibility), With<scene::ParkProp>>,
) {
    if replay.active {
        for (entity, mut visibility) in &mut live {
            if !view.hidden.iter().any(|(e, _)| *e == entity) {
                view.hidden.push((entity, *visibility));
            }
            *visibility = Visibility::Hidden;
        }
    } else {
        for (entity, value) in view.hidden.drain(..) {
            if let Ok((_, mut visibility)) = live.get_mut(entity) {
                *visibility = value;
            }
        }
    }
    let objects = replay.park_objects();
    let unchanged = match (&view.objects, &objects) {
        (Some(old), Some(new)) => Arc::ptr_eq(old, new),
        (None, None) => true,
        _ => false,
    };
    if unchanged && view.pending.is_empty() {
        ready.loading = false;
        return;
    }
    if !unchanged {
        ready.error = None;
        ready.loading = objects.is_some();
        if let Some(root) = view.root.take() {
            commands.entity(root).despawn();
        }
        view.objects = objects.clone();
        view.pending = objects.as_ref().map_or_else(Vec::new, |o| o.to_vec());
        let Some(_) = objects else {
            return;
        };
        let root = commands
            .spawn((
                Name::new("recorded park visuals"),
                Transform::default(),
                Visibility::Inherited,
            ))
            .id();
        view.root = Some(root);
    }
    let Some(root) = view.root else {
        return;
    };
    for object in std::mem::take(&mut view.pending) {
        let definition = object.definition();
        if object.model == Model::Asset {
            library.request(object.asset);
        }
        for part in &object.parts {
            if part.model == Model::Asset {
                library.request(part.asset);
            }
        }
        if object.model == Model::Asset || object.parts.iter().any(|p| p.model == Model::Asset) {
            if let Some(parts) =
                library::render_parts(&definition, &mut library, &catalog, &mut meshes)
            {
                let piece = commands
                    .spawn((
                        math::transform(object.pose),
                        Visibility::Inherited,
                        ChildOf(root),
                    ))
                    .id();
                for (mesh, material) in parts {
                    commands.spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        Transform::default(),
                        ChildOf(piece),
                    ));
                }
            } else {
                let missing = std::iter::once(object.asset)
                    .chain(object.parts.iter().map(|p| p.asset))
                    .find(|id| library.errors.contains_key(id));
                if let Some(id) = missing {
                    ready.error=Some(format!("Recorded object {id:016x} is unavailable; check your local game conversions"));
                }
                view.pending.push(object);
            }
        } else if let Ok(geometry) = geometry::definition(&definition) {
            let mesh = if object.model == Model::Compound {
                meshes.add(geometry.mesh())
            } else {
                catalog.pieces[object.model as usize - 1].mesh.clone()
            };
            let material = catalog.pieces[if object.model == Model::Compound {
                3
            } else {
                object.model as usize - 1
            }]
            .material
            .clone();
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                math::transform(object.pose),
                ChildOf(root),
            ));
        }
    }
    ready.loading = !view.pending.is_empty();
}
pub(super) fn plugin(app: &mut App) {
    app.init_resource::<View>()
        .init_resource::<ReplaySceneReady>()
        .add_systems(
            Update,
            draw.after(scene::sync)
                .after(crate::skate::replay::ReplayDrawSet),
        );
}
