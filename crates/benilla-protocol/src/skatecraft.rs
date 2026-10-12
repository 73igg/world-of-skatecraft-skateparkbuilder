//! World of Skatecraft extension. These packets are sent only after capability negotiation.
//! Poses carry every solved joint; ordinary WoW movement is a separate gameplay channel.

use std::collections::HashSet;
use std::io::{self, Read, Write};

use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};

use crate::wire::{read_f32_le, read_u16_le, read_u32_le, read_u64_le, read_u8};

#[path = "skatecraft/timeline.rs"]
pub mod timeline;

pub const VERSION: u16 = 1;
pub const CLIENT_OPCODE: u16 = 0x03f0;
pub const SERVER_OPCODE: u16 = 0x03f1;
pub const MAX_BONES: usize = 128;
pub const MAX_EVENTS: usize = 32;
pub const MAX_BODY_BYTES: usize = 16 * 1024;
pub const DISCONTINUITY: u8 = 1;
const MAX_NAME_BYTES: usize = 64;
const MAX_STATE_BYTES: usize = 96;
const MAX_COMPRESSED_BYTES: usize = MAX_BODY_BYTES + 256;

/// Column-major affine transform: X.xyz, Y.xyz, Z.xyz, translation.xyz.
/// Root translation is Bevy world yards. Bone translations are root-relative engine meters.
pub type Affine = [f32; 12];

#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    pub generation: u64,
    /// Fingerprint of the owner's rig and board assets, checked by each observer.
    pub fingerprint: [u8; 20],
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventKind {
    Pop = 1,
    Land = 2,
    Flip = 3,
    GrindStart = 4,
    GrindEnd = 5,
    Bail = 6,
    Recovery = 7,
    Skid = 8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: u32,
    pub source_ms: u64,
    pub kind: EventKind,
    pub strength: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub generation: u64,
    pub sequence: u32,
    pub source_ms: u64,
    pub flags: u8,
    pub root: Affine,
    /// Bevy world yards per second.
    pub velocity: [f32; 3],
    pub facing: f32,
    pub state: String,
    pub bones: Vec<Affine>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Hello,
    Welcome,
    Definition(Definition),
    Snapshot(Snapshot),
    Stop { generation: u64 },
    Events { generation: u64, events: Vec<Event> },
}

