# World of Skatecraft multiplayer implementation plan

Friends playing together must see the same skater and skateboard motion: pushing, steering, crouching, jumping, flips, spins, grabs, manuals, grinds, landings, bails, and getting up. Each player keeps immediate local control. Remote players are shown through a short interpolation buffer, so internet latency affects observation without delaying the owner’s controls.

The implementation order is controller responsiveness, the replication protocol and server relay, remote rendering, lifecycle integration, then multiplayer verification. Public registration, administrative permissions, progression anti-cheat, and hosting exposure are outside this friends-only work. Protocol bounds and identity checks remain necessary for stability.

## Architecture and ownership

The owner’s existing skate engine solves their physics and animation. Replicate its actual root, velocity, complete solved bone pose, board joints, simulation state, and presentation events. Do not infer tricks from ordinary WoW movement flags, substitute generic emotes, or resimulate remote physics from controller inputs. Those approaches cannot reproduce the owner’s exact authored motion reliably.

The WoW server remains authoritative for accounts, character identity, map and instance membership, the board aura, spells, death, teleports, and ordinary gameplay. It relays validated skate presentation snapshots from the authenticated owner to clients that can see that character. This is owner-authoritative skate presentation for trusted friends; it does not claim server-authoritative skate physics or deterministic rollback.

Use a versioned binary extension on the authenticated world connection, with explicitly allocated custom opcodes. Advertise the extension before a client sends it; negotiate version and rig layout. Stock servers receive no extension packets. Stock clients continue receiving ordinary WoW movement and are not sent custom packets. Every friend needs the matching skate client and their own required assets.

Reuse the existing connection for the first implementation. TCP supplies authenticated identity, ordering, framing, and reliable lifecycle delivery without opening another port. Its loss recovery can stall later traffic. Bound pending presentation work, discard obsolete snapshots before serialization, and measure congestion. If the acceptance tests show sustained visual delay under the supported network conditions, introduce an authenticated UDP presentation channel before calling the work complete; retain lifecycle and capability negotiation on the world connection. Do not hide transport stalls by increasing the interpolation buffer indefinitely.

## Controller input and simulation timing

Replace the unbounded render-frame FIFO with a bounded, timestamped input handoff. Controller sampling must continue independently of collision preparation and rendering. Keep the newest held state and a short chronological history of meaningful button and stick changes. Preserve quick presses and the stick paths used by flick tricks; do not simply drain every frame and lose intervening gestures.

Run the skate engine on its native simulation period using a monotonic clock. Limit catch-up work and abandon old elapsed time after a stall. Repeated slow frames must never produce an ever-growing backlog. Camera and boost settings use replaceable state; one-shot Rocket Boost events must remain one-shot. Collision rebuild requests and completed poses are also bounded and coalesced independently of lifecycle events.

Reset input history, native pad state, button edges, and accumulated elapsed time when the controller disconnects or changes slots, skating suspends, or a new skating session starts. Fresh input after reconnect must not replay an old jump or steering command. Stop sampler and worker threads when their owners disappear. Avoid busy loops while suspended.

Instrument sample time, sample age when consumed, tick duration, missed deadlines, dropped stale samples, collision installation duration, and pose age at presentation. Log periodic summaries and exceptional stalls, not every poll in normal play. These measurements distinguish software backlog from delay before input reaches XInput. A software fix cannot prove that Bluetooth or Steam Input no longer adds delay without an actual controller session.

Acceptance: deterministic overload and reconnect tests show bounded memory, no replay of stale commands, preserved fresh taps and flick sequences, no duplicate boosts, and prompt recovery after injected stalls. On a healthy 60 Hz simulation, sample-to-consumption time should normally remain within two simulation periods. Verify a longer real Bluetooth session and compare USB if upstream delay remains.

## Snapshot format and compatibility

Define one codec shared by the client sender and receiver and a matching server validator. Document byte order, units, bounds, and golden packet fixtures. Use an explicit protocol version and message kind. Separate capability negotiation, rig definition, active pose snapshots, and stop/reset events.

Each snapshot includes a skating session generation, monotonically increasing sequence, source time, server receipt time, map context, root translation and orientation, velocity, facing, simulation phase, and the complete pose used by the owner’s skater and board rendering. Include discontinuity and event identifiers. The server supplies the sender GUID from its session; clients cannot choose another player’s identity.

