# Skate replay

The client records up to the last 90 seconds of its own native skate poses while skating by
default. The F6 editor's Window −15s / Window +15s buttons choose a 15–90 second recording
window, remembered locally across launches. They also select that recent portion of the
current replay and pause at its start. Increasing the window can use footage still in the
frozen clip, but cannot recover footage already discarded while skating.
Recording happens before the replaceable display queue, so a missed render does not drop a
simulation pose. A 192 MiB memory budget can shorten this window in unusually large recordings.
Resuming after a skateboard-mode break discards expired history, so the gap cannot extend the
rolling recording beyond its limit and prevent an editable clip from saving.
Same-map flag returns retain the preceding run and create a cut at the return. Changing maps,
instances or appearance starts a new rolling recording. Opening replay freezes that recording
and parks the live character. The server, other players,
and Bluetooth polling continue running. Recorded poses never enter movement packets, XP awards,
flag commands or builder edits. Exit restores the live location and resumes offboard carrying
the skateboard, after held controls have been released.

## Controls

Open the editor while in Skate mode after recording some footage. Controller names below use
PlayStation / Xbox labels. Keyboard shortcuts apply while the replay editor is open, except
F6, which also opens it. Actions without a shortcut are listed in the editor-buttons table.

| Action | Keyboard / mouse | Controller |
|---|---|---|
| Open / close replay | F6; Skate Replay ability also toggles it | Click both sticks together (L3 + R3) |
| Close replay | Escape | Circle / B |
| Cancel an active export and return to skating | Escape or F6 | Circle / B or both stick clicks |
| Play / pause | Space | Cross / A |
| Scrub backward / forward and pause | Hold Left / Right arrow, or drag the timeline | Hold L2 / R2 (LT / RT) |
| Previous / next recorded simulation frame | Comma / Period | D-pad Left / Right |
| Set trim start / end at the current time | Left bracket `[` / right bracket `]` | Use Set start / Set end buttons |
| Jump to trim start / end and pause | Home / End | Use the timeline |
| Cycle camera mode | C | Triangle / Y |
| Move free camera forward / backward / left / right | W / S / A / D | Left stick |
| Move free camera down / up | Q / E | L1 / R1 (LB / RB) |
| Double free camera movement speed while held | Either Shift key | Hold left stick click (L3) |
| Aim free camera | Hold right mouse and move | Right stick |
| Save or replace a camera key at the current time | T | Square / X |
| Save or replace a camera cut at the current time | **Left Shift + T** | Use Set camera cut button |
| Select previous / next camera key and pause | Page Up / Page Down | Use Previous key / Next key buttons |
| Remove the selected camera key or cut | Delete | Use Remove key button |
| Save an editable clip | Ctrl + S | Use Save clip button |
| Open / close the saved-clip library | Ctrl + O | Use Saved clips button |
| Export the trimmed clip as video | Ctrl + E | Use Export video button |

Either Ctrl key works for save, library and export shortcuts. Holding Ctrl suppresses WASD
and Q/E camera movement so these shortcuts do not move the shot. The cut shortcut specifically
uses **Left Shift**; Right Shift still works for the free camera speed boost. During export,
editing and playback controls are disabled; the cancel controls above remain available.

Select / Back / Create remains reserved for the personal flag. Many actions above also have
clickable editor buttons. These additional controls have no dedicated keyboard or pad shortcut:

| Editor button | What it does |
|---|---|
| Window −15s / Window +15s | Choose a 15–90 second recording window in 15-second steps; select that recent portion of the current recording and pause at its start. |
| Reset trim | Restore the full available recording as the trim range. |
| Slower / Faster | Adjust overall clip playback speed, scaling existing key speeds too, within 0.1x–4x. |
| Toggle camera keys | Enable or disable the edited camera path without deleting keys. |
| Zoom in / Zoom out | Adjust the current camera's field of view; save a key to include it in the path. |
| Move slower / Move faster | Adjust free camera travel speed from 1 to 64 yards per second. |
| Move response | Cycle Direct, Smooth and Soft free camera acceleration and braking. |
| Key motion | Cycle the selected key's outgoing segment through Smooth continuous, Linear and Ease in/out. |
| Key slower / Key faster | Adjust playback speed at the selected key in 0.25x steps, within 0.1x–4x. |

