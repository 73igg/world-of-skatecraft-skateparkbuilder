//! Local filming preferences: hide skate displays without changing scoring or return points.
use std::path::Path;

use bevy::prelude::*;

use crate::console::ConsoleCommandApp;

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct SkateVisibility {
    pub show_score: bool,
    pub show_flag: bool,
}

impl Default for SkateVisibility {
    fn default() -> Self {
        Self {
            show_score: true,
            show_flag: true,
        }
    }
}

impl SkateVisibility {
    fn read(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    // Save first: a failed write leaves both the display and its saved preference unchanged.
    fn apply(&mut self, next: Self, path: Option<&Path>) -> Result<(), String> {
        if let Some(path) = path {
            let bytes = serde_json::to_vec_pretty(&next).map_err(|error| error.to_string())?;
            crate::local_state::write_atomic_bytes(path, &bytes)
                .map_err(|error| error.to_string())?;
        }
        *self = next;
        Ok(())
    }

    fn status(self) -> String {
        let label = |shown| if shown { "on" } else { "off" };
        format!(
            "Skate score display: {}; respawn flag display: {}.",
            label(self.show_score),
            label(self.show_flag)
        )
    }
}

#[derive(Clone, Copy)]
enum Target {
    Score,
    Flag,
    Both,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Self::Score => "skatescore",
            Self::Flag => "skateflag",
            Self::Both => "skatehud",
        }
    }
}

fn command(world: &mut World, args: &str, target: Target) -> Vec<String> {
    let current = *world.resource::<SkateVisibility>();
    let showing = match target {
        Target::Score => current.show_score,
        Target::Flag => current.show_flag,
        Target::Both => current.show_score || current.show_flag,
    };
    let enabled = match args.trim().to_ascii_lowercase().as_str() {
        "" | "toggle" => !showing,
        "on" => true,
        "off" => false,
        "status" => return vec![current.status()],
        _ => {
            return vec![format!(
                "Use /{} [on|off|toggle|status]. No argument toggles visibility.",
                target.name()
            )];
        }
    };
    let mut next = current;
    match target {
        Target::Score => next.show_score = enabled,
        Target::Flag => next.show_flag = enabled,
        Target::Both => {
            next.show_score = enabled;
            next.show_flag = enabled;
        }
    }
    let path = crate::local_state::skate_visibility_path();
    match world
        .resource_mut::<SkateVisibility>()
        .apply(next, path.as_deref())
    {
        Ok(()) => {
            let storage = if path.is_some() {
                "Saved locally for all your characters."
            } else {
                "Applied for this session; local persistence is disabled."
            };
            vec![format!("{} {storage}", next.status())]
        }
        Err(error) => vec![format!(
            "Skate display settings were not changed: could not save preferences: {error}"
        )],
    }
}

fn score_command(world: &mut World, args: &str) -> Vec<String> {
    command(world, args, Target::Score)
}

fn flag_command(world: &mut World, args: &str) -> Vec<String> {
    command(world, args, Target::Flag)
}

fn hud_command(world: &mut World, args: &str) -> Vec<String> {
    command(world, args, Target::Both)
}

fn load(mut visibility: ResMut<SkateVisibility>) {
    if let Some(path) = crate::local_state::skate_visibility_path() {
        match SkateVisibility::read(&path) {
            Ok(saved) => *visibility = saved,
            Err(error) => warn!("skate displays: could not load preferences: {error}"),
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<SkateVisibility>()
        .add_systems(Startup, load)
        .console_command(
            "skatescore",
            "Toggle the skate score display: /skatescore [on|off|toggle|status].",
            score_command,
        )
        .console_command(
            "skateflag",
            "Toggle your respawn flag display: /skateflag [on|off|toggle|status].",
            flag_command,
        )
        .console_command(
            "skatehud",
            "Toggle both skate displays: /skatehud [on|off|toggle|status].",
            hud_command,
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_commands_toggle_independently_without_a_character_or_server() {
        let _lock = crate::local_state::test_env::ENV_LOCK.lock().unwrap();
        let _capture = crate::local_state::test_env::EnvGuard::set("WOW_CAPTURE", "1");
        let mut app = App::new();
        plugin(&mut app);
        let run = |app: &mut App, line| crate::console::execute(app.world_mut(), line);
        run(&mut app, "skatescore");
        assert_eq!(
            *app.world().resource::<SkateVisibility>(),
            SkateVisibility {
                show_score: false,
                show_flag: true,
            }
        );
        run(&mut app, "skateflag OFF");
        let hidden = *app.world().resource::<SkateVisibility>();
        assert!(!hidden.show_score && !hidden.show_flag);
        assert!(run(&mut app, "skatehud status")[0].contains("score display: off"));
        run(&mut app, "skatescore off extra");
        assert_eq!(*app.world().resource::<SkateVisibility>(), hidden);
        run(&mut app, "skatehud");
        assert_eq!(
            *app.world().resource::<SkateVisibility>(),
            SkateVisibility::default()
        );
        run(&mut app, "skatescore off");
        run(&mut app, "skatehud toggle");
        assert_eq!(*app.world().resource::<SkateVisibility>(), hidden);
        run(&mut app, "skatehud on");
        assert_eq!(
            *app.world().resource::<SkateVisibility>(),
            SkateVisibility::default()
        );
    }

    #[test]
    fn preferences_survive_restart_and_failed_saves_preserve_the_current_state() {
        let directory = std::env::temp_dir().join(format!(
            "benilla-skate-visibility-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("visibility.json");
        let mut visibility = SkateVisibility::read(&path).unwrap();
        assert_eq!(visibility, SkateVisibility::default());
        let hidden = SkateVisibility {
            show_score: false,
            show_flag: false,
        };
        visibility.apply(hidden, Some(&path)).unwrap();
        assert_eq!(SkateVisibility::read(&path).unwrap(), hidden);
        assert!(visibility
            .apply(SkateVisibility::default(), Some(&path.join("blocked.json")))
            .is_err());
        assert_eq!(visibility, hidden);
        assert_eq!(SkateVisibility::read(&path).unwrap(), hidden);
        // Hand-written partial configs keep the other display enabled by default.
        std::fs::write(&path, r#"{"show_score": false}"#).unwrap();
        let partial = SkateVisibility::read(&path).unwrap();
        assert!(!partial.show_score && partial.show_flag);
        std::fs::write(&path, r#"{"show_score": "off"}"#).unwrap();
        assert!(SkateVisibility::read(&path).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
