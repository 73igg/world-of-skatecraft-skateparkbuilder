//! Compact session-owned object messages carried by the existing reliable addon channel.
//! The server assigns ownership, map and instance; clients request edits to their own objects.

use std::io;

pub const PREFIX: &str = "SKATEBUILD\t";
pub const VERSION: u8 = 3;
pub const CATALOG_VERSION: u32 = 3;
pub const MAX_OBJECTS_PER_OWNER: usize = 256;
pub const MAX_OBJECTS_PER_MAP: usize = 2048;
pub const MAX_PARTS_PER_OBJECT: usize = 256;
pub const MAX_PARTS_PER_OWNER: usize = 1024;
pub const MAX_PARTS_PER_MAP: usize = 8192;
pub const MIN_SCALE: f32 = 0.01;
pub const MAX_SCALE: f32 = 100.0;
const MAX_BYTES: usize = 128;
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Model {
    Box = 1,
    Ramp = 2,
    Rail = 3,
    Platform = 4,
    QuarterPipe = 5,
    Stairs = 6,
    Compound = 7,
    Asset = 8,
}
impl Model {
    pub const ALL: [Self; 6] = [
        Self::Box,
        Self::Ramp,
        Self::Rail,
        Self::Platform,
        Self::QuarterPipe,
        Self::Stairs,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Ramp => "Ramp",
            Self::Rail => "Rail",
            Self::Platform => "Platform",
            Self::QuarterPipe => "Quarter pipe",
            Self::Stairs => "Stairs",
            Self::Compound => "Fused object",
            Self::Asset => "Catalog object",
        }
    }
    fn read(value: u8) -> io::Result<Self> {
        if value == Self::Compound as u8 {
            return Ok(Self::Compound);
        }
        if value == Self::Asset as u8 {
            return Ok(Self::Asset);
        }
        Self::ALL
            .into_iter()
            .find(|m| *m as u8 == value)
            .ok_or_else(|| invalid("unknown park object"))
    }
    pub fn validate_asset(self, asset: u64) -> io::Result<()> {
        if if self == Self::Asset {
            !(1..=2).contains(&(asset >> 62))
        } else {
            asset != 0
        } {
            return Err(invalid("invalid catalog asset identity"));
        }
        Ok(())
    }
}

