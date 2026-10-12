use super::{
    geometry, library, math,
    state::{Key, ParkEditor, ParkWorld},
};
use avian3d::prelude::RigidBody;
use benilla_protocol::park::{Definition, Model, Object, Pose};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, ClearColorConfig};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use std::collections::{HashMap, HashSet};

pub(super) const OVERLAY_LAYER: usize = 27;
#[derive(Component)]
pub(crate) struct ParkProp;
#[derive(Component)]
pub(super) struct Preview;
#[derive(Component)]
pub(super) struct SelectionOutline;
#[derive(Component)]
pub(super) struct GizmoPart {
    axis: usize,
    part: u8,
}
#[derive(Component)]
pub(super) struct GizmoCamera;
pub(super) struct Piece {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}
#[derive(Resource)]
pub(super) struct Catalog {
    pub pieces: Vec<Piece>,
    head: Handle<Mesh>,
    cube: Handle<Mesh>,
    wire: Handle<Mesh>,
    ghost: Handle<StandardMaterial>,
    selection_colors: [Handle<StandardMaterial>; 2],
}
#[derive(Resource, Default)]
pub(crate) struct ParkScene {
    pub(crate) entities: HashSet<Entity>,
    props: HashMap<Key, (Entity, Object)>,
    retired: HashSet<Entity>,
    seen: u64,
    library_seen: u64,
    preview_signature: Vec<(Key, Definition)>,
    preview_children: Vec<Entity>,
    outlines: Vec<Entity>,
}

pub(super) fn startup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let colors = [
        Color::srgb(0.55, 0.39, 0.23),
        Color::srgb(0.55, 0.49, 0.41),
        Color::srgb(0.65, 0.7, 0.76),
        Color::srgb(0.45, 0.49, 0.53),
        Color::srgb(0.53, 0.46, 0.36),
        Color::srgb(0.52, 0.54, 0.57),
    ];
    let pieces: Vec<Piece> = Model::ALL
        .into_iter()
        .enumerate()
        .map(|(i, model)| {
            let geometry = geometry::build(model);
            let mesh = meshes.add(geometry.mesh());
            let material = materials.add(StandardMaterial {
                base_color: colors[i],
                unlit: true,
                perceptual_roughness: 0.7,
                metallic: if model == Model::Rail { 0.8 } else { 0. },
                ..default()
            });
            Piece { mesh, material }
        })
        .collect();
    let ghost = materials.add(StandardMaterial {
        base_color: Color::srgba(0.2, 0.8, 1., 0.5),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        ..default()
    });
    let axes = [
        Color::srgb(1., 0.15, 0.12),
        Color::srgb(0.2, 1., 0.25),
        Color::srgb(0.25, 0.5, 1.),
    ]
    .map(|color| {
        materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            ..default()
        })
    });
    let shaft = meshes.add(Cuboid::new(0.018, 1., 0.018));
    let head = meshes.add(Cone {
        radius: 0.07,
        height: 0.2,
    });
    let cube = meshes.add(Cuboid::from_length(0.12));
    let ring = meshes.add(Torus::new(0.685, 0.715));
    commands.spawn((
        Preview,
        Transform::default(),
        Visibility::Hidden,
        RenderLayers::layer(OVERLAY_LAYER),
    ));
    for (axis, material) in axes.iter().enumerate() {
        for (part, mesh) in [(0, &shaft), (1, &head), (2, &ring)] {
            commands.spawn((
                GizmoPart { axis, part },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                RenderLayers::layer(OVERLAY_LAYER),
            ));
        }
    }
    commands.spawn((
        GizmoCamera,
        Name::new("park transform handles"),
        Camera3d::default(),
        bevy::render::view::Msaa::Off,
        RenderLayers::layer(OVERLAY_LAYER),
        Camera {
            order: 3,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            msaa_writeback: bevy::camera::MsaaWriteback::Off,
            ..default()
        },
    ));
    let wire = meshes.add(Cuboid::from_length(1.));
    let selection_colors =
        [Color::srgb(0.15, 0.85, 1.), Color::srgb(1., 0.78, 0.15)].map(|color| {
            materials.add(StandardMaterial {
                base_color: color,
                unlit: true,
                ..default()
            })
        });
    commands.insert_resource(Catalog {
        pieces,
        head,
        cube,
        wire,
        ghost,
        selection_colors,
    });
}