/// Server-authored routing context. A client request never contains a sender GUID.
#[derive(Debug, Clone, PartialEq)]
pub struct Relay {
    pub guid: u64,
    pub server_ms: u32,
    pub map: u32,
    pub instance: u32,
    pub message: Message,
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

fn generation(value: u64) -> io::Result<u64> {
    if value == 0 {
        Err(invalid("zero skate session generation"))
    } else {
        Ok(value)
    }
}

fn finite(value: f32, bound: f32) -> io::Result<f32> {
    if value.is_finite() && value.abs() <= bound {
        Ok(value)
    } else {
        Err(invalid("invalid skate numeric value"))
    }
}

fn string(r: &mut &[u8], max: usize) -> io::Result<String> {
    let len = usize::from(read_u8(r)?);
    if len > max || len > r.len() {
        return Err(invalid("invalid skate string length"));
    }
    let (value, rest) = r.split_at(len);
    let value = std::str::from_utf8(value).map_err(|_| invalid("invalid skate UTF-8"))?;
    if value.bytes().any(|c| c < 0x20 || c == 0x7f) {
        return Err(invalid("control character in skate string"));
    }
    *r = rest;
    Ok(value.to_owned())
}

fn put_string(out: &mut Vec<u8>, value: &str, max: usize) -> io::Result<()> {
    if value.len() > max || value.bytes().any(|c| c < 0x20 || c == 0x7f) {
        return Err(invalid("invalid skate string"));
    }
    out.push(value.len() as u8);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn count(r: &mut &[u8], max: usize, nonempty: bool) -> io::Result<usize> {
    let n = usize::from(read_u16_le(r)?);
    if n > max || (nonempty && n == 0) {
        return Err(invalid("invalid skate element count"));
    }
    Ok(n)
}

fn read_affine(r: &mut &[u8], world: bool) -> io::Result<Affine> {
    let mut affine = [0.; 12];
    for (i, value) in affine.iter_mut().enumerate() {
        let bound = if i < 9 {
            16.
        } else if world {
            100_000.
        } else {
            256.
        };
        *value = finite(read_f32_le(r)?, bound)?;
    }
    if world {
        validate_root(&affine)?;
    }
    Ok(affine)
}

fn validate_root(a: &Affine) -> io::Result<()> {
    let determinant = a[0] * (a[4] * a[8] - a[5] * a[7]) - a[3] * (a[1] * a[8] - a[2] * a[7])
        + a[6] * (a[1] * a[5] - a[2] * a[4]);
    if determinant.abs() < 1e-8 {
        return Err(invalid("singular skate root transform"));
    }
    Ok(())
}

fn put_affine(out: &mut Vec<u8>, affine: &Affine, world: bool) -> io::Result<()> {
    if world {
        validate_root(affine)?;
    }
    for (i, &value) in affine.iter().enumerate() {
        let bound = if i < 9 {
            16.
        } else if world {
            100_000.
        } else {
            256.
        };
        out.extend_from_slice(&finite(value, bound)?.to_le_bytes());
    }
    Ok(())
}

fn read_snapshot(r: &mut &[u8]) -> io::Result<Snapshot> {
    let generation = generation(read_u64_le(r)?)?;
    let sequence = read_u32_le(r)?;
    let source_ms = read_u64_le(r)?;
    let flags = read_u8(r)?;
    if flags & !DISCONTINUITY != 0 {
        return Err(invalid("unknown skate pose flags"));
    }
    let root = read_affine(r, true)?;
    let velocity = [
        finite(read_f32_le(r)?, 10_000.)?,
        finite(read_f32_le(r)?, 10_000.)?,
        finite(read_f32_le(r)?, 10_000.)?,
    ];
    let facing = finite(read_f32_le(r)?, std::f32::consts::TAU * 2.)?;
    let state = string(r, MAX_STATE_BYTES)?;
    let n = count(r, MAX_BONES, true)?;
    if r.len() != n * 48 {
        return Err(invalid("skate pose byte count does not match its joints"));
    }
    let bones = (0..n)
        .map(|_| read_affine(r, false))
        .collect::<io::Result<_>>()?;
    Ok(Snapshot {
        generation,
        sequence,
        source_ms,
        flags,
        root,
        velocity,
        facing,
        state,
        bones,
    })
}

fn put_snapshot(snapshot: &Snapshot) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(128 + snapshot.bones.len().min(MAX_BONES) * 48);
    out.extend_from_slice(&generation(snapshot.generation)?.to_le_bytes());
    out.extend_from_slice(&snapshot.sequence.to_le_bytes());
    out.extend_from_slice(&snapshot.source_ms.to_le_bytes());
    if snapshot.flags & !DISCONTINUITY != 0 {
        return Err(invalid("unknown skate pose flags"));
    }
    out.push(snapshot.flags);
    put_affine(&mut out, &snapshot.root, true)?;
    for value in snapshot.velocity {
        out.extend_from_slice(&finite(value, 10_000.)?.to_le_bytes());
    }
    out.extend_from_slice(&finite(snapshot.facing, std::f32::consts::TAU * 2.)?.to_le_bytes());
    put_string(&mut out, &snapshot.state, MAX_STATE_BYTES)?;
    if snapshot.bones.is_empty() || snapshot.bones.len() > MAX_BONES {
        return Err(invalid("invalid skate joint count"));
    }
    out.extend_from_slice(&(snapshot.bones.len() as u16).to_le_bytes());
    for bone in &snapshot.bones {
        put_affine(&mut out, bone, false)?;
    }
    Ok(out)
}

pub fn encode(message: &Message) -> io::Result<Vec<u8>> {
    let mut out = VERSION.to_le_bytes().to_vec();
    match message {
        Message::Hello => out.push(0),
        Message::Welcome => out.push(1),
        Message::Definition(definition) => {
            out.push(2);
            out.extend_from_slice(&generation(definition.generation)?.to_le_bytes());
            out.extend_from_slice(&definition.fingerprint);
            if definition.names.is_empty() || definition.names.len() > MAX_BONES {
                return Err(invalid("invalid skate dictionary size"));
            }
            out.extend_from_slice(&(definition.names.len() as u16).to_le_bytes());
            let mut seen = HashSet::new();
            for name in &definition.names {
                if name.is_empty() || !seen.insert(name) {
                    return Err(invalid("empty or duplicate skate joint"));
                }
                put_string(&mut out, name, MAX_NAME_BYTES)?;
            }
        }
        Message::Snapshot(snapshot) => {
            out.push(3);
            let raw = put_snapshot(snapshot)?;
            let mut compressor = ZlibEncoder::new(Vec::new(), Compression::fast());
            compressor.write_all(&raw)?;
            let compressed = compressor.finish()?;
            out.extend_from_slice(&(raw.len() as u16).to_le_bytes());
            out.extend_from_slice(&compressed);
        }
        Message::Stop { generation: value } => {
            out.push(4);
            out.extend_from_slice(&generation(*value)?.to_le_bytes());
        }
        Message::Events {
            generation: value,
            events,
        } => {
            out.push(5);
            out.extend_from_slice(&generation(*value)?.to_le_bytes());
            if events.len() > MAX_EVENTS {
                return Err(invalid("too many skate events"));
            }
            out.extend_from_slice(&(events.len() as u16).to_le_bytes());
            for event in events {
                out.extend_from_slice(&event.id.to_le_bytes());
                out.extend_from_slice(&event.source_ms.to_le_bytes());
                out.push(event.kind as u8);
                out.extend_from_slice(&finite(event.strength, 100.)?.to_le_bytes());
            }
        }
    }
    if out.len() > MAX_COMPRESSED_BYTES {
        return Err(invalid("skate message is too large"));
    }
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> io::Result<Message> {
    if bytes.len() > MAX_COMPRESSED_BYTES {
        return Err(invalid("skate message is too large"));
    }
    let mut r = bytes;
    if read_u16_le(&mut r)? != VERSION {
        return Err(invalid("unsupported skate network version"));
    }
    let message = match read_u8(&mut r)? {
        0 => Message::Hello,
        1 => Message::Welcome,
        2 => {
            let generation = generation(read_u64_le(&mut r)?)?;
            let mut fingerprint = [0; 20];
            r.read_exact(&mut fingerprint)?;
            let n = count(&mut r, MAX_BONES, true)?;
            let mut seen = HashSet::new();
            let mut names = Vec::with_capacity(n);
            for _ in 0..n {
                let name = string(&mut r, MAX_NAME_BYTES)?;
                if name.is_empty() || !seen.insert(name.clone()) {
                    return Err(invalid("empty or duplicate skate joint"));
                }
                names.push(name);
            }
            Message::Definition(Definition {
                generation,
                fingerprint,
                names,
            })
        }
        3 => {
            let len = usize::from(read_u16_le(&mut r)?);
            if len > MAX_BODY_BYTES {
                return Err(invalid("inflated skate pose is too large"));
            }
            let mut inflater = ZlibDecoder::new(r);
            let mut raw = Vec::with_capacity(len);
            inflater
                .by_ref()
                .take((MAX_BODY_BYTES + 1) as u64)
                .read_to_end(&mut raw)?;
            if raw.len() != len || inflater.total_in() as usize != r.len() {
                return Err(invalid("invalid compressed skate pose length"));
            }
            let snapshot = read_snapshot(&mut raw.as_slice())?;
            r = &[];
            Message::Snapshot(snapshot)
        }
        4 => Message::Stop {
            generation: generation(read_u64_le(&mut r)?)?,
        },
        5 => {
            let generation = generation(read_u64_le(&mut r)?)?;
            let n = count(&mut r, MAX_EVENTS, false)?;
            if r.len() != n * 17 {
                return Err(invalid("invalid skate event byte count"));
            }
            let mut events = Vec::with_capacity(n);
            for _ in 0..n {
                let id = read_u32_le(&mut r)?;
                let source_ms = read_u64_le(&mut r)?;
                let kind = match read_u8(&mut r)? {
                    1 => EventKind::Pop,
                    2 => EventKind::Land,
                    3 => EventKind::Flip,
                    4 => EventKind::GrindStart,
                    5 => EventKind::GrindEnd,
                    6 => EventKind::Bail,
                    7 => EventKind::Recovery,
                    8 => EventKind::Skid,
                    _ => return Err(invalid("unknown skate event")),
                };
                let strength = finite(read_f32_le(&mut r)?, 100.)?;
                events.push(Event {
                    id,
                    source_ms,
                    kind,
                    strength,
                });
            }
            Message::Events { generation, events }
        }
        _ => return Err(invalid("unknown skate network message")),
    };
    if !r.is_empty() {
        return Err(invalid("trailing skate message data"));
    }
    Ok(message)
}

pub fn encode_relay(relay: &Relay) -> io::Result<Vec<u8>> {
    validate_relay_direction(relay.guid, &relay.message)?;
    let mut out = relay.guid.to_le_bytes().to_vec();
    out.extend_from_slice(&relay.server_ms.to_le_bytes());
    out.extend_from_slice(&relay.map.to_le_bytes());
    out.extend_from_slice(&relay.instance.to_le_bytes());
    out.extend_from_slice(&encode(&relay.message)?);
    Ok(out)
}

pub fn decode_relay(bytes: &[u8]) -> io::Result<Relay> {
    let mut r = bytes;
    let guid = read_u64_le(&mut r)?;
    let server_ms = read_u32_le(&mut r)?;
    let map = read_u32_le(&mut r)?;
    let instance = read_u32_le(&mut r)?;
    let message = decode(r)?;
    validate_relay_direction(guid, &message)?;
    Ok(Relay {
        guid,
        server_ms,
        map,
        instance,
        message,
    })
}

fn validate_relay_direction(guid: u64, message: &Message) -> io::Result<()> {
    if matches!(message, Message::Hello) {
        return Err(invalid("client handshake in server relay"));
    }
    if (guid == 0) != matches!(message, Message::Welcome) {
        return Err(invalid("invalid skate sender identity"));
    }
    Ok(())
}

pub fn decode_client(bytes: &[u8]) -> io::Result<Message> {
    let message = decode(bytes)?;
    if matches!(message, Message::Welcome) {
        return Err(invalid("server handshake in client request"));
    }
    Ok(message)
}

#[cfg(test)]
#[path = "skatecraft/tests.rs"]
mod tests;