/// A primitive's local affine basis and translation. Affine data preserves shear when fusing
/// rotated, nonuniformly scaled assemblies; no lossy TRS decomposition is used.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub model: Model,
    pub asset: u64,
    pub affine: [f32; 12],
}
impl Part {
    pub fn validate(&self) -> io::Result<()> {
        self.model.validate_asset(self.asset)?;
        if self.model == Model::Compound
            || self
                .affine
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 1_000_000.)
        {
            return Err(invalid("invalid fused primitive"));
        }
        let a = self.affine.map(f64::from);
        let det = a[0] * (a[4] * a[8] - a[5] * a[7]) - a[3] * (a[1] * a[8] - a[2] * a[7])
            + a[6] * (a[1] * a[5] - a[2] * a[4]);
        if !det.is_finite() || det <= 1e-18 {
            return Err(invalid("singular or reflected fused primitive"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    pub model: Model,
    pub asset: u64,
    pub pose: Pose,
    pub parts: Vec<Part>,
}
impl Definition {
    pub fn validate(&self) -> io::Result<()> {
        self.model.validate_asset(self.asset)?;
        self.pose.validate()?;
        validate_parts(self.model, self.parts.len())?;
        for part in &self.parts {
            part.validate()?;
        }
        Ok(())
    }
    pub fn primitive(model: Model, pose: Pose) -> Self {
        Self {
            model,
            asset: 0,
            pose,
            parts: Vec::new(),
        }
    }
    pub fn asset(asset: u64, pose: Pose) -> Self {
        Self {
            model: Model::Asset,
            asset,
            pose,
            parts: Vec::new(),
        }
    }
}
fn validate_parts(model: Model, count: usize) -> io::Result<()> {
    if count > MAX_PARTS_PER_OBJECT || (model == Model::Compound) != (count > 0) {
        return Err(invalid("invalid fused part count"));
    }
    Ok(())
}

/// Bevy world yards, quaternion XYZW, positive nonuniform scale in model-local axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}
impl Default for Pose {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            rotation: [0., 0., 0., 1.],
            scale: [1.; 3],
        }
    }
}
impl Pose {
    pub fn validate(self) -> io::Result<()> {
        if self
            .position
            .into_iter()
            .any(|v| !v.is_finite() || v.abs() > 100_000.)
            || self.rotation.into_iter().any(|v| !v.is_finite())
            || self
                .scale
                .into_iter()
                .any(|v| !v.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(&v))
        {
            return Err(invalid("park transform outside its finite bounds"));
        }
        let norm: f32 = self.rotation.into_iter().map(|v| v * v).sum();
        if !(0.999..=1.001).contains(&norm) {
            return Err(invalid("park rotation is not normalized"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub owner: u64,
    pub id: u32,
    pub revision: u32,
    pub map: u32,
    pub instance: u32,
    pub model: Model,
    pub asset: u64,
    pub pose: Pose,
    pub parts: Vec<Part>,
}
impl Object {
    pub fn definition(&self) -> Definition {
        Definition {
            model: self.model,
            asset: self.asset,
            pose: self.pose,
            parts: self.parts.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Hello,
    Put {
        request: u32,
        id: u32,
        revision: u32,
        model: Model,
        asset: u64,
        pose: Pose,
    },
    Delete {
        request: u32,
        id: u32,
        revision: u32,
    },
    Sync,
    Begin {
        snapshot: u32,
        map: u32,
        instance: u32,
        count: u16,
    },
    Object {
        request: u32,
        slot: u16,
        part_count: u16,
        object: Object,
    },
    End {
        snapshot: u32,
    },
    Removed {
        request: u32,
        owner: u64,
        id: u32,
    },
    ClearOwner {
        owner: u64,
    },
    Rejected {
        request: u32,
        reason: u8,
    },
    Toggle,
    Available,
    EditBegin {
        request: u32,
        edits: u16,
        drops: u16,
    },
    EditObject {
        request: u32,
        slot: u16,
        id: u32,
        revision: u32,
        model: Model,
        asset: u64,
        pose: Pose,
        parts: u16,
    },
    EditPart {
        request: u32,
        slot: u16,
        index: u16,
        part: Part,
    },
    EditDrop {
        request: u32,
        slot: u16,
        id: u32,
        revision: u32,
    },
    EditCommit {
        request: u32,
    },
    ChangeBegin {
        request: u32,
        owner: u64,
        edits: u16,
        drops: u16,
    },
    ObjectPart {
        owner: u64,
        id: u32,
        revision: u32,
        index: u16,
        part: Part,
    },
    ChangeEnd {
        request: u32,
        owner: u64,
    },
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes());
}
fn put64(out: &mut Vec<u8>, value: u64) {
    out.extend(value.to_le_bytes());
}
fn put_pose(out: &mut Vec<u8>, pose: Pose) -> io::Result<()> {
    pose.validate()?;
    for value in pose
        .position
        .into_iter()
        .chain(pose.rotation)
        .chain(pose.scale)
    {
        out.extend(value.to_le_bytes());
    }
    Ok(())
}
fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_le_bytes());
}
fn put_part(out: &mut Vec<u8>, part: &Part) -> io::Result<()> {
    part.validate()?;
    out.push(part.model as u8);
    put64(out, part.asset);
    for value in part.affine {
        out.extend(value.to_le_bytes());
    }
    Ok(())
}

impl Message {
    pub fn is_request(&self) -> bool {
        matches!(
            self,
            Self::Hello
                | Self::Put { .. }
                | Self::Delete { .. }
                | Self::Sync
                | Self::EditBegin { .. }
                | Self::EditObject { .. }
                | Self::EditPart { .. }
                | Self::EditDrop { .. }
                | Self::EditCommit { .. }
        )
    }
    pub fn encode(&self) -> io::Result<String> {
        let mut bytes = vec![VERSION];
        match self {
            Self::Hello => {
                bytes.push(0);
                put32(&mut bytes, CATALOG_VERSION);
            }
            Self::Put {
                request,
                id,
                revision,
                model,
                asset,
                pose,
            } => {
                bytes.push(1);
                put32(&mut bytes, *request);
                put32(&mut bytes, *id);
                put32(&mut bytes, *revision);
                model.validate_asset(*asset)?;
                bytes.push(*model as u8);
                put64(&mut bytes, *asset);
                put_pose(&mut bytes, *pose)?;
            }
            Self::Delete {
                request,
                id,
                revision,
            } => {
                bytes.push(2);
                put32(&mut bytes, *request);
                put32(&mut bytes, *id);
                put32(&mut bytes, *revision);
            }
            Self::Sync => bytes.push(3),
            Self::Begin {
                snapshot,
                map,
                instance,
                count,
            } => {
                bytes.push(4);
                put32(&mut bytes, *snapshot);
                put32(&mut bytes, *map);
                put32(&mut bytes, *instance);
                bytes.extend(count.to_le_bytes());
            }
            Self::Object {
                request,
                slot,
                part_count,
                object,
            } => {
                bytes.push(5);
                put32(&mut bytes, *request);
                put64(&mut bytes, object.owner);
                put32(&mut bytes, object.id);
                put32(&mut bytes, object.revision);
                put32(&mut bytes, object.map);
                put32(&mut bytes, object.instance);
                object.model.validate_asset(object.asset)?;
                bytes.push(object.model as u8);
                put64(&mut bytes, object.asset);
                put_pose(&mut bytes, object.pose)?;
                put16(&mut bytes, *part_count);
                put16(&mut bytes, *slot);
            }
            Self::End { snapshot } => {
                bytes.push(6);
                put32(&mut bytes, *snapshot);
            }
            Self::Removed { request, owner, id } => {
                bytes.push(7);
                put32(&mut bytes, *request);
                put64(&mut bytes, *owner);
                put32(&mut bytes, *id);
            }
            Self::ClearOwner { owner } => {
                bytes.push(8);
                put64(&mut bytes, *owner);
            }
            Self::Rejected { request, reason } => {
                bytes.push(9);
                put32(&mut bytes, *request);
                bytes.push(*reason);
            }
            Self::Toggle => bytes.push(10),
            Self::Available => {
                bytes.push(11);
                put32(&mut bytes, CATALOG_VERSION);
            }
            Self::EditBegin {
                request,
                edits,
                drops,
            } => {
                bytes.push(12);
                put32(&mut bytes, *request);
                put16(&mut bytes, *edits);
                put16(&mut bytes, *drops);
            }
            Self::EditObject {
                request,
                slot,
                id,
                revision,
                model,
                asset,
                pose,
                parts,
            } => {
                bytes.push(13);
                put32(&mut bytes, *request);
                put16(&mut bytes, *slot);
                put32(&mut bytes, *id);
                put32(&mut bytes, *revision);
                model.validate_asset(*asset)?;
                bytes.push(*model as u8);
                put64(&mut bytes, *asset);
                put_pose(&mut bytes, *pose)?;
                put16(&mut bytes, *parts);
            }
            Self::EditPart {
                request,
                slot,
                index,
                part,
            } => {
                bytes.push(14);
                put32(&mut bytes, *request);
                put16(&mut bytes, *slot);
                put16(&mut bytes, *index);
                put_part(&mut bytes, part)?;
            }
            Self::EditDrop {
                request,
                slot,
                id,
                revision,
            } => {
                bytes.push(15);
                put32(&mut bytes, *request);
                put16(&mut bytes, *slot);
                put32(&mut bytes, *id);
                put32(&mut bytes, *revision);
            }
            Self::EditCommit { request } => {
                bytes.push(16);
                put32(&mut bytes, *request);
            }
            Self::ChangeBegin {
                request,
                owner,
                edits,
                drops,
            } => {
                bytes.push(17);
                put32(&mut bytes, *request);
                put64(&mut bytes, *owner);
                put16(&mut bytes, *edits);
                put16(&mut bytes, *drops);
            }
            Self::ObjectPart {
                owner,
                id,
                revision,
                index,
                part,
            } => {
                bytes.push(18);
                put64(&mut bytes, *owner);
                put32(&mut bytes, *id);
                put32(&mut bytes, *revision);
                put16(&mut bytes, *index);
                put_part(&mut bytes, part)?;
            }
            Self::ChangeEnd { request, owner } => {
                bytes.push(19);
                put32(&mut bytes, *request);
                put64(&mut bytes, *owner);
            }
        }
        let mut text = String::from(PREFIX);
        for chunk in bytes.chunks(3) {
            let value = (u32::from(chunk[0]) << 16)
                | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
                | u32::from(*chunk.get(2).unwrap_or(&0));
            text.push(ALPHABET[(value >> 18) as usize] as char);
            text.push(ALPHABET[((value >> 12) & 63) as usize] as char);
            text.push(if chunk.len() > 1 {
                ALPHABET[((value >> 6) & 63) as usize] as char
            } else {
                '='
            });
            text.push(if chunk.len() > 2 {
                ALPHABET[(value & 63) as usize] as char
            } else {
                '='
            });
        }
        Ok(text)
    }
    pub fn decode(text: &str) -> io::Result<Self> {
        let text = text
            .strip_prefix(PREFIX)
            .ok_or_else(|| invalid("park message prefix"))?;
        if text.is_empty() || text.len() > MAX_BYTES.div_ceil(3) * 4 || text.len() % 4 != 0 {
            return Err(invalid("park message size"));
        }
        let mut bytes = Vec::with_capacity(MAX_BYTES);
        for (index, chunk) in text.as_bytes().as_chunks::<4>().0.iter().enumerate() {
            let mut value = 0u32;
            let mut padding = 0;
            for (i, &byte) in chunk.iter().enumerate() {
                value <<= 6;
                if byte == b'=' {
                    if i < 2 {
                        return Err(invalid("park padding"));
                    }
                    padding += 1;
                } else {
                    if padding != 0 {
                        return Err(invalid("park padding"));
                    }
                    value |= ALPHABET
                        .iter()
                        .position(|&b| b == byte)
                        .ok_or_else(|| invalid("park alphabet"))?
                        as u32;
                }
            }
            if padding > 0 && index != text.len() / 4 - 1
                || padding > 2
                || padding == 1 && value & 255 != 0
                || padding == 2 && value & 65535 != 0
            {
                return Err(invalid("noncanonical park encoding"));
            }
            bytes.push((value >> 16) as u8);
            if padding < 2 {
                bytes.push((value >> 8) as u8);
            }
            if padding == 0 {
                bytes.push(value as u8);
            }
        }
        let mut reader = Reader(&bytes);
        if reader.byte()? != VERSION {
            return Err(invalid("park protocol version"));
        }
        let message = match reader.byte()? {
            0 => {
                reader.catalog()?;
                Self::Hello
            }
            1 => Self::Put {
                request: reader.u32()?,
                id: reader.u32()?,
                revision: reader.u32()?,
                model: Model::read(reader.byte()?)?,
                asset: reader.u64()?,
                pose: reader.pose()?,
            },
            2 => Self::Delete {
                request: reader.u32()?,
                id: reader.u32()?,
                revision: reader.u32()?,
            },
            3 => Self::Sync,
            4 => Self::Begin {
                snapshot: reader.u32()?,
                map: reader.u32()?,
                instance: reader.u32()?,
                count: u16::from_le_bytes(reader.take()?),
            },
            5 => {
                let request = reader.u32()?;
                let object = Object {
                    owner: reader.u64()?,
                    id: reader.u32()?,
                    revision: reader.u32()?,
                    map: reader.u32()?,
                    instance: reader.u32()?,
                    model: Model::read(reader.byte()?)?,
                    asset: reader.u64()?,
                    pose: reader.pose()?,
                    parts: Vec::new(),
                };
                let part_count = reader.u16()?;
                let slot = reader.u16()?;
                validate_parts(object.model, usize::from(part_count))?;
                Self::Object {
                    request,
                    slot,
                    part_count,
                    object,
                }
            }
            6 => Self::End {
                snapshot: reader.u32()?,
            },
            7 => Self::Removed {
                request: reader.u32()?,
                owner: reader.u64()?,
                id: reader.u32()?,
            },
            8 => Self::ClearOwner {
                owner: reader.u64()?,
            },
            9 => Self::Rejected {
                request: reader.u32()?,
                reason: reader.byte()?,
            },
            10 => Self::Toggle,
            11 => {
                reader.catalog()?;
                Self::Available
            }
            12 => Self::EditBegin {
                request: reader.u32()?,
                edits: reader.u16()?,
                drops: reader.u16()?,
            },
            13 => Self::EditObject {
                request: reader.u32()?,
                slot: reader.u16()?,
                id: reader.u32()?,
                revision: reader.u32()?,
                model: Model::read(reader.byte()?)?,
                asset: reader.u64()?,
                pose: reader.pose()?,
                parts: reader.u16()?,
            },
            14 => Self::EditPart {
                request: reader.u32()?,
                slot: reader.u16()?,
                index: reader.u16()?,
                part: reader.part()?,
            },
            15 => Self::EditDrop {
                request: reader.u32()?,
                slot: reader.u16()?,
                id: reader.u32()?,
                revision: reader.u32()?,
            },
            16 => Self::EditCommit {
                request: reader.u32()?,
            },
            17 => Self::ChangeBegin {
                request: reader.u32()?,
                owner: reader.u64()?,
                edits: reader.u16()?,
                drops: reader.u16()?,
            },
            18 => Self::ObjectPart {
                owner: reader.u64()?,
                id: reader.u32()?,
                revision: reader.u32()?,
                index: reader.u16()?,
                part: reader.part()?,
            },
            19 => Self::ChangeEnd {
                request: reader.u32()?,
                owner: reader.u64()?,
            },
            _ => return Err(invalid("park message kind")),
        };
        if !reader.0.is_empty() {
            return Err(invalid("park trailing bytes"));
        }
        match &message {
            Self::Object { object, .. } => object.model.validate_asset(object.asset)?,
            Self::Put { model, asset, .. } | Self::EditObject { model, asset, .. } => {
                model.validate_asset(*asset)?
            }
            _ => {}
        }
        match &message {
            Self::Object { object, .. }
                if object.owner == 0 || object.id == 0 || object.revision == 0 =>
            {
                return Err(invalid("park object identity"))
            }
            Self::Begin { count, .. } if usize::from(*count) > MAX_OBJECTS_PER_MAP => {
                return Err(invalid("park snapshot count"))
            }
            Self::Put {
                request,
                id,
                revision,
                ..
            } if *request == 0 || (*id == 0) != (*revision == 0) => {
                return Err(invalid("park edit identity"))
            }
            Self::Delete {
                request,
                id,
                revision,
            } if *request == 0 || *id == 0 || *revision == 0 => {
                return Err(invalid("park deletion identity"))
            }
            Self::EditBegin {
                request,
                edits,
                drops,
            } if *request == 0
                || *edits as usize > MAX_OBJECTS_PER_OWNER
                || *drops as usize > MAX_OBJECTS_PER_OWNER
                || *edits == 0 && *drops == 0 =>
            {
                return Err(invalid("invalid edit transaction size"))
            }
            Self::EditObject {
                request,
                id,
                revision,
                model,
                parts,
                slot,
                ..
            } => {
                if *request == 0
                    || (*id == 0) != (*revision == 0)
                    || *slot as usize >= MAX_OBJECTS_PER_OWNER
                {
                    return Err(invalid("invalid transaction object"));
                }
                validate_parts(*model, usize::from(*parts))?;
            }
            Self::EditDrop {
                request,
                id,
                revision,
                slot,
            } if *request == 0
                || *id == 0
                || *revision == 0
                || *slot as usize >= MAX_OBJECTS_PER_OWNER =>
            {
                return Err(invalid("invalid transaction deletion"))
            }
            Self::EditPart {
                request,
                slot,
                index,
                ..
            } if *request == 0
                || *slot as usize >= MAX_OBJECTS_PER_OWNER
                || *index as usize >= MAX_PARTS_PER_OBJECT =>
            {
                return Err(invalid("invalid transaction part"))
            }
            Self::EditCommit { request } if *request == 0 => {
                return Err(invalid("invalid transaction commit"))
            }
            Self::ChangeBegin {
                owner,
                request,
                edits,
                drops,
            } if *owner == 0
                || *request == 0
                || *edits as usize > MAX_OBJECTS_PER_OWNER
                || *drops as usize > MAX_OBJECTS_PER_OWNER =>
            {
                return Err(invalid("invalid committed transaction"))
            }
            Self::ChangeEnd { owner, request } if *owner == 0 || *request == 0 => {
                return Err(invalid("invalid committed transaction identity"))
            }
            Self::ObjectPart {
                owner,
                id,
                revision,
                index,
                ..
            } if *owner == 0
                || *id == 0
                || *revision == 0
                || *index as usize >= MAX_PARTS_PER_OBJECT =>
            {
                return Err(invalid("invalid object part identity"))
            }
            _ => {}
        }
        Ok(message)
    }
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let head = self
            .0
            .get(..N)
            .ok_or_else(|| invalid("short park message"))?;
        let value = head.try_into().unwrap();
        self.0 = &self.0[N..];
        Ok(value)
    }
    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take::<1>()?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn part(&mut self) -> io::Result<Part> {
        let model = Model::read(self.byte()?)?;
        let asset = self.u64()?;
        let mut affine = [0.; 12];
        for value in &mut affine {
            *value = f32::from_le_bytes(self.take()?);
        }
        let part = Part {
            model,
            asset,
            affine,
        };
        part.validate()?;
        Ok(part)
    }
    fn catalog(&mut self) -> io::Result<()> {
        if self.u32()? != CATALOG_VERSION {
            return Err(invalid("park catalog version"));
        }
        Ok(())
    }
    fn pose(&mut self) -> io::Result<Pose> {
        let mut numbers = [0.; 10];
        for number in &mut numbers {
            *number = f32::from_le_bytes(self.take()?);
        }
        let pose = Pose {
            position: numbers[..3].try_into().unwrap(),
            rotation: numbers[3..7].try_into().unwrap(),
            scale: numbers[7..].try_into().unwrap(),
        };
        pose.validate()?;
        Ok(pose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_identity_survives_root_and_affine_part_messages() {
        for asset in [(1_u64 << 62) | 0x1234abcd, (2_u64 << 62) | 0x987654321] {
            let pose = Pose::default();
            let part = Part {
                model: Model::Asset,
                asset,
                affine: [1., 0., 0., 0.2, 2., 0., 0., 0., 0.5, -12., 0.01, 8.],
            };
            for message in [
                Message::Put {
                    request: 3,
                    id: 0,
                    revision: 0,
                    model: Model::Asset,
                    asset,
                    pose,
                },
                Message::EditObject {
                    request: 3,
                    slot: 0,
                    id: 9,
                    revision: 2,
                    model: Model::Asset,
                    asset,
                    pose,
                    parts: 0,
                },
                Message::Object {
                    request: 3,
                    slot: 0,
                    part_count: 0,
                    object: Object {
                        owner: 5,
                        id: 9,
                        revision: 3,
                        map: 0,
                        instance: 0,
                        model: Model::Asset,
                        asset,
                        pose,
                        parts: vec![],
                    },
                },
                Message::EditPart {
                    request: 3,
                    slot: 0,
                    index: 1,
                    part: part.clone(),
                },
                Message::ObjectPart {
                    owner: 5,
                    id: 9,
                    revision: 3,
                    index: 1,
                    part: part.clone(),
                },
            ] {
                let text = message.encode().unwrap();
                assert!(text.len() < 255);
                assert_eq!(Message::decode(&text).unwrap(), message);
            }
        }
        for invalid in [0, 1, 3_u64 << 62] {
            assert!(Model::Asset.validate_asset(invalid).is_err());
        }
        assert!(Model::Box.validate_asset(1_u64 << 62).is_err());
    }
    #[test]
    fn transaction_and_affine_part_messages_are_bounded_and_bit_exact() {
        let part = Part {
            model: Model::QuarterPipe,
            asset: 0,
            affine: [1., 0., 0., 0.25, 2., 0., 0., 0., 0.5, -12.3, 0.006, 8.5],
        };
        let messages = [
            Message::EditBegin {
                request: 1,
                edits: 1,
                drops: 2,
            },
            Message::EditObject {
                request: 1,
                slot: 0,
                id: 0,
                revision: 0,
                model: Model::Compound,
                asset: 0,
                pose: Pose::default(),
                parts: 2,
            },
            Message::EditPart {
                request: 1,
                slot: 0,
                index: 1,
                part: part.clone(),
            },
            Message::EditDrop {
                request: 1,
                slot: 0,
                id: 5,
                revision: 2,
            },
            Message::EditCommit { request: 1 },
            Message::ChangeBegin {
                request: 1,
                owner: 9,
                edits: 1,
                drops: 2,
            },
            Message::ObjectPart {
                owner: 9,
                id: 8,
                revision: 1,
                index: 1,
                part,
            },
            Message::ChangeEnd {
                request: 1,
                owner: 9,
            },
        ];
        for message in messages {
            let encoded = message.encode().unwrap();
            assert!(encoded.len() < 255);
            assert_eq!(Message::decode(&encoded).unwrap(), message);
        }
        let bad = Part {
            model: Model::Box,
            asset: 0,
            affine: [0.; 12],
        };
        assert!(bad.validate().is_err());
    }
    #[test]
    fn messages_fit_addon_channel_and_round_trip_float_bits() {
        let pose = Pose {
            position: [-10000.123, 800.75, 4297.31],
            rotation: [0., 0.6, 0., 0.8],
            scale: [0.01, 2.5, 100.],
        };
        let object = Object {
            owner: 0x123456789abcdef,
            id: 128,
            revision: 4,
            map: 1,
            instance: 52,
            model: Model::Rail,
            asset: 0,
            pose,
            parts: Vec::new(),
        };
        let messages = [
            Message::Hello,
            Message::Available,
            Message::Sync,
            Message::Toggle,
            Message::Put {
                request: 99,
                id: 0,
                revision: 0,
                model: Model::Ramp,
                asset: 0,
                pose,
            },
            Message::Delete {
                request: 100,
                id: 128,
                revision: 4,
            },
            Message::Begin {
                snapshot: 2,
                map: 1,
                instance: 52,
                count: 2048,
            },
            Message::Object {
                request: 99,
                slot: 0,
                part_count: 0,
                object,
            },
            Message::End { snapshot: 2 },
            Message::Removed {
                request: 100,
                owner: 123,
                id: 128,
            },
            Message::ClearOwner { owner: 123 },
            Message::Rejected {
                request: 101,
                reason: 2,
            },
        ];
        for message in messages {
            let text = message.encode().unwrap();
            assert!(text.len() < 255);
            assert_eq!(Message::decode(&text).unwrap(), message);
        }
    }
    #[test]
    fn invalid_transforms_are_rejected() {
        for pose in [
            Pose {
                position: [f32::NAN, 0., 0.],
                ..Pose::default()
            },
            Pose {
                rotation: [0.; 4],
                ..Pose::default()
            },
            Pose {
                scale: [0., 1., 1.],
                ..Pose::default()
            },
            Pose {
                scale: [-1., 1., 1.],
                ..Pose::default()
            },
        ] {
            assert!(pose.validate().is_err());
        }
    }
    #[test]
    fn truncated_corrupt_and_overlong_payloads_are_rejected() {
        let encoded = Message::Put {
            request: 1,
            id: 0,
            revision: 0,
            model: Model::Box,
            asset: 0,
            pose: Pose::default(),
        }
        .encode()
        .unwrap();
        for end in PREFIX.len()..encoded.len() {
            assert!(Message::decode(&encoded[..end]).is_err());
        }
        for text in [
            format!("{PREFIX}AAAA"),
            format!("{PREFIX}AQ==AAAA"),
            format!("{PREFIX}AQ=A"),
            format!("{PREFIX}AR=="),
            format!("{PREFIX}{}", "A".repeat(1024)),
        ] {
            assert!(Message::decode(&text).is_err());
        }
    }
    #[test]
    fn malformed_create_revision_is_rejected() {
        let request = Message::Put {
            request: 1,
            id: 0,
            revision: 2,
            model: Model::Box,
            asset: 0,
            pose: Pose::default(),
        };
        assert!(Message::decode(&request.encode().unwrap()).is_err());
    }
}
