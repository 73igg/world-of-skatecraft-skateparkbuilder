//! A dedicated replay input surface; timeline and camera actions do not reach the WoW UI.
use super::{Action, Replay};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, ClearColorConfig};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;

#[derive(Component)]
struct HudCamera;
#[derive(Component)]
struct Panel;
#[derive(Component)]
struct ExportPanel;
#[derive(Component)]
struct ExportStatus;
#[derive(Component)]
struct Status;
#[derive(Component)]
pub(super) struct Timeline;
#[derive(Component)]
struct Playhead;
#[derive(Component)]
struct Marks;
#[derive(Component)]
struct Library;
#[derive(Component, Clone)]
pub(super) struct Control(Action);

fn button(parent: &mut ChildSpawnerCommands, action: Action, label: &str, font: &Handle<Font>) {
    parent
        .spawn((
            Button,
            Control(action),
            Node {
                padding: UiRect::axes(px(9), px(7)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.12, 0.17, 0.22)),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label),
                TextFont {
                    font: font.clone(),
                    font_size: 15.,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

fn startup(mut commands: Commands, assets: Res<AssetServer>) {
    let font: Handle<Font> = assets.load("mpq://Fonts/ARIALN.ttf");
    let camera = commands
        .spawn((
            HudCamera,
            Camera2d,
            RenderLayers::none(),
            bevy::render::view::Msaa::Off,
            Camera {
                order: 6,
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
            ExportPanel,
            UiTargetCamera(camera),
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                top: px(20),
                left: percent(25),
                width: percent(50),
                padding: UiRect::all(px(12)),
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.04, 0.055, 0.97)),
        ))
        .with_children(|p| {
            p.spawn((
                ExportStatus,
                Text::new("Exporting"),
                TextFont {
                    font: font.clone(),
                    font_size: 17.,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            button(
                p,
                Action::Toggle,
                "Cancel export / return to skating",
                &font,
            );
        });
    commands.spawn((Panel,UiTargetCamera(camera),Node {display:Display::None,position_type:PositionType::Absolute,
        bottom:px(20),left:percent(4),width:percent(92),padding:UiRect::all(px(14)),
        flex_direction:FlexDirection::Column,row_gap:px(9),..default()},BackgroundColor(Color::srgba(0.02,0.035,0.05,0.94))))
        .with_children(|p|{
            p.spawn((Status,Text::new("REPLAY"),TextFont {font:font.clone(),font_size:17.,..default()},TextColor(Color::WHITE)));
            p.spawn((Timeline,Button,Node {width:percent(100),height:px(18),..default()},BackgroundColor(Color::srgb(0.17,0.23,0.28))))
                .with_children(|bar|{bar.spawn((Playhead,Node {width:percent(100),height:percent(100),..default()},BackgroundColor(Color::srgb(0.25,0.8,0.65))));bar.spawn((Marks,Node{position_type:PositionType::Absolute,width:percent(100),height:percent(100),..default()}));});
            p.spawn(Node {flex_wrap:FlexWrap::Wrap,column_gap:px(7),row_gap:px(5),..default()}).with_children(|row|{
                for (a,l) in [(Action::Step(-1),"◀ Frame"),(Action::Play,"Play / Pause"),(Action::Step(1),"Frame ▶"),
                    (Action::In,"Set start"),(Action::Out,"Set end"),(Action::ResetTrim,"Reset trim"),(Action::Window(-15.),"Window −15s"),(Action::Window(15.),"Window +15s"),(Action::Rate(-0.25),"Slower"),(Action::Rate(0.25),"Faster"),
                    (Action::Camera,"Camera"),(Action::CameraPath,"Toggle camera keys"),(Action::Fov(-0.1),"Zoom in"),(Action::Fov(0.1),"Zoom out"),
                    (Action::FlySpeed(0.8),"Move slower"),(Action::FlySpeed(1.25),"Move faster"),(Action::FlyResponse,"Move response")]
                {button(row,a,l,&font);}
            });
            p.spawn(Node {flex_wrap:FlexWrap::Wrap,column_gap:px(7),row_gap:px(5),..default()}).with_children(|row|{
                for (a,l) in [(Action::Keyframe(false),"Set camera key"),(Action::Keyframe(true),"Set camera cut"),
                    (Action::SelectKey(-1),"Previous key"),(Action::SelectKey(1),"Next key"),(Action::DeleteKeyframe,"Remove key"),(Action::KeyInterpolation,"Key motion"),(Action::KeyRate(-0.25),"Key slower"),(Action::KeyRate(0.25),"Key faster"),(Action::Save,"Save clip"),(Action::Library,"Saved clips"),
                    (Action::Export,"Export video"),(Action::Toggle,"Exit replay")]
                {button(row,a,l,&font);}
            });
            p.spawn((Text::new("F6 / both stick clicks: replay · Space / ✕: play · LT/RT: scrub · C / △: camera · □ / T: camera key\nFree camera: sticks / WASD + right mouse · LB/RB / Q/E: height · Shift: fast · [ / ]: trim · Ctrl+S/O/E: save/open/export"),
                TextFont {font:font.clone(),font_size:13.,..default()},TextColor(Color::srgb(0.7,0.77,0.82))));
        });
    commands.spawn((
        Library,
        ScrollPosition::default(),
        UiTargetCamera(camera),
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            top: px(65),
            left: percent(15),
            width: percent(70),
            max_height: percent(55),
            overflow: Overflow::scroll_y(),
            padding: UiRect::all(px(12)),
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            ..default()
        },
        BackgroundColor(Color::srgba(0.025, 0.04, 0.055, 0.97)),
    ));
}

pub(super) fn buttons(
    mut replay: ResMut<Replay>,
    query: Query<(&Interaction, &Control), Changed<Interaction>>,
    timeline: Query<(&Interaction, &ComputedNode, &UiGlobalTransform), With<Timeline>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    for (interaction, control) in &query {
        if *interaction == Interaction::Pressed {
            replay.actions.push(control.0.clone());
        }
    }
    if replay.active && replay.exporting.is_none() {
        if let (Ok((interaction, node, transform)), Ok(window)) =
            (timeline.single(), windows.single())
        {
            if *interaction == Interaction::Pressed {
                if let Some(cursor) = window.cursor_position() {
                    let width = node.size().x;
                    let center = transform.translation.x;
                    if width > 0. {
                        let fraction = ((cursor.x * window.scale_factor() - center + width * 0.5)
                            / width)
                            .clamp(0., 1.);
                        let (a, b) = replay.bounds();
                        replay
                            .actions
                            .push(Action::Seek(a + (b - a) * f64::from(fraction)));
                    }
                }
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn draw(
    replay: Res<Replay>,
    park: Res<crate::park::ParkEditor>,
    mut nodes: Query<(
        &mut Node,
        Option<&Panel>,
        Option<&Library>,
        Option<&Playhead>,
        Option<&ExportPanel>,
    )>,
    mut cameras: Query<&mut Camera, With<HudCamera>>,
    mut status: Query<&mut Text, (With<Status>, Without<ExportStatus>)>,
    mut export_status: Query<&mut Text, (With<ExportStatus>, Without<Status>)>,
    libraries: Query<Entity, With<Library>>,
    marks: Query<Entity, With<Marks>>,
    mut old_marks: Local<Vec<(f64, bool)>>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut seen: Local<Option<Vec<std::path::PathBuf>>>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    mut positions: Query<(&mut ScrollPosition, &ComputedNode), With<Library>>,
) {
    if replay.active && replay.library_open {
        for (mut position, node) in &mut positions {
            let factor = if scroll.unit == bevy::input::mouse::MouseScrollUnit::Line {
                32.
            } else {
                1.
            };
            position.0.y = (position.0.y - scroll.delta.y * factor).clamp(
                0.,
                (node.content_size().y - node.size().y).max(0.) * node.inverse_scale_factor(),
            );
        }
    }
    let exporting = replay.exporting.is_some();
    if let Ok(mut camera) = cameras.single_mut() {
        camera.is_active = replay.active && !park.active;
    }
    for (mut node, panel, library, playhead, export_panel) in &mut nodes {
        if export_panel.is_some() {
            node.display = if exporting {
                Display::Flex
            } else {
                Display::None
            };
        }
        if panel.is_some() {
            node.display = if replay.active && !exporting {
                Display::Flex
            } else {
                Display::None
            };
        }
        if library.is_some() {
            node.display = if replay.active && replay.library_open && !exporting {
                Display::Flex
            } else {
                Display::None
            };
        }
        if playhead.is_some() {
            let (a, b) = replay.bounds();
            node.width = percent(if b > a {
                ((replay.cursor - a) / (b - a) * 100.) as f32
            } else {
                0.
            });
        }
    }
    if let Ok(mut text) = export_status.single_mut() {
        let progress = (replay.cursor - replay.in_point)
            / (replay.out_point - replay.in_point).max(0.001)
            * 100.;
        **text=format!("Exporting video: {:.0}% ? {} frames rendered.\nEscape cancels. Keep the window size fixed.",progress,replay.exporting.as_ref().map_or(0,|s|s.frame_count()));
    }
    if let Ok(mut text) = status.single_mut() {
        let (start, end) = replay.bounds();
        let motion = replay
            .keys
            .iter()
            .find(|k| (k.at - replay.cursor).abs() < 0.05)
            .map_or("Select a key", |k| k.interpolation.label());
        **text=format!(
        "REPLAY  {:.2} / {:.2}s  ·  {}  ·  {:.2}×  ·  {} camera  ·  {} keys  ·  clip {:.2}–{:.2}s\nRecording window: {:.0}s (15–90s)  ·  Selected clip: {:.2}s\nFree move: {:.1} yd/s (Shift: 2x) | {} response | Outgoing key motion: {}\n{}",
        replay.cursor-start,end-start,if replay.playing{"Playing"}else{"Paused"},replay.rate,replay.mode.label(),
        replay.keys.len(),replay.in_point-start,replay.out_point-start,replay.window,replay.out_point-replay.in_point,
        replay.fly.speed, replay.fly.response.label(), motion, replay.status);
    }
    let wanted: Vec<_> = replay.keys.iter().map(|k| (k.at, k.cut)).collect();
    if *old_marks != wanted {
        *old_marks = wanted.clone();
        if let Ok(entity) = marks.single() {
            commands.entity(entity).despawn_children();
            let (start, end) = replay.bounds();
            if end > start {
                commands.entity(entity).with_children(|parent| {
                    for (at, cut) in wanted {
                        parent.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: percent(((at - start) / (end - start) * 100.) as f32),
                                width: px(3),
                                height: percent(100),
                                ..default()
                            },
                            BackgroundColor(if cut {
                                Color::srgb(1., 0.3, 0.2)
                            } else {
                                Color::srgb(1., 0.85, 0.25)
                            }),
                        ));
                    }
                });
            }
        }
    }
    if seen.as_ref() != Some(&replay.library) {
        *seen = Some(replay.library.clone());
        if let Ok(entity) = libraries.single() {
            commands.entity(entity).despawn_children();
            let font = assets.load("mpq://Fonts/ARIALN.ttf");
            commands.entity(entity).with_children(|parent| {
                parent.spawn((
                    Text::new("Saved replays"),
                    TextFont {
                        font: font.clone(),
                        font_size: 18.,
                        ..default()
                    },
                ));
                if replay.library.is_empty() {
                    parent.spawn((
                        Text::new("No saved clips yet. Use Save clip to keep this recording."),
                        TextFont {
                            font: font.clone(),
                            font_size: 15.,
                            ..default()
                        },
                    ));
                }
                for (i, path) in replay.library.iter().enumerate() {
                    button(
                        parent,
                        Action::Load(i),
                        &path.file_stem().unwrap_or_default().to_string_lossy(),
                        &font,
                    );
                }
            });
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Startup, startup)
        .add_systems(Update, draw.after(super::draw));
}