pub(super) fn sync(
    mut commands: Commands,
    world: Res<ParkWorld>,
    mut scene: ResMut<ParkScene>,
    catalog: Res<Catalog>,
    mut library: ResMut<library::Library>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut epoch: ResMut<benilla_world::collision::ColliderEpoch>,
) {
    if world.generation == scene.seen && library.generation == scene.library_seen {
        return;
    }
    scene.seen = world.generation;
    scene.library_seen = library.generation;
    let previous_retired = std::mem::take(&mut scene.retired);
    for entity in previous_retired {
        scene.entities.remove(&entity);
    }
    let changed: Vec<Key> = scene
        .props
        .iter()
        .filter_map(|(key, (_, old))| (world.objects.get(key) != Some(old)).then_some(*key))
        .collect();
    for key in changed {
        let (entity, _) = scene.props.remove(&key).unwrap();
        commands.entity(entity).despawn();
        scene.retired.insert(entity);
    }
    for (key, object) in &world.objects {
        if scene.props.contains_key(key) {
            continue;
        }
        let Ok(geometry) = geometry::definition(&object.definition()) else {
            continue;
        };
        let imported =
            object.model == Model::Asset || object.parts.iter().any(|p| p.model == Model::Asset);
        if imported {
            let Some(parts) =
                library::render_parts(&object.definition(), &mut library, &catalog, &mut meshes)
            else {
                continue;
            };
            let entity = commands
                .spawn((
                    ParkProp,
                    Name::new(format!("park object {}:{}", object.owner, object.id)),
                    math::transform(object.pose),
                    Visibility::Visible,
                    RigidBody::Static,
                    if object.model == Model::Asset {
                        library.models[&object.asset].collider.clone()
                    } else {
                        geometry.collider()
                    },
                ))
                .id();
            for (mesh, material) in parts {
                commands.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    Transform::default(),
                    ChildOf(entity),
                ));
            }
            scene.entities.insert(entity);
            scene.props.insert(*key, (entity, object.clone()));
            continue;
        }
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
        let entity = commands
            .spawn((
                ParkProp,
                Name::new(format!("park object {}:{}", object.owner, object.id)),
                Mesh3d(mesh),
                MeshMaterial3d(material),
                math::transform(object.pose),
                RigidBody::Static,
                geometry.collider(),
            ))
            .id();
        scene.entities.insert(entity);
        scene.props.insert(*key, (entity, object.clone()));
    }
    epoch.bump();
}

pub(super) fn selected_pose(editor: &ParkEditor, world: &ParkWorld) -> Option<(Model, Pose)> {
    editor.preview.or_else(|| {
        editor.selected.and_then(|key| {
            editor
                .previews
                .get(&key)
                .map(|d| (d.model, d.pose))
                .or_else(|| {
                    world
                        .objects
                        .get(&key)
                        .map(|object| (object.model, object.pose))
                })
        })
    })
}
pub(super) fn handle_size(pivot: Vec3, camera: Vec3) -> f32 {
    (pivot.distance(camera) * 0.13).clamp(0.3, 1200.)
}