Keep large world coordinates separate from the root-relative skeleton. Negotiate a stable ordered bone dictionary and fingerprint before using bone indices. Check duplicate names, unknown versions, unreasonable counts, nonfinite values, invalid transforms, malformed lengths, and trailing data. A mismatched rig must produce a clear compatibility error rather than twisted limbs or a crash.

Start with independent full snapshots. Compress pose representation only with measured round-trip error; preserve translations, rotations, and any scale used by the engine. Do not send names or immutable bind poses every frame. Full snapshots allow immediate recovery after joining or dropping an obsolete update. Add delta compression only if bandwidth measurements justify the complexity, with acknowledged baselines and periodic full recovery snapshots.

Publish at a fixed 20 Hz initially, independent of render frequency. Send state changes promptly and keep stationary skaters refreshed. Never replay a burst of all missed publishes after a stall. Validate actual flip and spin motion at this rate; raise the rate or include angular continuity information if interpolation aliases fast rotation.

## Server relay and visibility

Process extension messages under the same session and world ownership rules as player movement. Accept active snapshots only while the owner is alive, in the world, and allowed to skate. Bound message bytes, rate, bone counts, cached state, and recipient fan-out. A malformed presentation packet must not corrupt the world stream or terminate another player’s session.

Relay only to compatible clients in the correct map and instance whose normal visibility set contains the owner. Respect GM invisibility and other existing visibility rules. Do not broadcast the continent’s poses to every account or base visibility solely on a client-provided coordinate.

Cache only the current valid pose and rig definition per session. When a compatible observer enters view, send a complete definition and current snapshot immediately. Remove cached state on logout, lost connection, board dismissal, death, map transfer, and session replacement. Make stop/reset notifications idempotent. Never retain live Player pointers after their world lifetime ends.

Keep ordinary movement flowing so targeting, proximity, spells, nameplates, and existing server gameplay follow the skater’s actual location. Remote rendering must use the skate snapshot for the body and board together; it must not combine a delayed skate skeleton with a separately extrapolated WoW position.

## Remote presentation

Store bounded snapshots by GUID and session generation. Map sender times onto a stable local timeline without requiring synchronized PC clocks. Start around a 100 ms interpolation delay, adapt within a finite range to measured jitter, and record the actual buffer age. Reject duplicate, older, and retired-session packets. Rejoining must create a new generation.

Interpolate the root and every animated bone on the same timeline. Interpolate articulated local transforms in the known hierarchy, so limbs, trucks, and wheels keep their relationships; independently interpolating global joint positions can shorten a limb or deform a rotating board. The renderer can retarget both snapshot endpoints into the recipient’s M2 hierarchy before interpolating those local transforms. Board interpolation uses its deck, truck, and wheel hierarchy. Use quaternion interpolation for rotations and linear interpolation for translation and stretch, with explicit handling of discontinuities. Native matrices may contain small scale or shear effects; preserve the complete affine transform at snapshot endpoints and use a polar decomposition if ordinary rotation and scale decomposition loses those effects. Verify spins and board flips rather than assuming endpoint interpolation captures them. Do not interpolate across a teleport, respawn, map change, or large instantaneous reset.

Give each visible skater their own retargeting cache, held animation state, board mesh instance, and pose. Share immutable board textures, materials, bind data, and model layouts. Reuse the same retargeting and board skinning functions for local and remote poses; there must not be a reduced remote skeleton. Existing race, gender, equipment, attachments, and sheath behavior must still work.

While valid skate presentation owns a remote character, prevent ordinary remote locomotion, body twist, terrain conform, and gait animation from overwriting it. Continue updating the normal networking state underneath so walking resumes correctly when skating ends. Destroy board entities and per-instance mesh assets when a character leaves view; repeated visibility changes must not leak assets.

On a short underrun, hold the latest complete pose or extrapolate only the root for a strictly limited time. Never invent another trick or continue moving indefinitely. After the stale timeout, relinquish skate rendering cleanly and log the timeout. A fresh independent snapshot restores the skater without requiring a manual reconnect.

## Events and sound

Replicate the presentation events needed for remote pop, landing, grind, flip, skid, and bail effects with event IDs, session generation, and source time. Deliver each event once even when poses are coalesced. Trigger remote effects on the same interpolated timeline as the visible pose. Rolling and grinding loops follow the interpolated phase and speed.

