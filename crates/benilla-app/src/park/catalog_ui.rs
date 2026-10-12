//! A separate catalog context keeps text entry and preview controls out of builder transforms.
use super::{
    input::{Action, Actions},
    library::{self, Library},
    scene,
    state::ParkEditor,
};
use benilla_park_assets::{Search, Sort};
use bevy::camera::{visibility::RenderLayers, CameraOutputMode, ClearColorConfig, RenderTarget};
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::render_resource::{BlendState, TextureFormat};
use bevy_egui::{
    egui, EguiContext, EguiContextSettings, EguiContexts, EguiFullOutput, EguiGlobalSettings,
    EguiMultipassSchedule, EguiPlugin, EguiPostUpdateSet, EguiTextureHandle,
};
use std::collections::BTreeSet;
const LAYER: usize = 26;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct CatalogPass;
#[derive(Component)]
pub(super) struct CatalogCamera;
#[derive(Component)]
struct PreviewCamera;
#[derive(Component)]
struct PreviewRoot;
#[derive(Resource, Default)]
pub(super) struct Browser {
    pub open: bool,
    pub query: Search,
    pub selected: Option<u64>,
    pub preview: Option<u64>,
    pub results: Vec<usize>,
    pub use_rect: Option<egui::Rect>,
    cached_query: Search,
    cached_generation: u64,
    category_generation: u64,
    category_game: Option<String>,
    categories: Vec<String>,
    overlay: Option<Entity>,
    image: Handle<Image>,
    texture: Option<egui::TextureId>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    fresh: bool,
    shown: Option<u64>,
    children: Vec<Entity>,
}
pub(super) fn plugin(app: &mut App) {
    if !app.is_plugin_added::<EguiPlugin>() {
        app.add_plugins(EguiPlugin::default());
        let mut settings = app.world_mut().resource_mut::<EguiGlobalSettings>();
        settings.auto_create_primary_context = false;
        settings.enable_cursor_icon_updates = false;
    }
    app.init_resource::<Browser>()
        .add_systems(Startup, startup)
        .add_systems(
            Update,
            capture
                .after(super::ui::buttons)
                .before(super::input::update),
        )
        .add_systems(
            Update,
            preview.after(library::tick).after(super::input::update),
        )
        .add_systems(CatalogPass, draw)
        .add_systems(
            PostUpdate,
            empty_output
                .after(EguiPostUpdateSet::EndPass)
                .before(EguiPostUpdateSet::ProcessOutput),
        );
}
fn startup(
    mut commands: Commands,
    mut browser: ResMut<Browser>,
    mut images: ResMut<Assets<Image>>,
) {
    browser.yaw = 0.65;
    browser.pitch = 0.32;
    browser.zoom = 1.;
    browser.image = images.add(Image::new_target_texture(
        512,
        512,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    browser.overlay = Some(
        commands
            .spawn((
                CatalogCamera,
                EguiContext::default(),
                EguiMultipassSchedule::new(CatalogPass),
                EguiContextSettings {
                    run_manually: true,
                    ..default()
                },
                Name::new("park catalog overlay"),
                Camera2d,
                bevy::render::view::Msaa::Off,
                RenderLayers::none(),
                Camera {
                    order: 5,
                    is_active: false,
                    clear_color: ClearColorConfig::Custom(Color::NONE),
                    output_mode: CameraOutputMode::Write {
                        blend_state: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        clear_color: ClearColorConfig::None,
                    },
                    msaa_writeback: bevy::camera::MsaaWriteback::Off,
                    ..default()
                },
            ))
            .id(),
    );
    commands.spawn((
        PreviewCamera,
        Name::new("park catalog model preview"),
        Camera3d::default(),
        bevy::render::view::Msaa::Off,
        RenderLayers::layer(LAYER),
        RenderTarget::Image(browser.image.clone().into()),
        Camera {
            order: -5,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.065, 0.085, 0.11)),
            ..default()
        },
    ));
    commands.spawn((
        PreviewRoot,
        Transform::default(),
        Visibility::Visible,
        RenderLayers::layer(LAYER),
    ));
}
pub(super) fn capture(
    mut browser: ResMut<Browser>,
    mut editor: ResMut<ParkEditor>,
    mut actions: ResMut<Actions>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut cameras: Query<(&mut Camera, &mut EguiContextSettings), With<CatalogCamera>>,
) {
    if !editor.active {
        browser.open = false;
    }
    if editor.active && actions.0.iter().any(|a| matches!(a, Action::Catalog)) {
        super::input::reset_camera_drag(&mut editor);
        browser.open = !browser.open;
        browser.fresh = browser.open;
    }
    actions.0.retain(|a| !matches!(a, Action::Catalog));
    if browser.open {
        actions.0.clear();
        if keys.just_pressed(KeyCode::Escape) {
            browser.open = false;
            keys.reset(KeyCode::Escape);
        }
    }
    for (mut camera, mut settings) in &mut cameras {
        camera.is_active = browser.open;
        settings.run_manually = !browser.open;
    }
}
fn empty_output(
    mut contexts: Query<
        (&mut EguiContext, &mut EguiFullOutput, &EguiContextSettings),
        With<CatalogCamera>,
    >,
    mut cached: Local<Option<egui::FullOutput>>,
) {
    for (mut context, mut output, settings) in &mut contexts {
        if settings.run_manually && output.0.is_none() {
            let mut empty = if let Some(previous) = cached.as_ref() {
                previous.clone()
            } else {
                let output = context.get_mut().run(egui::RawInput::default(), |_| {});
                let mut keep = output.clone();
                keep.textures_delta = default();
                keep.shapes.clear();
                *cached = Some(keep);
                output
            };
            empty.pixels_per_point = context.get_mut().pixels_per_point();
            output.0 = Some(empty);
        }
    }
}
fn draw(
    mut contexts: EguiContexts,
    mut browser: ResMut<Browser>,
    mut library: ResMut<Library>,
    mut actions: ResMut<Actions>,
    editor: Res<ParkEditor>,
) -> Result {
    if !browser.open || !editor.active {
        return Ok(());
    }
    let Some(entity) = browser.overlay else {
        return Ok(());
    };
    if browser.texture.is_none() {
        browser.texture =
            Some(contexts.add_image(EguiTextureHandle::Strong(browser.image.clone())));
    }
    let context = contexts.ctx_for_entity_mut(entity)?;
    let mut open = true;
    let mut choice = None;
    let mut place = false;
    browser.use_rect = None;
    let fresh = browser.fresh;
    browser.fresh = false;
    let screen = context.content_rect().size();
    egui::Window::new("Object Catalog").id(egui::Id::new("park-catalog")).open(&mut open)
        .default_pos(egui::pos2(24.,24.)).default_size(egui::vec2(900.,620.)).min_size(egui::vec2(430.,260.)).max_size(screen-egui::vec2(24.,24.))
        .show(context,|ui| {
            ui.horizontal(|ui| {
                let edit=ui.add(egui::TextEdit::singleline(&mut browser.query.text).hint_text("Search names, words, paths or IDs…").desired_width((ui.available_width()-60.).max(180.)));
                if fresh || context.input(|i|i.modifiers.command && i.key_pressed(egui::Key::F)){edit.request_focus();}
                if ui.button("Clear").clicked(){browser.query.text.clear();edit.request_focus();}
            });
            ui.horizontal_wrapped(|ui| {
                let old_game=browser.query.game.clone();
                egui::ComboBox::from_id_salt("catalog-game").selected_text(browser.query.game.as_deref().unwrap_or("All games")).show_ui(ui,|ui| {
                    ui.selectable_value(&mut browser.query.game,None,"All games");
                    for game in ["WoW","Skate 3","Builder"]{ui.selectable_value(&mut browser.query.game,Some(game.into()),game);}
                });
                if old_game!=browser.query.game {browser.query.category=None;}
                if browser.category_generation!=library.generation || browser.category_game!=browser.query.game || browser.categories.is_empty() {
                    browser.categories=library.entries.iter().filter(|e|browser.query.game.as_ref().is_none_or(|g|g==&e.game)).map(|e|e.category.clone()).collect::<BTreeSet<_>>().into_iter().collect();
                    browser.category_generation=library.generation;browser.category_game=browser.query.game.clone();
                }
                let categories=browser.categories.clone();
                egui::ComboBox::from_id_salt("catalog-category").selected_text(browser.query.category.as_deref().unwrap_or("All categories")).show_ui(ui,|ui| {
                    ui.selectable_value(&mut browser.query.category,None,"All categories");
                    for category in categories{ui.selectable_value(&mut browser.query.category,Some(category.clone()),category);}
                });
                egui::ComboBox::from_id_salt("catalog-sort").selected_text(sort_name(browser.query.sort)).show_ui(ui,|ui| {
                    for sort in [Sort::Relevance,Sort::Name,Sort::NameReverse,Sort::Game,Sort::Category,Sort::Height,Sort::Width]{ui.selectable_value(&mut browser.query.sort,sort,sort_name(sort));}
                });
            });
            if browser.cached_generation!=library.generation || browser.cached_query!=browser.query || fresh {
                browser.results=benilla_park_assets::search(&library.entries,&browser.query);browser.cached_query=browser.query.clone();browser.cached_generation=library.generation;
            }
            ui.label(format!("{} results · {} objects{}",browser.results.len(),library.entries.len(),if library.indexed{""}else{" · indexing game files…"}));
            ui.separator();
            let height=(ui.available_height()-24.).max(130.);
            ui.columns(2,|columns| {
                egui::ScrollArea::vertical().id_salt("catalog-results").max_height(height).show_rows(&mut columns[0],52.,browser.results.len(),|ui,range| {
                    if browser.results.is_empty(){ui.label("No matching objects. Try fewer words or clear a filter.");}
                    for row in range {
                        let e=&library.entries[browser.results[row]];
                        ui.horizontal(|ui| {
                            if ui.selectable_label(browser.selected==Some(e.id),&e.name).on_hover_text(format!("{}\n{}\n{:016x}",e.category,e.path,e.id)).clicked(){choice=Some(e.id);}
                            if ui.small_button("Preview").clicked(){choice=Some(e.id);}
                        });
                        ui.label(egui::RichText::new(format!("{} · {}",e.game,e.category)).small().color(egui::Color32::GRAY));
                    }
                });
                let right=&mut columns[1];
                if let Some(id)=browser.selected {
                    if let Some(e)=library.entry(id) {
                        right.heading(&e.name);right.label(format!("{} · {}",e.game,e.category));
                        let bounds=library.models.get(&id).map(|m|m.package.bounds).or(e.bounds);
                        if let Some([min,max])=bounds {right.label(format!("{:.1} × {:.1} × {:.1} yd",max[0]-min[0],max[2]-min[2],max[1]-min[1]));}
                    }
                    if library.ready(id) {
                        let side=right.available_width().min((height-125.).max(90.));
                        let response=right.add(egui::Image::new((browser.texture.unwrap(),egui::vec2(side,side))).sense(egui::Sense::drag()));
                        if response.dragged(){let delta=context.input(|i|i.pointer.delta());browser.yaw-=delta.x*0.01;browser.pitch=(browser.pitch+delta.y*0.01).clamp(-1.4,1.4);}
                        if response.hovered(){let scroll=context.input(|i|i.smooth_scroll_delta.y);browser.zoom=(browser.zoom*(-scroll*0.003).exp()).clamp(0.25,5.);}
                        right.label(egui::RichText::new("Drag to rotate · Wheel to zoom").small());
                        right.horizontal(|ui| {
                            if ui.button("Reset view").clicked(){browser.yaw=0.65;browser.pitch=0.32;browser.zoom=1.;}
                            let use_button=ui.add_enabled(editor.pending.is_none(),egui::Button::new("Use this object"));
                            browser.use_rect=Some(use_button.rect);
                            if use_button.clicked(){place=true;}
                        });
                    }else if let Some(error)=library.errors.get(&id).cloned() {
                        right.colored_label(egui::Color32::LIGHT_RED,&error);
                        if right.button("Retry loading").clicked(){library.retry(id);}
                    }else{right.spinner();right.label("Loading model and textures…");}
                }else{right.heading("Choose an object");right.label("Click a result or its Preview button to inspect the model before placing it.");}
            });
            ui.separator();ui.label(egui::RichText::new("Words narrow the results together. Use quotes for an exact phrase. Ctrl+F focuses search.").small());
        });
    if let Some(id) = choice {
        browser.selected = Some(id);
        browser.preview = Some(id);
        browser.yaw = 0.65;
        browser.pitch = 0.32;
        browser.zoom = 1.;
        library.request(id);
    }
    if place {
        if let Some(id) = browser.selected {
            actions.0.push(if id <= 6 {
                Action::Create(benilla_protocol::park::Model::ALL[id as usize - 1])
            } else {
                Action::CreateAsset(id)
            });
            open = false;
        }
    }
    browser.open = open;
    Ok(())
}
fn sort_name(sort: Sort) -> &'static str {
    match sort {
        Sort::Relevance => "Relevance",
        Sort::Name => "Name A–Z",
        Sort::NameReverse => "Name Z–A",
        Sort::Category => "Category",
        Sort::Game => "Game",
        Sort::Height => "Height: small first",
        Sort::Width => "Width: small first",
    }
}
#[allow(clippy::type_complexity)]
fn preview(
    mut commands: Commands,
    mut browser: ResMut<Browser>,
    mut library: ResMut<Library>,
    catalog: Res<scene::Catalog>,
    mut meshes: ResMut<Assets<Mesh>>,
    root: Query<Entity, With<PreviewRoot>>,
    mut cameras: Query<
        (&mut Camera, &mut Transform, &mut Projection),
        (With<PreviewCamera>, Without<PreviewRoot>),
    >,
) {
    if browser.open {
        if let Some(id) = browser.preview {
            library.request(id);
        }
    }
    let id = browser.preview.filter(|id| library.ready(*id));
    if browser.shown != id {
        for entity in browser.children.drain(..) {
            commands.entity(entity).despawn();
        }
        if let (Some(id), Ok(root)) = (id, root.single()) {
            let definition = if id <= 6 {
                benilla_protocol::park::Definition::primitive(
                    benilla_protocol::park::Model::ALL[id as usize - 1],
                    default(),
                )
            } else {
                benilla_protocol::park::Definition::asset(id, default())
            };
            if let Some(parts) =
                library::render_parts(&definition, &mut library, &catalog, &mut meshes)
            {
                for (mesh, material) in parts {
                    browser.children.push(
                        commands
                            .spawn((
                                Mesh3d(mesh),
                                MeshMaterial3d(material),
                                Transform::default(),
                                RenderLayers::layer(LAYER),
                                ChildOf(root),
                            ))
                            .id(),
                    );
                }
            }
        }
        browser.shown = id;
    }
    for (mut camera, mut transform, mut projection) in &mut cameras {
        camera.is_active = browser.open && id.is_some();
        if let Some(bounds) = id.and_then(|id| {
            library
                .models
                .get(&id)
                .map(|m| m.package.bounds)
                .or_else(|| library.entry(id).and_then(|e| e.bounds))
        }) {
            let min = Vec3::from_array(bounds[0]);
            let max = Vec3::from_array(bounds[1]);
            let center = (min + max) * 0.5;
            let radius = ((max - min) * 0.5).length().max(0.1);
            let distance = radius * 3.1 * browser.zoom;
            let rotation = Quat::from_euler(EulerRot::YXZ, browser.yaw, -browser.pitch, 0.);
            *transform = Transform::from_translation(center + rotation * Vec3::Z * distance)
                .looking_at(center, Vec3::Y);
            *projection = Projection::Perspective(PerspectiveProjection {
                fov: 0.8,
                near: (distance * 0.001).max(0.0001),
                far: distance + radius * 5.,
                ..default()
            });
        }
    }
}