In Saved clips, click a clip to load it. Use Previous key / Next key (Page Up / Page Down)
before changing Key motion, Key slower / Key faster, or removing a key. The status line shows
the selected key's motion and the current free camera speed and response.

## Camera modes and keys

C cycles **Recorded → Free → Follow → Tripod → Recorded**:

- **Recorded:** the camera actually shown during skating, including collision and smoothing.
- **Free:** move and aim the camera yourself to compose shots.
- **Follow:** move with the skater at a fixed offset and aim at them.
- **Tripod:** keep the camera position fixed and turn to aim at the skater.

Changing camera mode disables the edited path for comparison. **Toggle camera keys** restores
it; selecting or saving a key also enables it. When enabled, the path controls the camera
during seeking, playback and export. While paused in Free mode, you can reposition the camera
before saving or replacing a key.

To create a camera path:

1. Pause and scrub to the start of the shot, then switch to Free camera with C.
2. Move, aim and zoom to frame the skater, then press T to save the first key.
3. Scrub forward, compose the next shot and press T again. Repeat where framing should change.
4. Use Home and Space to preview from the trim start. Adjust keys with Page Up / Page Down,
   reposition the camera, then press T to replace the selected key.
5. Save the editable clip with Ctrl + S; use Ctrl + E to export the trimmed video.

Yellow timeline markers are camera keys; red markers are cuts. Each key stores camera position,
orientation, zoom and playback speed at that replay time. Keyed shots use the aim you save;
they do not automatically track the skater. Covering the same camera distance in less replay
time makes the camera move faster.

**Key motion applies from the selected key to the following key.** New keys use Smooth
continuous; select a key and click Key motion to cycle the options:

| Key motion | Result |
|---|---|
| Smooth continuous | Position, rotation and zoom flow through intermediate keys, with a stop at each end of the shot. Use it on consecutive keys for continuous movement. |
| Linear | Move directly between keys at a steady rate within each segment; direction or speed can change abruptly at a key. |
| Ease in/out | Accelerate away from each key and slow to a stop at the next. |

A cut holds the preceding key's shot until the cut time, then switches instantly to the cut's
shot. Smoothing restarts at the cut. Press Left Shift + T or Set camera cut to create one.
Existing saved clips retain their original Ease in/out motion until you change their keys.
Smooth paths can curve outside the straight line between keys; preview shots near obstacles.
Playback speed blends between key speeds independently of the selected camera-motion type.

## Free camera movement settings

Move slower / Move faster adjusts travel speed from 1 to 64 yards per second, starting at
8 yards per second. Either Shift key or left stick click doubles the selected speed while
held. Move response cycles:

- **Direct:** immediate movement and stopping.
- **Smooth:** a short acceleration and braking response; the default.
- **Soft:** a longer acceleration and braking response.

These controls affect positioning the free camera. Key motion controls the saved path, and
Slower / Faster or Key slower / Key faster controls clip playback speed. Free camera speed
and response are remembered between launches.

## Render distance and fog

Graphics options allow Terrain Distance from 177 to 3,000 yards, including while editing and
exporting replays. Turn Distance Fog off for a clear view to that range; the sky and coarse
horizon silhouettes retain their colours. Both settings are remembered locally. Longer
distances load more map tiles and objects, increasing memory use and reducing frame rate;
this does not load the entire continent. The defaults remain 350 yards with distance fog on.
Exports include baked map geometry as well as terrain, interactable props and historical
builder objects, using the world camera's anti-aliasing setting.

## Saved clips and video

Editable `.skate-replay` clips and exported MP4 files go in `benilla-config/Replays`, beneath
this checkout's central local-state folder. This directory is ignored by Git. Clips contain
motion, final character joint poses, camera tracks, and historical builder definitions. They
contain no model meshes, textures, extracted sounds, accounts, credentials or install paths.
All referenced models and sounds are loaded from the user's own game conversions.
Recording-window and free-camera preferences are stored in `benilla-config/Replays/settings.json`.
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