Remote skating sounds use spatial playback at the skater or board, distance attenuation, and the existing sound settings. Stop loops on dismissal, stale timeout, visibility loss, death, and disconnect. The owner’s camera, speed adjustment messages, personal score HUD, and private UI remain personal; world-visible spell effects continue using the ordinary server spell system.

## Implementation sequence

1. Save this design and establish repeatable fixtures from the native engine: idle, push, crouch, jump, flip, spin, grab, grind, landing, bail, and recovery.
2. Implement and verify bounded controller sampling, worker scheduling, output publication, reconnect resets, and timing diagnostics. Build the usable client before moving to replication changes.
3. Add the versioned codec, packet fixtures, capability negotiation, and authenticated server extension. Test malformed messages, identity, map boundaries, visibility, and lifecycle cache removal.
4. Refactor local rendering into reusable per-skater presentation. Add remote snapshot buffering, interpolation, body ownership, full board skinning, and asset cleanup.
5. Connect live publication and reception in both directions. Handle visibility entry, joining mid-trick, walking transitions, controller reconnect, teleports, death, logout, and world reconnect.
6. Add synchronized presentation events and spatial skate audio. Verify ordinary spells and buffs still appear on the correct characters.
7. Run deterministic latency and overload tests, a live two-client session, then a longer multi-client soak. Package matching client and server artifacts and document the friends’ connection setup.

## Connecting friends

Make the launcher’s authentication host configurable and use a realm address that every invited friend can reach. The existing localhost values cannot identify the host computer from another machine. For a private LAN or VPN, bind the authentication and world ports on the appropriate interface and advertise that interface’s address in the realm list. For internet connections, the advertised address, router forwarding, and host firewall must agree. Keep server discovery and the advertised world address distinct; changing only the launcher’s authentication address is insufficient.

Provide a matching client build and pinned engine preparation steps, with explicit protocol and asset compatibility checks. Each friend supplies their own required game assets. Test connection from a second machine, then test mutual skating visibility on the same map and in an instance. Hosting configuration is applied only when the intended connection method and reachable address are known.

## Verification and release criteria

Use isolated test characters and accounts; never log a scripted client into the owner’s account. Keep installs read-only, probes silent and unattended, and test results in benilla-config. Protocol tests exercise actual encode/decode boundaries, server acceptance and rejection, observer visibility, and restart behavior. Native engine fixtures compare the transmitted and reconstructed body and board numerically, including the maximum error after compression and interpolation.

A live two-client test must prove reciprocal visibility: both clients see the other’s board and every supported trick phase while their own input remains responsive. Cover different race and gender rigs and equipped items. Confirm the remote body and board share a timestamp and transform during airborne tricks and bails.

Exercise 0, 50, 100, and 200 ms added round-trip latency, variable jitter, loss-induced TCP stalls, a brief outage, render slowdown, and collision refresh. Keep memory and pending snapshot counts bounded. Recover from a one-second stall without playing a second of outdated motion or controls. Measure average and worst pose age, receive rate, interpolation underruns, CPU cost per remote skater, and bytes per second.

Soak repeated mounting, dismissing, visibility changes, map transfers, disconnects, and rejoining. Verify no duplicate board, hidden lingering board, stale session resurrected by a packet, sound loop left running, mesh leak, or competing transform writer. Check ordinary walking, movement correction, camera preferences, speed settings, Speed Demon, Rocket Boost, and profession rewards for regression.

Completion requires passing these functional checks and recorded measurements. Compilation alone does not demonstrate synchronized tricks or fix an intermittent Bluetooth symptom. An incomplete phase remains explicitly incomplete; do not describe a partial relay or generic animation substitute as full multiplayer skating.

## Technical references

[SnapNet’s snapshot interpolation architecture](https://snapnet.dev/blog/netcode-architectures-part-3-snapshot-interpolation/) explains separate local and interpolated timelines, interpolation under jitter, and serialization tradeoffs. This design applies interpolation to owner-solved presentation; the server authority boundary is stated above.

[Valve’s Steam Input gamepad emulation guidance](https://partner.steamgames.com/doc/features/steam_controller/steam_input_gamepad_emulation_bestpractices) describes the XInput path used by emulated PlayStation controllers. Timing diagnostics begin where that input reaches this client and cannot measure earlier Bluetooth transport delay.