#[allow(clippy::type_complexity)]
pub(super) fn draw(
    editor: Res<ParkEditor>,
    world: Res<ParkWorld>,
    mut scene: ResMut<ParkScene>,
    catalog: Res<Catalog>,
    mut library: ResMut<library::Library>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut previews: Query<
        (Entity, &mut Transform, &mut Visibility),
        (
            With<Preview>,
            Without<GizmoPart>,
            Without<ParkProp>,
            Without<SelectionOutline>,
        ),
    >,
    mut gizmos: Query<
        (&GizmoPart, &mut Transform, &mut Mesh3d, &mut Visibility),
        (
            Without<Preview>,
            Without<ParkProp>,
            Without<SelectionOutline>,
        ),
    >,
    mut props: Query<
        &mut Visibility,
        (
            With<ParkProp>,
            Without<Preview>,
            Without<GizmoPart>,
            Without<SelectionOutline>,
        ),
    >,
    mut outlines: Query<
        (
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        (
            With<SelectionOutline>,
            Without<Preview>,
            Without<GizmoPart>,
            Without<ParkProp>,
            Without<GizmoCamera>,
        ),
    >,
    mut overlay: Query<
        (&mut Camera, &mut Transform, &mut Projection),
        (
            With<GizmoCamera>,
            Without<GizmoPart>,
            Without<Preview>,
            Without<SelectionOutline>,
        ),
    >,
    world_camera: Query<
        &Projection,
        (With<benilla_world::view::WorldCamera>, Without<GizmoCamera>),
    >,
) {
    if let Ok((mut camera, mut transform, mut projection)) = overlay.single_mut() {
        camera.is_active = editor.active;
        *transform = editor.camera;
        if let Ok(world_projection) = world_camera.single() {
            *projection = world_projection.clone();
        }
    }
    let selected = editor.target_pose(&world).filter(|_| editor.active);
    if let Ok((root, mut transform, mut visibility)) = previews.single_mut() {
        let (sources, start, result) = if let Some((start, result)) = editor.preview_root {
            (editor.preview_sources.clone(), start, result)
        } else if !editor.drafts.is_empty() {
            let pose = editor.target_pose(&world).unwrap();
            (
                editor
                    .drafts
                    .iter()
                    .enumerate()
                    .map(|(i, d)| ((0, i as u32), d.clone()))
                    .collect(),
                pose,
                pose,
            )
        } else {
            (Vec::new(), Pose::default(), Pose::default())
        };
        *transform = math::transform(result);
        *visibility = if editor.active && !sources.is_empty() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if scene.preview_signature != sources {
            for entity in scene.preview_children.drain(..) {
                commands.entity(entity).despawn();
            }
            let inverse = math::transform(start).to_matrix().inverse();
            for (_, definition) in &sources {
                if let Ok(geometry) = geometry::definition(definition) {
                    if definition.model == Model::Asset
                        || definition.parts.iter().any(|p| p.model == Model::Asset)
                    {
                        if let Some(parts) =
                            library::render_parts(definition, &mut library, &catalog, &mut meshes)
                        {
                            for (mesh, material) in parts {
                                let mut ghost =
                                    materials.get(&material).cloned().unwrap_or_default();
                                ghost.base_color = Color::srgba(0.3, 0.8, 1., 0.65);
                                if ghost.alpha_mode == AlphaMode::Opaque {
                                    ghost.alpha_mode = AlphaMode::Blend;
                                }
                                let material = materials.add(ghost);
                                scene.preview_children.push(
                                    commands
                                        .spawn((
                                            Mesh3d(mesh),
                                            MeshMaterial3d(material),
                                            Transform::from_matrix(
                                                inverse
                                                    * math::transform(definition.pose).to_matrix(),
                                            ),
                                            RenderLayers::layer(OVERLAY_LAYER),
                                            ChildOf(root),
                                        ))
                                        .id(),
                                );
                            }
                        }
                        continue;
                    }
                    let mesh = if matches!(definition.model, Model::Compound | Model::Asset) {
                        meshes.add(geometry.mesh())
                    } else {
                        catalog.pieces[definition.model as usize - 1].mesh.clone()
                    };
                    let child = commands
                        .spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(catalog.ghost.clone()),
                            Transform::from_matrix(
                                inverse * math::transform(definition.pose).to_matrix(),
                            ),
                            RenderLayers::layer(OVERLAY_LAYER),
                            ChildOf(root),
                        ))
                        .id();
                    scene.preview_children.push(child);
                }
            }
            scene.preview_signature = sources;
        }
    }
    for (key, (entity, _)) in &scene.props {
        if let Ok(mut visibility) = props.get_mut(*entity) {
            *visibility = if editor.active && editor.previews.contains_key(key) {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
        }
    }
    let mut wire_parts = Vec::new();
    if editor.active {
        for key in &editor.selection {
            let Some(object) = world.objects.get(key) else {
                continue;
            };
            let source = editor
                .preview_sources
                .iter()
                .find(|(source, _)| source == key)
                .map_or_else(|| object.definition(), |(_, d)| d.clone());
            let Ok(geometry) = geometry::definition(&source) else {
                continue;
            };
            let (min, max) = geometry.pick_triangles().iter().flatten().fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(min, max), &p| (min.min(p), max.max(p)),
            );
            let mut matrix = math::transform(source.pose).to_matrix();
            if let Some((start, result)) = editor.preview_root {
                matrix = math::transform(result).to_matrix()
                    * math::transform(start).to_matrix().inverse()
                    * matrix;
            }
            let centre = (min + max) * 0.5;
            let half = (max - min) * 0.5;
            for axis in 0..3 {
                for corner in 0..4 {
                    let mut p = centre;
                    let other = [(axis + 1) % 3, (axis + 2) % 3];
                    p[other[0]] += half[other[0]] * if corner & 1 == 0 { -1. } else { 1. };
                    p[other[1]] += half[other[1]] * if corner & 2 == 0 { -1. } else { 1. };
                    let mut a = p;
                    let mut b = p;
                    a[axis] -= half[axis];
                    b[axis] += half[axis];
                    let a = matrix.transform_point3(a);
                    let b = matrix.transform_point3(b);
                    let delta = b - a;
                    let len = delta.length();
                    if len < 1e-6 {
                        continue;
                    }
                    let centre = (a + b) * 0.5;
                    let width = (centre.distance(editor.camera.translation) * 0.0012).max(0.006);
                    wire_parts.push((
                        Transform {
                            translation: centre,
                            rotation: Quat::from_rotation_arc(Vec3::Y, delta / len),
                            scale: Vec3::new(width, len, width),
                        },
                        usize::from(editor.selected == Some(*key)),
                    ));
                }
            }
        }
    }
    while scene.outlines.len() < wire_parts.len() {
        let entity = commands
            .spawn((
                SelectionOutline,
                Mesh3d(catalog.wire.clone()),
                MeshMaterial3d(catalog.selection_colors[0].clone()),
                Transform::default(),
                Visibility::Hidden,
                RenderLayers::layer(OVERLAY_LAYER),
            ))
            .id();
        scene.outlines.push(entity);
    }
    for (index, &entity) in scene.outlines.iter().enumerate() {
        if let Ok((mut transform, mut visibility, mut material)) = outlines.get_mut(entity) {
            if let Some((next, color)) = wire_parts.get(index) {
                *transform = *next;
                material.0 = catalog.selection_colors[*color].clone();
                *visibility = Visibility::Visible;
            } else {
                *visibility = Visibility::Hidden;
            }
        }
    }
    for (part, mut transform, mut mesh, mut visibility) in &mut gizmos {
        let Some(pose) = selected else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let pivot = Vec3::from_array(pose.position);
        let size = handle_size(pivot, editor.camera.translation);
        let local = editor.local || editor.tool == math::Tool::Scale;
        let axis = if local {
            Quat::from_array(pose.rotation) * math::AXES[part.axis]
        } else {
            math::AXES[part.axis]
        };
        transform.rotation = Quat::from_rotation_arc(Vec3::Y, axis);
        transform.scale = Vec3::splat(size);
        if editor.tool == math::Tool::Rotate {
            transform.translation = pivot;
            *visibility = if part.part == 2 {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        } else {
            transform.translation = pivot + axis * size * if part.part == 0 { 0.5 } else { 1. };
            *visibility = if part.part < 2 {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
            if part.part == 1 {
                mesh.0 = if editor.tool == math::Tool::Scale {
                    catalog.cube.clone()
                } else {
                    catalog.head.clone()
                };
            }
        }
    }
}

type AuthoredCollision = (Vec<[[f32; 3]; 3]>, Vec<Vec<[f32; 3]>>);

pub(crate) fn authored_collision(
    world: &ParkWorld,
    centre: Vec3,
    half: Vec3,
    origin: Vec3,
) -> AuthoredCollision {
    let mut triangles = Vec::new();
    let mut rails = Vec::new();
    for object in world.objects.values() {
        let Ok(geometry) = geometry::definition(&object.definition()) else {
            continue;
        };
        let matrix = math::transform(object.pose).to_matrix();
        let transformed: Vec<[Vec3; 3]> = geometry
            .triangles
            .iter()
            .map(|triangle| triangle.map(|point| matrix.transform_point3(point)))
            .collect();
        let min = transformed
            .iter()
            .flatten()
            .fold(Vec3::splat(f32::INFINITY), |a, &b| a.min(b));
        let max = transformed
            .iter()
            .flatten()
            .fold(Vec3::splat(f32::NEG_INFINITY), |a, &b| a.max(b));
        if (min.cmpgt(centre + half) | max.cmplt(centre - half)).any() {
            continue;
        }
        triangles.extend(transformed.into_iter().map(|triangle| {
            triangle.map(|point| crate::skate::to_skate(point, origin).to_array())
        }));
        rails.extend(geometry.rails.iter().map(|rail| {
            rail.iter()
                .map(|&point| {
                    crate::skate::to_skate(matrix.transform_point3(point), origin).to_array()
                })
                .collect()
        }));
    }
    (triangles, rails)
}
