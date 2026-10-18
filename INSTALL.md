# Installing World of Skatecraft

`setup/setup.sh` (Linux) or `setup.bat` (Windows, experimental) does all of this for you. This page explains what it does and what it asks, so
you know what to expect. If a step fails, setup stops, says what to fix, and continues from that
step when you run it again.

A new installation without an existing server initializes a fresh database. No developer characters, accounts, passwords, saved
flags, or database dumps are included. Setup creates the required databases and asks you to
create your own game account. The mod's additional tables are initialized by source code,
without importing anyone else's player records.

**Already played the base mod?** Keep your existing `server/` folder and database. Setup detects
the existing Docker project's server folder and upgrades it in place, keeping your characters,
accounts, passwords, gear and progress. If it cannot find that folder automatically, run
`setup.bat -Server "C:\original-skatecraft\server"` on Windows, or
`setup/setup.sh --server "/path/to/original-skatecraft/server"` on Linux. Use your existing login
after setup. See [database upgrades and recovery](docs/database-upgrades.md) for standalone
commands and backups.

## 1. Before you start

You need:

| | Where it comes from |
|---|---|
| WoW 1.12.1, English, build 5875 | Your own copy. The folder with `WoW.exe` and `Data/` |
| Skate 3 (Xbox 360), extracted | Your own disc or digital copy, extracted so you have `default.xex` and `data/` side by side. For a disc image use [extract-xiso](https://github.com/XboxDev/extract-xiso): `extract-xiso -x "Skate 3.iso"` |
| An Xbox-style controller | Xbox pads on the `xpad` driver work best; Bluetooth and PlayStation pads work too |
| Docker with the compose plugin | Your distribution's packages |
| ~20 GB free | WoW, the server's map data, the build |

Then run `setup/setup.sh --doctor`. It checks everything below and prints the exact install
command for your distribution if something is missing:

- Rust (via [rustup](https://rustup.rs)), a C compiler, pkg-config, ALSA and libudev development
  files, git, curl, Python 3 with venv.
- Docker installed, running, and usable without `sudo` (you are in the `docker` group; log out and
  back in after adding yourself).

On NixOS the scripts enter the repo's `shell.nix` by themselves; you only need Docker enabled in
your system configuration.

## 2. What setup does

1. **Checks your machine** (the doctor above).
2. **Links your WoW client.** It asks for your WoW folder and checks it has the 1.12.1 data
   files. The game reads it directly; nothing in it is changed.
3. **Converts Skate 3 data.** It asks for your extracted Skate 3 folder, downloads the Skate 3
   Rust Engine's converter at a fixed version, and writes the skater, animations and physics
   settings to `skate-data/`. This takes a few minutes.
4. **Converts Skate 3 sounds** into `skate-audio/` (rolling, pops, landings, grinds). It
   downloads vgmstream to decode them. If this fails, skating still works, just silently.
5. **Sets up the local server** ([vmangos](https://github.com/vmangos/core) through
   [vmangos-deploy](https://github.com/mserajnik/vmangos-deploy)) in `server/`, and extracts
   map data from your WoW client. **The extraction takes several hours** and uses a couple of CPU
   cores; leave it running. Existing server data is reused. Setup builds matching database and
   patched server images from pinned source, backs up an existing database before changing it,
   upgrades the deployment configuration and adds the mod's missing tables. Existing world edits
   are kept; automatic world-database resets are disabled. Extraction uses a scratch folder and
   mounts your WoW `Data/` read-only.
6. **Creates your game account on a fresh database.** It asks for a name and a password. The
   account is a GM. If accounts already exist, setup keeps them and their passwords.
7. **Builds the game.** The first build takes several minutes. Cargo downloads the skate engine
   from its authors' repository during this step.

Afterwards, start the game with `./play.sh`.
Setup defaults to one compilation job for both Rust and the Docker server build to keep the PC
responsive while other projects are running.

## 3. Where things are

| Folder | What | Safe to delete? |
|---|---|---|
| `WoW` | a link to your WoW folder | yes, setup asks again |
| `skate-data/`, `skate-audio/` | converted from your Skate 3 | yes, setup converts again |
| `server/` or your selected existing server folder | server configuration and map data; Docker stores characters in the database volume specified by its Compose file | keep it and its database volume |
| `benilla-config/` | game settings, keybindings, addons (`benilla-config/AddOns/`) | yes, settings reset |
| `.setup/database-backups/` | private database snapshots and original configuration for recovery | keep until you have verified the upgrade; copy important backups elsewhere |
| `.setup/server-path` | the server folder used by setup and the launchers | keep when using an existing installation outside this checkout |
| other `.setup/` files, `setup.log` | setup's tools and log | regenerable; keep account/launcher settings you want to retain |

## 4. Everyday use

- Play: `./play.sh` (starts the server if it isn't running).
- Stop the server: `docker compose -p world-of-skatecraft -f server/compose.yaml down`.
- Client-only update on Windows: close the game, run `git pull`, then double-click
  **`rebuild.bat`**. It only runs the release client build, reuses the Cargo cache, and writes
  `target\release\benilla.exe`. It does not start the game or run setup, server migration, or
  asset conversion. Use `rebuild.bat --no-pause` from a terminal to skip the final key press.
- Full update: `git pull`, then run setup again (Windows: `setup.bat`; Linux: `setup/setup.sh`).
  Use this when the update requires server/database changes, native-engine preparation or new
  asset conversion. Setup backs up and migrates an existing database and verifies that the
  matching patched server runs.
- New account or new password: `python3 tools/make_account.py <name> <password>`.

## 5. Windows (experimental)

The Windows setup uses Docker Desktop and the Visual Studio C++ tools. This fork is developed
on Windows and builds the patched server from source before starting it. Please report any
setup issues (see the README).

1. Install the tools. In a terminal (PowerShell or Command Prompt):

   ```bat
   winget install -e --id Git.Git
   winget install -e --id Rustlang.Rustup
   winget install -e --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   winget install -e --id Python.Python.3.12
   winget install -e --id Docker.DockerDesktop
   ```

   Restart Windows afterwards, then start Docker Desktop and let it finish starting (it uses
   WSL 2; it will ask to set that up the first time).
2. Clone to a short path (long paths can break the build) and run setup:

   ```bat
   git clone https://github.com/73igg/world-of-skatecraft-skateparkbuilder.git C:\skatecraft
   cd C:\skatecraft
   setup.bat
   ```

3. Start the game with `play.bat`.

For subsequent client-only updates, pull the new source and use `rebuild.bat` instead of running
setup again. The initial Rust/C++ tools and prepared Skate engine must already be installed.

The folders are the same as on Linux, with `WoW` as a junction (a folder link) instead of a
symlink. Stop the server with `docker compose -p world-of-skatecraft -f server\compose.yaml down`.
If you selected another server folder, use its `compose.yaml` path instead. Never add `--volumes`
to `down` unless you intend to delete your characters.
