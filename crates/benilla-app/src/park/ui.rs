use super::{
    input::{Action, Actions},
    math::{self, Tool},
    scene,
    state::{ParkEditor, ParkWorld},
};
use benilla_protocol::park::Model;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, ClearColorConfig};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;

#[derive(Component)]
pub(super) struct Panel;
#[derive(Component)]
pub(super) struct Status;
#[derive(Component)]
pub(super) struct SelectionCount;
#[derive(Component)]
pub(super) struct MenuButton(Action);
#[derive(Component)]
pub(super) struct HudCamera;

pub(super) fn startup(mut commands: Commands, assets: Res<AssetServer>) {
    let font: Handle<Font> = assets.load("mpq://Fonts/ARIALN.ttf");
    let camera = commands
        .spawn((
            HudCamera,
            Name::new("park editor controls"),
            Camera2d,
            bevy::render::view::Msaa::Off,
            RenderLayers::none(),
            Camera {
                order: 4,
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
        .id();
    commands
        .spawn((
            Panel,
            UiTargetCamera(camera),
            Node {
                position_type: PositionType::Absolute,
                left: px(12.),
                top: px(12.),
                width: px(245.),
                max_height: percent(96.),
                padding: UiRect::all(px(12.)),
                flex_direction: FlexDirection::Column,
                row_gap: px(5.),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.045, 0.055, 0.075, 0.95)),
            ScrollPosition::default(),
            Visibility::Hidden,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("PARK BUILDER"),
                TextFont {
                    font: font.clone(),
                    font_size: 21.,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            parent.spawn((
                Text::new("Objects stay until their owner logs out."),
                TextFont {
                    font: font.clone(),
                    font_size: 13.,
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.78, 0.85)),
            ));
            parent.spawn((
                SelectionCount,
                Text::new("Ctrl-click to select multiple objects"),
                TextFont {
                    font: font.clone(),
                    font_size: 13.,
                    ..default()
                },
                TextColor(Color::srgb(0.75, 0.88, 1.)),
            ));
            button(parent, &font, "Object Catalog…".into(), Action::Catalog);
            parent
                .spawn(Node {
                    display: Display::Grid,
                    grid_template_columns: vec![RepeatedGridTrack::flex(3, 1.)],
                    column_gap: px(4.),
                    ..default()
                })
                .with_children(|row| {
                    for (caption, action) in [
                        ("G Move", Action::Tool(Tool::Move)),
                        ("R Rotate", Action::Tool(Tool::Rotate)),
                        ("S Size", Action::Tool(Tool::Scale)),
                    ] {
                        button(row, &font, caption.into(), action);
                    }
                });
            button(
                parent,
                &font,
                "Align Z height to last selected".into(),
                Action::AlignHeight,
            );
            button(
                parent,
                &font,
                "Fuse selection into one object".into(),
                Action::Fuse,
            );
            parent
                .spawn(Node {
                    display: Display::Grid,
                    grid_template_columns: vec![RepeatedGridTrack::flex(2, 1.)],
                    column_gap: px(4.),
                    row_gap: px(4.),
                    ..default()
                })
                .with_children(|grid| {
                    for (i, model) in Model::ALL.into_iter().enumerate() {
                        button(
                            grid,
                            &font,
                            format!("{} {}", i + 1, model.name()),
                            Action::Create(model),
                        );
                    }
                });
            for (caption, action) in [
                ("Enter  Place / Apply", Action::Place),
                ("Ctrl+D  Duplicate", Action::Duplicate),
                ("Delete  Remove", Action::Delete),
                ("Ctrl+Z  Undo", Action::Undo),
                ("Ctrl+Shift+Z  Redo", Action::Redo),
                ("F8  Back to skating", Action::Exit),
            ] {
                button(parent, &font, caption.into(), action);
            }
            parent.spawn((
                Status,
                Text::new(""),
                TextFont {
                    font: font.clone(),
                    font_size: 13.,
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.86, 0.94)),
            ));
        });
}
fn button(parent: &mut ChildSpawnerCommands, font: &Handle<Font>, caption: String, action: Action) {
    parent
        .spawn((
            Button,
            MenuButton(action),
            Node {
                width: percent(100.),
                min_height: px(27.),
                padding: UiRect::axes(px(8.), px(4.)),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgb(0.12, 0.15, 0.19)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(caption),
                TextFont {
                    font: font.clone(),
                    font_size: 14.,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}
pub(super) fn buttons(
    editor: Res<ParkEditor>,
    mut actions: ResMut<Actions>,
    browser: Res<super::catalog_ui::Browser>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut panels: Query<(&mut ScrollPosition, &ComputedNode), With<Panel>>,
    mut query: Query<(&Interaction, &MenuButton, &mut BackgroundColor), Changed<Interaction>>,
) {
    if editor.active && !browser.open {
        if let Ok(window) = windows.single() {
            if window
                .cursor_position()
                .is_some_and(|cursor| cursor.x < 270.)
            {
                for (mut position, node) in &mut panels {
                    let max = (node.content_size().y - node.size().y).max(0.);
                    position.0.y = (position.0.y - scroll.delta.y * 32.).clamp(0., max);
                }
            }
        }
    }
    for (interaction, button, mut background) in &mut query {
        background.0 = match interaction {
            Interaction::Pressed => Color::srgb(0.2, 0.4, 0.55),
            Interaction::Hovered => Color::srgb(0.2, 0.25, 0.31),
            Interaction::None => Color::srgb(0.12, 0.15, 0.19),
        };
        if editor.active && !browser.open && *interaction == Interaction::Pressed {
            actions.0.push(button.0.clone());
        }
    }
}
pub(super) fn draw(
    editor: Res<ParkEditor>,
    world: Res<ParkWorld>,
    self_guid: Res<crate::net::SelfGuid>,
    mut panels: Query<&mut Visibility, With<Panel>>,
    mut cameras: Query<&mut Camera, With<HudCamera>>,
    mut status: Query<&mut Text, With<Status>>,
    mut selection: Query<&mut Text, (With<SelectionCount>, Without<Status>)>,
) {
    for mut visibility in &mut panels {
        *visibility = if editor.active {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut camera in &mut cameras {
        camera.is_active = editor.active;
    }
    if !editor.active {
        return;
    }
    for mut label in &mut selection {
        label.0 = format!(
            "Selected: {}  ·  Ctrl-click to add/remove",
            editor.selection.len()
        );
    }
    let owned = world
        .objects
        .keys()
        .filter(|key| Some(key.0) == self_guid.0)
        .count();
    let axis = editor.axis.map_or("Free", |i| ["X", "Y", "Z"][i]);
    let space = if editor.local || editor.tool == Tool::Scale {
        "Local"
    } else {
        "World"
    };
    let mut text = format!(
        "{}\n\nYour objects: {owned}/256\nMove / rotate / resize: {axis} {space}\n",
        editor.status
    );
    if let Some((model, pose)) = scene::selected_pose(&editor, &world) {
        let basis = Mat3::from_cols(math::AXES[0], math::AXES[1], math::AXES[2]);
        let rotation = Quat::from_mat3(
            &(basis.transpose() * Mat3::from_quat(Quat::from_array(pose.rotation)) * basis),
        );
        let (x, y, z) = rotation.to_euler(EulerRot::XYZ);
        text+=&format!("{}\nPosition: {:.2}, {:.2}, {:.2}\nRotation: {:.1}°, {:.1}°, {:.1}°\nSize: {:.2}, {:.2}, {:.2}\n",model.name(),pose.position[0],-pose.position[2],pose.position[1],x.to_degrees(),y.to_degrees(),z.to_degrees(),pose.scale[0],pose.scale[2],pose.scale[1]);
    }
    text+="\nCtrl-click: add/remove selection\nAlign Z uses the last selected height.\nFuse is one object; Ctrl+Z restores pieces.\nDrag model or colored handles.\nX / Y / Z: constrain axis\nPress axis twice: local\nType a value, then Enter\nShift: fine · Ctrl: snap\nEsc: cancel current edit\n\nHold right mouse + WASD: fly\nQ / E: down / up · Wheel: dolly\nCamera navigation cancels the active drag.";
    for mut label in &mut status {
        label.0 = text.clone();
    }
}
