//! Fork preference: an exact visual board multiplier per playable race, saved locally.
use std::collections::BTreeMap;
use std::path::Path;

use bevy::prelude::*;

use crate::console::ConsoleCommandApp;

const MIN: f32 = 0.1;
const MAX: f32 = 10.;
const USAGE: &str =
    "Use /boardsize <0.1-10> (1 = default), /boardsize reset, or /boardsize to check.";

fn race_name(race: u8) -> Option<&'static str> {
    Some(match race {
        1 => "Human",
        2 => "Orc",
        3 => "Dwarf",
        4 => "Night Elf",
        5 => "Undead",
        6 => "Tauren",
        7 => "Gnome",
        8 => "Troll",
        _ => return None,
    })
}

#[derive(Resource, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub(super) struct BoardSizes(BTreeMap<u8, f32>);

impl BoardSizes {
    pub(super) fn scale(&self, race: Option<u8>) -> f32 {
        race.and_then(|race| self.0.get(&race).copied())
            .unwrap_or(1.)
    }

    fn read(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        let sizes: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if sizes
            .0
            .iter()
            .any(|(&race, &size)| race_name(race).is_none() || !valid(size))
        {
            return Err("Board size settings contain an invalid race or multiplier".into());
        }
        Ok(sizes)
    }

    // Save before applying: a failed write must not claim a persistent change.
    fn set(&mut self, race: u8, size: f32, path: Option<&Path>) -> Result<(), String> {
        if race_name(race).is_none() || !valid(size) {
            return Err(USAGE.into());
        }
        let mut next = self.clone();
        if size == 1. {
            next.0.remove(&race);
        } else {
            next.0.insert(race, size);
        }
        if let Some(path) = path {
            let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
            crate::local_state::write_atomic_bytes(path, &bytes).map_err(|e| e.to_string())?;
        }
        *self = next;
        Ok(())
    }
}

fn valid(size: f32) -> bool {
    size.is_finite() && (MIN..=MAX).contains(&size)
}

fn load(mut sizes: ResMut<BoardSizes>) {
    if let Some(path) = crate::local_state::board_sizes_path() {
        match BoardSizes::read(&path) {
            Ok(saved) => *sizes = saved,
            Err(e) => warn!("board size: could not load preferences: {e}"),
        }
    }
}

fn command(world: &mut World, args: &str) -> Vec<String> {
    let race = world
        .query_filtered::<&crate::net::ObjectStore, With<crate::net::SelfPlayer>>()
        .single(world)
        .ok()
        .and_then(|store| store.0.unit_race());
    let Some((race, name)) = race.and_then(|race| Some((race, race_name(race)?))) else {
        return vec!["Log into a character before using /boardsize.".into()];
    };
    if args.is_empty() {
        let size = world.resource::<BoardSizes>().scale(Some(race));
        return vec![format!("{name} board size: {size}x. {USAGE}")];
    }
    let size = if args.eq_ignore_ascii_case("reset") {
        1.
    } else {
        match args.parse::<f32>() {
            Ok(size) if valid(size) => size,
            _ => return vec![USAGE.into()],
        }
    };
    let path = crate::local_state::board_sizes_path();
    match world
        .resource_mut::<BoardSizes>()
        .set(race, size, path.as_deref())
    {
        Ok(()) => {
            let storage = if path.is_some() {
                "Saved for all your characters of this race."
            } else {
                "Applied for this session; local persistence is disabled."
            };
            vec![format!("{name} board size: {size}x. {storage}")]
        }
        Err(e) => vec![format!(
            "Board size was not changed: could not save preferences: {e}"
        )],
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<BoardSizes>()
        .add_systems(Startup, load)
        .console_command("boardsize", USAGE, command);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn race_preferences_round_trip_and_reset_independently() {
        let folder = std::env::temp_dir().join(format!(
            "benilla-board-size-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = folder.join("boards.json");
        let mut sizes = BoardSizes::read(&path).unwrap();
        for race in 1..=8 {
            assert_eq!(sizes.scale(Some(race)), 1.);
        }
        sizes.set(7, 0.7, Some(&path)).unwrap();
        sizes.set(6, 1.75, Some(&path)).unwrap();
        let mut reloaded = BoardSizes::read(&path).unwrap();
        assert_eq!(reloaded.scale(Some(7)), 0.7);
        assert_eq!(reloaded.scale(Some(6)), 1.75);
        assert_eq!(reloaded.scale(Some(5)), 1.);
        reloaded.set(7, 1., Some(&path)).unwrap();
        let reset = BoardSizes::read(&path).unwrap();
        assert_eq!(reset.scale(Some(7)), 1.);
        assert_eq!(reset.scale(Some(6)), 1.75);
        // A file cannot be a parent directory; a failed save leaves the preference unchanged.
        assert!(reloaded
            .set(6, 2., Some(&path.join("blocked.json")))
            .is_err());
        assert_eq!(reloaded.scale(Some(6)), 1.75);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn invalid_values_and_races_do_not_change_preferences() {
        let mut sizes = BoardSizes::default();
        for size in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.,
            -1.,
            0.09,
            10.1,
        ] {
            assert!(sizes.set(7, size, None).is_err());
            assert_eq!(sizes.scale(Some(7)), 1.);
        }
        assert!(sizes.set(9, 1., None).is_err());
        assert_eq!(sizes.scale(None), 1.);
    }
}
