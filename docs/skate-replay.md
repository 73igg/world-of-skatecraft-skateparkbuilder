# Skate replay

The client records up to the last 90 seconds of its own native skate poses while skating by
default. The F6 editor's Window −15s / Window +15s buttons choose a 15–90 second recording
window, remembered locally across launches. They also select that recent portion of the
current replay and pause at its start. Increasing the window can use footage still in the
frozen clip, but cannot recover footage already discarded while skating.
Recording happens before the replaceable display queue, so a missed render does not drop a
simulation pose. A 192 MiB memory budget can shorten this window in unusually large recordings.
Same-map flag returns retain the preceding run and create a cut at the return. Changing maps,
instances or appearance starts a new rolling recording. Opening replay freezes that recording
and parks the live character. The server, other players,
and Bluetooth polling continue running. Recorded poses never enter movement packets, XP awards,
flag commands or builder edits. Exit restores the live location and resumes offboard carrying
the skateboard, after held controls have been released.

## Controls

| Action | Keyboard / mouse | Controller |
|---|---|---|
| Open / close | F6 or Skate Replay ability; Escape closes | Both stick clicks; Circle / B closes |
| Recording window | Window −15s / Window +15s in the editor | Use the editor buttons |
| Play / pause | Space or Play button | Cross / A |
| Scrub | Drag timeline; Left / Right | L2 / R2 |
| Previous / next native frame | Comma / Period | D-pad Left / Right |
| Camera mode | C or Camera button | Triangle / Y |
| Move free camera | WASD, Q / E; Shift goes faster | Left stick, L1 / R1; left stick click goes faster |
| Aim free camera | Hold right mouse and move | Right stick |
| Zoom | Zoom buttons | Use the editor buttons |
| Trim start / end | [ / ]; Reset trim restores the full recording | Use the editor buttons |
| Jump to start / end | Home / End | Use the timeline |
| Save camera key / cut | T / Shift-T | Square / X saves a key |
| Previous / next camera key | Page Up / Page Down | Use the editor buttons |
| Remove selected key | Delete | Use the editor button |
| Save / saved clips / export | Ctrl-S / Ctrl-O / Ctrl-E | Use the editor buttons |

Select / Back / Create remains reserved for the personal flag. Free camera is an editor fly
camera; move it to the shot you want, then save a key. Yellow timeline markers are camera keys,
red markers are cuts. Seeking previews the camera path; while paused, free camera remains
editable. Camera mode changes let you compare the original and the other camera modes; Toggle camera
keys restores your edited path. Playing or exporting interpolates between enabled keys, including their FOV and playback
speed. Slower / Faster scales the entire clip; Key slower / Key faster adjusts only the key
you have selected with Previous / Next key. A cut holds the preceding shot until the cut time. Recorded camera mode uses the final
camera actually shown during skating, including collision and smoothing.

## Saved clips and video

Editable `.skate-replay` clips and exported MP4 files go in `benilla-config/Replays`, beneath
this checkout's central local-state folder. This directory is ignored by Git. Clips contain
motion, final character joint poses, camera tracks, and historical builder definitions. They
contain no model meshes, textures, extracted sounds, accounts, credentials or install paths.
All referenced models and sounds are loaded from the user's own game conversions.
The recording-window preference is stored separately in `benilla-config/Replays/settings.json`.
Skate Replay is learned at character creation and on login, appears in the General spellbook,
and can be placed on an action bar. The ability opens the same editor as F6.

Loading currently requires the recorded map and instance, and the same character model,
outfit and size. Outfit or model changes start a fresh rolling recording. Loading does not
teleport the live player. The editor reconstructs historical park visuals separately, so
scrubbing moved, deleted or fused objects does not change the authoritative park. Terrain is
streamed around the replay actor as well as the live location.

Video export needs an `ffmpeg` executable on PATH, or `SKATECRAFT_FFMPEG` pointing to it.
A private local installation at `.setup/ffmpeg/bin/ffmpeg.exe` is also detected on Windows.
No FFmpeg binary is distributed in this repository. Export renders at 60 output frames per
second at the current window resolution, uses edited cameras and speeds, and reconstructs
skating sounds from the local converted sound bank. Without that bank the sound track is
silent. WoW ambience, music, chat and voice are not recorded. Keep the window size fixed until
export finishes. Escape cancels the export and returns to skating; incomplete export files
are removed. Encoding runs off the render and controller-polling threads.

## Multiplayer scope

Each client can record and edit its own skating while connected to the server. Replay does
not pause the world. Other characters continue to display their live state: their past poses,
NPCs, combat, weather and chat are not recorded in this version. A complete multi-skater
historical replay requires the separate native-pose replication and recording work; ordinary
WoW movement replication alone does not contain friends' detailed trick skeletons.

## Character retargeting

Runtime retargeting transfers joint rotations onto the target character's own bind skeleton,
then solves pelvis height, stance and leg contacts without lengthening the bones. Torso joints
retain their authored rest alignment. Feet and hooves retain their resting sole orientation
while following animated foot turns, without being tilted onto a human ankle-to-toe direction.
Hands near a grabbing contact use a two-bone reach
solve; unreachable contacts clamp to the limb's actual length. A carried board follows the
retargeted hand. All races default to the standard **1x** board. `/boardsize <number>` changes
the exact visual multiplier for the current race (0.1 to 10); `/boardsize` reports the setting
and `/boardsize reset` restores 1x. Preferences are saved immediately in
`benilla-config/skate/board-sizes.json` and shared by characters of that race using the same
client. This is a local appearance preference, independent of server physics and character
size. Carried decks resize around the hand's grip. The same sizing applies in replay.
Engine camera height also adapts to
the character's natural pelvis height.
The same method applies to all 16 vanilla race/sex models and character size changes. It does
not rewrite M2, skin, mesh, texture or animation assets, so there is no asset conversion step
to redistribute. Highly different body shapes can still need visual tuning for particular
tricks; the solver cannot give a short arm an anatomically impossible reach.
