# Personal Skate flag

In Skate mode, hold the controller's Select/Back button (PS5 Create/Share through the controller mapping) for 0.7 seconds to save a flag. Release a short press to return. Holding replaces the previous point; releasing a completed hold does not teleport.

The orange flag is visible only to its owner and has no collision. Placement records the character's current position and actual facing, including while walking with the board. Return uses the native engine's walking state, holds the board, and resets movement. Triangle mounts the board again.

Every player can use `/skateflag` to toggle the marker, or `/skateflag off` / `/skateflag on` to
set it explicitly. Hiding it leaves the saved point and return controls working. Use
`/skatehud off` to hide both the flag and the skate score display for filming; `/skatehud on`
shows both again. These are local preferences remembered across launches. See the
[filming commands and configuration](skate-replay.md#hide-score-and-respawn-flag-for-filming).

WoW mode disables both controls and hides the marker without deleting it. Changing camera does not change this rule. The server restores the saved point at login; enter Skate mode and tap Select to return. Logging out or restarting the server preserves it. Only replacing the point or permanently deleting the character removes its old position.

The server validates and stores one row per character in `character_skatecraft_flag`. Versioned private `SKATEFLAG` addon messages use authenticated character ownership, request numbers, and saved revisions. A standard server teleport distributes the returned position and facing to observers. The flag itself is never shared with other players. Builder objects continue to use their existing shared session ownership and are removed at owner logout independently of the saved flag.

Same-map, same-instance, same-zone returns never raise a loading cover. While skating, the terrain streamer retains the saved destination as one additional bounded residency window, and a return waits for destination collision before native physics resumes. Cross-map transfers retain the ordinary loading transition. An instance flag remains saved when its original instance is unavailable; the server refuses an unsafe return to a different instance.

A temporary loading hold during return keeps Skate mode active while native activation retries. The character resumes in walking mode with the board held once the destination is ready.

No bail thresholds, bail abilities, or scoring rules are changed by this feature.
