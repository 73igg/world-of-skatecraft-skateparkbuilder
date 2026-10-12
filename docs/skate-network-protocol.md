# World of Skatecraft presentation protocol

Version 1 carries the owner’s complete solved skater and board pose through the authenticated world connection. It is an extension for the [multiplayer implementation plan](skate-multiplayer-plan.md). Capability negotiation must complete before custom packets are sent. The codec and receiver clock are implemented in [skatecraft.rs](../crates/benilla-protocol/src/skatecraft.rs) and [timeline.rs](../crates/benilla-protocol/src/skatecraft/timeline.rs).

## Framing and direction

The fork reserves client opcode `0x03f0` and server opcode `0x03f1`. Ordinary world header framing and session encryption still apply. These opcodes require explicit registration in the patched server; stock servers must never receive them.

An owner message begins with little-endian `u16 version`, then `u8 kind`. All integers and IEEE 754 float32 values are little endian. Strings begin with a byte length and contain UTF-8 without a terminator. Strings reject control characters. Every decoder consumes the entire body; trailing bytes are an error.

The server prefixes each outgoing message with `u64 sender_guid`, `u32 server_receipt_ms`, `u32 map_id`, and `u32 instance_id`. The owner supplies none of these routing fields. Sender GUID zero is reserved for Welcome, and Welcome must use zero. Hello is a client request and cannot appear in a server relay.

| Kind | Value | Direction | Payload |
| --- | --- | --- | --- |
| Hello | 0 | Client to server | No payload; request version 1 after advertisement |
| Welcome | 1 | Server to client | No payload; acknowledge compatible version |
| Definition | 2 | Owner to server to observers | Session generation, asset fingerprint, ordered joint names |
| Snapshot | 3 | Owner to server to observers | Compressed independent full pose |
| Stop | 4 | Owner or server to observers | Session generation to retire |
| Events | 5 | Owner to server to observers | Session generation and ordered presentation events |

Advertise support using the server’s existing private addon setting channel. The client requests Hello only after a server advertisement. The server sends custom packets only to sessions that successfully negotiate. A missing advertisement leaves the original WoW connection untouched. An unsupported version must fail compatibility explicitly; it must never decode with a guessed layout.

## Definitions

Definition contains `u64 generation`, 20 fingerprint bytes, `u16 joint_count`, then that many length-prefixed joint names. Generation zero is invalid. Joint count is 1 through 128. Names are nonempty, unique, and at most 64 UTF-8 bytes.

The fingerprint identifies the compatible rig and board assets. The integration must derive it from the pinned rig and board content, compare it with the observer’s assets, and validate the ordered dictionary before applying a pose. The packet’s hash field alone does not establish compatibility. New skating sessions use a fresh generation; reconnecting must not reuse a prior generation. Definitions are sent once per session and when an observer enters visibility.

## Full pose snapshots

Snapshot begins with `u16 inflated_length`, followed by one complete zlib stream. The maximum inflated body is 16,384 bytes. Inflation is bounded even if the sender declares a smaller size. A decoder verifies inflated size, compressed stream integrity, full compressed consumption, and exact joint byte count. There are no dependent deltas or missing baselines.

The inflated fields are:

| Field | Representation | Meaning |
| --- | --- | --- |
| Generation | u64 | Definition and owner session |
| Sequence | u32 | Increasing pose sequence, with wrap handling |
| Source time | u64 | Milliseconds on the owner’s monotonic presentation clock |
| Flags | u8 | Bit 0 means discontinuity; other bits are invalid |
| Root | 12 float32 values | Full affine basis and Bevy world translation |
| Velocity | 3 float32 values | Bevy world yards per second |
| Facing | float32 | Character facing in radians |
| State | Length-prefixed string | Native simulation state, at most 96 UTF-8 bytes |
| Joint count | u16 | 1 through 128, must match the accepted definition |
| Joints | 12 float32 values each | Complete root-relative solved affine matrices |

An affine matrix is column-major `X.xyz, Y.xyz, Z.xyz, translation.xyz`; its omitted row is `0, 0, 0, 1`. Root translation uses world yards. Joint translations use engine meters. The receiver must convert units once, preserving the root and skeleton’s shared frame. Board skinning uses these same joint matrices.

All numeric values must be finite. Basis components are bounded to magnitude 16, world translation to 100,000 yards, local joint translation to 256 meters, velocity to 10,000 yards per second, and facing to twice a complete revolution. Root basis must be nonsingular. These are serialization bounds, not changes to the owner’s speed abilities; integration must report unsupported outgoing data rather than silently publishing a different pose. Do not clamp a matrix to fit the wire format.

Independent full snapshots are initially published at 20 Hz. One pending snapshot replaces obsolete unsent snapshots. A delayed network writer must not accumulate every missed publish. Discontinuities clear interpolation history so teleports and session resets cannot blend through the intervening world.

## Presentation events

Events contains `u64 generation`, `u16 count`, and up to 32 records. Each record is `u32 event_id`, `u64 source_ms`, `u8 event_kind`, and `float32 strength`. Record length is exactly 17 bytes. Event identifiers advance within a generation, and receivers reject duplicates and earlier identifiers with wrap handling.

| Event | Value |
| --- | --- |
| Pop | 1 |
| Land | 2 |
| Flip | 3 |
| Grind start | 4 |
| Grind end | 5 |
| Bail | 6 |
| Recovery | 7 |
| Skid | 8 |

Events use the same source timeline as poses. The integration must retain short events independently of replaceable snapshots and schedule sound and effects at the interpolated source time. Repeated delivery must not replay a landing or bail. Rolling loops follow pose state and velocity rather than repeated start events.

## Receiver clock and session lifecycle

The receiver holds at most 32 full snapshots for a visible owner. Its source-to-local offset is estimated from arrival times; PC wall clocks do not need synchronization. Arrival timestamps are captured by the network thread on a monotonic clock. Jitter adjusts the interpolation delay around 100 ms within 75 through 200 ms. The playback clock advances monotonically even when delay changes.

Snapshots must match the current generation and joint count. Duplicate or older sequence numbers, backward source times, and backward arrival times are rejected. A Stop disables the generation; a delayed snapshot or repeated Definition cannot reactivate it. A new generation retires the previous one, with a bounded record of recently retired generations. The authenticated server’s ordered connection and visibility lifecycle remain part of this guarantee.

On underrun, root extrapolation is limited to 100 ms and the skeleton holds its last solved pose. After 750 ms without an arrival, the buffer yields no skate presentation. The rendering integration relinquishes pose ownership, hides the board, and stops sound loops. A fresh valid snapshot restores presentation. The caller validates the sender’s visible entity, map, instance, and compatible assets before passing packets to the buffer.

## Required integration checks

The server validates owner identity, negotiated version, dictionary, board aura, world state, map context, packet rate, and payload bounds before relaying. It routes only through normal visibility to compatible clients. It caches at most the current definition and pose and removes them on dismissal, death, logout, disconnect, transfer, and replacement of the owner session.

Remote rendering must reuse the local retarget and board skinning functions, maintain per-owner caches and mesh instances, and give the body and board a single interpolated timestamp. Interpolate articulated local transforms in the known M2 and board hierarchies rather than shortening limbs or collapsing wheel spacing by blending global joint positions independently. Quaternion rotation interpolation must preserve native scale and shear at snapshot endpoints. Ordinary movement remains available for server gameplay and walking, while conflicting remote locomotion and terrain adjustments yield to active skate presentation.

These checks are release requirements. Passing codec and buffer tests establishes the data and timing foundation; it does not establish a working server relay or complete remote rendering.
